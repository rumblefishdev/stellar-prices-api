# Runbook — Deploy prep for portal sign-in (Discord OAuth)

**Audience:** whoever provisions the onboarding portal's Discord sign-in, and
whoever later moves the portal to a custom domain. No prior context assumed.

**Owner of the Discord application and of the redirect URI: Adam Kot (`akot`)**,
per [ADR 0010 §6](../../lore/2-adrs/0010_discord-account-model-and-abuse-barrier.md).
This is a decision recorded in the ADR, not a convention — the registration lives
in one person's Discord Developer Portal account and re-pointing it is a manual
act nobody else can perform.

Introduced by task 0186 (sign-in, identity only). Task 0189 amended **§1
step 3** (the `guilds.members.read` scope — a Developer Portal change the
operator must make, see there) and added **§2a** (the two operator-seeded
eligibility parameters). Task 0187 added **§7**, which is about key issuance
rather than sign-in and needs no manual provisioning — but it does add a
deploy-ordering precondition to §5.

## What this covers

Two things, and they are the same artefact seen from two sides:

1. **Registering the Discord application** and creating the one Secrets Manager
   secret the api-handler reads.
2. **What has to change, and in what order, when the backend's hostname
   moves** — done once already on 2026-08-31 ([0194]), and kept because getting
   that order wrong breaks sign-in silently, which is precisely the failure this
   runbook exists to prevent.

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
- The two hostnames, both from `infra/envs/production.json` since [0194]
  (2026-08-31): the **page** is served at `portalWebOrigin`
  (`https://sorobanscan.rumblefish.dev`, the block explorer's distribution)
  and the **backend** — where the callback lands and the session cookie is
  set — at `apiDomain.domainName` (`https://prices-api.sorobanscan.rumblefish.dev`).
  The redirect URI below names the API host. (Our own CloudFront distribution
  and its `/prices/production/portal-distribution-domain` parameter were
  retired by [0195]; nothing reads that parameter any more.)

---

## 1. Register the Discord application

In the [Discord Developer Portal](https://discord.com/developers/applications):

1. **New Application** → name it for the service, not for the environment
   (there is one environment and it is production).
2. **OAuth2 → Redirects → Add Redirect**, and enter the callback URL exactly:

   ```
   https://<portal-host>/api/auth/callback
   ```

   `<portal-host>` is **the host the backend answers on**, not the host the
   page is served from — since task 0194 those are two hostnames:
   `prices-api.sorobanscan.rumblefish.dev` runs the callback and sets the
   session cookie, `sorobanscan.rumblefish.dev/api/` is the page it sends the
   visitor back to (`PORTAL_WEB_ORIGIN`). Register the former. In production:

   ```
   https://prices-api.sorobanscan.rumblefish.dev/api/auth/callback
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
   The code requests exactly **`identify guilds.members.read`** (task 0189;
   `discord::SCOPE`) — `identify` for who the visitor is, `guilds.members.read`
   so the issue round-trip can ask whether they are a member of the Stellar
   guild with their own consented token. **Declare exactly that pair in the
   registration.** Do not add `guilds` or `email` — ADR 0010 rejects both, the
   first for returning every server a user belongs to and the second for
   collecting data we have decided not to hold. The handler **verifies the
   granted scope on the token response** — as a set, order-independent — and
   refuses anything wider _or narrower_, so a registration that drifts fails
   closed rather than quietly collecting more (or quietly turning every
   membership check into a refusal).

   **Drift is caught in two places, and they fire at different moments.** If the
   registration asks for LESS than the code requests, Discord refuses at the
   authorize step and never returns a code: the visitor lands on
   `/api/?signin=failed` and the handler logs
   `portal sign-in refused by Discord error=invalid_scope`. If the registration
   grants MORE, the flow completes as far as the token exchange and the
   granted-scope check refuses it there, logging the scopes. Either way it fails
   closed and says so — but only the first is visible before a code is ever
   issued, so `invalid_scope` in the logs points at this step and nothing else.

   > **Consent-screen note (0180 item 5, still to capture):** while making this
   > change, screenshot Discord's consent screen once with `identify` alone and
   > once with the pair, into the 0189 task's `sources/` — it is free while the
   > browser flow is open and awkward to reproduce later.

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
        --arg redirect "https://<portal-host>/api/auth/callback" \
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

## 2a. Seed the eligibility parameters (task 0189)

The eligibility gate reads two knobs from SSM **at runtime, per issuance** —
which Discord guild membership is checked against, and the minimum account age.
Seed both by hand, alongside the mTLS material and the secret above:

```bash
# The guild whose membership gates key issuance: Stellar Developers,
# 897514728459468821 — the official and ONLY Stellar guild this project refers
# to (Adam, 2026-09-02, task 0254). It is the same guild `STELLAR_DISCORD_INVITE`
# in `web/portal/src/landing/links.ts` sends a refused visitor to, and the two
# must agree or the refusal is a loop; `npm run discord:verify-guild -- --ssm`
# resolves the invite through Discord's API and compares it with this value.
# Seeded as the stellar_test guild (1536303837785362432) while building;
# re-pointing is a `put-parameter --overwrite`, no deploy.
aws ssm put-parameter \
    --name /prices/production/discord-guild-id \
    --type String \
    --value "897514728459468821"

# Minimum Discord account age, in whole minutes. 5 matches the Stellar guild's
# own verification_level: 2 ("registered on Discord for longer than 5 minutes")
# — ADR 0010 §3: we do not set a stricter bar than the server whose gate we
# depend on.
aws ssm put-parameter \
    --name /prices/production/min-account-age-minutes \
    --type String \
    --value "5"
```

|                                                                                                 | owned by                                |
| ----------------------------------------------------------------------------------------------- | --------------------------------------- |
| The parameter **names** (`PORTAL_GUILD_ID_PARAM`, `PORTAL_MIN_ACCOUNT_AGE_PARAM` on the Lambda) | CDK — `compute-stack.ts`                |
| The IAM grant to read them (api-handler role)                                                   | CDK — `PortalReadEligibilityParameters` |
| The parameter **values**                                                                        | **you**, by hand, out of band           |

The same ownership split as the secret, with the same reason sharpened: **CDK
must never create these parameters.** A CloudFormation-managed parameter is
restored to the committed value by the next `cdk deploy`, which after task 0179
would silently un-flip production back to the test guild. CI enforces the rule
(`verify-openapi-routes.mjs` check 7 refuses any `AWS::SSM::Parameter` with
either name in any synthesized template).

**Changing a value needs no redeploy.** The handler resolves both parameters
per issuance through the Parameters and Secrets extension; the extension's
in-process cache (~5 minutes) is the only delay between a `put-parameter
--overwrite` and the running Lambda honouring it.

**A bad seed fails loudly, at the right moment.** With the portal open, cold
start probes both values once: a missing parameter, an empty guild id, or a
threshold that is not a whole number of minutes fails Lambda init (`Init
Errors`) with the parameter named — not a per-visitor refusal. A guild id that
is not a bare snowflake (e.g. a guild _name_) additionally refuses at issuance
as "could not verify", with a warning in CloudWatch naming the value.

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
not change any behaviour, and forgetting to create it before opening the portal
closes the portal again at the _next_ cold start — `/config` answers
`enabled: false` and the api-handler logs `portal closed at cold start` naming
`PORTAL_OAUTH_SECRET_NAME` — rather than silently serving a broken sign-in
(`AppConfig::load_portal_or_close`). `/v1` is unaffected either way.

## 4. Verify locally before opening production

Do the round-trip on a laptop before the flag ever moves in production. The full
procedure — the file, the two commands, what to expect and what each failure
means — is in
[`packages/prices-api/README.md`](../../packages/prices-api/README.md#running-portal-sign-in-locally-task-0186),
which is the one place it is maintained.

Two things that are **this runbook's**, because they are changes to the
registration rather than to a developer's machine:

1. **Add a second redirect URI** to the same Discord application, pointing at
   `http://localhost:4200/api/auth/callback`. Discord accepts several
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
  shape. (Task 0187's `/key` is at depth 3 as well, so it is the same deploy.)
- **`ApiGatewayStack` has deployed at least once since task 0187 merged**, so
  that `/prices/production/pricing-api-free-plan-id` exists. See §7 — if it does
  not, opening the portal fails Lambda init and takes `/v1` down with it.
- **Both eligibility parameters are seeded** (§2a) and the Developer Portal
  registration carries the scope pair (§1 step 3). A missing parameter fails
  Lambda init exactly like a missing plan id; a registration still on
  `identify` alone refuses every issue round-trip at the authorize step.

### After the first live issue attempt: check for `pending_absent`

One behaviour the gate depends on is **undocumented and still unmeasured**
(task 0180 item 2): whether Discord's REST member object carries the `pending`
field at all. The code treats an absent `pending` as "could not verify" and
refuses — never as "cleared" — which is the safe direction, but it is safe in a
way that fails for **every** visitor if the field turns out never to be sent.
Nothing about that failure looks different from a Discord outage on the page:
each visitor sees "we could not verify your Discord membership just now".

The one signal that tells the two apart is in the log, so look for it after the
first real issue attempt:

```bash
aws logs filter-log-events \
    --log-group-name /aws/lambda/prices-production-api-handler \
    --filter-pattern pending_absent \
    --start-time "$(($(date +%s) - 3600))000"
```

Nothing found: the field is present and the gate is working as designed. One
line per attempt: the field is absent, **every member is being refused**, and
the fix is the one arm named in `eligibility::decide` — not a parameter, not a
redeploy of anything else. Record the result in task 0189's step 0 table with
its date either way; that is the measurement, taken from production instead of
from a scratch guild.

---

## 6. Moving the callback to another host — ordering

Done once already: on 2026-08-31 ([0194]) the callback moved from our
CloudFront distribution to `prices-api.sorobanscan.rumblefish.dev`, in the
order below, and the old distribution was retired afterwards ([0195]). Kept
as the procedure for the next move, whenever the API host changes again.

While a move is in progress the backend is reachable at **two** hostnames.
The redirect URI is registered per-URL, and our stored `redirect_uri` is a
single string, so an ordering mistake breaks sign-in for everyone at the old
host, at the new host, or both.

**Do it in this order.** Each step is safe to sit in indefinitely.

1. **Add** the new redirect URI to the Discord application, _keeping the old
   one_. Two registered redirects, both valid. Nothing changes yet: our stored
   `redirect_uri` still names the old host, and Discord matches whichever we
   send.
2. **Verify the new hostname serves the backend** — `https://<new-host>/api/config`
   answers JSON, with `Access-Control-Allow-Origin` naming the page's origin
   when asked with an `Origin` header. Sign-in still runs through the old
   host at this point.
3. **Update the secret's `redirect_uri`** to the new host:

   ```bash
   aws secretsmanager put-secret-value \
       --secret-id prices/production/portal-discord-oauth \
       --secret-string "$(jq -n --arg id "$CLIENT_ID" --arg secret "$CLIENT_SECRET" \
            --arg redirect "https://<new-host>/api/auth/callback" \
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
(no `Domain` attribute) and scoped to `Path=/api/`, so visitors signed in
at the old hostname are signed out at the new one and sign in again. That is one
click and no consent screen — Discord does not re-prompt for scopes already
granted — and it is preferable to a `Domain`-scoped cookie shared with every
other host under the registrable domain.

---

## 7. Self-service key issuance (task 0187) — nothing to provision, one ordering

Issuing keys needs **no manual step**: there is no secret, no registration, and
no console work. Everything it depends on is created by CDK. What it does have
is an ordering constraint and two operational facts worth knowing before the
flag moves.

### The ordering

`ApiGatewayStack` creates the `pricing-api-free` usage plan and publishes its id
to SSM. `ComputeStack` tells the api-handler to read that parameter by name. The
two stacks cannot reference each other — Compute is a dependency of Gateway, so
importing the plan would close a cycle — so the handshake is a parameter, and
the parameter has to exist before the handler asks for it:

```bash
aws ssm get-parameter \
    --name /prices/production/pricing-api-free-plan-id \
    --query Parameter.Value --output text

aws lambda get-function-configuration \
    --function-name prices-production-api-handler \
    --query 'Environment.Variables.PORTAL_FREE_PLAN_PARAM' --output text
```

The second must print the name the first was queried with. `npm run
openapi:verify-routes` asserts exactly this against the synthesized templates,
so a drift fails CI rather than a deploy — but the _existence_ of the deployed
parameter is not something CI can see.

**If the parameter is missing when `PORTAL_ENABLED` becomes `true`, the
api-handler closes the portal at cold start** — `/config` answers
`enabled: false` and the log carries `portal closed at cold start` naming
`PORTAL_FREE_PLAN_PARAM` — and `/v1` is unaffected. It used to fail init
instead, which took `/v1` down with it (one router serves every route group,
ADR 0008); task 0194's PR review is where that changed, and the reasoning is on
`AppConfig::load_portal_or_close`. The shape is still "found only at the moment
of opening", as with the OAuth secret in §3, and the alternative it avoids is
still a portal with a key button that answers `503` — a closed portal answers
before any button renders. What it costs: nothing pages on it (the api-handler
has no error alarm), so the `/config` probe after the deploy is the check, not
an optional confirmation.

While the portal is closed the handler reads neither, so nothing here changes
any behaviour until the flag moves.

### The IAM, and the three limits that come with it

CDK grants the api-handler role seven control-plane actions and nothing else:
`GET`/`POST` on `/apikeys`, `GET`/`PATCH`/`DELETE` on `/apikeys/*` (`PATCH`
is task 0191's revoke — see below), `POST` on
`/usageplans/{the free plan}/keys`, and — task 0188 — `GET` on
`/usageplans/{the free plan}/usage` (`GetUsage`, the dashboard's usage read).
The last two are declared in `api-gateway-stack.ts` rather than
`compute-stack.ts`, because that is the only stack that knows the plan id.

Two of the six cannot be scoped any further, and one can but is not yet. All
three are written out in full in `compute-stack.ts`; the short version:

- **`POST /apikeys` cannot be narrowed.** There is no ARN for "keys this
  function created", so permission to create a key is permission to create any
  key. Mitigated by the `ManagedBy=prices-portal` tag every created key carries.
- **`GET /apikeys` cannot be narrowed either**, and it is the one to know about:
  a collection has a single ARN, and `GetApiKeys` accepts `includeValues=true`,
  so the grant permits reading the value of **every API key in the account**,
  partner keys included. The handler always asks for `includeValues=false`, so
  this is what code execution in the Lambda would buy an attacker, not what the
  feature does.
- **`GET`/`PATCH`/`DELETE` on `/apikeys/*` are tag-scoped** to
  `ManagedBy=prices-portal` (task 0191). Without the condition the per-key
  `GET` was the real exposure — `GetApiKey(includeValue=true)` reads the value
  of any key in the account by id — and `PATCH` could rename a partner key
  into a portal name. The behaviour change to know: an exact-name key created
  **by hand** in the console is untagged and is no longer adopted; the routes
  answer `502` and the key is left alone. The listing (`/apikeys`) cannot
  take the condition and stays account-wide (values never requested).

Task 0194 audits all three.

### Replacing a key (task 0191) — one new grant, and a wait that is the point

"Replace my key" on the dashboard is a **revocation**: `UpdateApiKey
(enabled=false)` on the caller's key, session-authorized, no Discord
round-trip (a leaked key has to be killable while Discord is down). Nothing
is issued: the replacement is an ordinary issue round-trip, and the issue path
refuses it until the quota period of the revocation has rolled — the 1st of
the next month, 00:00 UTC, **our** rule. That wait is deliberate: quota is
scoped to `(usagePlanId, apiKeyId)`, so a replacement issued on the spot would
be a clean counter and "replace my key" would be the button people press on
the 20th of a heavy month.

**The seventh grant** is `apigateway:PATCH` on `/apikeys/*`
(`PortalReadDisableAndDeleteOwnApiKeys` in `compute-stack.ts`). Disable rather
than delete, because the disabled key _is_ the record of the revocation: its
`lastUpdatedDate` is what refuses the re-issue, and there is no registry
(task 0190) to hold that fact otherwise. The handler sends exactly one patch
operation, never re-enables a key, and `PATCH /apikeys/*` cannot reach a plan
or a stage. No `UpdateUsagePlan`, no `UpdateUsage`.

Operationally: a disabled key answers `403 Forbidden`, byte-identical to no
key at all, after the usual tens of seconds of propagation (0180 item 8); its
counter is preserved, so the dashboard keeps reporting it honestly. Once the
period rolls, the next issue deletes the disabled key and creates the new one.
A revoked user who asks for a key early lands on `?issue=capped` with the
date; nothing to operate.

### Verifying it, and the warning that comes with that

The round-trip is in
[`packages/prices-api/README.md`](../../packages/prices-api/README.md#running-self-service-key-issuance-locally-task-0187).

> **Every key a local run creates and deletes is a production key.** The flag
> lives in the Lambda; it protects nothing on a laptop holding production
> credentials, and the reconciler calls `DeleteApiKey`. Exercise it against keys
> you created and delete them afterwards.

## Related

- [ADR 0010 — Discord identity is the account](../../lore/2-adrs/0010_discord-account-model-and-abuse-barrier.md)
- `packages/prices-api/src/portal/auth/secret.rs` — the loader, and why all four
  fields live in one secret
- `infra/src/lib/mtls.ts` — `portalOauthSecretName()`
- `infra/README.md` § "Uploading the real mTLS PEMs" — the same operator-owned
  pattern, for the ClickHouse material
