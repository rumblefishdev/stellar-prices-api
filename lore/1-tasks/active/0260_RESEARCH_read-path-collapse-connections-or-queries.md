---
id: "0260"
title: 'Read path collapsed at 100 req/s of cache misses — connection ceiling or query performance?'
type: RESEARCH
status: active
related_adr: ['0007']
related_tasks: ['0121', '0047', '0122']
tags:
  [
    layer-backend,
    layer-infra,
    priority-high,
    effort-medium,
    performance,
    clickhouse,
    incident,
    milestone-M3,
  ]
milestone: 3
links:
  - '../../../docs/prices-api-load-test-100rps.md'
history:
  - date: 2026-09-03
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0121]]'s regime 3, which took the production read path down
      on 2026-09-03. The load test can report the failure but cannot diagnose
      it — that needs access to the prod account and to the ClickHouse box.
  - date: 2026-09-16
    status: backlog
    who: stkrolikiewicz
    note: >
      Tagged to milestone 3 — both halves of the convention, the `milestone-M3`
      tag and the `milestone` field. This task carries acceptance criterion 5,
      the load-test report (p95 <100ms at 100 req/s), but had neither, so it was
      invisible to every M3-scoped view. That is how a Tranche 3 gap came to be
      written up as having no owner at all.
      ⚠️ [[0047]] was weighed alongside it and deliberately NOT tagged: it is
      `deferred` / `phase-post-deploy`, its own history drops it to priority-low
      as explicitly not a blocker, and it gates ADR 0007 rather than any
      acceptance criterion.
  - date: 2026-09-16
    status: active
    who: stkrolikiewicz
    note: >
      Activated, and chosen over the 500/1000 req/s runs deliberately. Those are
      the other half of acceptance criterion 5, but regime 3 at 100 req/s already
      took the production read path down for 19-47 minutes; the ClickHouse box is
      shared with soroban-block-explorer and was NOT quiet today; and raising the
      rate before this task names the ceiling would buy a larger outage to learn
      what forensics can establish for free.
      ⏳ TIME-BOXED BY RETENTION — the evidence is Lambda logs from 2026-09-03
      and the log groups keep 30 days, so it expires around 2026-10-03, in 17
      days. 8,878 events sit in the 06:30-06:40 UTC window and the metrics are
      complete (15-month retention).
      🔑 First finding already in hand, before any analysis: `Throttles` = 0
      across 06:00-08:00 UTC on `prices-production-api-handler`. Lambda
      concurrency throttling is ruled out as the ceiling.
      ⚠️ The ClickHouse half (`max_connections`, `max_concurrent_queries`, the
      server log) still needs operator access through CHQ and is not reachable
      from this session.
---

# Read path collapse at 100 req/s of misses — connections or queries?

## Summary

On 2026-09-03 a load test ([[0121]], regime 3) drove 100 req/s of **cache
misses** at `GET /assets/{id}/price` for five minutes. 94.38 % of requests
returned `500 db_error`, and the whole ClickHouse read path — `/price` and
`/v1/assets` alike — stayed down for 19–47 minutes afterwards, failing even a
single request with no concurrency behind it. It recovered unattended.

This task answers the one question that decides what to do about it: **is the
ceiling the number of connections, or the cost of the queries?**

## Context

Two earlier regimes offered the same 100 req/s and were clean, because 98–99.9 %
of their requests were served by the API Gateway cache and never reached the
database. Regime 3's 4301-asset pool defeats that cache by construction, so it
was the first run in which 100 req/s actually arrived at ClickHouse.

**The evidence pointing at connections rather than queries** is that the failures
returned *fast* — p50 65 ms, max 457 ms — instead of timing out. A database
saturated on query execution produces slow responses and timeouts. Immediate
`500`s look more like a `max_connections` ceiling, an exhausted client pool, or
mTLS session setup failing under concurrency. That is a hypothesis, not a
finding; nothing available from outside the prod account can confirm it.

**Why the distinction decides everything.** [[0121]] listed three remediation
levers in advance: raise the per-endpoint TTL ([[0122]]), Lambda provisioned
concurrency, move a hot column producer-side. All three address latency or miss
rate, and two work by *avoiding* the database. If the ceiling is connections,
none of them raises it — they raise the measured p95 and move the failure to
whenever the cache hit rate drops. Choosing a lever before answering this
question risks buying a number instead of a fix.

A separate measurement from the same day sharpens the stakes: **a cache miss
costs ~170–240 ms with zero contention**, already at the Tranche 2 bar and about
double the Tranche 3 one.

## Implementation

- Get read access to the production account, or pair with someone who has it —
  this is the hard prerequisite and the reason 0121 could not do this work.
- From the incident window (2026-09-03, 06:32–06:39 UTC): Lambda
  `ConcurrentExecutions`, `Errors`, `Throttles`, `InitDuration`; API Gateway
  `Latency` vs `IntegrationLatency`.
- On the ClickHouse box: `max_connections`, `max_concurrent_queries`, the
  connection and query counts during the window, and whatever the server log
  recorded. Note that the box is **shared with soroban-block-explorer** — the
  ceiling may be reached jointly, which is [[0047]]'s question.
- Establish where the ~170–240 ms of an uncontended miss actually goes: the
  AWS→Hetzner hop (~80–130 ms per §6), the query itself, or connection setup.
- Determine what recovered the system, since nothing was done on our side.

## Acceptance Criteria

- [ ] The failure mode is named: connection ceiling, query saturation, or
      something else — with evidence, not inference from response times
- [ ] The uncontended miss budget is broken down into network / query /
      connection setup
- [ ] It is stated whether the ceiling is ours alone or shared with
      soroban-block-explorer ([[0047]])
- [ ] A remediation is recommended **against the identified cause**, explicitly
      confirming or ruling out each of 0121's three assumed levers
- [ ] Recovery mechanism explained, or recorded as unexplained
- [ ] If the cause is structural, ADR 0007's sidecar-ClickHouse fallback is
      revisited on the record

## Notes

- Reproducing the collapse deliberately is a **potentially destructive test of
  shared infrastructure**. If it is repeated, agree an abort signal and an
  observer who can see the box first — neither existed on 2026-09-03.
- A cheaper first step: a ramp between 65 req/s (clean during that run's setup
  phase) and 100 req/s (collapse) locates the knee without sitting on it.
