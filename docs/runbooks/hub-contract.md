# Runbook — The API hub's dashboard contract (task 0348)

**Audience:** whoever deploys or operates Prices' half of the API hub at
`https://sorobanscan.com/api`, changes the secret its tokens are signed with, or
puts Cloudflare rules in front of `prices-api.sorobanscan.com`. No prior context
assumed.

**Owner of the secret: the operator** (whoever administers Prices' AWS
account), who creates it and changes it; the hub's backend and Prices both read
it.

The hub (`rumblefishdev/sorobanscan-api-portal`) signs a member in with Discord
once, for every API, and shows a tab per product. Its backend (the BFF) calls
Prices server-to-server, under `/api/hub/v1/`, with a 60-second token signed
with a secret both sides hold. Prices answers from its own key, cap and usage
logic, the same as the portal's routes. Design: that repo's
`docs/superpowers/specs/2026-11-17-shared-sign-in-design.md`, "The dashboard
contract" and decisions 11–16. The portal's own routes (`/api/auth/*`,
`/api/key`, `/api/usage`, `/api/config`) are unchanged and keep working until
the hub takes over (the spec's cutover, steps 3 and 4).

## What CDK does and does not do

|                                                                         | owned by                                                                         |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| The secret's **name** (`sorobanscan-hub/production/prices-token`)       | the hub's spec; CDK repeats it — `infra/src/lib/mtls.ts`, `hubTokenSecretName()` |
| The IAM grant to read it (`PortalReadHubTokenSecret`, api-handler only) | CDK — `compute-stack.ts`                                                         |
| `PORTAL_HUB_SECRET_NAME` on the api-handler                             | CDK — `compute-stack.ts`                                                         |
| The secret's **value**                                                  | **the operator**, by hand, out of band                                           |
| The hub's reader of the same secret                                     | the hub's backend, through its own AWS role                                      |

CI pins the name and the grant (`tools/scripts/verify-openapi-routes.mjs`,
check 7b): the hub reads the same name, so a drift here would close the
contract on the next deploy with nothing failing in this repo.

## 1. The secret

One Secrets Manager secret in `eu-central-1`:

```json
{ "current": "<base64 of 32 bytes>", "previous": "<base64 of 32 bytes>" }
```

- `current` is required. **The hub signs with `current` only.**
- `previous` is optional. **Prices accepts a token under `current` or
  `previous`**; the hub ignores `previous`.
- Each value is exactly what `openssl rand -base64 32` prints: standard base64
  with its padding, 32 bytes. Both sides refuse anything else (Prices:
  `portal/hub/secret.rs`; the hub: `bff/src/product-secret.ts`).

**Creating it.** Once, by the operator, with a value no one sees: the command is in the
hub's `README.md` (search for `prices-token`). It is not repeated here, so the
two can't drift. Never `new secretsmanager.Secret` in CDK: the value has a
reader this repo does not deploy, and a CloudFormation-managed value would be
put back by our next deploy.

**Who can mint a token.** Anyone who can read this secret can sign a token for
any Discord account, `issue` included. That is the people who administer
Prices' AWS account already (spec decision 12). The api-handler's role reads
this one secret and nothing else under `sorobanscan-hub/`.

**How each side holds it.**

|                     | reads it                                                                                      | keeps it                                                                                |
| ------------------- | --------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Prices' api-handler | on the first hub call in an execution environment, through the Parameters & Secrets extension | **for the execution environment's life**. A failed read is retried after a 2 s cooldown |
| The hub's backend   | on its first call to Prices, through the AWS SDK                                              | **5 minutes**, then reads again. A failed read is retried after 10 s                    |

### Changing the value without a gap

Prices keeps the value it read until its execution environment is recycled; the
hub picks up a new `current` within 5 minutes. So a new key must be accepted by
every Prices environment **before** the hub can sign with it. Four steps, each
safe to stop after. Each step builds the new JSON from the secret as stored,
never from shell variables, so a step can be run in a new shell. A step run out
of turn refuses rather than undoing an earlier one: step 1 refuses while the
secret holds a `previous` (a rotation step 4 has not finished), and step 3
refuses unless the stored secret is still the version step 1 wrote, so run
twice it does not swap the keys back. No step puts a key on a command line:
the new JSON reaches `aws` on its standard input.

Set these once per shell. They hold no key. A step run without them stops:

```bash
SECRET_ID=sorobanscan-hub/production/prices-token
# true only for {"current": K} or {"current": K, "previous": K}, K being
# standard padded base64 of 32 bytes (what `openssl rand -base64 32` prints)
KEYS_OK='def ok: type == "string" and test("^[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]=$");
  (keys - ["current", "previous"] == []) and (.current | ok)
  and (if has("previous") then .previous | ok else true end)'
```

Every step below does the same three things in a subshell: reads the stored
secret, builds the new JSON from it, and checks it with `KEYS_OK` before the
put. If a check fails, the step writes nothing and stops, with `jq`'s
`refused…` or a `stopped: …` line (`jq -e` fails, `pipefail` carries that on).
Nothing is put, so the secret stays as it was.

1. **Add the new key as `previous`.** The hub keeps signing with the old one.
   Refused while the secret already holds a `previous`: within 10 minutes of
   another rotation's step 3 that is the key the hub may still sign with
   (step 4).

   ```bash
   (
     set -euo pipefail
     NEW=$(openssl rand -base64 32)
     JSON=$(aws secretsmanager get-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --query SecretString --output text \
       | NEW="$NEW" jq -ce 'if has("previous")
           then error("refused: the secret holds a previous; finish the last rotation (step 4) first")
           else {current, previous: env.NEW} end' \
       | jq -ce "if ($KEYS_OK) then . else error(\"refused\") end")
     printf %s "$JSON" | aws secretsmanager put-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --secret-string file:///dev/stdin \
         --query VersionId --output text
   )
   ```

   It prints the version it wrote. Keep it: step 3 needs it, as
   `STEP1_VERSION`.

2. **Recycle Prices' api-handler**, so every environment reads the pair:

   ```bash
   aws lambda update-function-configuration --region eu-central-1 \
       --function-name prices-production-api-handler \
       --description "recycle $(date -u +%FT%TZ)"
   ```

   The description is only a label: the configuration change is what makes
   the calls that follow start new environments. A deploy of the api-handler
   does the same. Wait for the update to finish before the next step:

   ```bash
   aws lambda wait function-updated-v2 --region eu-central-1 \
       --function-name prices-production-api-handler
   ```

   Then see that the new environments hold the pair. Each loads the secret on
   its first hub call, so make some (each answers `401 invalid_token` and logs
   the expected WARN `hub call refused: its token did not verify`):

   ```bash
   for i in 1 2 3; do
     curl -s -o /dev/null -w '%{http_code}\n' -H 'Authorization: Bearer v1.probe.probe' \
         https://prices-api.sorobanscan.com/api/hub/v1/account
   done
   ```

   and in Logs Insights on `/aws/lambda/prices-production-api-handler`, from
   the recycle on:

   ```
   fields @timestamp, fields.keys
   | filter fields.message = "the hub contract's secret loaded"
   | sort @timestamp desc
   ```

   At least one line, and every one with `keys` 2. A `1` means that
   environment read the secret without step 1's key: do not go on; run step 2
   again. No line: no environment has loaded it yet; probe again.

3. **Swap them: the new key becomes `current`.** Within 5 minutes the hub signs
   with it. Every Prices environment accepts it: the ones recycled in step 2
   hold it as `previous`, newer ones as `current`. Refused unless the stored
   secret is still the version step 1 wrote: after this step has run once, or
   after anything else was put since step 1, it stops and puts nothing.

   ```bash
   (
     set -euo pipefail
     : "${STEP1_VERSION:?set STEP1_VERSION to the version step 1 printed}"
     STORED=$(aws secretsmanager get-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --query '{v: VersionId, s: SecretString}' --output json)
     [ "$(printf %s "$STORED" | jq -r .v)" = "$STEP1_VERSION" ] || {
       echo "stopped: the stored secret is not the version step 1 wrote; nothing was put" >&2
       exit 1
     }
     JSON=$(printf %s "$STORED" | jq -r .s \
       | jq -ce '{current: .previous, previous: .current}' \
       | jq -ce "if ($KEYS_OK) then . else error(\"refused\") end")
     printf %s "$JSON" | aws secretsmanager put-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --secret-string file:///dev/stdin \
         --query VersionId --output text
   )
   ```

4. **After at least 10 minutes, drop the old key**, and recycle again (step 2)
   so no Prices environment still accepts it (and wait for it with the
   `aws lambda wait function-updated-v2` command above). The hub's cache is 5 minutes and
   a read that started just before step 3 can land after it, so 5 leaves no
   margin. While the hub still signs with the old key, dropping it in Prices
   refuses every hub call.

   ```bash
   (
     set -euo pipefail
     JSON=$(aws secretsmanager get-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --query SecretString --output text \
       | jq -ce '{current}' \
       | jq -ce "if ($KEYS_OK) then . else error(\"refused\") end")
     printf %s "$JSON" | aws secretsmanager put-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --secret-string file:///dev/stdin \
         --query VersionId --output text
   )
   ```

`put-secret-value` replaces the whole JSON. Never `echo` a key. If a step
fails, the hub's Prices tab shows Prices' refusal as a `502` (the hub turns
Prices' `401`/`403` into one, since it is a fault between the two backends),
and Prices logs `hub call refused: its token did not verify`. Put back the
previous version and recycle (step 2). This moves `AWSCURRENT` back to the
version that holds `AWSPREVIOUS`, and stops without a change if either is
missing:

```bash
(
  set -euo pipefail
  : "${SECRET_ID:?}"
  stage() {
    aws secretsmanager list-secret-version-ids --region eu-central-1 \
        --secret-id "$SECRET_ID" \
        --query "Versions[?contains(VersionStages, '$1')].VersionId | [0]" --output text
  }
  PREV_ID=$(stage AWSPREVIOUS)
  BAD_ID=$(stage AWSCURRENT)
  for id in "$PREV_ID" "$BAD_ID"; do
    case "$id" in "" | None) echo "stopped: no AWSPREVIOUS or no AWSCURRENT version; nothing was moved" >&2; exit 1 ;; esac
  done
  aws secretsmanager update-secret-version-stage --region eu-central-1 \
      --secret-id "$SECRET_ID" --version-stage AWSCURRENT \
      --move-to-version-id "$PREV_ID" --remove-from-version-id "$BAD_ID"
  echo "AWSCURRENT is $PREV_ID again; recycle (step 2)"
)
```

### If the secret leaks

Whoever holds it can sign an `issue`, `reveal` or `revoke` token for any
account until Prices stops accepting it. Do not rotate in four steps: replace
it at once, and accept that the hub's calls are refused until it reads the new
value.

1. Write a new key alone, without `previous`:

   ```bash
   (
     set -euo pipefail
     JSON=$(NEW=$(openssl rand -base64 32) jq -nce '{current: env.NEW}' \
       | jq -ce "if ($KEYS_OK) then . else error(\"refused\") end")
     printf %s "$JSON" | aws secretsmanager put-secret-value --region eu-central-1 \
         --secret-id "$SECRET_ID" --secret-string file:///dev/stdin \
         --query VersionId --output text
   )
   ```

2. Recycle at once (step 2, both commands). From then on Prices accepts only
   the new key.
3. Wait. The hub reads the new `current` within 5 minutes; until then every
   hub call is refused (the hub's Prices tab shows a `502`, Prices logs
   `hub call refused: its token did not verify`).
4. Find what the leaked secret signed. In Logs Insights on
   `/aws/lambda/prices-production-api-handler`, from the leak (if it is not
   known, from the secret's last change) to the recycle:

   ```
   fields @timestamp, fields.jti, fields.act, fields.outcome, fields.key_id
   | filter fields.message in ["hub issued an API key", "hub revealed an API key", "hub revoked an API key"]
   | sort @timestamp asc
   ```

   The hub's backend logs the `jti` of each reveal, revoke and issue that
   succeeded (`prices_key_revealed`, `prices_key_revoked`,
   `prices_key_issue …`), beside the account it was for; a `jti` here that it
   never logged was not the hub's. Each `key_id` is an
   API Gateway key, and its name, `discord-<id>-key`, is its owner:

   ```bash
   aws apigateway get-api-key --region eu-central-1 --api-key "$KEY_ID" \
       --query name --output text
   ```

   A key a forged `reveal` returned is known to whoever forged the token:
   treat it as a leaked key.

**Skipping steps 1–2** (writing `{current: NEW, previous: OLD}` and recycling
afterwards) leaves a window, from the hub's next read to the recycle, in which
warm Prices environments refuse every hub call.

## 2. Deploying

`make -C infra deploy-production-compute` ships the code, the variable and the
grant together. There is no ordering against the secret:

- **No secret yet, or the grant missing:** the four routes answer
  `503 hub_unconfigured`, and the api-handler logs an ERROR,
  `the hub contract's secret did not load`, with the `error` and `variable`
  fields. The first hub call after a 2 s cooldown reads again, so creating
  the secret fixes it without a redeploy. The portal, `/api/config` and `/v1`
  are unaffected, and the portal-load-failed alarm does not fire for it.
- **The portal's own sources** (the OAuth secret, the plan and API ids, the
  eligibility parameters) are needed by the contract too: a failed load
  answers `503 keys_unconfigured` and pages as
  `prices-production-api-handler-portal-load-failed`, as for the portal.

### Checking it after the deploy

```bash
# The name CDK put on the Lambda — must print sorobanscan-hub/production/prices-token.
aws lambda get-function-configuration --region eu-central-1 \
    --function-name prices-production-api-handler \
    --query 'Environment.Variables.PORTAL_HUB_SECRET_NAME' --output text

# No token at all: 401 invalid_token, before anything is read.
curl -s -w ' %{http_code}\n' https://prices-api.sorobanscan.com/api/hub/v1/account

# A token that can't verify: 401 invalid_token means the secret loaded in the
# environment that answered; 503 hub_unconfigured means it did not.
curl -s -w ' %{http_code}\n' -H 'Authorization: Bearer v1.probe.probe' \
    https://prices-api.sorobanscan.com/api/hub/v1/account
```

The second probe logs one WARN `hub call refused: its token did not verify`;
that is expected. A real call needs a token only the hub (or someone holding
the secret) can make: try the hub's Prices tab (the spec's cutover, step 2).

**Testing touches real keys** (spec decision 14). Issuing and revoking in the
Prices tab act on the tester's real Prices key, and a revoke holds back a new
key until the next calendar month (UTC). Try revoke with a second Discord
account, or last.

## 3. The contract

Four routes, mounted inside the `PORTAL_ENABLED` gate (closed portal: the
gate's empty `404`), outside the portal's CORS (no browser calls them; no
`Access-Control-*` header on any answer), and never in the OpenAPI document.
They reach the Lambda through the gateway's `/api/{proxy+}`, and share the
portal's stage throttle (`PORTAL_THROTTLE` in `api-gateway-stack.ts`).

| Route                         | `act`    | Answers                                                                                                                                  |
| ----------------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /api/hub/v1/account`     | `read`   | `200` with the key's state and its use, below                                                                                            |
| `POST /api/hub/v1/key/reveal` | `reveal` | `200 {key_id, name, value, created_at, last_updated_at}`; `404 no_key`; `404 key_revoked` with `details: {next_eligible_at, revoked_at}` |
| `POST /api/hub/v1/key/revoke` | `revoke` | `200 {revoked: true, next_eligible_at, revoked_at, partial?}`; `404 no_key`                                                              |
| `POST /api/hub/v1/key/issue`  | `issue`  | always `200 {outcome, next_eligible_at?}`, below                                                                                         |

Every answer is `Cache-Control: no-store`. An error is the API's envelope,
`{"code", "message"}` (with `details` only for `key_revoked`), and never carries
a token, a stack or an AWS message.

### The token

`Authorization: Bearer v1.<payload>.<mac>` (spec decision 15: base64url of the
claims' JSON, HMAC-SHA256 under the context `sorobanscan-hub/prices-token/v1`).
Accepted when the MAC verifies under `current` or `previous`, `iss` is
`https://sorobanscan.com/api/hub`, `aud` is `prices`, `sub` is
`discord:<snowflake>`, `exp > now − 5 s`, `iat ≤ now + 5 s`, `exp − iat ≤ 60 s`,
and `act` is the route's. One action per token: a `read` token cannot revoke, a
`reveal` token cannot issue. **A token is not single-use**: `jti` is logged,
never checked, so a captured token works again, for its own action, until
`exp` + 5 s (at most 65 s after `iat`), and a captured `reveal` token returns
the key each time. Accepted (ADR 0010 §8a): the hub's backend sends it only to
Prices, over HTTPS, and a cache of seen `jti`s would hold per execution
environment only.

### Errors on every route

| Status | `code`               | Meaning                                                                                                                                                                                                                                                                                                                          |
| ------ | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `401`  | `invalid_token`      | no bearer, a malformed one, not `v1.`, a MAC under neither key, a claim missing or of the wrong type, another `iss` or `aud`, expired, issued more than 5 s ahead, too long-lived, a `sub` that is not a snowflake, an `act` that is none of the four. One answer for all, so it is not an oracle. No control-plane call is made |
| `403`  | `action_not_allowed` | a good token for another `act`. No control-plane call is made                                                                                                                                                                                                                                                                    |
| `503`  | `hub_unconfigured`   | no secret: not configured, or not readable now (see §2)                                                                                                                                                                                                                                                                          |
| `503`  | `keys_unconfigured`  | the portal's own sources did not load                                                                                                                                                                                                                                                                                            |
| `502`  | `key_unavailable`    | the control plane failed (`account`, `reveal`, `revoke`)                                                                                                                                                                                                                                                                         |
| `503`  | `key_unavailable`    | the control plane ran out of time (same three)                                                                                                                                                                                                                                                                                   |

### `GET account`

```json
{
  "key": { "state": "none" | "active" | "revoked", "key_id"?, "created_at"?,
           "last_updated_at"?, "revoked_at"?, "next_eligible_at"?, "can_issue": bool },
  "details": { "usage": <the /api/usage body> | null, "free_rate_limit_per_second": n | null }
}
```

- `none`: no key. `can_issue: true`.
- `active`: the live key, with its id and dates. `can_issue: false`, since an
  account has one live key.
- `revoked` with `next_eligible_at` and `can_issue: false`: revoked this
  period; the month cap holds until `next_eligible_at`.
- **`revoked` with `can_issue: true`** (no `next_eligible_at`): revoked in an
  earlier period, so a new key can be issued now. The tab offers "Get API Key".
  `usage` is `null`, as `/api/usage` and the reveal say "no key" for it.
- **`200` with `usage: null` beside `active`** means the usage read failed or
  ran out of time with nothing cached that agrees with the key just listed.
  The key's state is still right; the tab shows no figure, as the portal's
  dashboard does while `/usage` fails, and Prices logs why. Only the key
  listing failing is an error (`502`/`503 key_unavailable`).
- `free_rate_limit_per_second` is the free plan's per-key limit, requests per
  second, as `/api/config` gives it: the api-handler's `PORTAL_RATE_LIMIT`,
  which CDK sets from the free usage plan's own setting. `null` when the
  deployment does not set it. It is the free plan's figure whatever plan the
  key is on; the key's own plan and its limits are in `usage`.
- The key's value is never read here; `reveal` reads it.
- The usage is cached for 60 s per account, shared with the portal's
  `/api/usage` in the same execution environment. A hub issue or revoke drops
  the account's entry.

### `POST key/issue`

| `outcome`   | Meaning                                                                                                                                                                                                           |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `issued`    | a key was created, tagged `IssuedBy=hub-contract` (and `ManagedBy=prices-portal`, as every portal key), on the plan Prices' rules give it: the previous key's plan after a revoke in an earlier period, else free |
| `has_key`   | the account already has its one live key; nothing was created                                                                                                                                                     |
| `capped`    | revoked this period: no new key before `next_eligible_at`                                                                                                                                                         |
| `too_young` | the Discord account is younger than Prices' minimum (`/prices/production/min-account-age-minutes`); `next_eligible_at` is when it clears it. Checked before any control-plane call                                |
| `failed`    | our side: the control plane, a deadline, or the minimum age could not be read. Try again                                                                                                                          |

Membership and Membership Screening are the hub's to prove, before it signs an
`issue` token; Prices re-checks only the account's age, as a backstop (ADR
0010 §8a).

## 4. Cloudflare in front of `prices-api.sorobanscan.com`

The hub's backend calls `https://prices-api.sorobanscan.com/api/hub/v1`
(`PRICES_HUB_API` in the hub, by default), through Cloudflare: the record is
proxied (task 0340, Step 2). So does a developer's local hub, which works on
production's Prices (spec decision 11). Today the host has no Cloudflare rules
of its own (0340, "Decided"), so nothing stops these calls.

Task 0340 plans rules for this host: managed WAF and bot rules after watching
the traffic, and an `X-Edge-Secret` origin lock with its Step 5. Spec decision
16: **those rules must let the hub's calls through, agreed with 0340's owner
before they land.** What that means:

- **No challenge on `/api/hub/*`.** The hub's backend is a Node Lambda and
  can't answer a JavaScript or managed challenge, nor Bot Fight Mode. A
  challenge page comes back as non-JSON, which the hub shows as
  `prices_unavailable` (502). Skip bot and challenge rules for the path; the
  token authenticates every call. **Bot Fight Mode must stay off for
  `sorobanscan.com`**: on the Free plan it is zone-wide and a WAF skip rule
  can't bypass it; only Super Bot Fight Mode (Pro and above) can be skipped
  per path.
- **A per-IP rate limit counts all users as one.** Every user's calls come
  from the hub's few AWS egress addresses. Exempt `/api/hub/*`, or set the
  limit above the hub's peak. The gateway's own stage throttle still applies.
- **The origin lock must not need the hub to know it.** If `X-Edge-Secret` is
  a header Cloudflare adds at its edge, the hub's calls carry it like any other
  request through the proxy, and nothing changes on the hub's side. Once it is
  enforced, the hub must call the Cloudflare host, never `execute-api` or the
  old `prices-api.sorobanscan.rumblefish.dev`.
- **The `Authorization` header must reach the origin unchanged.** It carries
  the token.
- After any rule lands, check the WAF's events for `/api/hub/` and the hub's
  Prices tab.

## 5. What to watch

There is no alarm of the contract's own. The api-handler logs JSON lines
(`fields.message`, `fields.jti`, `fields.act`, `fields.outcome`,
`fields.key_id`); never the token, a key's value, the Discord id or the
username.

| Line                                                                                                         | Level | What it means                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------ | ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `the hub contract's secret did not load…`                                                                    | ERROR | the secret is missing, malformed or not readable: the contract answers `503 hub_unconfigured` until it is fixed. **Act**                                                                                                                                                                                  |
| `the hub contract has no secret (PORTAL_HUB_SECRET_NAME is unset)…`                                          | WARN  | the variable is missing on the Lambda. **Act**                                                                                                                                                                                                                                                            |
| `hub contract call answered as unconfigured without a read…`                                                 | WARN  | a hub call inside the 2 s after a failed read: `503 hub_unconfigured`, no read made. Follows the ERROR above                                                                                                                                                                                              |
| `hub call refused: its token did not verify`                                                                 | WARN  | **expected from strangers**: every scanner with a bad bearer on the public URL makes one. Only a burst together with the hub's Prices tab failing (its `502`) means the two sides disagree: the secret changed on one side only (§1), or a clock more than 5 s adrift. **Never alarm on this line alone** |
| `hub call refused: its token is for another action`                                                          | WARN  | the hub never sends one. A hub bug, or a captured token replayed. Look                                                                                                                                                                                                                                    |
| `hub call carried an id that names no key; refusing`, `hub call carried an id with no account age; refusing` | WARN  | a token that verified but whose `sub` this side cannot use: answered `401 invalid_token`. The hub never signs one (`token::verify` lets only a snowflake through), so a hub bug. Look                                                                                                                     |
| `hub account lookup failed`, `hub account lookup ran out of time`                                            | ERROR | the control plane, as for the portal's routes                                                                                                                                                                                                                                                             |
| `hub reveal failed`, `hub revoke failed`, `hub issue failed`                                                 | WARN  | the control plane, as for the portal's routes; the reveal's cause is on the ERROR line before it, `portal key lookup failed` or `portal key lookup ran out of time`, from the lookup the portal's reveal shares                                                                                           |
| `hub issue refused: the minimum account age could not be read`                                               | ERROR | the issue answered `failed`: the SSM parameter `/prices/production/min-account-age-minutes` did not read. **Act** if it repeats                                                                                                                                                                           |
| `hub account answered without its usage`                                                                     | WARN  | `usage: null` beside the key's state; see §3                                                                                                                                                                                                                                                              |
| `hub issued an API key`, `hub revoked an API key`, `hub revealed an API key`                                 | INFO  | the calls, with `jti`, `act`, `outcome` and `key_id`; the revoke line also says `partial=true` when a duplicate key could not be disabled and still answers (**Act**: see the portal's partial-revocation ERROR, which carries no `jti`)                                                                  |
| `the hub contract's secret loaded`                                                                           | INFO  | an execution environment read the secret, with `keys` (1, or 2 while a change is under way; §1, step 2). Never a key                                                                                                                                                                                      |
| `hub answered an account`                                                                                    | INFO  | with `jti`, `act`, the key's `state`, `key_id` (none for `none` or `revoked`) and `usage` (whether it came with one); no `outcome`                                                                                                                                                                        |
| `hub issue refused`, `hub reveal found…`, `hub revoke found no key`                                          | INFO  | `too_young`, `capped`, `key_revoked`, `no_key`: the account's state, not a fault                                                                                                                                                                                                                          |

A call without any bearer is logged at DEBUG only.

Logs Insights, on `/aws/lambda/prices-production-api-handler`, everything the
contract said but the strangers' refusals:

```
fields @timestamp, level, fields.message, fields.jti, fields.act, fields.outcome, fields.key_id
| filter fields.message like /^hub |hub contract/
| filter fields.message != "hub call refused: its token did not verify"
| sort @timestamp desc
```

A metric filter or alarm added later must leave out the line
`hub call refused: its token did not verify`, for the same reason.

## Related

- [ADR 0010 — Discord identity is the account](../../lore/2-adrs/0010_discord-account-model-and-abuse-barrier.md), §8a: keys the hub issues
- [`portal-oauth-deploy-prep.md`](portal-oauth-deploy-prep.md) — the portal's
  own sign-in, its secret and its eligibility parameters, which the contract
  reuses
- `packages/prices-api/src/portal/hub/` — the token, the secret, the four routes
- `infra/src/lib/mtls.ts` — `hubTokenSecretName()`
- Task 0340 — the Cloudflare host and its planned rules
