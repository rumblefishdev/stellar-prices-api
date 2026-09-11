---
id: "0281"
title: "Our ClickHouse client turns a real server exception into BadResponse(\"\") — the error code, elapsed time and query are all discarded"
type: BUG
status: completed
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
  - date: 2026-09-11
    status: completed
    who: okarcz
    note: >
      Fixed, deployed and verified by induction on production the same day.
      PR #310 (`9de447b`), deployed to EventBridge and Compute, digest moved
      `Mx3IIUuT…` → `8/XgyfNI…`. The same statement that logged
      `bad response: ` at 11:49 logged the full `Code: 159 TIMEOUT_EXCEEDED`
      at 12:57. Closes [[0215]]'s last technical criterion.
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

# 📕 DEPLOY RUNBOOK — shipping the readable-errors fix

**PR #310 is merged (`9de447b`) and NOT deployed.** Merging is not shipping —
0091 merged the proto27 fix and production stayed frozen until 0094 shipped it.

## ⚠️ The fix touches EVERY component, so decide the scope deliberately

`with_readable_errors` sits in `mtls::client_with_mtls`, the single funnel used
by the API, every worker, every probe and every operator CLI. So the benefit is
repo-wide, but the stacks deploy separately:

| stack | carries |
|---|---|
| `Prices-production-EventBridge` | enrichment, oracle, cleanup, supply, asset-discovery, coarse-sweep, 3 probes |
| `Prices-production-Compute` | ledger-processor, api-handler |

⛔ **The enrichment worker is in EventBridge, NOT Compute** — the trap that
cost a wasted step on 0215's run.

## 1. [local machine, repo] Build every asset, in ONE invocation

```bash
args=(); while IFS= read -r n; do args+=(-p "$n"); done < <(tools/scripts/lambda-assets.sh)
cargo lambda build --release --arm64 --features lambda "${args[@]}"
```

⚠️ Not `-p enrichment-worker` alone — cargo feature unification makes a
single-crate build a different binary.

## 2. [local machine, infra/] Diff, then deploy

```bash
cd infra && make diff-production
make deploy-production-eventbridge     # enrichment and friends
make deploy-production-compute         # the API and ledger-processor
```

Expect only `Code.S3Key` changes. Any Rule or IAM change → stop.

**Checkpoint.** `CodeSha256` must move from the value recorded in [[0215]]
(`Mx3IIUuT1CSHLdMsTWTGvADpmNmVcDGQ96zRnLuNjSM=`, 2026-09-11 11:13:27Z).

## 3. [local machine, AWS CLI] Re-run 0215's induction — it is the acceptance

The full procedure is in [[0215]]'s own runbook. In short: set
`ENRICH_MAX_EXECUTION_TIME_SECS=1`, invoke, read the log, remove the variable
again in the same sitting.

**This time the log must read `Code: 159 … TIMEOUT_EXCEEDED`**, not
`bad response: `. That contrast is the whole acceptance, for this task and for
0215's last criterion together.

## 4. [local machine] Confirm the restore

Bound removed, 13 env keys, `prices-production-cleanup` still `DISABLED`.

## ✅ VERIFIED ON PRODUCTION 2026-09-11 — the same failure, now readable

Deployed (EventBridge + Compute, digest `Mx3IIUuT…` → `8/XgyfNI…`) and induced
with `ENRICH_MAX_EXECUTION_TIME_SECS=1`, exactly as the failed run this
morning.

### The proof is two log lines, seventy minutes apart

```
11:49:40  "error":"clickhouse: bad response: "

12:57:59  Clickhouse(BadResponse("Code: 159. DB::Exception: Timeout exceeded:
           elapsed 1013.727811 ms, maximum: 1000 ms. (TIMEOUT_EXCEEDED)
           (version 26.3.10.60 (official build))"))
```

Same statement, same bound, same failure. The only difference is whether a
reader can tell what happened.

### A sanity invoke first, deliberately

Before inducing, one invocation at the default bound: `execution bound armed
max_execution_time_secs=120`, `enrichment pass complete, batches 5, enriched
7663`, 6.5 s, zero errors. That separates "disabling compression broke the read
path" from "the bound fired", which is precisely the entanglement that made the
morning's run hard to read. The read path is unaffected.

### The diff corroborated the fix rather than just showing churn

`make diff-production` changed **eight** EventBridge functions, not nine:
`MtlsNotafterProbeFunction` was absent — and it is the one component that
builds no ClickHouse client. Exactly the crates using the changed code changed.
`Prices-production-Compute` carried `LedgerProcessorFunction` and
`ApiHandlerFunction`, so the API's errors are readable now too.

### Production state after the run

| | |
|---|---|
| `ENRICH_MAX_EXECUTION_TIME_SECS` | **removed** — the compiled-in 120 applies |
| env keys | **13**, `CH_DOMAIN` and `MTLS_SECRET_NAME` intact |
| `prices-production-cleanup` | **DISABLED** |

## Acceptance Criteria

- [x] A failing `INSERT … SELECT` surfaces the ClickHouse error **code** and
      message to the caller; `BadResponse("")` no longer occurs for a statement
      ClickHouse recorded an exception for. **PR #310.**
- [x] Reproduced in a test against CH **26.3.10.60**, red before the fix.
      `execution_bound_error_it`, verified red with the exact message
      *"the error carries no message at all — this is the 0281 defect"* and
      green after. ⚠️ Requires a proxy and fails loudly without one.
- [x] The enrichment worker logs `TIMEOUT_EXCEEDED` (159) when its bound is
      exceeded — which closes [[0215]]'s last criterion. **Verified 12:57:59
      UTC, 2026-09-11.**
- [x] Verified by inducing on prod, the same way 0215's run was. **Done
      2026-09-11**; restored in the same sitting, bound removed, 13 keys,
      CleanupRule `DISABLED`.
