---
id: "0296"
title: "Tranche 3 AC 9 asks for a 7-day post-launch monitoring report — nobody owns it, and 'launch' has no agreed date"
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

- **"Launch" is undefined.** The API has been serving production for weeks.
  Candidates: the custom domain cutover, the self-service portal opening, the
  repo going public. The window follows from that choice, so the choice comes
  first.
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
- **A month-long backfill starts 2026-09-21** on the shared ClickHouse box;
  latency during it describes a loaded box. Say so if the window overlaps.
- **Nothing measures uptime directly.** There is no canary; `GET /health` is a
  keyless gateway mock that exercises neither Lambda nor ClickHouse. Uptime has
  to be derived — e.g. minutes with a 5XX rate over a stated threshold, from
  `AWS/ApiGateway` — and the definition has to be in the report.
- **Retention.** CloudWatch keeps 1-minute datapoints for 15 days and 5-minute
  for 63. Pull the window within two weeks of its end, or the p95 is coarser
  than the report implies.

## Implementation

- Agree the launch event and the 7-day window; record both and who decided.
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

- [ ] The launch event and the 7-day window are agreed and recorded
- [ ] Uptime is defined in the report and computed from a stated source
- [ ] Error rate and p95 latency are reported from gateway-side metrics, with
      the queries that produced them
- [ ] The two obsolete metrics are replaced by live ingestion signals and the
      replacement is declared as a deviation in [[0294]]
- [ ] Load-test days and the backfill are either outside the window or
      annotated inside it
- [ ] Figures pulled while 1-minute resolution is still retained

## Notes

- If an api-handler error alarm exists by then ([[0249]]), its history is the
  cheapest incident log for the window.
