# prices-api

Public Prices REST API — a **single axum Lambda** serving all route groups, on
ClickHouse over mTLS. Implements task **0040** (overview §4). Topology decision:
[ADR 0008](../../lore/2-adrs/0008_single-axum-lambda-for-prices-api.md) (single
Lambda, not five per-route — copied in skeleton from BE's `crates/api`).

## Layout

```
src/
├── lib.rs        # app(state) -> Router — the single source of routes (bin + tests share it)
├── main.rs       # Lambda entrypoint (feature `lambda`): cold-start CH client + lambda_http::run
├── config.rs     # AppConfig::from_env()
├── state.rs      # AppState { ch: Option<clickhouse::Client> } (Arc-backed, cloned per request)
├── common/       # framework primitives (errors, cache_control; cursor/conditional later)
└── ops/          # GET /health (non-versioned, auth-exempt)
tests/health.rs   # in-process oneshot smoke test
```

## Route ownership (target — §4)

| Path | Status |
|------|--------|
| `GET /health` | ✅ Phase 0 |
| `GET /v1/assets`, `GET /v1/assets/{id}` | Phase 3 / Phase 2 |
| `GET /v1/assets/{id}/price` | Phase 2 (load-test target) |
| `GET /v1/assets/{id}/ohlcv` | Phase 3 |
| `POST /v1/prices/batch` | Phase 2 |
| `GET /v1/oracles/{id}` | Phase 2 |
| `GET /v1/backfill/status` | Phase 2 |

Full plan: [`0040 G-implementation-plan.md`](../../lore/1-tasks/active/0040_FEATURE_prices-api-gateway-and-read-handlers/notes/G-implementation-plan.md).

## Build & test

```bash
# default build/test — no AWS/mTLS stack, router exercised via tower oneshot
cargo test -p prices-api

# Lambda artifact (ARM64) — the `lambda` feature is REQUIRED or the bin is skipped
cargo lambda build -p prices-api --release --arm64 --features lambda
```

## Running portal sign-in locally (task 0186)

The whole round-trip — `/auth/login` → Discord → `/auth/callback` → session →
`/auth/me` → `/auth/logout` — runs on a laptop. Two processes and one file.

**No ClickHouse required.** `serve` builds a lazy plaintext CH client that never
connects unless a `/v1` route is hit, and the portal routes touch none. It also
ignores `CH_ENABLED`, which is a Lambda-only knob.

### 1. The Discord application

In the [Discord Developer Portal](https://discord.com/developers/applications),
**OAuth2 → Redirects → Add Redirect**, exactly:

```
http://localhost:4200/api-tokens/api/auth/callback
```

Port `4200`, not `8080`: the redirect URI must be the origin the **browser**
sees, which is the Vite dev server. Discord accepts `http://` for `localhost`,
and accepts several redirects on one application, so a production one can sit
alongside this. Matching is character-exact — scheme, port, path, and no
trailing slash.

Scope needs no configuration here: the handler requests exactly `identify` and
verifies the grant on the token response, refusing anything wider.

### 2. `.portal-oauth.json`

Gitignored (`.portal-oauth.json` and `**/.portal-oauth.json`), and it holds a
real client secret — check `git status` before committing anything.

```json
{
  "client_id": "000000000000000000",
  "client_secret": "REPLACE_ME_from_the_Developer_Portal",
  "redirect_uri": "http://localhost:4200/api-tokens/api/auth/callback",
  "session_signing_key": "REPLACE_ME_openssl_rand_hex_32"
}
```

| field | notes |
|-------|-------|
| `client_id` | from the Developer Portal. Not secret |
| `client_secret` | **Reset Secret** in the Developer Portal. Never an env var, never committed |
| `redirect_uri` | must equal the registration **character for character** |
| `session_signing_key` | ≥32 bytes. `openssl rand -hex 32` |

A **file** rather than an env var on purpose: there is no code path anywhere
that reads a client secret out of the environment, and adding one "just for
local" is a path production can be misconfigured onto. In production this same
JSON lives in Secrets Manager and is read through the Parameters & Secrets
extension — see
[`docs/runbooks/portal-oauth-deploy-prep.md`](../../docs/runbooks/portal-oauth-deploy-prep.md).

### 3. Run it

```bash
# terminal 1 — the API on :8080
PORTAL_ENABLED=true \
PORTAL_OAUTH_SECRET_FILE=.portal-oauth.json \
PORT=8080 RUST_LOG=info \
  cargo run -p prices-api --features local-server --bin serve
```

```bash
# terminal 2 — the portal on :4200, proxying /api-tokens/api/* to :8080
echo "DEV_API_PROXY_TARGET=http://localhost:8080" > web/portal/.env.development
npx nx dev portal
```

Open **`http://localhost:4200/api-tokens/`** — not `:8080`, which serves no
pages and answers `/api-tokens/` with the same empty `404` as any unrouted path.

Both ports move if they have to (`PORT=` on the API, `--port` on Vite), but the
Vite one is part of the redirect URI, so changing it means changing
`redirect_uri` **and** the Discord registration to match.

The proxy is what makes the browser see **one origin**, exactly as CloudFront
does in production. That is the property the `SameSite=Lax` session cookie
depends on; a separate backend port would break it in a way no test in this repo
can see.

`PORTAL_ENABLED` is the whole gate. Omit it and all four routes answer an empty
`404` — which is what production does today.

### 4. What you should see

Click **Sign in with Discord** → Discord's consent screen (first time only;
afterwards it is a bare redirect) → back at `/api-tokens/` showing your username
and Discord ID. **Sign out** clears the session.

Without the UI, the same thing over HTTP:

```bash
curl -sL -c jar.txt -b jar.txt -o /dev/null \
     -w '%{url_effective} %{http_code}\n' \
     http://localhost:4200/api-tokens/api/auth/login
curl -s -b jar.txt http://localhost:4200/api-tokens/api/auth/me
# → {"authenticated":true,"user_id":"...","username":"..."}
```

### 5. When it does not work

`RUST_LOG=info` above is there so there is something to read.

| symptom | cause |
|---------|-------|
| Discord's own page: **"Invalid OAuth2 redirect_uri"** | `redirect_uri` ≠ the registration. You never reached this service, so nothing is in its log |
| `400 invalid_state`, log says `portal sign-in callback rejected` | the `portal_oauth_pending` cookie did not come back. Almost always: started on one port, returned on another |
| the page says **"Sign-in could not be completed"** and the log says `portal sign-in refused by Discord error=invalid_scope` | the Developer Portal registration no longer matches `discord::SCOPE`. This is the SECOND place scope drift is caught, and the earlier one: Discord refuses at the authorize step, so the token-response check never runs. Fix the registration |
| the same page text with `error=server_error` or `temporarily_unavailable` | Discord's own problem. Nothing to do but retry |
| the page says **"Sign-in cancelled"** | only `access_denied` produces this — the visitor pressed Cancel. If you see it for anything else, the split in `callback` has regressed |
| `400 invalid_query`, log says `portal sign-in callback was malformed` | the callback carried neither `code` nor `error`, or the query string could not be deserialized. A client-side fault; deliberately **not** a 5xx, so it cannot be used to manufacture alarm noise |
| `500 sign_in_misconfigured` | the authorize URL is not a valid header value — almost always a stray character in `DISCORD_AUTHORIZE_URL` |
| `502 discord_unavailable`, log says `stage="token exchange"` | wrong `client_secret`, or no outbound route to `discord.com` |
| `502 discord_unavailable`, log says `stage="identity read"` | the exchange worked, `GET /users/@me` did not |
| `502` and the log names the granted scopes | Discord granted more than `identify`. Deliberate refusal — see ADR 0010 |
| `serve` exits at startup with `NoSource` | `PORTAL_ENABLED=true` with no secret source. Fatal on purpose: better than a sign-in button that answers `503` |
| `404` on `http://localhost:8080/api-tokens/` | expected — `:8080` is the API, the pages are on `:4200` |
| `serve` exits with `failed to bind: AddrInUse` | something already holds the port, often a `serve` from an earlier session. `PORT=8081 … serve` plus `npx nx dev portal --port 4201`, and change `redirect_uri` **and the Discord registration** to match — the browser-facing port is part of the URI |

### 6. Without a Discord account

`DISCORD_AUTHORIZE_URL` and `DISCORD_API_BASE` override the two endpoints, so the
flow can be pointed at a stand-in serving `/oauth2/authorize`, `/oauth2/token`
and `/users/@me`. This is a **test seam only**: `compute-stack.ts` sets neither,
so the deployed handler always takes Discord's real endpoints. The integration
suite (`tests/portal_auth.rs`) uses the same seam against a mock bound to
loopback.

### 7. Afterwards

Remove the `localhost` redirect from the Discord application once you are done —
or leave it and accept that any holder of that `client_secret` can complete a
sign-in from their own machine.

## Running self-service key issuance locally (tasks 0187 + 0189)

Since task 0189, `GET`/`POST /api-tokens/api/key` only **reveal** — the route is
read-only by construction. Issuing runs through the eligibility round-trip:
"Get my API key" navigates to `/auth/login?action=issue`, and the callback
checks Stellar Discord membership and account age against a **fresh** Discord
token before creating anything. Locally that means the run needs the sign-in
above, the control-plane pieces below, **and** the two eligibility knobs
(next section).

> **Every key this creates and deletes is a PRODUCTION key.**
>
> There is one environment and it is production (`infra/envs/` holds only
> `production.json`). `PORTAL_ENABLED=false` protects the deployed Lambda; it
> protects nothing on a laptop holding production credentials. The reconciler
> calls `DeleteApiKey`, so exercise it against keys this task created and
> nothing else, and delete them afterwards — task 0194 audits what is left.

### 1. AWS credentials and the usage plan id

```bash
# The plan the portal attaches keys to, published by ApiGatewayStack.
aws ssm get-parameter \
    --name /prices/production/pricing-api-free-plan-id \
    --query Parameter.Value --output text
```

The principal you run as needs `apigateway:GET/POST/DELETE` on `/apikeys`,
`/apikeys/*` and `POST` on `/usageplans/{id}/keys` — the same five the Lambda's
role has (`compute-stack.ts`, `api-gateway-stack.ts`).

### 2. Run it

```bash
PORTAL_ENABLED=true \
PORTAL_OAUTH_SECRET_FILE=.portal-oauth.json \
PORTAL_FREE_PLAN_ID=<the plan id from above> \
PORTAL_GUILD_ID=<a guild snowflake your Discord account is a member of> \
PORTAL_MIN_ACCOUNT_AGE_MINUTES=5 \
AWS_PROFILE=<a profile with the five grants> \
  cargo run -p prices-api --features local-server --bin serve
```

`PORTAL_FREE_PLAN_ID`, `PORTAL_GUILD_ID` and `PORTAL_MIN_ACCOUNT_AGE_MINUTES`
are **local-only** variables and are compiled out of the Lambda build, exactly
like `PORTAL_OAUTH_SECRET_FILE` and the Discord endpoint overrides. In the
Lambda all three come from SSM by name (`PORTAL_FREE_PLAN_PARAM`,
`PORTAL_GUILD_ID_PARAM`, `PORTAL_MIN_ACCOUNT_AGE_PARAM`), because
`lambda:UpdateFunctionConfiguration` — a permission distinct from
`UpdateFunctionCode` — would otherwise be enough to move every newly issued key
onto a plan of somebody else's choosing, or point the eligibility gate at a
guild of somebody else's choosing.

**The membership check is real.** The issue round-trip calls
`GET /users/@me/guilds/{PORTAL_GUILD_ID}/member` against real Discord with
your own token, so the guild id must be one **your** account is a member of
(any test guild of your own works) — otherwise the page honestly tells you to
join a server you were never meant to join. The Discord application also needs
`guilds.members.read` declared (deploy-prep runbook §1 step 3), or the round
trip refuses its own grant at the token exchange.

### 3. What you should see

Sign in at <http://localhost:4200/api-tokens/>, press **Get my API key** — a
redirect through Discord and back (no second consent screen), landing on
`?issue=ok` — and:

```bash
# The key exists, is enabled, and is named for your Discord id.
aws apigateway get-api-keys --name-query "discord-<your id>-key" \
    --query 'items[].{id:id,name:name,enabled:enabled}'

# It is on the free plan.
aws apigateway get-usage-plan-keys --usage-plan-id <plan id> \
    --query 'items[].name'

# And it works. This is the acceptance criterion that cannot be tested in CI.
curl -sS -o /dev/null -w '%{http_code}\n' \
    -H "X-API-Key: <the value the page showed>" \
    https://<api host>/production/v1/assets
# → 200
```

Press the link a second time: the same key, no new one. That is the
reconciler, not a cache.

### 3a. The eligibility refusals (task 0189)

The three refusal states are worth seeing once, because each renders
differently on purpose:

- **Not a member** — set `PORTAL_GUILD_ID` to a guild you are *not* in (any
  valid snowflake): the page names the server and links the invite. Only a
  confirmed Discord `404` (JSON code 10007/10004) lands here.
- **Too young** — not reproducible with a real account (you cannot make yours
  younger); the backend computes the wait from the snowflake and the page
  renders it. Covered by the test suites; take their word for it.
- **Could not verify** — point `DISCORD_API_BASE` at a dead port: the refusal
  explicitly does *not* claim you are not a member. A parameter that will not
  parse (e.g. `PORTAL_MIN_ACCOUNT_AGE_MINUTES=five`) fails the cold-start
  probe instead — that is the deploy-time guard, not a bug.

Changing `PORTAL_MIN_ACCOUNT_AGE_MINUTES` requires restarting `serve` locally
(it is an env var here); **in the Lambda the SSM parameter is re-read per
issuance**, so an operator's `put-parameter` needs no redeploy — that is the
point of the design, and the reason the local seam is the exception.

### 3b. Usage against quota (task 0188)

The same local run also serves `GET /api-tokens/api/usage`, and the page renders
it under the key. The principal needs one more grant than the five above:
`apigateway:GET` on `/usageplans/{id}/usage` (in the Lambda's role it lives in
`ApiGatewayStack`'s standalone portal policy, next to the `/keys` attach).

Two things that look like bugs and are not:

- **The figure lags.** `GetUsage` is not a read-after-write surface — curl the
  API a few times and the dashboard can still say `0` (or "nothing recorded
  yet") for minutes. The page's "last updated" line is the honest statement of
  exactly this.
- **A refresh does not re-ask AWS.** The handler caches per caller for 60s, so
  hammering refresh proves the cache, not the counter. Wait out the TTL (or
  restart `serve`) to force a fresh `GetUsage`.

The endpoint is **read-only by construction**: it never creates, attaches or
deletes a key. Since task 0189 the `/key` route is read-only too, which is why
the page now calls both on load — creating a key is the issue round-trip's job
and nobody else's.

### 3c. Replacing a key (task 0191)

On the page, **Replace my key…** beside the key opens a confirmation; the
confirm button arms only once you type `delete-key`. Pressing it **deactivates
the key and issues nothing** — `POST /api-tokens/api/key/rework`,
session-authenticated, no Discord round-trip. The control plane answers at
once; the **data plane follows within about half a minute** (~25 s measured,
0180 item 8), so treat the value as live until then — which is what the
dialog says, and why neither it nor this section claims "immediately".

A new key can be issued only from the start of the next quota period (the 1st
of next month, 00:00 UTC, **our** rule): until then "Get my API key" lands on
`?issue=capped` with the date, and the page says so instead of offering the
link.

```bash
# Before: one enabled key.
aws apigateway get-api-keys --name-query "discord-<your id>-key" \
    --query 'items[].{id:id,enabled:enabled,updated:lastUpdatedDate}'

# Confirm on the page, then:
aws apigateway get-api-keys --name-query "discord-<your id>-key" \
    --query 'items[].{id:id,enabled:enabled,updated:lastUpdatedDate}'
# → the SAME id, enabled: false, lastUpdatedDate just now — the revocation record.

# The value is refused within tens of seconds (0180 item 8) — the data-plane
# check CI cannot make. 403 is byte-identical to sending no key at all.
curl -sS -o /dev/null -w '%{http_code}\n' -H "X-API-Key: <the value>" \
    https://<api host>/production/v1/assets      # → 403
```

Press **Get my API key** now: the round-trip passes eligibility and lands on
`?issue=capped&next_eligible_at=<1st of next month>` — nothing created, the
disabled key untouched. On or after the 1st the same press deletes the
disabled key, creates a new one and attaches it; the old value stays dead.

The principal needs the seventh grant, `apigateway:PATCH` on `/apikeys/*`, in
its own statement (`PortalDisableOwnApiKeys`) and scoped by
`aws:ResourceTag/ManagedBy = prices-portal`, so the revoke can only touch keys
this portal created. The `GET`/`DELETE` statement beside it is 0187's and stays
unconditioned — narrowing those two is task 0194's call, because it would stop
the portal adopting a key created by hand in the console. No `UpdateUsagePlan`,
no `UpdateUsage`.

### 4. Afterwards

```bash
aws apigateway delete-api-key --api-key <id>
```

(A revoke leaves the key in place, disabled; delete it by id like any other.)

## Env (Lambda)

| Var | Purpose |
|-----|---------|
| `CH_ENABLED` | build the mTLS CH client at cold start (default true; `0`/`false` to skip) |
| `MTLS_SECRET_NAME`, `CH_DOMAIN` | mTLS bundle + endpoint (read by `prices-clickhouse::mtls`) |
| `API_BASE_URL` | OpenAPI `servers` URL. Set by `ComputeStack` from `apiBaseUrl` in `infra/envs/production.json`; MUST include the stage path (`…/production`) |
| `PORTAL_ENABLED` | serve the portal's backend routes (task 0183). **Defaults to false**; anything else is an empty `404` |
| `PORTAL_OAUTH_SECRET_NAME` | Secrets Manager **name** of the Discord OAuth bundle (task 0186). Never the value — read through the Parameters & Secrets extension, and only when the portal is open |
| `PORTAL_OAUTH_SECRET_FILE` | local-only alternative to the above: a path to the same JSON. Not set by CDK |
| `DISCORD_AUTHORIZE_URL`, `DISCORD_API_BASE` | endpoint overrides, test/local seam only. Not set by CDK, so production always takes Discord's real endpoints |
| `PORTAL_FREE_PLAN_PARAM` | SSM parameter **name** holding the `pricing-api-free` usage-plan id (task 0187). A name, not the id: the plan is created by `ApiGatewayStack`, which depends on `ComputeStack`, so a cross-stack reference would be a cycle |
| `PORTAL_FREE_PLAN_ID` | local-only alternative to the above. Compiled out of the Lambda build, and not set by CDK |
| `PORTAL_GUILD_ID_PARAM` | SSM parameter **name** holding the guild snowflake the eligibility gate checks membership against (task 0189). Operator-seeded, re-read per issuance — never a CloudFormation resource |
| `PORTAL_MIN_ACCOUNT_AGE_PARAM` | SSM parameter **name** holding the minimum Discord account age in minutes (task 0189). Same rules as the guild id |
| `PORTAL_GUILD_ID`, `PORTAL_MIN_ACCOUNT_AGE_MINUTES` | local-only direct values for the two above. Compiled out of the Lambda build, and not set by CDK |

## OpenAPI

The spec is generated from the axum routes by `utoipa`, so it cannot drift from
the implementation. It is served at `GET /api-docs-json` — **anonymous**, both
at the API Gateway (`apiKeyRequired: false`) and in the in-app key gate
(`auth::is_exempt`) — and cached for an hour at the gateway, 5 minutes at the
client (the gateway entry is flushed on deploy; a partner's cache is not).

```bash
npm run openapi:extract   # → target/openapi.json (servers stamped from config)
npm run openapi:lint      # extract + Redocly recommended-strict; runs in CI
```

`tests/openapi.rs` asserts the document's contract: route coverage against the
gateway in both directions, the `x-api-key` scheme, the anonymous opt-outs, and
the `servers` stamp.

## Cache-Control / TTL decisions

Per-endpoint TTL tiers live in `common/cache_control.rs`; the API Gateway stage
cache (Phase 4) mirrors them. Final TTLs documented here when Phase 4 lands
(overview §2.1: `/assets` 60s, `/ohlcv` 60s, `/price` 15s, `/backfill` 30s,
batch uncached).
