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
  - date: 2026-09-16
    status: active
    who: stkrolikiewicz
    note: >
      🔴 CORRECTION to the findings recorded earlier today. The connection-pool
      mechanism candidate is DISPROVED, not merely hedged. It rested on the read
      path being concurrent within a container, and it is not: Lambda serves one
      request per execution environment, and no endpoint fans out —
      `current_prices_batch` issues a single awaited query and a repo-wide search
      for join_all / try_join_all / FuturesUnordered / buffer_unordered in
      `prices-api/src/` returns nothing. One container needs one connection; the
      pool holds two. The framing "sized for writers, inherited by readers"
      implied a mismatch that does not exist and would have sent the next reader
      down a false trail.
      Kept as a positive finding instead: the read path is serial per container,
      so ClickHouse saw tens of connections at 26 concurrent executions rather
      than hundreds. Whatever refused them, it was not our client exhausting
      sockets — which raises the value of the server-side numbers.
      Also corrected: a timer around the ClickHouse call would NOT have answered
      AC 2, which asks for a network/query/connection split. Recorded as a note
      against [[0249]] rather than spawned as a separate task.
  - date: 2026-09-17
    status: active
    who: stkrolikiewicz
    note: >
      CAUSE NAMED from the ClickHouse box itself (read-only over ssh; the access
      existed all along). It is neither connections nor query cost: the
      `prices_reader` user carries an hourly QUOTA of 10,000 queries
      (`prices_read`, defined in sbe's `users.d/quotas.xml`). The server logged
      exactly 28,853 code-201 refusals — the same number as the gateway 5XX —
      first at 06:34:10, last at 06:57:54, and the message itself names the
      recovery: "Interval will end at 07:00:00". AC 1, 3, 4, 5 and 6 close; AC 2
      stays open, reframed. Four statements from 2026-09-16 are WITHDRAWN and
      marked in place: "no query round trip happened", the 80–130 ms "floor",
      "degradation preceded the errors", and "needs the box / not reachable".
  - date: 2026-09-17
    status: active
    who: stkrolikiewicz
    note: >
      QUOTA FIXED ON THE BOX, 12:13 UTC. sbe task 0561 / PR #462 (merged,
      develop 07210e28) set `queries` and `execution_time` of `prices_read`
      to unlimited and kept read_rows 50 B / read_bytes 1 TiB / result_rows
      10 B as the resource guards; deployed by an in-place overwrite of the
      bind-mounted quotas.xml, no restart. `system.quota_limits` now shows
      max_queries NULL, max_execution_time NULL for prices_read. The
      remaining wall for the M3 runs is read_bytes at ~700 k queries/h — the
      1000 req/s × 5 min run (300 k) fits with ~2× margin. AC 2's controlled
      re-run is unblocked on the ClickHouse side; the gateway-5XX alarm
      ([[0249]]) and the stage throttle check remain prerequisites.
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

## ✅ Quota fixed — 2026-09-17 12:13 UTC

sbe task 0561, PR #462 (merged, develop `07210e28`), deployed the same day by
an in-place overwrite of the bind-mounted `quotas.xml` (inode kept, ClickHouse
hot-reloaded, no restart). `system.quota_limits` for `prices_read`:

| | max_queries | max_execution_time | max_read_rows | max_read_bytes | max_result_rows |
|---|---|---|---|---|---|
| before | 10,000 | 1000 | 50 B | 1 TiB | 10 B |
| after | **NULL** | **NULL** | 50 B | 1 TiB | 10 B |

The byte/row guards stay by design — they are what keeps this tenant from
draining the shared box, and the reason `prices_reader` was not moved to
sbe's `unlimited` quota. Next wall for a load run is `read_bytes` at roughly
700 k queries/h; the largest M3 run (1000 req/s × 5 min = 300 k) fits with
~2× margin. Every other quota on the box was byte-identical before and after.

## 🔑 Cause named — 2026-09-17, from the ClickHouse box

Read-only, over the operator path the runbooks already use. Raw readings:
[notes/R-clickhouse-box-readings-2026-09-17.md](notes/R-clickhouse-box-readings-2026-09-17.md).

### AC 1 — it is a per-user hourly query QUOTA, not connections and not query cost

```
Code: 201. DB::Exception: Quota for user `prices_reader` for 3600s has been
exceeded: queries = 10001/10000. Interval will end at 2026-09-03 07:00:00.
```

| | ClickHouse `query_log` | what this task already had |
|---|---|---|
| refusals | **28,853** × code 201 | **28,853** gateway 5XX |
| first | 06:34:10 | 06:34:10.799 |
| last | 06:57:54 | tail "until ~06:58" |
| back to normal | 07:00:00 — the quota interval rolls over | "recovered unattended" |

The server admitted exactly 10,000 `prices_reader` queries in the hour
(3,124 at 06:32, 5,489 at 06:33, the rest at 06:34) and refused every one after
that before it started. It explains each oddity of the day at once:

- **failures were fast** — a quota refusal happens before execution;
- **the whole read path fell**, `/price` and `/v1/assets` alike — one DB user;
- **a single request with nothing behind it failed** — the budget is per clock
  hour, not per unit of load;
- **it healed itself** — at the top of the hour. The outage lasts until the next
  `:00`, so up to 59 minutes depending on when the budget runs out.

ClickHouse was idle throughout: at 5,489 queries in a minute its median stayed
**7–8 ms**, against limits of `max_connections` 4096 and
`max_concurrent_queries` 1000. Caddy's logs were not read — ClickHouse itself
logged the refusals and the count matches to the unit.

### Where the quota comes from

`soroban-block-explorer/crates/db-clickhouse/users.d/quotas.xml:126`, quota
`prices_read`, commented `mirrors api_throttle`, added 2026-06-23 (sbe
`lore-0314`). sbe's own API user hit the template it mirrors — `api_reader` shows
code 201 on seven days in June — and on 2026-07-01 sbe moved `api_reader` to
`unlimited`. **`api_throttle` now applies to nobody; we run a copy of the
template its authors abandoned.** Their comment in the same file: *"read_rows /
read_bytes are the real resource guards"*.

### AC 3 — ours alone

The quota is keyed by user name. Every other tenant ran normally through the
window (~300 queries/min baseline, unchanged). Not a joint ceiling; [[0047]]'s
question is answered "no" for this incident.

### AC 4 — remediation, and 0121's three levers

**Fix the quota, in sbe's repo, deployed to the shared box.** Raising `queries`
alone moves the wall rather than removing it — the same quota carries three more
hourly caps, and the 10,000 admitted queries measured what each one costs
(9 ms, 17.5 k rows, ~1.5 MiB per query):

| cap | value / h | trips at about |
|---|---|---|
| `queries` | 10,000 | 10 k queries — tripped 2026-09-03 |
| `execution_time` | 1000 s | **~110 k** queries |
| `read_bytes` | 1 TiB | ~700 k queries |
| `read_rows` | 50 B | ~2.8 M queries |

Against that, the misses a 5-minute run can send: 100 req/s → 30 k, 500 → 150 k,
1000 → 300 k. So 30 k is zero margin for the first run and `execution_time`
stops the second. **Recommended: drop the `queries` cap and raise
`execution_time`, keep `read_rows` / `read_bytes`** — sbe's own resolution. If a
number must stay: ≥ 500 k queries and ≥ 5000 s, at which point it guards nothing.

[[0121]]'s levers, each ruled out **as a fix**: a higher TTL ([[0122]]),
provisioned concurrency and a producer-side hot column all lower the miss rate
or the latency. None touches an hourly count. They stay valid as latency work.

Today's exposure: the worst organic hour in 14 days is 909 queries, ~11× under
the cap. But 10,000/h is **2.8 misses/s sustained** — any burst of misses beyond
that blacks out the read path until the next `:00`, and nothing pages:
handler-returned 500s are not Lambda `Errors`, and there is no gateway-5XX alarm
([[0249]]).

### AC 5 — recovery explained

The non-randomized 3600 s interval rolled over at 07:00:00. The "tail of 2–4 per
minute until ~06:58" was organic traffic still being refused, not a decay.

### AC 6 — not structural

A configuration value, not an architecture limit. ADR 0007's sidecar fallback is
not triggered by this incident.

### AC 2 — still open, but the question changed

The 170–240 ms uncontended miss is **not** a network floor. Under load the full
Lambda→ClickHouse→Lambda path ran at 10–20 ms `Duration` with 7–8 ms inside
ClickHouse, so the warm round trip is a few milliseconds. The overview's own
line agrees ("single-digit-ms p50 SELECTs … once the connection is warm"): its
80–130 ms is what a *cold* connection costs. The split worth measuring is now
cold start / TLS setup versus everything else — and only a controlled re-run
under sustained load shows whether p95 < 100 ms holds on misses.

### What a re-run still needs — beyond the quota

- a gateway-5XX alarm first ([[0249]]) — 26 minutes of outage paged nobody;
- the stage throttle: the overview states 200 req/s per method stage-wide, which
  would stop the 500 and 1000 req/s runs at the gateway — to verify in `infra/`;
- an observer on `system.quotas_usage` and an agreed abort signal; starting a few
  minutes before `:00` caps the cost of a mistake at minutes;
- a way to defeat the cache at 500/1000 req/s: production lists ~3.5 k assets and
  a full-miss pool must be ≫ RATE × TTL — not designed yet.

### 🔴 Withdrawn from the 2026-09-16 findings below

1. *"No query round trip happened at all … at or before connection
   establishment."* A round trip happened; ClickHouse answered with a refusal in
   milliseconds. `bad response: ` with an empty body was an HTTP response all
   along.
2. *The 80–130 ms hop as a floor.* It is a cold-connection cost (AC 2 above).
3. *"Degradation preceded the errors by two minutes."* 06:32–06:33 were ~8,600
   healthy queries on the warm path; low `Duration` was the system working.
4. *"Needs the box / not reachable from this session."* The key and the
   `sorban-prod` host entry were on this machine; what was missing was asking.

What stands: query saturation ruled out; throttling, function failure and
reserved concurrency ruled out; the client pool is not the ceiling; and
[[0281]] ate the evidence — the discarded body said "Quota … exceeded" in
words, and cost fourteen days.

## 🔬 Findings — 2026-09-16, from CloudWatch, X-Ray and the source

> ⚠️ **Partly superseded on 2026-09-17** — see "Withdrawn" above. Kept as written
> so the correction is traceable; withdrawn statements are marked in place.

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

> 🔴 **WITHDRAWN 2026-09-17.** A round trip happened and ClickHouse refused in
> milliseconds (quota, code 201). The 80–130 ms is a cold-connection cost, not a
> floor. "Not query cost" stands; the inference about connections does not.

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

### AC 3, our half — the client side is NOT the ceiling (candidate disproved)

`main.rs:70` → `client_from_lambda_env` (`mtls.rs:346`) → `client_with_mtls`
(`mtls.rs:252`), so the read path runs on `.pool_max_idle_per_host(2)`
(`mtls.rs:304`), `.pool_idle_timeout(8s)` and `.enable_http1()` only — no
HTTP/2 — with an `Arc`-backed client shared per warm container (`state.rs:10`).

🔴 **An earlier pass today recorded this as a mechanism candidate. It is
disproved.** That reading rested on a pool justified for "serial writes" being
inherited by a *concurrent* read path. The read path is not concurrent:

- **Lambda serves one request per execution environment.** Concurrency comes from
  more containers, each with its own client and its own pool — never from
  parallel requests inside one container.
- **No endpoint fans out to ClickHouse.** `current_prices_batch` takes every id
  in a single awaited query (`batch/handlers.rs:72`), and a repo-wide search for
  `join_all` / `try_join_all` / `FuturesUnordered` / `buffer_unordered` across
  `prices-api/src/` returns nothing.

So one container needs **one** connection at a time while the pool holds two —
double the requirement, not half of it. The comment's "plenty for serial writes"
is equally true of serial reads, and a warm container reuses its pooled socket
inside the 8 s idle window, so there is no per-request handshake either.

🔑 Worth keeping as a finding in its own right: **the read path is serial per
container**, so at 26 concurrent executions ClickHouse saw on the order of *tens*
of connections, not hundreds. Whatever refused them, it was not our client
running out of sockets — which makes the server-side numbers more important, not
less.


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
  🔴 **WITHDRAWN 2026-09-17** — those two minutes were ~8,600 healthy queries on
  the warm path, spending the hourly quota. Nothing was failing yet.
- The burst is 06:35–06:39 (5,173 / 5,999 / 5,999 / 5,995 / 5,146 per minute),
  then 500 at 06:40, then a **tail of 2–4 per minute until ~06:58** while
  `Duration` returns to 40–65 ms.

The tail is the interesting part: it is not a clean recovery at one instant but a
decay. Still unexplained, and still nobody acted on it.

### What still needs the box

> ✅ **Done 2026-09-17** — see "Cause named" at the top. The access existed.

Everything below is operator/CHQ territory and was not reachable from this
session:

- `max_connections`, `max_concurrent_queries`, and what the ClickHouse server log
  recorded in the window — the positive naming of the ceiling;
- whether Caddy:443 (the mTLS terminator) has its own connection limit, and
  whether it or ClickHouse refused;
- whether the ceiling was reached **jointly** with soroban-block-explorer, which
  is the other half of AC 3 and [[0047]]'s question.

### Future work this surfaced

**The read path has no timing instrumentation.** `db_error` (`errors.rs:97`) logs
the error and context only, and X-Ray carries no subsegment for the ClickHouse
call.

⚠️ Correcting an overstatement made earlier today: a timer would **not** have
answered AC 2, which asks for a split across network, query and connection setup.
A single `Instant` yields the total and nothing more. It would still have been
worth having, and the next investigation will hit the same wall.

Deliberately **not** filed as its own task: the change is one `Instant` on a path
somebody will touch anyway, and [[0249]] already owns the api-handler's
observability gaps. Fold it in there rather than adding an 86th backlog item.

## Acceptance Criteria

- [x] The failure mode is named: connection ceiling, query saturation, or
      something else — with evidence, not inference from response times
      — *something else: the `prices_reader` hourly query quota, 28,853 × code 201*
- [ ] The uncontended miss budget is broken down into network / query /
      connection setup — *open, reframed: cold-connection cost vs the rest;
      needs a controlled re-run — the quota is fixed as of 2026-09-17 12:13
      UTC (sbe 0561), so the ClickHouse side no longer blocks it*
- [x] It is stated whether the ceiling is ours alone or shared with
      soroban-block-explorer ([[0047]]) — *ours alone, the quota is per user*
- [x] A remediation is recommended **against the identified cause**, explicitly
      confirming or ruling out each of 0121's three assumed levers
- [x] Recovery mechanism explained, or recorded as unexplained
      — *the quota interval rolled over at 07:00:00*
- [x] If the cause is structural, ADR 0007's sidecar-ClickHouse fallback is
      revisited on the record — *not structural; not triggered*

## Notes

- Reproducing the collapse deliberately is a **potentially destructive test of
  shared infrastructure**. If it is repeated, agree an abort signal and an
  observer who can see the box first — neither existed on 2026-09-03.
- A cheaper first step: a ramp between 65 req/s (clean during that run's setup
  phase) and 100 req/s (collapse) locates the knee without sitting on it.
