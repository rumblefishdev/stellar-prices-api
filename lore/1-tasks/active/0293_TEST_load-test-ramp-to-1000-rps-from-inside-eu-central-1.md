---
id: "0293"
title: "Load-test run 2: ramp 100 → 250 → 500 → 1000 req/s of cache misses, driven from inside eu-central-1 — and the three-row report M3 asks for"
type: TEST
status: active
related_adr: []
related_tasks: ["0260", "0121", "0122", "0047", "0249"]
tags: [layer-infra, priority-high, effort-medium, performance, clickhouse, load-test, milestone-M3]
milestone: 3
links:
  - "../../../docs/prices-api-load-test-100rps.md"
  - "../../../packages/prices-api/loadtest/price_load.js"
  - "../../../docs/loadtest-results/2026-09-17-regime3-wide.json"
history:
  - date: 2026-09-17
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0260]] on closing it. 0260 answered the research
      question (the 2026-09-03 collapse was a 10,000 queries/hour ClickHouse
      quota, fixed 2026-09-17) and ran run 1 — 100 req/s of misses held with
      0 errors, gateway p95 74 ms — but the client on a laptop measured p95
      464 ms, so no number from that run can stand in the report. This task
      owns run 2 and the report.
  - date: 2026-09-17
    status: active
    who: stkrolikiewicz
    note: >
      Activated the same day. First job: commit run 1's k6 export and the
      report section on this task's branch; then the four preconditions.
  - date: 2026-09-18
    status: active
    who: stkrolikiewicz
    note: >
      Team decision at the daily: the ramp runs as planned, 500 and 1000
      req/s are informational, and the M3 evidence uses whatever the run
      measures. See "Decision — 2026-09-18".
  - date: 2026-09-18
    status: active
    who: stkrolikiewicz
    note: >
      Diagnostic run from the laptop, bracketed by two cache controls:
      regime 3 at 100 req/s gave k6 p95 127 ms (464 the day before) with the
      gateway at ~80 — yesterday's tail WAS the client network, now shown
      rather than argued. Not an evidence run: the pool was a day old (1,536
      of 4,039 assets 404 at setup, 67 more mid-run), a backfill was running,
      and the after-control showed the network tail growing. New finding: a
      ~4 % slow mode of ~+60 ms sits between Lambda and ClickHouse — not the
      database, not idle containers, not cold starts.
---

# Load-test run 2: ramp to 1000 req/s from inside eu-central-1, and the report

## Summary

M3's Work list asks for a load-test report with results at 100 / 500 / 1000
req/s; AC 5 asks for p95 < 100 ms at 100 req/s with the usage plan named.
[[0260]]'s run 1 (2026-09-17) showed the data path holds 100 req/s of cache
misses — 0 errors on 29,968 requests, ClickHouse median 8 ms, **gateway-measured
p95 74 ms** — while k6 on the operator's laptop measured **p95 464 ms**, with
the whole difference sitting between the laptop and the gateway. A number for
the report has to come from a client whose network is not the variable.

This task: drive the ramp from a client inside `eu-central-1`, stop at the knee
if there is one, and write the three-row report — with one honest hit-rate
column, because on this API hit and miss cannot be tagged per request.

## Context

- The 2026-09-03 collapse was the `prices_read` quota (10,000 queries/h on
  `prices_reader`). Fixed on the box 2026-09-17 12:13 UTC (sbe task 0561,
  PR #462): `queries` and `execution_time` unlimited, `read_rows` 50 B and
  `read_bytes` 1 TiB kept as tenant guards. Next wall: `read_bytes` at ~700 k
  queries/h (~1.5 MiB per miss). A 1000 req/s × 5 min run is 300 k.
- The `/price` route throttles at 10,000 req/s (burst 5,000) — not a blocker.
  The binding gateway cap is the usage plan `prices-production-loadtest-plan`
  (`i12bsj`): **150 req/s, burst 300, 1 M/month** (891 k left on 2026-09-17).
- Run 1's pool: 4,039 listed assets, 4,020 with a price row. The cache key on
  `/price` is the path **plus `min_volume_usd`** ([[0122]]), so distinct keys
  can be multiplied without more assets.
- Each miss reads ~17.5 k rows / ~1.5 MiB in ClickHouse. 1000 req/s of misses
  is ~1.5 GB/s of reads on a box **shared with soroban-block-explorer** — nobody
  has measured that ceiling; this is where [[0047]]'s question becomes live.
- Run 1's decomposition (0260, "Run 1 done"): query ≈ 8 ms, Lambda warm 16 ms,
  gateway overhead ~6 ms, cold start ≈ 560 ms on 0.14 % of invocations.

## Implementation

Preconditions, all four before the first request:

1. **Client inside `eu-central-1`** — an EC2 instance or CloudShell with k6.
   Record instance type and AZ in the report. This also retires the laptop
   as the source of any number.
2. **Raise the loadtest usage plan** to ≥ 1000 req/s, burst ≥ 2000
   (`aws apigateway update-usage-plan`, per `docs/runbooks/manual-api-key-tier.md`).
   Count the monthly quota: each full ramp is ~480 k requests.
3. **Cache-defeating key variant in `price_load.js`**: rotate a few
   `min_volume_usd` values per asset so distinct keys ≫ RATE × TTL at 1000
   req/s (≫ 10,000). State the resulting expected hit rate in the report.
4. **A window agreed with sbe** (Karol), an observer on the box
   (`system.quotas_usage`, `query_log` exceptions and median — the
   `obserwator` loop from run 1 — plus CloudWatch gateway 5XX), and the abort
   rule: stop at the first gateway 5XX, the first ClickHouse exception, or
   ClickHouse median leaving single-digit ms. Start a few minutes before `:00`.

The run: ramp **100 → 250 → 500 → 1000 req/s**, each step held long enough for
p95 to settle (≥ 2 min), 5 min at the top. Stop at the first step that breaks
the abort rule and **report that step as the knee** — a knee below 1000 is a
result, not a failed test.

Then the report, in `docs/prices-api-load-test-100rps.md` (or its successor):
one row per rate, the plan named, client location named, gateway-measured and
client-measured p95 side by side, and the hit-rate estimate per row. Include
run 1 (the export `docs/loadtest-results/2026-09-17-regime3-wide.json` is
still uncommitted in the working tree — commit it on this task's branch).

## Decision — 2026-09-18 (daily)

Agreed with the team:

- **The ramp runs as planned.** 500 req/s, and 1000 if the box allows, are
  **informational** — they answer the Work list's "results at 100/s, 500/s,
  1000/s" and name our ceiling. Neither is a pass/fail bar for M3.
- **The only bar is AC 5: p95 < 100 ms at 100 req/s, plan named** (200 ms was
  Tranche 2's AC 2, not this one). The 20-asset AC scenario already meets it
  (p95 47 ms, 2026-09-03), and run 1's miss-only p95 measured at the gateway is
  74 ms — also under the bar. Only the laptop-measured 464 ms is over it, which
  is why the in-region client comes first: it may make any further argument
  unnecessary.
- **The evidence package uses the numbers the run produces, as p95.** The
  criterion names p95, so a mean does not replace it; it can sit beside it as
  an extra column. Report the miss-only rows and the AC scenario side by side,
  each with its hit rate.
- **The cache argument, stated so production data cannot contradict it.** Do
  not claim that today's users hit the cache: CloudWatch for
  `prices-production-api`, 2026-09-04 → 09-16 (load-test day excluded), shows
  63 `CacheHitCount` against ~1,650 `CacheMissCount` (~4 %) on 0–1,100
  requests/day — at that volume a 10 s TTL almost never sees the same key
  twice. The honest form: at low traffic nearly everything is a miss and a miss
  costs ~74 ms p95 at the gateway; the cache earns its keep under volume
  concentrated on popular assets, which is the regime where load matters
  (regime 2: ~98 % hits, p95 47 ms at 100 req/s). 1000 req/s of pure misses is
  a synthetic worst case, reported as the data path's ceiling.

## Run 2026-09-18 — diagnostic, laptop on a measured network

Exports: `docs/loadtest-results/2026-09-18-{control-before,regime3-wide,control-after}.json`.
Observer on the box and CloudWatch throughout: 0 gateway 5XX, 0 ClickHouse
exceptions, ClickHouse median 8 ms, quota counter to `36707/inf`.

| run (2026-09-18, laptop, k6 v2.2.0) | UTC | k6 med | k6 p95 | k6 p99 | gateway p95 | errors |
|---|---|---|---|---|---|---|
| control before — 1 asset, cache hits | 09:33–09:34 | 45.4 | 49.4 | 135 | 6 | 0 |
| **regime 3 — wide pool, 100 req/s × 5 min** | 09:35–09:42 | **70.6** | **127.3** | 176 | 74–85 (last two minutes 38–41) | 0 × 5XX, 67 × 404 |
| control after — 1 asset, cache hits | 09:43–09:44 | 45.4 | 82.3 | 373 | 6 | 0 |

**1. Yesterday's 464 ms was the client network.** A control on one cached asset
isolates the path: the gateway answers a hit in ~6 ms, so everything above that
is network — 45 ms median with a 4 ms tail before the run. With the path clean,
k6 equals network + gateway at every percentile (70.6 = 40 + 31; 127 ≈ 45 + 80;
176 ≈ 45 + 130). The run was genuinely miss-only: the gateway counted ~6,000
`CacheMissCount` and 0 `CacheHitCount` per minute in the main phase.

**2. This is not an evidence run.** (a) The pool was the previous day's —
1,536 of 4,039 assets answered 404 at setup, leaving 2,503 (still ≫ RATE × TTL,
so the regime held). `current_prices` keeps assets with a 1m candle in the last
24 h and roughly a third of that long tail turns over daily: 3,730 assets in
the window on 09-17 13:56, 3,340 on 09-18 09:36, 2,436 in common. Candle inflow
was healthy (500–1,200 assets/hour over 48 h, no step). **Regenerate the pool in
the same command chain as the run.** (b) 67 requests in `phase:main` got 404 as
assets slid out of the 24 h window mid-run; k6 counts non-2xx as failed, so
`http_req_failed` crossed 0.1 % and k6 exited 99 with zero server errors. A 404
for an asset without a price is a correct answer — count it separately in the
script. (c) A backfill (after the 0282/0286 changes) was running on the shared
box. (d) The after-control's tail grew (p95 82, p99 373): the network started
degrading late in the chain, so 127 is slightly pessimistic; ~120 on a fully
clean path.

**3. Where the gateway's p95 ≈ 80 ms comes from — a ~4 % slow mode, not the
database.** 90 % of requests clear the gateway in ≤ 37 ms; p95 flips between
~40 and ~80 depending on whether the slow share is just under or over 5 %.

| suspect | finding | verdict |
|---|---|---|
| cold starts | 0 in `phase:main` (18 containers, all warm) | no |
| idle containers re-dialling ClickHouse (pool idle 8 s) | gaps > 8 s do cost 75–210 ms — but 20 of 26,972 invocations | real, negligible |
| ClickHouse | p50 8, p95 **10**, p99 37 ms; 0.85 % of queries > 40 ms | no |
| **Lambda → Caddy → ClickHouse hop** | **4.2 % of invocations > 50 ms on busy, warm containers** (99.6 % arrive < 1 s after the previous one; p99 77 ms in that group) | **yes, unresolved** |

The read path has no timing around connect vs query ([[0249]] owns that gap),
so the hop cannot be split further from outside. If the slow mode went away the
gateway's p95 would sit near 40 ms and the laptop's near 85 — under the M3 bar
even from Poland.

**4. The "every other minute" pattern is on the box.** ClickHouse shows ~115
`prices_reader` queries > 40 ms in each even minute (09:36, 09:38, 09:40) and
none in the odd ones. Something runs every two minutes. It moves p99, not p95.

## Acceptance Criteria

- [ ] k6 ran from a client inside `eu-central-1`; type, AZ and k6 version recorded
- [ ] The usage plan for the run is named, with its rate/burst at run time
- [ ] Hit rate per row is stated, derived from pool size × key variants vs RATE × TTL
- [ ] The ramp reached 1000 req/s, or the knee is named with the abort reason and the ClickHouse/gateway readings at that step
- [ ] Zero gateway 5XX and zero ClickHouse exceptions at every step reported as passed
- [ ] p95 at 100 req/s reported both gateway-measured and client-measured, with the M3 bar (100 ms) beside it
- [ ] Report committed with the k6 exports for every row, including run 1's
- [ ] If the knee is shared-box saturation, [[0047]] gets the numbers

## Notes

- Not before [[0249]] is at least discussed: 26 minutes of outage on 2026-09-03
  paged nobody, and run 1 was watched by hand. A supervised run is acceptable;
  an unsupervised one is not.
- The gateway's per-minute p95 in run 1 alternated 45 / 81 / 44 / 89 ms —
  something periodic adds ~40 ms every other minute. Unexplained; watch for it.
