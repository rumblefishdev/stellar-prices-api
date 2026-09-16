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
  - date: 2026-09-16
    status: active
    who: stkrolikiewicz
    note: >
      Findings recorded below. AC 1 is closed on the half reachable from outside
      the ClickHouse box: query saturation is RULED OUT by three independent
      measurements — Lambda `Duration` collapsed from 40-75 ms to 10-11 ms during
      the 5XX burst and recovered afterwards, X-Ray shows a median 26 ms
      end-to-end over 322 faulted traces, and the network hop alone is 80-130 ms.
      A saturated database is slower, not faster; no query round trip happened.
      Also ruled out: Lambda throttling (0), function failure (Errors 0 against
      28,853 gateway 5XX), and a reserved-concurrency ceiling (none configured,
      peak concurrency 26).
      ⛔ The direct proof is gone: all 28,853 records carry the empty
      `bad response: ` of [[0281]], fixed 2026-09-11, eight days after this
      incident. The fix IS deployed, so a controlled re-test would capture the
      real ClickHouse code — which is now worth more than further forensics here.
      ⚠️ AC 2 is NOT obtainable as written: the read path has no query-duration
      instrumentation, X-Ray carries no ClickHouse subsegment, and the pre-burst
      window is cache hits (median 6 ms), not healthy misses.
      Mechanism candidate, stated as a candidate: the read path inherits
      `pool_max_idle_per_host(2)` justified for "serial writes" plus HTTP/1.1
      only. That explains connection churn, not refusal — the refusal came from
      the far side and still needs `max_connections` and the server log.
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

## 🔬 Findings — 2026-09-16, from CloudWatch, X-Ray and the source

All read-only. Nothing was re-run against production.

### AC 1 — query saturation is RULED OUT, on evidence rather than inference

Three independent measurements of the same window agree, and they agree against
the obvious reading:

| measurement | during the burst | either side of it |
|---|---|---|
| Lambda `Duration` (avg, per minute) | **10–11 ms** | 40–75 ms |
| X-Ray trace duration (322 faulted traces, 06:36–06:37 UTC) | median **26 ms**, max 146 ms | — |
| X-Ray Lambda segment of one faulted trace | **14 ms** | — |

🔑 **A saturated database is slower, not faster.** The read path became
**4–7× quicker** exactly while it was returning 500s, and recovered to its
previous cost as they cleared. The AWS→Hetzner hop alone is 80–130 ms (§6 of the
load-test report), so a 26 ms end-to-end failure means **no query round trip
happened at all**. The failure is at or before connection establishment.

This closes the half of AC 1 that can be closed from outside the box: it is not
query cost. Naming the positive mechanism still needs the server side — see
"What still needs the box".

### Also ruled out

- **Lambda throttling** — `Throttles` = 0 across 06:00–08:00 UTC.
- **Function failure** — `Errors` = 0 against 28,853 gateway 5XX. The handler
  caught the error and returned a 500 body (`errors.rs:97`), which Lambda counts
  as a successful invocation. The read path never crashed; it answered, fast,
  with an error.
- **A reserved-concurrency ceiling** — none is configured for `apiHandler`
  (`production.json` sets only `memoryMb: 512`, `timeoutSeconds: 15`), and
  `ConcurrentExecutions` peaked at **26**.

### ⛔ The direct proof was destroyed by [[0281]], eight days before it was fixed

Every one of the 28,853 error records carries the same field:

```json
"fields":{"message":"clickhouse query failed","error":"bad response: ","context":"price lookup"}
```

The `error` body is **empty** — the exact signature of [[0281]], where a real
ClickHouse exception (code, message, elapsed) was discarded because Caddy sits in
front and the crate's LZ4 fallback never fires. No other error message appears in
the window at all.

✅ **0281 is fixed and the fix is live**: merged 2026-09-11 12:38 UTC, api-handler
deployed 12:52 UTC, archived "verified on production" 12:59 UTC. So a **future**
occurrence would carry ClickHouse's own code and message — which is what makes a
controlled re-test worth more than any further forensics on this one.

### AC 3, our half — the client pool is ours, and it is sized for writers

`main.rs:70` → `client_from_lambda_env` (`mtls.rs:346`) → `client_with_mtls`
(`mtls.rs:252`). The api-handler's read path therefore runs on:

- `.pool_max_idle_per_host(2)` (`mtls.rs:304`), justified in its own comment with
  *"two pooled connections per host is plenty for **serial writes**"* — reasoning
  written for low-concurrency workers, inherited by the read path;
- `.enable_http1()` only (`mtls.rs:292`) — **no HTTP/2**, so no multiplexing: N
  concurrent requests need N sockets;
- `.pool_idle_timeout(8s)`;
- the client is `Arc`-backed and shared per warm container (`state.rs:10`).

⚠️ **Stated carefully, because it is easy to overclaim:** `pool_max_idle_per_host`
is **not** a concurrency cap. Exceeding it does not refuse a request — the extra
connection is created and then dropped instead of being pooled. So this explains
**connection churn** (a fresh mTLS handshake on most misses at 100 rps), not the
immediate refusal. The refusal came from the far side. This is a strong mechanism
candidate, not the finding.

### AC 2 — not obtainable as the task assumed

The uncontended miss budget cannot be split from the evidence that exists:

- the read path has **no query-duration instrumentation** — `db_error`
  (`errors.rs:97`) logs the error and context only, and every `Instant::now()` in
  `prices-api` is in `portal/`, not the price path;
- **X-Ray has no subsegment for the ClickHouse call** — the outbound HTTP request
  is not instrumented, so a trace shows gateway and Lambda and nothing beyond;
- the pre-burst window is **not** a healthy-miss baseline: 3,985 traces, zero
  faults, median **6 ms** — those are cache hits, because regimes 1 and 2 were
  cache-dominated by construction. There is no window in which healthy misses ran
  at volume, so the ~170–240 ms figure cannot be decomposed after the fact.

### AC 4 — what this says about [[0121]]'s three levers

This task predicted the answer and the evidence now supports it: **all three
levers avoid the database rather than raising its ceiling.** A higher TTL
([[0122]]), provisioned concurrency, and a producer-side hot column each reduce
how often a miss reaches ClickHouse. If the ceiling is on connection
establishment, none of them moves it — they move the measured p95 and relocate
the failure to whenever the hit rate drops. That is a number bought, not a fix.

### AC 5 — recovery still unexplained, but the shape is sharper than recorded

- First error **06:34:10.799 UTC** — earlier than the 06:35 minute bucket.
- 🔑 **Degradation preceded the errors by two minutes**: 06:33 and 06:34 show
  `Duration` already collapsed to 20 ms and 13 ms with **zero** 5XX. Something was
  already failing to do work before anything was reported.
- The burst is 06:35–06:39 (5,173 / 5,999 / 5,999 / 5,995 / 5,146 per minute),
  then 500 at 06:40, then a **tail of 2–4 per minute until ~06:58** while
  `Duration` returns to 40–65 ms.

The tail is the interesting part: it is not a clean recovery at one instant but a
decay. Still unexplained, and still nobody acted on it.

### What still needs the box

Everything below is operator/CHQ territory and was not reachable from this
session:

- `max_connections`, `max_concurrent_queries`, and what the ClickHouse server log
  recorded in the window — the positive naming of the ceiling;
- whether Caddy:443 (the mTLS terminator) has its own connection limit, and
  whether it or ClickHouse refused;
- whether the ceiling was reached **jointly** with soroban-block-explorer, which
  is the other half of AC 3 and [[0047]]'s question.

### Future work this surfaced (not yet filed)

1. **The read path has no timing instrumentation.** A single `Instant` around the
   ClickHouse call, logged on both arms, would have answered AC 2 from the logs.
   The same gap will defeat the next investigation.
2. **The mTLS client pool is sized for serial writes and used by the read path.**
   Whether `pool_max_idle_per_host(2)` plus HTTP/1.1-only is right for a
   request-per-invocation API is a design question, independent of this incident.

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
