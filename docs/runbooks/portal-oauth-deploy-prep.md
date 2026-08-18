# Runbook — Deploy prep for portal sign-in (Discord OAuth)

**Audience:** whoever provisions the onboarding portal's Discord sign-in, and
whoever later moves the portal to a custom domain. No prior context assumed.

**Owner of the Discord application and of the redirect URI: Adam Kot (`akot`)**,
per [ADR 0010 §6](../../lore/2-adrs/0010_discord-account-model-and-abuse-barrier.md).
This is a decision recorded in the ADR, not a convention — the registration lives
in one person's Discord Developer Portal account and re-pointing it is a manual
act nobody else can perform.

Introduced by task 0186 (sign-in, identity only). Task 0189 amends **step 2**
when it adds the `guilds.members.read` scope.

## What this covers

Two things, and they are the same artefact seen from two sides:

1. **Registering the Discord application** and creating the one Secrets Manager
   secret the api-handler reads.
2. **What has to change, and in what order, at the custom-domain cutover**
   ([0195] / [0126]) — because getting that order wrong breaks sign-in silently,
   which is precisely the failure this runbook exists to prevent.

## What CDK does and does not do

|                                                             | owned by                                                 |
| ----------------------------------------------------------- | -------------------------------------------------------- |
| The secret's **name** (`prices/{env}/portal-discord-oauth`) | CDK — `infra/src/lib/mtls.ts`, `portalOauthSecretName()` |
| The IAM grant to read it (api-handler role only)            | CDK — `compute-stack.ts`                                 |
| `PORTAL_OAUTH_SECRET_NAME` on the Lambda                    | CDK — `compute-stack.ts`                                 |
| The secret's **value**                                      | **you**, by hand, out of band                            |
| The Discord application itself                              | **you**, in the Developer Portal                         |

This split is the same one `SecretsStack` states for the mTLS material, and here
it has a second reason. The secret's `redirect_uri` field is re-pointed by hand
at the domain cutover; a CloudFormation-managed value would be **restored to the
committed one by the next `cdk deploy`**, un-pointing sign-in some hours or weeks
after the cutover appeared to succeed. Never write `new secretsmanager.Secret`
for this.

---

## Prerequisites

- `export AWS_PROFILE=soroban-explorer`, `export AWS_REGION=eu-central-1`.
- The portal's public hostname. Until [0195] lands, that is the CloudFront
  distribution domain, published by CDK at
  `/prices/production/portal-distribution-domain`:

  ```bash
  aws ssm get-parameter \
      --name /prices/production/portal-distribution-domain \
      --query Parameter.Value --output text
  ```

---

## 1. Register the Discord application

In the [Discord Developer Portal](https://discord.com/developers/applications):

1. **New Application** → name it for the service, not for the environment
   (there is one environment and it is production).
2. **OAuth2 → Redirects → Add Redirect**, and enter the callback URL exactly:

   ```
   https://<portal-host>/api-tokens/api/auth/callback
   ```

   ⚠️ **Assume the match is character-exact.** Discord's documentation states
   that a redirect URI must be registered but does **not** state that matching is
   exact; task 0156 recorded that gap rather than resolving it. Treat it as exact
   — trailing slash, scheme, port and case all included — and verify once by
   completing a sign-in (step 5). A mismatch is refused by Discord on Discord's
   own error page, so **nothing appears in our logs** and there is nothing to
   debug on our side.

   The path is fixed by `CALLBACK_PATH` in
   `packages/prices-api/src/portal/auth/mod.rs`, and the loader refuses a
   `redirect_uri` that does not end in it — so a typo in the path fails at cold
   start with a clear message, while a typo in the _host_ can only be caught by
   completing the flow.

3. **Scopes are part of the registration, not just of the authorize URL.**
   Task 0186 requests exactly **`identify`**. Do not add `guilds` or `email` —
   ADR 0010 rejects both, the first for returning every server a user belongs to
   and the second for collecting data we have decided not to hold. The handler
   **verifies the granted scope on the token response** and refuses anything
   wider, so a registration that drifts fails closed rather than quietly
   collecting more.

   > Task 0189 adds `guilds.members.read` here **and** in `discord::SCOPE`. Both,
   > or the flow refuses its own grant.

4. Copy the **Client ID** and reset/copy the **Client Secret**.

## 2. Create the secret

Generate a session signing key and write all four fields as one JSON object:

```bash
SIGNING_KEY=$(openssl rand -hex 32)

aws secretsmanager create-secret \
    --name prices/production/portal-discord-oauth \
    --description "Onboarding portal Discord OAuth (task 0186) — operator-owned" \
    --secret-string "$(jq -n \
        --arg id "$CLIENT_ID" \
        --arg secret "$CLIENT_SECRET" \
        --arg redirect "https://<portal-host>/api-tokens/api/auth/callback" \
        --arg key "$SIGNING_KEY" \
        '{client_id:$id, client_secret:$secret, redirect_uri:$redirect, session_signing_key:$key}')"
```

| field                 | notes                                                                 |
| --------------------- | --------------------------------------------------------------------- |
| `client_id`           | from step 1. Not secret; here so the registration is one artefact     |
| `client_secret`       | **never** an env var, a config file, or a commit                      |
| `redirect_uri`        | must match the registration **exactly**; changed at the cutover       |
| `session_signing_key` | ≥32 bytes, rejected below that. Signs the session and `state` cookies |

**Rotating `session_signing_key` signs everyone out.** Every live session cookie
verifies under the old key and stops verifying under the new one, so all holders
read as signed out and click the button again. That is the whole cost — no key is
lost, because the API keys themselves live in API Gateway and not in the cookie.

**Rotating `client_secret`** invalidates nothing on our side: it is used only
during the token exchange, so a rotation takes effect on the next sign-in and
breaks no session in progress beyond the ten-minute pending window.

Note that `create-secret` fails if the name exists — that is deliberate, and it
is why CDK does not create it.

## 3. Confirm the wiring

```bash
# The name CDK put on the Lambda, and the name you just created, must match.
aws lambda get-function-configuration \
    --function-name prices-production-api-handler \
    --query 'Environment.Variables.PORTAL_OAUTH_SECRET_NAME' --output text

# Also published for tooling to read rather than hard-code:
aws ssm get-parameter \
    --name /prices/production/portal-oauth-secret-name \
    --query Parameter.Value --output text
```

Both must print `prices/production/portal-discord-oauth`.

`PORTAL_ENABLED` is still `false` at this point and the routes still answer an
empty `404`. That is correct: **the api-handler does not read this secret while
the portal is closed** (see `AppConfig::load_portal_oauth`), so creating it does
not change any behaviour and forgetting to create it before opening the portal
fails the _next_ cold start rather than silently serving a broken sign-in.

## 4. Verify locally before opening production

Do the round-trip on a laptop before the flag ever moves in production. The full
procedure — the file, the two commands, what to expect and what each failure
means — is in
[`packages/prices-api/README.md`](../../packages/prices-api/README.md#running-portal-sign-in-locally-task-0186),
which is the one place it is maintained.

Two things that are **this runbook's**, because they are changes to the
registration rather than to a developer's machine:

1. **Add a second redirect URI** to the same Discord application, pointing at
   `http://localhost:4200/api-tokens/api/auth/callback`. Discord accepts several
   redirects per application, so this sits alongside the production one and
   neither disturbs the other.
2. **Decide what happens to it afterwards.** Remove it once the flow is
   verified, or leave it and accept that anyone holding that `client_secret` can
   complete a real sign-in from their own machine, against the production
   application.

The local secret file carries the **localhost** `redirect_uri`; the loader
accepts it and Discord matches it against the second registration. It is a file
rather than an environment variable on purpose — there is no code path anywhere
that reads a client secret out of the environment, and adding one "just for
local" is a path production can be misconfigured onto.

## 5. Opening the portal

Flipping `PORTAL_ENABLED` to `'true'` in `compute-stack.ts` is **task 0194's**,
after task 0189's eligibility gate passes. Do not do it as a side effect of
finishing a slice. Before it happens, both must be true:

- The secret above exists and parses (a malformed one now **fails Lambda init**,
  which takes out `/v1` as well — the read is skipped entirely while the portal
  is closed, so this is a failure mode that only appears at the moment of
  opening).
- The gateway maps the portal prefix as a greedy `{proxy+}`. The four sign-in
  routes are at **depth 3** (`auth/login`), and the deployed mapping at the time
  of writing is the intermediate `{proxy}` + `{proxy}/{sub}` pair, which answers
  `403 Missing Authentication Token` at that depth. Task 0205 ships the committed
  shape.

---

## 6. The custom-domain cutover — ordering

When [0195] / [0126] put the portal on a custom domain, the portal is reachable
at **two** hostnames for a while: the CloudFront distribution domain and the new
one. The redirect URI is registered per-URL, and our stored `redirect_uri` is a
single string, so an ordering mistake breaks sign-in for everyone at the old
host, at the new host, or both.

**Do it in this order.** Each step is safe to sit in indefinitely.

1. **Add** the new redirect URI to the Discord application, _keeping the old
   one_. Two registered redirects, both valid. Nothing changes yet: our stored
   `redirect_uri` still names the old host, and Discord matches whichever we
   send.
2. **Verify the new hostname serves the portal** and that
   `https://<new-host>/api-tokens/api/config` answers — i.e. the distribution and
   its behaviours are live on the new name. Sign-in still runs through the old
   host at this point.
3. **Update the secret's `redirect_uri`** to the new host:

   ```bash
   aws secretsmanager put-secret-value \
       --secret-id prices/production/portal-discord-oauth \
       --secret-string "$(jq -n --arg id "$CLIENT_ID" --arg secret "$CLIENT_SECRET" \
            --arg redirect "https://<new-host>/api-tokens/api/auth/callback" \
            --arg key "$SIGNING_KEY" \
            '{client_id:$id, client_secret:$secret, redirect_uri:$redirect, session_signing_key:$key}')"
   ```

   ⚠️ `put-secret-value` replaces the **whole** JSON. Pass all four fields, and
   keep `session_signing_key` **unchanged** unless you intend to sign everyone
   out.

   ⚠️ **This does not take effect immediately.** The value is read once per
   execution environment and cached by the Parameters & Secrets extension
   (`PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED=true`), so warm Lambda containers
   keep sending the _old_ redirect URI until they are recycled. Both are
   registered at this point (step 1), so both work — which is exactly why step 1
   comes first. Force the change through by deploying the api-handler, or wait
   out the recycle.

4. **Verify a complete sign-in at the new host**, in a browser, signed out.
5. **Only then** remove the old redirect URI from the Discord application, and
   only after the old hostname stops being served.

**What breaks if the order is inverted:** removing the old redirect first (or
updating `redirect_uri` before registering the new one) makes every `/auth/login`
redirect carry a URI Discord does not recognise. Discord refuses on its own error
page, before the visitor ever comes back to us — so there is no request in our
access logs, no entry in CloudWatch, and no alarm. The first signal is a person
saying sign-in is broken.

**A session survives the cutover only if the host does.** The cookie is host-only
(no `Domain` attribute) and scoped to `Path=/api-tokens/`, so visitors signed in
at the old hostname are signed out at the new one and sign in again. That is one
click and no consent screen — Discord does not re-prompt for scopes already
granted — and it is preferable to a `Domain`-scoped cookie shared with every
other host under the registrable domain.

---

## Related

- [ADR 0010 — Discord identity is the account](../../lore/2-adrs/0010_discord-account-model-and-abuse-barrier.md)
- `packages/prices-api/src/portal/auth/secret.rs` — the loader, and why all four
  fields live in one secret
- `infra/src/lib/mtls.ts` — `portalOauthSecretName()`
- `infra/README.md` § "Uploading the real mTLS PEMs" — the same operator-owned
  pattern, for the ClickHouse material
