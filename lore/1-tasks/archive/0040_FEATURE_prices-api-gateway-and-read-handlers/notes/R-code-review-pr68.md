# Code Review — PR #68 (feat/0040-prices-api-scaffold)

> PR #68 · `feat(0040): prices-api — all 7 read endpoints + API Gateway CDK + load test`
> Base `develop` ← `feat/0040-prices-api-scaffold`. Reviewed at high effort
> (8 finder angles + verification). Two candidates were refuted by running
> `cdk synth` against the synthesized templates.

## Verdict

Functionally complete, but **three correctness bugs should be fixed before
merge** (backfill progress inversion, OHLCV silent truncation, prod stage
throttle dropped). The rest are efficiency / consistency / cleanup items.

## Findings (ranked)

### 1. `GET /backfill/status` reports progress backwards — **HIGH**
`packages/prices-api/src/backfill/handlers.rs:36,60`

`current_ledger` advances upward start→target (confirmed by the §2.2 checkpoint
contract: *"on restart the task resumes from `current_ledger + 1`"*). With
`start=1, target=57234198, current=34891234` the stream is **60.96% done with
22,342,964 ledgers remaining**, but the code reports:

- `progress_pct = (target-current)/(target-start) ≈ 39.04%` → that's the
  **remaining** fraction, not progress.
- `ledgers_remaining = current-start = 34891233` → that's ledgers **done**.

`tests/endpoints_it.rs:235-239` bakes in the inverted values, so the suite is
green while the public §4.5/§5.6 contract (consumed by BE analytics) is
backwards. **Fix:** `progress = (current-start)/(target-start)*100`,
`ledgers_remaining = target-current`, and update the test expectations.

### 2. OHLCV silently truncates to the OLDEST 5000 candles — **MED/HIGH**
`packages/prices-api/src/assets/queries_ch.rs:474` · handler `handlers.rs:289-337`

`granularity` is user-overridable with no validation against the timeframe span,
and `limit` is always `OHLCV_MAX_POINTS=5000`. `?timeframe=1y&granularity=1m`
matches ~525,600 buckets; `ORDER BY timestamp ASC LIMIT 5000` returns the oldest
~3.5 days (starting a year ago) and **omits everything recent** — exactly the
candles a chart wants — with no 400, no error, no truncation flag. Same for a
wide `?start`/`?end` range at fine granularity. **Fix:** cap/validate
granularity-vs-window, or return the most-recent N (`ORDER BY … DESC` then
reverse), and signal truncation.

### 3. Production stage-wide throttle dropped — **MED** (synth-confirmed)
`infra/src/lib/stacks/api-gateway-stack.ts:179`

`cfnStage.methodSettings = [...]` overwrites the array CDK computes from
`deployOptions.throttlingRateLimit(200)/Burst(400)`. Verified against the
synthesized template: the prod stage emits exactly 8 MethodSettings, **none with
ThrottlingRateLimit/BurstLimit**, and **no `/*/*` entry**. Only the per-key
usage-plan throttle (100/200) survives; the §2.1 aggregate stage ceiling is
gone. Prod-only (branch runs when `apiGatewayCacheEnabled=true`). **Fix:** keep a
`{ resourcePath: '/*', httpMethod: '*', throttlingRateLimit, throttlingBurstLimit }`
entry in the replacement array (or set the throttle via `addMethodSettings`).

### 4. `POST /prices/batch` is N+1 — **MED** (efficiency)
`packages/prices-api/src/batch/handlers.rs:54`

Loops up to `MAX_BATCH=100` awaiting `current_price()` once per asset — each a
`current_prices FINAL ⨝ assets FINAL` collapse. 100 serialized round-trips
(~500 ms+) blows the §6 <100 ms p95 goal and holds the Lambda for the whole
chain. **Fix:** one `WHERE (code,issuer,contract) IN (...)` query.

### 5. Malformed `?start`/`?end` → 500 instead of 400 — **LOW/MED**
`packages/prices-api/src/assets/handlers.rs:281` · `queries_ch.rs:446/451`

`p.start`/`p.end` are passed through unvalidated into ClickHouse
`parseDateTimeBestEffort(?)`, which throws on garbage → `internal_error(DB_ERROR)`
→ 500. A client input error should be a 400.

### 6. Cache-Control tiers disagree with gateway TTLs — **LOW** (altitude)
`packages/prices-api/src/common/cache_control.rs:13` · `api-gateway-stack.ts:24`

The module comment claims the gateway "keys its per-endpoint TTLs off the same
tiering", but they diverge: `/price` SHORT=10s vs CDK 15s; `/oracles` and
`/backfill` MEDIUM=60s vs CDK 30s. Clients/CDNs hold data longer than the gateway
refreshes (serve staler than origin), and each new endpoint must touch two
unrelated files no test ties together. **Fix:** derive both from one source.

### 7. USDC quote-miss → empty 200 OHLCV — **LOW**
`packages/prices-api/src/assets/handlers.rs:318`

If the USDC quote leg isn't resolvable (different issuer / Soroban SAC /
partially-seeded `assets`), default USD OHLCV returns a 200 with empty `data`
instead of a 404/500, masking the data gap.

### 8. PriceResponse 0072-stub duplication — **LOW** (simplification)
`packages/prices-api/src/batch/handlers.rs:59` (+ `assets::get_price`)

The `price_xlm:"0"`, `change_24h_pct:"0"`, `sources:{}` stub mapping is copied
verbatim in two sites. Task 0072 must flip both in lockstep or the two endpoints
return divergent shapes for the same asset. **Fix:** one `PriceResponse::from_row`.

### 9. `validateConfig` doesn't relate stage rate to per-key rate — **LOW**
`infra/src/lib/types.ts:193`

Each throttle pair is validated internally but never cross-checked, so
`apiGatewayThrottleRate=50` with `apiKeyRateLimit=100` passes yet silently caps
every key below its advertised SLA. Manifests only as prod 429s.

### 10. DB-error mapping copy-pasted ~10× — **LOW** (simplification)
`packages/prices-api/src/assets/handlers.rs:68` (and peers)

Each call site re-derives the same 500 mapping with a bespoke message. Centralize
via `From<clickhouse::Error>` / a `db_error(e, ctx)` helper so a status/policy
change is one edit.

## Refuted by `cdk synth` (not bugs)

- **Circular Compute↔ApiGateway stack dependency** — synth succeeds; CDK places
  all 14 `AWS::Lambda::Permission` resources in the **ApiGateway** stack (0 in
  Compute), so there is no back-reference / cycle.
- **Cache-key cross-page bleed** — the synthesized methods correctly declare
  `RequestParameters` + `Integration.CacheKeyParameters` (incl. `cursor`), so
  paginated reads cache under distinct keys.

## Verification notes

- `cursor.rs` encode/decode round-trips; the `list_assets` keyset predicate
  (`(sort_expr, asset_id) {cmp} (?, ?)` with matching ORDER BY + `limit+1`
  truncate) is sound — no dup/skip across pages.
- All user values in `queries_ch.rs` are `.bind()`-parameterized; only static
  enum-derived strings are interpolated — no SQL injection.
- `auth::ct_eq` + exempt-path gate correct; `identity.rs` strkey parsing (CRC
  validated) correct; positional RowBinary struct field orders match SELECTs.
- `.trash/cache.rs` is a clean `git mv` (no `rm`); no dangling `moka` dep.
