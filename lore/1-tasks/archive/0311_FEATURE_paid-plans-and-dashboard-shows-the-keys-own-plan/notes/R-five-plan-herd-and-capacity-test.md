# R: five-plan herd and capacity test (production, 2026-09-25)

Follow-up to the 2026-09-24 portal-closed alarm (a 60 s five-plan limit check that opened
69 cold starts at once). Run on production 2026-09-25 09:32–10:30 UTC with Adam's go-ahead,
in parallel with the 0286 phase-3 backfill (Oskar's machine).

Tags: **[M]** measured, **[E]** derived, **[H]** hypothesis.

- **Artefacts:** branch `test/0311-five-plan-loadtest` (never merged), which holds
  `packages/prices-api/loadtest/five-plan/` and `docs/loadtest-results/2026-09-25-0311-*`.
- **Keys:** 10 temporary keys, two per plan, created and deleted; deletion was verified.
- **Generator:** k6 v2.2.0 on the laptop.
- **Observer:** samples ClickHouse every 5 s and AWS every 60 s, and aborts automatically.

## Answers

**Why the portal closed and `/v1` kept working.**
- **Closure by design [M].** Each cold start makes five portal reads through the Parameters and
  Secrets extension (`main.rs:56`, `config.rs:324-329`): one secret plus four SSM parameters.
  Any failed read closes the portal in that execution environment for its lifetime
  (`config.rs:367-393`).
- **`/v1` is not gated by the portal [M].** `gate_portal` 404s only portal paths
  (`portal/mod.rs:333-338`).
- **What makes a read fail [M].** SSM throttles under a burst of cold starts; the extension
  retries 3× with backoff, and our 2 s client timeout (`prices-clickhouse/src/mtls.rs:110`)
  expires first.
  - A closure logged as "extension unreachable" is that timeout. The extension logs the
    `ThrottlingException` seconds later (10:15:22 closure → 10:15:27 throttle line).
- **The portal still sits on `/v1`'s path [M].** The five reads run sequentially before the
  mTLS read, so they delay every cold `/v1` request. Init max was 1.7–2.3 s when SSM
  throttled, against ~420 ms when it did not.

**How much ClickHouse holds, and at which mix.**
- **Realistic mix [M].** The mix was 60 % `/price`, 10 % batch, 10 % detail, 10 % ohlcv 1h/7d,
  5 % ohlcv 1d/1y and 5 % list, drawn from a 5,480-asset pool. It costs ~70 ms of host CPU per
  request and scales linearly: 44 rps → 3.1 cores, 88 rps → 6.1 cores. p99 stayed ≤ 176 ms and
  there was no knee.
- **ohlcv carries the cost [M].** ohlcv is 15 % of the requests but 63 % of the CPU (1d grain
  517 ms, 1h 206 ms CPU per query).
- **Budget [E].** With a typical 9-core API budget (12 cores minus ~3 of background), about
  **130 rps of this mix** can be sustained, i.e. ~3 full five-plan sets. Per plan alone at full
  rate: ~5 Pro, ~13 Lite, ~26 Analyst, ~43 Basic or ~130 free keys.
- **List is bounded by the quota, not CPU [M/E].** A cache-busting list miss costs ~385 ms CPU
  and ~0.93 M read rows, i.e. ~0.43 cores per req/s, with no knee up to 20 req/s. The
  `prices_read` quota (50e9 rows/h, API-wide) caps it at **~15 req/s sustained**. One Pro key
  paging `/v1/assets` exhausts the hour in ~36 min, after which **every** API query fails until
  the hour turns.
- **The backfill saturates the box on its own [M].** At 10:17:26–10:20 the 0286 backfill held
  the host at 19–24 cores with no API load: 842 `price_ohlcv_1m` inserts took 897 CPU-s,
  `ingest_cursor FINAL` selects 178 CPU-s, and the 15m MV cascade 121 CPU-s. During such steps
  the API has no headroom.

## Results per phase

| Phase | What | Result |
|---|---|---|
| F0 | background, 09:32–09:38 | host ~0.3 cores (RMV spikes at :00–:07 of even minutes); writer insert p95 ~65 ms; `assets` 7 parts, 2 copies [M] |
| F1 | `/backfill/status` at 1.5× each plan's rate, 120 s | 2xx 124 / 375 / 623 / 1,250 / 3,129 vs expected rate·t + burst 125 / 375 / 625 / 1,250 / 3,125 (±0.3 %); the rest 429; 0 × 403 / 5xx [M] |
| F2 | N simultaneous uncached `/price` requests on fresh environments (env bump) | see the curve below |
| F3 | `/v1/assets` at 1.5× plan rate, 180 s (replay of 09-24) | limits again within ±0.5 %; handler concurrency peaked at 25, not 69; CH ≤ 9 cores [M]. Herd ≈ arrival rate × miss latency: 66 rps × ~0.4 s ≈ 25 [E]. 09-24 misses took ~2.4 s (cold start + 4-copy `assets`), hence 69 |
| F4 | cache-busting list, 5 → 10 → 15 → 20 req/s | CPU/query p50 ~385 ms, wall p95 60–80 ms flat, host +2.5 / +4.5 / +6.5 / +8.8 cores; no knee; 0 × 5xx [M] |
| F5 | realistic mix, 5 keys (44 rps) then 10 keys (88 rps) | host avg 3.6 / 6.6 cores (max 8.2), in-flight ≤ 11, control `/price` p99 158 / 176 ms, 0 × 5xx / 429 [M] |
| F6 | F2 at N=200 with SSM high-throughput on (then reset) | 181 and 195 cold starts, **0 closures, 0 throttles**, Init max 423–429 ms [M] |

### F2: simultaneous cold starts → closed portals [M]

| Cold starts at once | Portals closed | SSM throttles | Init max |
|---|---|---|---|
| 10, 20, 28, 28, 54, 70, 96 | 0 | 0 (CloudTrail: 384 GetParameter/s at N=100 accepted) | ≤ 600 ms |
| 147 | 1 (0.7 %) | 1 | 2,326 ms |
| 200 | 3 (1.5 %) | 3 | 1,730 ms |
| 200 + SSM high-throughput (×2) | 0 | 0 | 429 ms |
| *2026-09-24: 68* | *7 (10 %)* | *5 + 2 "unreachable"* | *2,405 ms* |

- **The threshold is not stable [M].** Today SSM absorbed 384 calls/s. On 09-24 it threw
  throttles at 18 calls in the second after a 252-call second. "~10 cold starts close the
  portal" (the pre-test estimate) is refuted.
- **Why [H].** SSM's standard-throughput limit behaves like a shared, stateful bucket that we
  cannot observe.
- **Client artefact [M].** Opening 50+ TCP connections at once from the laptop stalls (connect
  p50 ~1 s). N=50 and the first N=70 therefore produced only 28 cold starts. Pre-connecting
  each VU on `/health` (a MOCK integration) fixed it.
- **09-24 was not a load problem for ClickHouse [M].** Its 69 list queries were 4× more
  expensive because `assets` held 4 unmerged copies under `FINAL`. `prices_writer` reseeds all
  209.9 k rows every 75–105 min and merges only every 4–6 h; this is not caused by 0216.

## What breaks first (in order of how easily it happens)

1. **The portal, on a cold-start burst.**
   - A plan burst can do it on its own: Pro's burst of 125 plus a slow miss.
   - It is stochastic: 0–10 % of the environments in a burst.
   - Closures persist until the environments are recycled.
2. **The `prices_read` quota, on list- or ohlcv-heavy cache misses.**
   - Takes the whole API down until the hour turns.
   - List-only traffic reaches it at ~15 req/s sustained.
3. **ClickHouse CPU.**
   - ~130 rps of the realistic mix fit in the typical budget.
   - During heavy backfill steps there is ~0 headroom.
   - At the 2026-09-18 collapse (~800 rps `/price`), the per-query thread fan-out is what
     amplifies the load.
4. **Lambda concurrency: not reached.**
   - Peak 200, out of an account pool of 1,000 shared with the explorer.
   - No throttles.

The gateway method throttles (10000/400, the API Gateway defaults) never bind.

## Mitigations, ranked by effect per cost

1. **Lazy portal load plus retry with jitter** (Stanisław's note of 2026-09-25, and ours).
   - A `/v1` cold start then does zero SSM reads.
   - A failed read fails one request instead of closing the environment.
   - Removes failure 1 entirely and trims ~100–2,000 ms from every cold `/v1`.
   - A code change only.
2. **SSM high-throughput.**
   - Measured: 3 → 0 closures at N=200 (small sample) and no retry tail on Init.
   - One account setting, $0.05 per 10 k calls.
   - The handler makes ~100 calls a day organically, so the cost is negligible.
   - It still closes the portal if a throttle ever happens, so it is a stopgap next to (1).
   - Reset to default after the test.
3. **Remove the `assets` sawtooth.**
   - Reseed only the changed rows, or merge after the reseed.
   - List and detail become 3–6× cheaper and the quota ceiling rises proportionally.
4. **An alarm on `prices_read` quota usage, Lambda throttles and concurrency.**
   - None exists today.
5. **Cheaper ohlcv.**
   - `usd_rate FINAL` (134 parts) and the 1h/1d part counts; the 0286 backfill currently
     inflates them.
   - ohlcv is 63 % of the mix's CPU.
6. **`prices_reader` `max_threads` (2–4) and `max_concurrent_queries_for_user`.**
   - Damps the 09-18 fan-out collapse.
   - Needs the explorer team.
7. **Reserved concurrency (50–100) for the handler.**
   - Protects the explorer's share of the pool.
   - Not needed at the measured peaks.

## After the fix (deployed 2026-09-25 12:47 UTC, feat/0311 @ a3ae084c)

Mitigations 1 and 2 are live: SSM high-throughput has been on since 10:41 UTC, and the
lazy portal load with retry shipped in Compute + Observability, with the alarm renamed
`api-handler-portal-load-failed`. The same burst was then rerun: F2 at N=200, fresh
environments, five plans.

| | before (10:21) | after (12:52) |
|---|---|---|
| simultaneous cold starts | 200 | 193 |
| SSM `GetParameter` in the burst minute | ~800 | **0** (AWS/Usage CallCount, no datapoint for 12:52) |
| Init p50 / max | 354 / 1,730 ms | **213 / 436 ms** |
| portal lines in the log | 3 closures | none |
| `/api/config` afterwards | — | `enabled: true`; the first call on a fresh environment loads in ~0.4–1.0 s, later calls take 0.1 s |

The only SSM reads after the deploy are single portal loads of 4 calls each, from
`/api/config` probes. The 10 temporary keys were deleted and the deletion verified;
`LOADTEST_EPOCH` was removed.

### Replay of the 2026-09-24 herd itself (13:20:45 UTC, after the fix) [M]

k6 phase `HERD`, set up like the 09-24 incident:
- five temporary keys, one per plan, carrying 2 / 5 / 8 / 16 / 38 virtual users (69 in total);
- every user pre-connected, then all fired one default `GET /v1/assets` at the same instant;
- the gateway cache was empty and every environment cold (env bump).

| | 2026-09-24 | 2026-09-25 after the fix |
|---|---|---|
| simultaneous cold starts | 69 | 69 (11 + 58 across 13:20:45–46) |
| SSM `GetParameter` in that minute | ~278, 19 throttled | **0** (no AWS/Usage datapoint for 13:20; the 4 at 13:22 are one `/api/config` probe) |
| Init p50 / max | 375 / 2,405 ms | 215 / 274 ms |
| portals closed / `portal-load-failed` or `portal-closed` alarm | 7 / ALARM | 0 / OK |
| 5xx | 0 | 0 |
| ClickHouse | 23.5 cores for ~3 s | ≤ 1.5 cores (69 list queries) |

The `curl` replay by five Haiku sub-agents, run earlier the same afternoon, did not form a herd.
The agents started seconds apart, so the first miss filled the 60 s cache and only 4 cold starts
happened. It still confirmed every key and limit; the steady-rate overshoot was again a
laptop-curl artefact.

## Side effects of the test

- `api-handler-portal-closed` went to ALARM at 10:16:51 (N=150/200, intended).
  - The environments were recycled when `LOADTEST_EPOCH` was removed (~10:26).
- `prices-production-asset-discovery-no-invocations` was in ALARM 10:18:25 → 10:21:25.
  - It coincides with the backfill's saturation minute; its link to the test is not established [H].
- No explorer alarm fired. Writer insert p95 stayed ≤ 85 ms during our phases.
- The observer aborted one phase itself.
  - The first N=200 attempt, at 10:17, stopped before it fired, due to the backfill saturation above.
  - It was rerun at 10:21 once the host was back at 0.3 cores.
  - Its summary file was overwritten by the rerun.
- The 0296 rule "capacity tests only with the loadtest key and never as a cold-start burst" was
  knowingly broken for this test, with Adam's approval and an announced window.
