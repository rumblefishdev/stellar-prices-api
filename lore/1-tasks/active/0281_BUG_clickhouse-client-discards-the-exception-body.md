---
id: "0281"
title: "Our ClickHouse client turns a real server exception into BadResponse(\"\") — the error code, elapsed time and query are all discarded"
type: BUG
status: active
related_adr: []
related_tasks: ["0215", "0111", "0214"]
tags: [layer-backend, priority-high, effort-small, clickhouse, observability, resilience]
links:
  - "../active/0215_BUG_deployed-lambda-never-issues-the-usdt-pivot.md"
history:
  - date: 2026-09-11
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0215]]'s induction run, which failed its own acceptance
      criterion and proved this instead. Measured on both sides the same
      minute: ClickHouse raised a complete `TIMEOUT_EXCEEDED` and the worker
      logged an empty body.
  - date: 2026-09-11
    status: active
    who: okarcz
    note: >
      Activated immediately. It blocks [[0215]]'s last criterion, and it is
      wider than that task: `prices-clickhouse` is shared, so every worker and
      operator CLI currently loses the cause of any failed write.
---

# The ClickHouse client discards the exception body

## Summary

When a statement fails server-side, our client reports
`clickhouse: bad response: ` — an **empty** string — while ClickHouse has
raised a fully-formed exception carrying the error code, the elapsed time, the
bound that was crossed and the query itself. Everything needed to diagnose the
failure exists and is thrown away one layer above.

This is [[0215]]'s hazard 3 surviving the fix that was written to end it.

## Evidence — both sides of the same statement, 2026-09-11 11:49:40

**ClickHouse** (`system.query_log`, `user = 'prices_writer'`):

```
type:              ExceptionWhileProcessing
exception_code:    159
query_duration_ms: 1003
exception:         Code: 159. DB::Exception: Timeout exceeded:
                   elapsed 1002.319598 ms, maximum: 1000 ms. (TIMEOUT_EXCEEDED)
                   (version 26.3.10.60 (official build))
query:             INSERT INTO prices.price_ohlcv_1m (timestamp, asset_id, …
```

**The worker**, same second (`/aws/lambda/prices-production-enrichment`):

```json
{"level":"WARN","fields":{
  "message":"historical sweep failed (non-fatal — live pass unaffected)",
  "error":"clickhouse: bad response: "}}
```

## What is already ruled out

- ⛔ **Not the request path.** The bound reached the server and was enforced to
  the millisecond — 1002.3 ms against a 1000 ms limit. `with_execution_bound`
  sets it as a URL option (`prices-clickhouse/src/lib.rs:197`) and that works.
- ⛔ **Not ClickHouse.** It threw, recorded the throw, and its message is
  complete.
- ⛔ **Not the worker's logging.** `main.rs:289` logs `error = %e`; the
  `Display` it is handed is already empty.

So the loss happens between the HTTP response and `clickhouse::error::Error`.

## Where to look

`clickhouse` **0.13.3**. `Error::BadResponse(String)` is built from the
response body, so an empty string means the body was empty **by the time it was
read**. Candidates, cheapest first:

1. The response is chunked and the exception arrives in a trailer or after the
   body stream ends, so a non-streaming read sees nothing.
2. The status is non-200 and the body is consumed or dropped before the error
   is constructed.
3. `wait_end_of_query` / progress-header settings change where ClickHouse puts
   the exception for an `INSERT … SELECT` specifically.

⚠️ **It must be reproduced on an `INSERT`, not a `SELECT`.** That distinction is
what made this invisible: on 2026-09-10, 23 `TIMEOUT_EXCEEDED` events were
observed as clean, complete errors — every one a `dev_read` **SELECT**. The
conclusion drawn was "ClickHouse throws and we will see it", and the SELECT path
genuinely does. The worker's statement is an `INSERT … SELECT` on a different
path.

⛔ **It cannot be reproduced as `dev_read`.** `readonly = 1` refuses a settings
change (code 164) and the profile cannot INSERT. Reproducing needs
`prices_writer`, i.e. the deployed worker or a local CH at the prod version
(**26.3.10.60**, [[feedback-local-tests-match-prod-version]]).

## Why it is worth a task rather than a note

The whole value of [[0215]]'s half 2 was the **failure mode**, not the ceiling.
A bound that fires but reports as an empty body leaves the next recurrence
exactly as undiagnosable as 2026-07-26 — which cost 26 days and was found by
hand. And this is not one caller's problem: every operator CLI and every worker
shares `prices-clickhouse`, so **any** server-side failure on an INSERT path
currently reaches us stripped of its cause.

## Implementation

- Reproduce against a local ClickHouse pinned to **26.3.10.60** with a small
  `INSERT … SELECT` and `max_execution_time=1`, asserting on the error text.
- Fix in `prices-clickhouse` (or at the `mtls.rs` client construction) so the
  server's message survives. If the crate cannot be made to surface it, read
  the body ourselves and wrap it.
- A test that fails on an empty error string, so this cannot regress quietly.
- ⭐ Consider falling back to `system.query_log` on an empty error: we know the
  query id, and the exception is durably recorded there. Belt and braces for a
  path whose whole purpose is being readable after the fact.

## Implementation Notes

Fixed in **PR #310**, branch `fix/0281_clickhouse-client-discards-the-exception-body`.

### The cause — a fallback that does not fire

`collect_bad_response` (crate 0.13.3) LZ4-decodes the error body and falls back
to the raw bytes on failure:

```rust
let bytes = collect_bytes(stream).await.unwrap_or(raw_bytes);
```

Straight to ClickHouse the decode **fails**, the fallback fires, the message
survives. Through a proxy the chunk reframing makes the decode **succeed with
zero bytes** — `Ok(empty)` — so `unwrap_or` never runs and the message becomes
`""`. Our client never called `with_compression`, so it ran the crate default
of `Lz4`, and every production client reaches ClickHouse through Caddy.

### Measured three ways, CH 26.3.10.60, same statement

| path | error the caller receives |
|---|---|
| direct | `bad response: Code: 159. DB::Exception: Timeout exceeded: elapsed 1000.178179 ms, maximum: 1000 ms. (TIMEOUT_EXCEEDED)` |
| through Caddy | `bad response: ` |
| through Caddy, compression off | the full message returns |

`curl` reads the body in **all three** — so Caddy forwards it correctly and the
loss is entirely in the crate's decode path.

### 🔴 It was never only timeouts

Through the proxy, **every** error status lost its body — measured on `404`
UNKNOWN_TABLE, `400` SYNTAX_ERROR and `408` TIMEOUT_EXCEEDED alike. So every
server-side failure in production — including the `Code: 243` disk-full class
from [[0204]] — has been arriving stripped of its cause, for every worker,
probe, the API and every operator CLI.

## Design Decisions

### Emerged

1. **The fix lives in a named shared function, not inline.** `with_readable_errors`
   in `prices-clickhouse`, called by `mtls::client_with_mtls` — the single
   funnel every component uses. Inline, the test would have had to build its own
   client and would not have guarded the real one.

2. **The test FAILS when its proxy is missing, rather than skipping.** Straight
   to ClickHouse it passes whether or not the defect is present, because the
   fallback fires. A test that cannot fail is worse than no test: it reports
   success. `CLICKHOUSE_PROXY_URL` unset is therefore an error with an
   explanation, and `scripts/ch-proxy-0281.sh` stands the proxy up.

3. **Compression disabled rather than the crate bumped.** 0.15.2 exists against
   our pinned 0.13.3 and may fix the fallback, but that is a multi-version bump
   across every crate here — its own change, on its own evidence. Recorded in
   the code comment so the cheaper option is not lost.

4. **The trade-off is stated in the code, not just the PR.** Response bandwidth
   on reads, against errors that cannot be diagnosed at all. The comment says
   plainly that this is not a performance setting and points at this task,
   because the obvious future "cleanup" is to restore the default.

## Issues Encountered

- ⚠️ **I exhausted the shared `dev_read` hourly quota on production** while
  hunting for a slow query to time out: `SELECT count() FROM numbers(1e11)`
  reads 100 billion rows and the quota is 100 billion/hour, so one probe spent
  the lot and blocked `chq` for the whole team until the top of the hour. The
  API and the workers were unaffected (different users). Lesson: `numbers()`
  looks free because it touches no table, but every generated row counts
  against the quota — use `sleepEachRow`, which burns time and no rows, or stay
  local. The whole investigation was reproducible locally for nothing.

## Acceptance Criteria

- [x] A failing `INSERT … SELECT` surfaces the ClickHouse error **code** and
      message to the caller; `BadResponse("")` no longer occurs for a statement
      ClickHouse recorded an exception for. **PR #310.**
- [x] Reproduced in a test against CH **26.3.10.60**, red before the fix.
      `execution_bound_error_it`, verified red with the exact message
      *"the error carries no message at all — this is the 0281 defect"* and
      green after. ⚠️ Requires a proxy and fails loudly without one.
- [ ] The enrichment worker logs `TIMEOUT_EXCEEDED` (159) when its bound is
      exceeded — which closes [[0215]]'s last criterion.
- [ ] Verified by inducing on prod, the same way 0215's run was: set
      `ENRICH_MAX_EXECUTION_TIME_SECS=1`, invoke, read the log, restore.
      ⚠️ Restore in the same sitting; at 1 s the historical sweep dies every
      invocation.
