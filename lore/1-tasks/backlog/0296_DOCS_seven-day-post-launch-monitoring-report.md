---
id: "0296"
title: "Tranche 3 AC 9 asks for a 7-day post-launch monitoring report — launch agreed as 2026-09-23 09:40 CEST, window to 2026-09-30 09:40"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0294", "0293", "0249", "0260"]
tags: [layer-docs, priority-high, effort-small, milestone-M3, observability, scf, evidence]
milestone: 3
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/prices-api-load-test-100rps.md"
history:
  - date: 2026-09-25
    status: backlog
    who: stkrolikiewicz
    note: >
      LAUNCH AGREED, window fixed. The daily on 2026-09-24 took the launch
      event as the moment the portal became public: the explorer's basic-auth
      flip on /api/* finished 2026-09-23 07:40:45 UTC (09:40 CEST,
      ApiSpaRoutingFunction UPDATE_COMPLETE, explorer 0519 config change,
      recorded in prices 0305 and explorer 0574). The 7-day window is therefore
      2026-09-23 09:40 → 2026-09-30 09:40 CEST. Decided by the operator with
      the team. Criterion 1 ticked. Corrections carried into the Context: the
      backfill did not start on 09-21 — 0286's phase-3 re-ingest (stage A,
      pre-Soroban months, SDEX-only) started 2026-09-23 16:10 CEST on the
      shared box and overlaps the whole window; the load-test days 09-17/18
      fall outside it. Incident log for the window opened below (two entries
      on 09-24). 1-minute datapoints for 09-23 expire around 2026-10-08:
      pull the figures right after 09-30.
  - date: 2026-09-18
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0294]]. Tranche 3 AC 9 had no owner. Blocked on one
      decision — what counts as "launch" — which is the operator's and the
      team's to make, not this task's.
---

# 7-day post-launch monitoring report

## Summary

Tranche 3 AC 9: _"7-day post-launch monitoring report: uptime %, error rate, p95
latency, SDEX push cadence and `earliest_data_available` trajectory."_ Pick the
seven days, pull the figures from CloudWatch and the API, and write the report
for the M3 package ([[0294]]).

## Context

- **"Launch" was undefined; it is agreed now (2026-09-24 daily).** The
  candidates were the custom domain cutover, the self-service portal opening
  and the repo going public. The team took the portal opening: the explorer's
  basic auth on `/api/*` came off on **2026-09-23 07:40:45 UTC = 09:40 CEST**
  (`ApiSpaRoutingFunction UPDATE_COMPLETE`, explorer 0519 config flip,
  recorded in [[0305]] and explorer 0574). **Window: 2026-09-23 09:40 →
  2026-09-30 09:40 CEST.**
- **Two of the five metrics no longer exist as written.** The SDEX archive
  completed on 2026-07-27, so there is no push cadence and
  `earliest_data_available` is flat at 2015-11-18. Same root as the AC 1
  amendment — report the live signals instead (ledger-processor lag,
  rollup-freshness, `realtime_tip_ledger` tracking the chain) and declare it in
  the M3 deviations.
- **The window must avoid, or annotate, the load tests.** 2026-09-17 and
  2026-09-18 carry ~650 k synthetic requests, and 2026-09-18 alone has 14,865
  gateway 5XX from the deliberate 1000 req/s overload ([[0293]]). A window that
  includes them reports a test, not the service.
- **[[0286]]'s phase-3 re-ingest runs on the shared ClickHouse box for the
  whole window.** It started 2026-09-23 16:10 CEST (stage A, pre-Soroban
  months, SDEX-only, from `fishuser-hero`; ~22–28 days in total). It does not
  touch live data or the API, but latency figures describe a loaded box — say
  so in the report. (The earlier "starts 2026-09-21" was a plan, not a fact.)
- **The load-test days are outside the window.** 2026-09-17/18 precede the
  launch by five days; nothing needs annotating for them.
- **Nothing measures uptime directly.** There is no canary; `GET /health` is a
  keyless gateway mock that exercises neither Lambda nor ClickHouse. Uptime has
  to be derived — e.g. minutes with a 5XX rate over a stated threshold, from
  `AWS/ApiGateway` — and the definition has to be in the report.
- **Retention.** CloudWatch keeps 1-minute datapoints for 15 days and 5-minute
  for 63. Pull the window within two weeks of its end, or the p95 is coarser
  than the report implies.

## Implementation

- ~~Agree the launch event and the 7-day window; record both and who decided.~~
  Done 2026-09-24/25 — see the history entry and the Context.
- Pull, for the window: request count, 4XX/5XX rate, `Latency` p50/p95/p99
  (gateway-measured — see [[0293]] for why the client side is not the number),
  cache hit ratio, Lambda errors/throttles, alarm state changes, and the live
  ingestion signals above. Keep the queries in the report so the figures can be
  re-run.
- Define uptime, state the definition, compute it.
- Note every incident inside the window with its cause; an unexplained gap
  reads worse than an explained one.
- Land it as `docs/scf/milestone-3-monitoring-report.md` (or a section of the
  evidence document — [[0294]] decides).

## Acceptance Criteria

- [x] The launch event and the 7-day window are agreed and recorded
      → 2026-09-23 09:40 CEST (basic auth off on `/api/*`), window to
      2026-09-30 09:40 CEST; daily 2026-09-24, recorded 2026-09-25
- [ ] Uptime is defined in the report and computed from a stated source
- [ ] Error rate and p95 latency are reported from gateway-side metrics, with
      the queries that produced them
- [ ] The two obsolete metrics are replaced by live ingestion signals and the
      replacement is declared as a deviation in [[0294]]
- [ ] Load-test days and the backfill are either outside the window or
      annotated inside it
- [ ] Figures pulled while 1-minute resolution is still retained

## Incidents in the window — running log

Every entry names the cause; an unexplained gap is a finding, not a footnote.

| when (CEST) | what | API impact | cause / task |
|---|---|---|---|
| 2026-09-24 15:17 | `prices-production-oracle` `Runtime.OutOfMemory` (256/256 MB), one 5-minute oracle tick skipped; `oracle-errors` ALARM 15:18 → OK 15:23 | none on `/v1` | the re-ingest writes the whole asset registry after every month (`sdex-backfill/src/run.rs:321`); the oracle reads `prices.assets` without `FINAL` and sees up to 4 un-merged copies — [[0226]], [[0140]]; fix agreed: the backfill writes deltas |
| 2026-09-24 15:23 → ~16:00 | portal closed in part of the fleet: a teammate's `curl` loop on `GET /v1/assets` (~4,100 requests in 2 min, 2,747 cache hits, 1,326 × 4XX, 69 cold starts) throttled Parameter Store (account default 40 TPS, 19 × `ThrottlingException`); 7 api-handler environments booted with the portal closed and served `/api/config` `enabled:false` until they were recycled by idleness; `api-handler-portal-closed` ALARM 15:24 | `/v1`: 0 × 5XX, 0 Lambda errors; portal sign-in/dashboard intermittently "closed" | design trade-off from 0194 ([[0249]] caught it as intended); rule agreed: throughput tests only with the load-test key, never as a cold-start burst; a retry-at-cold-start task is proposed |
| 2026-09-25 11:56 → 12:25 | portal closed in part of the fleet again, same mechanism: a teammate's k6 run for [[0311]] (`User-Agent: k6-0311-five-plan`, one home IP) against production `/health` and `/v1/assets/native/price` — ramps to 4,496 invocations/min, concurrency 200 at 12:21–12:24, 576 cold starts in ten minutes, 0 throttles, 0 errors, all 200/202; ≥ 4 environments logged "portal closed at cold start" (12:15, 12:21 × 3); `portal-closed` ALARM 12:16:51, still ALARM at 12:33 | none on `/v1`; the portal answered as closed from those environments until recycled | 0249 said it: expect this alarm during a load test. Cold-start bursts are the trigger, the fix is the retry at cold start ([[0249]] follow-up) |

## Notes

- If an api-handler error alarm exists by then ([[0249]]), its history is the
  cheapest incident log for the window.
