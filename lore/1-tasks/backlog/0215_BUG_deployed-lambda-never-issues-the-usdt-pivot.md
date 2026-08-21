---
id: "0215"
title: "Caddy's response_header_timeout of 30s cuts every enrichment pivot at 30.0s — the pass has failed on EVERY invocation since 2026-07-26 and nothing reported it"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0209", "0212", "0111", "0172", "0182", "0141", "0213"]
tags: ["priority-high", "effort-small", "enrichment", "clickhouse", "deploy", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Split from 0209, whose 2026-08-20 root cause this falsifies. Measured from
      system.query_log while re-measuring 0111. Cheap to fix and NOT blocked by
      0111 — which is why it is split rather than folded in.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Two root-cause hypotheses raised and FALSIFIED the same day — both are
      recorded in the task body so they are not re-run. Neither `strings` on the
      deployed bootstrap nor the Lambda's `LastModified` discriminated anything:
      `USDT_ISSUER` is present in a PRE-0172 binary too (USDT was a peg member
      then), and the artifact was deployed 2026-08-20, a week after the merge.
      What did discriminate was reading the SQL the binary EMITS out of
      system.query_log — the peg's `IN (3)` vs `IN (3, 111)`, and the resolver's
      `result_rows`. Feature flags were checked and are not involved: nothing on
      the pivot path is `#[cfg]`-gated beyond `#[cfg(test)]`. Next step is a
      LOCAL reproduction, not more prod archaeology.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      MECHANISM FOUND in CloudWatch, after THREE falsified hypotheses (all
      recorded in the body — do not re-run them). The pass fails on EVERY
      invocation with `Clickhouse(BadResponse(""))`; the XLM pivot's client gives
      up 18 s before ClickHouse finishes the same statement, `?` aborts, and the
      USDT pivot is never reached. Not the Lambda timeout — zero `Task timed out`
      in 48 h. The XLM pivot is the only statement over ~30 s, so this probably
      collapses into 0111; measure the actual timeout (client vs the Caddy mTLS
      proxy) before deciding. Also re-priced 0214: its latched alarm was hiding a
      continuous failure, and it is the only signal that could have caught this.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      ROOT CAUSE CONFIRMED — Caddy `response_header_timeout 30s`. Measured 18/18
      at exactly 30.0 s between `query_start_time` and the client error. CH and
      our client are both exonerated by measurement (all four CH socket/HTTP
      timeouts are 7200 with changed=1; the client sets no request timeout at
      all). Onset 2026-07-26 matches the table crossing 30 s after cleanup was
      disabled ~07-20. ⛔ RETRACTS the "probably collapses into 0111" note above:
      0111 would clear the symptom but leaves the limit armed, silent, and
      applying to every other caller including BE. Fix is two halves — BE raises
      the Caddy knob, we add `max_execution_time` per-caller because
      `max_execution_time = 0` today and Caddy's 30 s was accidentally the only
      bound. Sequence the timeout FIRST so 0111 has a clean baseline to measure
      against.
---

# Every invocation fails on the XLM pivot, so the USDT pivot is never reached

## Summary

**The enrichment pass fails on EVERY invocation** with
`Clickhouse(BadResponse(""))` — three per hour, continuously, not
intermittently. `enrich_peg_pivot_step` runs peg → XLM pivot → USDT pivot, each
`execute().await?`. The XLM pivot errors, `?` propagates, and the loop never
reaches the second reference. That is the whole reason
`prices.price_ohlcv_1m` has no USDT-priced rows.

⚠️ **Every signal looked healthy because ClickHouse completes the statement
anyway.** The client abandons the request; CH finishes it server-side and logs
`QueryFinish`; the 10,000 rows land. So `written_rows`, `query_log` and the
rollup alarms all read normal while the pass has not completed successfully in
at least two days.

## Evidence (prod `system.query_log`, measured 2026-08-21)

`pivot_sql` bakes its reference id into the SQL text as a literal
(`CAST({ref_id} AS UInt32) AS ref_asset_id`), so the log reads the deployed
binary's behaviour directly rather than the source's intent:

| pivot ref | runs | first seen | last seen |
|---|---|---|---|
| XLM (`id 4`) | 7,352 | 2026-08-07 09:20:15 | **2026-08-21 08:27:19** |
| USDT (`id 111`) | 6,493 | 2026-08-18 11:40:08 | **2026-08-18 14:30:04** |

USDT's entire lifetime is a 2 h 50 m window on 2026-08-18 — the run of [[0182]]
from execution host C, using a **locally built** binary. The hourly schedule has
never issued one.

⚠️ The run counts match `%ref_asset_id%` across **all** tiers, so they include
the repair tool's coarse-table work. The `first_seen`/`last_seen` boundaries
carry the finding; the totals do not.

Corroborated a third way by `written_rows`: over the 7 days to 2026-08-21 the
XLM pivot wrote 660-720K rows/day on `price_ohlcv_1m` while the USDT pivot wrote
nothing, because it was never invoked. [[0209]]'s `pivot_written = 0` therefore
describes THIS task, not a throughput limit.

Corroborated independently by the run ratio: through 2026-08-19 the log shows
`peg_insert : pivot_insert` at exactly **1:1** (70:70, 67:67, 66:66), where the
source issues one peg and **two** pivots per step.

## Root cause — the deployed artifact is not built from this source

⚠️ **Two hypotheses were measured and FALSIFIED. Do not re-run them.**

1. ⛔ **"The binary predates [[0172]]"** — falsified. `strings` on the deployed
   bootstrap finds `USDT_ISSUER` (that proves nothing: pre-0172 USDT was a *peg
   member*, so the constant was already compiled in), and the peg statement's own
   text settles it — `system.query_log` shows `quote_asset_id IN (3, 111)`
   running to 2026-08-13 10:19:39 and `IN (3)` from 2026-08-14 08:21:59 to now.
   `stable_ids()` is post-0172. `LastModified` is 2026-08-20T12:12:39, a week
   after the merge, and also proves nothing on its own ([[0141]]).
2. ⛔ **"`refs.usdt` is `None`"** — falsified. `resolve_reference_ids()` returns
   **`result_rows = 3`** on every scheduled invocation (72/day, last 2026-08-21
   09:27:38). All three reference assets come back.

### What the evidence forces

| # | measured | source implies |
|---|---|---|
| 1 | USDT pivot never sent — no `QueryStart`, no exception, 2 days | should be sent every step |
| 2 | peg emits `IN (3)` | binary is post-0172 |
| 3 | resolver returns 3 rows | `pivot_ids() == [xlm, usdt]` |
| 4 | `resolve → has_any() → run_peg_pivot_tier(&refs)`, both pivots in one `Vec`, no break | two statements per step |

Facts 2-4 make one statement per step impossible for this source. **So the
deployed artifact is not built from it.** `stable_ids()` and `pivot_ids()`
changed in the SAME commit (`6807025`), so no tagged commit produces the observed
half-state — post-0172 peg, pre-0172 pivot set. A binary built from an
**uncommitted working tree** does.

That is [[0141]] in a form neither of its existing checks catches: not a stale
artifact but a *work-in-progress* one. `LastModified` looked current and
`strings` found the constant. Record this — the discriminator that worked was
reading the **emitted SQL** out of `system.query_log`, never the artifact.


## ⛔ HYPOTHESIS 3 — "the artifact is stale" — ALSO FALSIFIED (2026-08-21)

Recorded because the falsifying step is subtle and would otherwise be repeated.

The deployed bootstrap is **byte-identical** to what the CI invocation produces
from a clean tree:

```
b8120fb22a480b73…  /tmp/enrich/bootstrap                      (downloaded from Lambda)
b8120fb22a480b73…  target/lambda/enrichment-worker/bootstrap  (rebuilt 2026-08-21)
```

⚠️ **The trap: `-p <one-crate>` is NOT a valid comparison build.** Building
`enrichment-worker` alone yields `c9e5580e…` / 12,048,000 bytes, while the CI
build (`-p` for all ten assets in ONE invocation) yields `b8120fb2…` /
12,478,560. The 430 KB delta is **Cargo feature unification** — the multi-crate
build enables extra features on shared dependencies — not different code. Same
source, two binaries. `rollup-freshness-probe` shows the same ~433 KB delta for
the same reason.

**Always reproduce with the full asset list**, exactly as
`.github/workflows/ci.yml` does:

```bash
args=(); while IFS= read -r n; do args+=(-p "$n"); done < <(tools/scripts/lambda-assets.sh)
cargo lambda build --release --arm64 --features lambda "${args[@]}"
```

So the deployed binary IS this source, correctly built. [[0141]] is not involved.

## ✅ MECHANISM FOUND 2026-08-21 — CloudWatch, after three falsified hypotheses

`/aws/lambda/prices-production-enrichment`, 48 h window:

```
09:27:52   XLM pivot   QueryStart                  (ClickHouse)
09:28:22   ERROR  Clickhouse(BadResponse(""))      (Lambda — client gives up)
09:28:40   XLM pivot   QueryFinish                 (ClickHouse completes anyway)
```

The client abandons the request **18 s before** ClickHouse finishes it. Repeats
every hour, every invocation, across the whole window.

**The three errors per hour share one `requestId`** — Lambda async-invoke retry
(1 + 2). So it is not three batches per invocation; it is **three invocation
attempts, each dying after one peg + one XLM pivot**. The statement counts
confirm it exactly: peg 72/day, XLM pivot 72/day, oracle 192/day.

⛔ **NOT the Lambda timeout.** Zero `Task timed out` in 48 h. `Timeout` is 300 s,
`MemorySize` 512 MB.

### ✅ ROOT CAUSE — `response_header_timeout 30s` in Caddy

`/srv/app/infra-hetzner/Caddyfile`, the `reverse_proxy clickhouse:8123` transport
block:

```
dial_timeout             10s
response_header_timeout  30s     ← this one
read_timeout             7200s
write_timeout            7200s
```

`response_header_timeout` bounds how long the **upstream may take to send its
first response byte**. An `INSERT … SELECT` sends nothing until it completes. The
XLM pivot runs **43.6-47.3 s**. So Caddy severs the connection at 30 s, every
time, and the Rust client sees an empty body — `BadResponse("")`.

**Measured 18/18, gap exactly 30.0 s** (`query_start_time` from `system.query_log`
against the CloudWatch error timestamp, 6 h window, 2026-08-21):

| query_start | client error (UTC) | gap |
|---|---|---|
| 06:19:30 | 06:20:00 | 30.0 |
| 07:22:35 | 07:23:05 | 30.0 |
| 08:26:35 | 08:27:05 | 30.0 |
| 09:27:52 | 09:28:22 | 30.0 |
| 11:27:57 | 11:28:27 | 30.0 |

…and every other sample identical.

**Onset matches the mechanism.** Failures begin 2026-07-26 (6 that day, 67 the
next, then a flat 72/day for 26 days). Cleanup was disabled ~2026-07-20 and the
table began growing from 14.0M rows; by 07-26 the pivot had crossed 30 s and it
has never dropped back under, because the table only grew (736.46M now).

**Both other layers are exonerated, by measurement:**

- **ClickHouse.** `http_send_timeout`, `http_receive_timeout`, `send_timeout`,
  `receive_timeout` are all **7200** with `changed = 1` — deliberately raised.
  The only 30 in `system.settings` is `http_headers_read_timeout`, which bounds
  reading the *request* headers.
- **Our client.** `mtls.rs` sets no request timeout at all — only
  `pool_idle_timeout(8s)` and `pool_max_idle_per_host(2)`. hyper's legacy client
  has no default. It was never going to give up on its own.

⚠️ **The Caddyfile's own comment shows how this happened.** It states the policy
— *"Timeouts cover the longest legitimate analytical … 7200 s window"* — and
sets `read_timeout`/`write_timeout` accordingly. `response_header_timeout` at 30 s
is the one knob inconsistent with that policy. Its stated rationale ("tighter
than the CH-side timeout so Caddy releases the upstream") is correct for a
streaming `SELECT`, where headers arrive in milliseconds, and simply does not
apply to `INSERT … SELECT`.

### ⚠️ Scope is wider than this task

The ceiling applies to **every** long statement through that proxy — our operator
CLIs (sdex-backfill, coarse-repair, the 0182 runner, all on the same mTLS client)
and **BE's own queries**, since it is their shared host. Any of them taking over
30 s to first byte dies the same silent way.

### ⛔ It does NOT collapse into [[0111]] — earlier guess retracted

0111 option 1 would drop the pivot to ~3-4 s and clear the symptom incidentally.
That is not a reason to skip the timeout fix:

1. **The trap stays armed.** The limit remains, invisible, and silent. The next
   thing on that path to exceed 30 s repeats this outage.
2. **It fixes one caller.** Every other tool and BE keep the ceiling.
3. **The hazard is the FAILURE MODE, not the 30 s.** ClickHouse succeeds, the
   client errors, the rows land, nothing reports it. Staying under the line does
   not change that.
4. 🔴 **Ordering — this decides the sequence.** Bounding the scan first changes
   two things at once (cost drops AND failures stop), so 0111's before/after
   cannot be attributed. Fix the timeout first for a clean baseline.

**Where the real guarantee comes from.** No fixed ceiling can be guaranteed
un-hit. Today the statement's duration scales with **total table size**, which
grows without bound, so *any* limit is crossed eventually. After 0111 option 1 it
scales with **one partition**, which is bounded. That structural change — not a
bigger number — is the guarantee.

### Why nobody saw it

[[0214]] — the enrichment errors alarm latched 24 days ago and never
re-notified. A continuous, every-invocation failure produced no page.

## Implementation — two halves, two owners

### Half 1 — BE's config, one line

`response_header_timeout` **30s → 7200s**, aligning it with `read_timeout`,
`write_timeout` and the policy the file's own comment states. Not a loosening of
their policy — a correction of the one setting that contradicts it. ⚠️ Shared
host, shared config: **request it, never edit it ourselves.**

### Half 2 — ours, and it needs nothing from BE

Caddy's 30 s was accidentally the only bound on a runaway query:
`max_execution_time` is **0** (unlimited, unchanged). Removing Caddy's ceiling
without replacing it leaves a two-hour hole.

Set `max_execution_time` **on our client** instead. ClickHouse then enforces it
and **throws a real exception with an error code** the worker logs, rather than
an empty body indistinguishable from a network blip. `timeout_overflow_mode` is
already `throw`.

⚠️ **It must be per-caller, not a constant.** ~120 s suits the scheduled Lambda
(2.6x headroom over today's 45.6 s worst, inside the 300 s Lambda budget so a
runaway surfaces as a clean CH error rather than a Lambda timeout). The operator
CLIs legitimately run far longer statements — a single global value breaks them
and reintroduces this failure class from the other direction.

### Also

- Make an empty-body error distinguishable in the logs from an ordinary network
  failure, so a recurrence is diagnosable without a 26-day archaeology dig.
- Track worst statement duration against the configured ceiling as a metric, so
  drift toward the limit is visible before it crosses.
- ⚠️ Both `system.settings` readings were taken as `default` via CHQ. The worker
  connects as `prices_writer`, an XML user that can carry a different profile.
  Confirm against `system.settings_profile_elements` before telling BE "there is
  no bound".

## Acceptance Criteria

- [x] The source is shown correct — `pivot_ids() == [xlm, usdt]` and the peg
      excludes USDT, both green (2026-08-21). The defect is the artifact.
- [ ] A test asserts `enrich_peg_pivot_step` issues TWO pivot statements, so a
      silently-narrowed pivot set fails the suite instead of the quote leg.
- [ ] After the fix, `system.query_log` shows `CAST(111 AS UInt32) AS
      ref_asset_id` running on the hourly schedule, outside any hand-run window.
- [ ] `peg_insert : pivot_insert` reaches 1:2 on `price_ohlcv_1m`.
- [ ] USDT-quoted `_1m` rows are measurably written — `written_rows > 0` on the
      USDT pivot, recorded before/after.
- [ ] `max_execution_time` is set per-caller on our client, and an exceeded bound
      produces a logged ClickHouse exception — verified by inducing, not inferred.
- [ ] `CleanupRule` verified `DISABLED` before and after the deploy.
- [ ] A missing reference asset fails loudly instead of silently narrowing
      `pivot_ids()`.

## Out of scope

- The 1.56M peg-valued `_1m` rows already on prod — that is [[0212]].
- The full-table scan and the 556.78M XLM-quoted backlog — that is [[0111]].
  ⚠️ Fixing this task adds a **third** statement per batch to a pass that
  already reads 739.68M rows each time, so it makes 0111 worse. Sequence
  accordingly.
