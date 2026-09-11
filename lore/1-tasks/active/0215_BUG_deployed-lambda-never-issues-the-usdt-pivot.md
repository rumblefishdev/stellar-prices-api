---
id: "0215"
title: "Caddy's response_header_timeout of 30s cuts every enrichment pivot at 30.0s — the pass has failed on EVERY invocation since 2026-07-26 and nothing reported it"
type: BUG
status: active
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
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      BE REPLIED AND CONFIRMED — they are deploying the bump. They verified the
      30 s from Caddy's admin API (the in-memory value the process enforces),
      not from the file, and they hold an independent alibi for the 2026-07-26
      onset: no restart since 06-29, last deploy 07-06, config unchanged since
      07-02. Two things change on our side. (1) The mechanism is TIME TO FIRST
      BYTE, not a property of INSERT ... SELECT — they were bitten on 07-02 by a
      plain heavy SELECT — which widens the affected class to every statement
      that computes before it emits, and puts the 26.4 s oracle insert next in
      line. (2) Our "Caddy was the only bound on a runaway" rationale for half 2
      is RETRACTED as stated; keep max_execution_time for BE's better reason —
      a typed exception with an error code instead of an empty body. Also
      recorded: their single-file bind mount pins an inode, so an edit plus a
      reload would have been a phantom deploy. Verification protocol agreed —
      they ping post-deploy, we confirm from CloudWatch and query_log.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      BEFORE-STATE RECORDED, and it changed the finding. Two discoveries. (1) The
      30 s ceiling cuts TWO statements, not one — the oracle statement (p95
      28.7 s, max 37 s, 737.6M rows read) crosses 2-6 times/day and 38 times on
      08-15, and because the oracle tier runs FIRST those invocations issue no
      peg or pivot work at all. Confirmed by counting the same event two ways:
      oracle-over-30s matches (72 minus peg runs) exactly on six consecutive
      days. So the earlier "next in line, a matter of weeks" framing was wrong,
      as was an in-session guess blaming 08-14/15's low peg counts on the 0182
      window. (2) max_execution_time resolved per profile — it is 30 on
      read_only (so prices_reader, our API, is already bounded server-side) and
      ABSENT on prices_write_ddl, so the worker really is unbounded and half 2's
      premise holds. It also proves half 2 must live in the client: the Lambda
      and the operator CLIs share the single prices_writer user, so no
      server-side setting can give them different ceilings.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      FIXED AND VERIFIED. BE deployed at 14:46:18 UTC with the --force-recreate;
      zero BadResponse since, a 46.4 s XLM pivot returned to the client, and the
      USDT pivot ran on the schedule for the first time ever (1,895 rows, then
      ~17/batch — so the "USDT backlog" of 0209/0212 was ~1,900 rows and the leg
      was simply never asked). Three ACs now green. The new bottleneck is ours
      and was predicted here before the deploy: Lambda 300 s, 3 batches per
      attempt, ~2.16M rows/day (3x baseline, ~258 days for the XLM leg) — so 0111
      is still required. Two new findings: 36% of every invocation is spent in
      the oracle tier draining a few hundred rows out of a 737M-row scan, which
      is the sharpest 0111 argument yet; and the coarse sweep behind main.rs:167's
      `?` has NEVER executed, starving 0114's remedy — spawned as 0218. The peg
      statement's constant 1,236 rows/batch spawned as 0219. Remaining here:
      max_execution_time per-caller, and the two guard tests.
  - date: 2026-09-10
    status: active
    who: okarcz
    note: >
      Activated to settle what is left. The **defect itself is closed** — BE's
      Caddy bump landed 2026-08-21 and 4 of 9 ACs are green. What remains is the
      hardening half, which is entirely ours: `max_execution_time` per-caller on
      the client (half 2), a test that `enrich_peg_pivot_step` issues TWO pivot
      statements, and a loud failure when a reference asset is missing. Also
      re-verifying that the fix has HELD for the three weeks since, rather than
      assuming it.
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

## ✅ BE CONFIRMED IT 2026-08-21 — and found the trap that would have voided the fix

They checked the box, agreed the change, and are shipping it. Three things in
their reply change or harden this task; a fourth is worth stealing outright.

### 1. The 30 s is enforced — verified from the running process, not the file

They read it out of **Caddy's admin API** — the in-memory value the process is
actually executing: `"response_header_timeout": 30000000000` (30 s in ns). That
is the strongest confirmation available, and it matches our 18/18 measurement at
exactly 30.0 s. We were measuring a real limit, not a config artifact.

### 2. The 2026-07-26 onset is ours, and now has an independent alibi

Caddy has **not restarted since 2026-06-29**, last deploy **07-06**, config
unchanged since **07-02**. Nothing happened on their side on 07-26. The
growth-crossing explanation recorded above is therefore no longer our inference
alone — it is the only surviving one.

### 3. ⚠️ CORRECTION — the mechanism is TIME TO FIRST BYTE, not `INSERT … SELECT`

This task framed the limit as something `INSERT … SELECT` runs into. That is too
narrow. **The same limit bit BE on 2026-07-02 on a plain heavy `SELECT`** — a
~6 min scan over ~9.5 bn rows, 504 to the client while the query kept running
server-side, identical signature. Conversely a *streaming* `SELECT` returns
headers in milliseconds and may then grind for two hours untouched.

So the affected class is **every statement that computes before it emits its
first byte** — our pivots and peg INSERTs, `count_candidates`, any aggregating
`SELECT`, and every operator-CLI statement of that shape. The discriminator is
buffering, not the verb.

🔴 **The oracle insert is not "next in line" — it is ALREADY crossing.** Measured
the same day: p95 **28.7 s**, max **37 s**, 2-6 crossings/day and **38 on
08-15**. See the baseline section below, which confirms it to the row. A second
statement class has been failing this whole time and would have been written off
as a network blip.

### 4. 🔴 THE FINDING TO STEAL — a single-file bind mount pins an inode

**The bump alone would not have been enough.** Their Caddyfile is bind-mounted as
a **single file**, and a single-file mount pins an *inode*. Their deploy writes
via rsync (temp + rename), which replaces the inode. So since 2026-07-06 the
container has been reading a file that no longer exists at that path — host inode
`16777224`, container inode `16777223`, dated 07-02. Both copies happen to say
the same thing, so nothing broke.

But **editing the file and running `caddy reload` would have been a phantom
deploy**: repo green, box green, limit still 30 s, and this outage continuing
behind a closed ticket. They are shipping the change with a one-time
`--force-recreate` plus a deploy fix so the inode stops drifting, and they verify
from the admin API — *"the file already lied once"*.

➡️ **Generalise it.** Same failure family as [[0141]] (a stale lambda artifact
where every signal read success) and `cdk synth` running `dist/` instead of
`src/`: **verify the state the running process holds, never the file you edited.**
Three independent instances now — a standing rule, not a war story.

### 5. Their deploy touches the shared proxy

`--force-recreate` drops every mTLS connection for a few seconds. Harmless for the
hourly Lambda; **do not have a long operator CLI statement in flight during their
window** (sdex-backfill, coarse-repair, the 0182 runner). Ask for the window.

### ⚠️ Scope is wider than this task

The ceiling applies to **every** statement through that proxy that buffers before
it emits — our operator CLIs (sdex-backfill, coarse-repair, the 0182 runner, all
on the same mTLS client) and **BE's own queries**, since it is their shared host.
Any of them taking over 30 s to first byte dies the same silent way; BE's
2026-07-02 `SELECT` is the confirmed second instance (see BE's reply, §3 above).

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

⛔ **The original rationale here is RETRACTED — BE refuted it, 2026-08-21.** This
section used to argue: *"Caddy's 30 s was accidentally the only bound on a runaway
query, so removing it leaves a two-hour hole."* BE's counter is correct and is the
cleanest statement of the asymmetry the Caddyfile comment only hinted at:

> A runaway query **streams**, so it sailed past the 30 s already. What died was
> precisely the statement doing the work and buffering its result. The limit was
> never a safeguard — it selected the exact opposite of what it looks like it
> selects.

⚠️ **One amendment, which makes half 2 more necessary rather than less.** BE's
point holds for streaming `SELECT`s. A runaway `INSERT … SELECT`, or a runaway
aggregation, buffers — and *that* is our entire workload. So for the enrichment
path the 30 s genuinely was the only bound, and after the bump nothing bounds it:
`max_execution_time` is **0** (unlimited, unchanged).

**Keep half 2, for BE's better reason.** Set `max_execution_time` **on our
client**. ClickHouse then enforces it and **throws a real exception with an error
code** the worker logs, rather than an empty body indistinguishable from a network
blip. `timeout_overflow_mode` is already `throw`. The value is in the *failure
mode* — hazard 3 below — not in restoring a ceiling.

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

## 📌 BASELINE — recorded 2026-08-21, BEFORE BE's deploy

Captured deliberately while the ceiling is still armed, because every AC here and
in [[0111]] says "recorded before/after" and this state is unrecoverable once the
bump lands. Read from `system.query_log` and `system.parts` only — no `FINAL`
scan of the hot table.

### Table

| | |
|---|---|
| `price_ohlcv_1m` | **736,707,689 rows / 18.45 GiB / 102 partitions / 322 parts** |

### Statements on `price_ohlcv_1m`, 7 days

| stmt | runs/day | avg | max | rows read | written/run |
|---|---|---|---|---|---|
| **XLM pivot** | 72 | **45.6 s** | 49.5 s | 687.6 M | **exactly 10,000** |
| **oracle** (`oracle_prices` join) | ~190 | 26.2 s (p95 **28.7**) | **37 s** | **737.6 M** | ~600-1,100 |
| peg | 66-72 | 14.4 s | 15.4 s | 423.0 M | ~1,237 |
| USDT pivot | **0 — absent from all 7 days** | | | | |

`BadResponse("")` in CloudWatch: **144 in 48 h = exactly 3/hour**, one per XLM
pivot run. The XLM pivot writes **exactly `batch_size`** every run (440,000/44,
660,000/66, 720,000/72 — perfectly linear), so it is `LIMIT`-bound every time and
has never exhausted its candidates.

## 🔴 The ceiling cuts TWO statements, not one — and the second one was missed

⚠️ **Corrects this task's own framing.** The oracle statement is not "next in
line" behind the XLM pivot; **it has been dying too**, and because the oracle
tier runs FIRST, an invocation that dies there issues **no peg and no pivot work
at all**.

Confirmed by counting the same event two ways. If an oracle statement exceeds
30 s, `?` aborts the pass before the peg-pivot tier, so peg runs must fall short
of the 72 invocations/day (1 EventBridge + 2 async retries) by exactly that many:

| date | oracle `> 30 s` | peg runs | 72 − peg | |
|---|---|---|---|---|
| 08-20 | 0 | 72 | 0 | ✅ |
| 08-19 | 2 | 70 | 2 | ✅ |
| 08-18 | 6 | 66 | 6 | ✅ |
| 08-17 | 5 | 67 | 5 | ✅ |
| 08-16 | 5 | 67 | 5 | ✅ |
| **08-15** | **38** | **34** | **38** | ✅ |

Six consecutive exact matches, including a day where the oracle ran ~2.5 s slower
(p50 28.4 s vs 26.0) and 38 of 181 statements crossed. That is not correlation.

⛔ **Also corrects an in-session guess:** 08-14/08-15's low peg counts were
attributed to the 0172/0182 window. Wrong — same mechanism, worse day.

**The oracle statement is a load-sensitive coin flip at the line**: p50 26.0-26.4,
**p95 28.7**, max 37, against 30. What varies day to day is cluster load, not
table size. And it reads **737.6 M rows — the whole table** — so 0111's growth
argument applies to it identically, and it will cross more often, not less.

`oracle_prices` (the `ASOF LEFT JOIN` target, `ch_enrich.rs:794`) appears in no
other statement, so the classification is exact.

## ✅ `max_execution_time` resolved per profile — the worker IS unbounded

The earlier reading was taken as `default` and this task flagged it as unverified
(the worker connects as `prices_writer`, an XML user with its own profile).
Resolved from `system.settings_profile_elements`:

| profile | users | `max_execution_time` |
|---|---|---|
| `read_only` | `api_reader`, `dev_read`, **`prices_reader`** | **30** |
| `prices_write_ddl` | **`prices_writer`** | **absent** |

`prices_write_ddl` carries nine elements — memory, insert-block sizes, all four
7200 s socket timeouts — and **no execution bound**; unlike `dict_loader` it does
not inherit `default` either. **So half 2's premise holds: the worker and every
operator CLI run unbounded.**

Two things fall out that were previously assertions:

- **Our read path is already bounded at 30 s, server-side, by ClickHouse.** So
  `prices_reader` gets a real `TIMEOUT_EXCEEDED` where `prices_writer` gets an
  empty body. The failure asymmetry this task is about was already half-solved on
  the API side — half 2 extends an existing house policy rather than inventing
  one, and 30 is that policy's number.
- 🔴 **It must live in the client, and now there is a hard reason.** The
  scheduled Lambda and the operator CLIs **share the single `prices_writer`
  user**, so there is no server-side place to give them different ceilings. A
  profile-level setting cannot express this.

## Verification after BE's deploy — agreed protocol, and what to expect

BE ping the thread once deployed; we confirm from our side. That two-sided check
is the hardest signal available, and it is deliberately **not** a re-read of the
config file.

Confirm in this order:

1. `Clickhouse(BadResponse(""))` stops in `/aws/lambda/prices-production-enrichment`.
2. `CAST(111 AS UInt32) AS ref_asset_id` appears in `system.query_log` on the
   hourly schedule — the USDT pivot issued for the first time.
3. `peg_insert : pivot_insert` moves **1:1 → 1:2**.
4. USDT pivot `written_rows > 0` on `price_ohlcv_1m`, recorded before/after.

### ⚠️ Predict this now, so it is not misread as a new defect

With the 30 s gone, the binding constraint becomes the **Lambda's 300 s**. A
peg-pivot batch is ~14 s peg + ~46 s XLM pivot + the (cheap, sort-key-pruned)
USDT pivot + ~8 s `count_candidates` ≈ **70 s**, after the oracle tier has taken
its share. So expect roughly **3-4 batches of the configured 20** per invocation,
then `Task timed out` — and [[0026]]'s `EnrichmentPassDurationMs` alarm firing
where it has been silent.

That is progress, not a fix: a **visible** failure signal replacing an invisible
one, ~3-4x the drain rate, and USDT unblocked. It is **not** the pass completing.
[[0111]]'s "20 batches → ~4.8 M rows/day → ~116 days" was always conditional on
bounding the scan; **the Caddy fix alone does not buy it.** Sequence unchanged:
their fix → clean baseline → 0111.

⚠️ **Tell BE about the load change.** Today the pass dies after one batch, so it
scans ~72 times/day. Completing passes will scan several times that on a disk we
share and are 3.3% of — the same disk BE filled on 2026-08-13, costing an 11.5 h
ingest stall. It is a courtesy heads-up, and it is a second argument for taking
0111 immediately after.

## ✅ FIXED AND VERIFIED — 2026-08-21 14:46:18 UTC

BE deployed the bump (`response_header_timeout` 30s → 7200s) with the one-time
`--force-recreate` their inode finding required. Verified from our side inside
the hour, against the baseline recorded above.

### The proof is two adjacent lines

```
15:20:17  pivot_xlm   46.4 s  10000 rows   QueryFinish
15:20:31  pivot_usdt  13.6 s   1895 rows   QueryFinish
```

A **46.4 s statement returned to the client and execution continued.** That has
been impossible since 2026-07-26. The second line is the first `_1m` row ever
priced by the USDT pivot on the schedule.

| check | baseline | after |
|---|---|---|
| `BadResponse("")` | 3/hour for 26 days | **0** |
| XLM pivot | 45.6 s, client cut at 30.0 s | 43.4-46.7 s, **all `QueryFinish`, all returned** |
| USDT pivot | **absent from every scheduled run** | **6 runs**, 1,895 rows then ~17/batch |
| peg : pivot | 1:1 | **1:2** |
| peg-pivot batches per attempt | 1, then death | **3** |
| oracle | 2-6 crossings/day killed the pass | 25.1-27.8 s, all completing |

⚠️ **The USDT backlog was ~1,900 rows — not a backlog at all.** Its first batch
wrote 1,895 and every batch since writes ~17. The leg was never behind, it was
never being *asked*. This closes the substance of [[0209]] and [[0212]]: their
premise was a throughput limit, and the truth was an unreached statement. The
USDT pivot also costs **13.6 s, not 46 s**, because `quote_asset_id = 111` prunes
on the sort key's 2nd column — the XLM pivot is expensive because XLM-quoted rows
*are* the table, not because pivots are expensive.

### The new bottleneck is ours: the Lambda's 300 s

```
REPORT … Duration: 300000.00 ms  Memory Size: 512 MB  Max Memory Used: 54 MB  Status: timeout
```

All three attempts, one shared `RequestId` (async retry 1 + 2). **Predicted in
this task before the deploy and confirmed to the batch count** — 3 peg-pivot
batches per attempt at ~72 s each (peg 14 + XLM pivot 45 + USDT pivot 13.6).

⚠️ `Status: timeout` is a **field on the REPORT line**, not a separate
`Task timed out` message, on `provided:al2023`. Grepping for the old string
returns zero and reads as "no timeouts". It produced a wrong reading here; don't
repeat it.

**Memory is 54 MB of 512 MB** — the pass is purely time-bound, so raising memory
buys nothing.

### Throughput — 3x, and still not enough

| | baseline | after |
|---|---|---|
| XLM pivot runs | 3/hour | **9/hour** (3 attempts × 3 batches) |
| rows/day | 720 K | **~2.16 M** |
| XLM backlog (556.78 M) | ~774 days | **~258 days** |

Candidate counts corroborate: 656,569,249 → 656,535,211 → 656,506,042 across the
three attempts, ~30 K each, matching 3 × `batch_size` exactly.

🔴 **36% of every invocation is spent in the oracle tier** — 105-108 s of the
300 s, draining **499, 517 and 3,564 candidates**. Three statements, each reading
the full 737 M-row table. This is the sharpest single argument for [[0111]]
option 1 yet recorded: the pass spends over a third of its budget scanning the
whole table to find a few hundred rows.

### 🔴 The coarse sweep has NEVER run — [[0114]]'s remedy is starved

`main.rs:167` is `let stats = pass.run().await?;` and the sweep sits **after** it,
so it is unreachable both ways: before the fix `run()` returned `Err` and `?`
propagated; now the Lambda is killed inside `run()`. `"enrichment pass complete"`
appears in none of the three attempts, and no `"coarse sweep complete"` line
exists in the window.

Its budget arithmetic cannot save it either — `budget = min(120 s, deadline − now
− 60 s)`, and a pass that runs to the hard deadline leaves that at **0**.

Spawned as **[[0218]]**. It matters because 0114 — the coarse tables carrying no
USD values — is the defect [[0111]] itself calls "more serious and outranking".

### Observation spawned as [[0219]]

The peg statement writes **exactly 1,236 rows on every batch**, eight consecutive
identical counts, and the baseline shows the same (54,414/44 = 1,236.7). New rows
would vary. It looks like the same rows are re-selected and re-written every
batch, inflating `version` for no gain. Pre-existing — not caused by this fix.

## ✅ OUR SIDE RE-CONFIRMED — 2026-09-10, 20 days after the fix

Read-only from `system.query_log` as `dev_read` (`readonly = 1`). This is our
half of the two-sided check; BE's half is the admin-API reading.

| date | peg | pivots | peg:pivot | XLM | USDT | max pivot | not `QueryFinish` |
|---|---|---|---|---|---|---|---|
| 09-03 | 109 | 218 | 1:2 | 109 | 109 | 2.58 s | **0** |
| 09-04 | 477 | 954 | 1:2 | 477 | 477 | 2.72 s | **0** |
| 09-05 | 518 | 1036 | 1:2 | 518 | 518 | 3.19 s | **0** |
| 09-06 | 496 | 992 | 1:2 | 496 | 496 | 3.28 s | **0** |
| 09-07 | 295 | 590 | 1:2 | 295 | 295 | 2.89 s | **0** |
| 09-08 | 67 | 134 | 1:2 | 67 | 67 | 0.85 s | **0** |
| 09-09 | 67 | 134 | 1:2 | 67 | 67 | 0.80 s | **0** |
| 09-10 | 63 | 128 | 1:2 | 64 | 64 | 0.84 s | **0** |

**1:2 on every one of eight days, and XLM:USDT exactly 1:1.** The baseline was
1:1 peg:pivot with USDT absent from all history. Zero statements ended in
anything but `QueryFinish` — no exception, and nothing abandoned.

### ⚠️ The 09-08 drop is the backlog ENDING, not a regression

Runs fall ~500/day → 67/day on 09-08, which is close enough to the old
72/day-one-batch-then-death signature to be misread. `written_rows` separates
them:

| | 09-06 | 09-09 |
|---|---|---|
| pivot rows/day | **4,606,769** | **11,143** |
| rows per run | **4,644** | **83** |

A `LIMIT`-bound run writes its full `batch_size`; 83 rows means the statement
ran out of candidates, not out of time. The pass is now keeping up with live
traffic and finishing early instead of exhausting its 20 batches. The
556.78M-row XLM backlog [[0111]] projected at ~258 days is **drained**.

# 📕 DEPLOY RUNBOOK — inducing the `max_execution_time` bound

The AC says *verified by inducing, not inferred*, and the induction is the only
part of half 2 that cannot be done from a test: it has to show ClickHouse
raising a real exception that the worker logs. Deploy-gated, so it is the
operator's to run.

**Do not induce with a slow query.** After [[0111]] nothing on this path takes
more than ~3.3 s, so there is no statement long enough to trip a sane bound.
Move the bound instead — same mechanism, one reversible env var.

### 0. [local machine, AWS CLI] Pre-check — has the deploy already happened?

```bash
export AWS_PROFILE=soroban-explorer AWS_REGION=eu-central-1
aws sts get-caller-identity --query Arn --output text
aws lambda get-function-configuration \
  --function-name prices-production-enrichment --region eu-central-1 \
  --query '{LastModified:LastModified,Bound:Environment.Variables.ENRICH_MAX_EXECUTION_TIME_SECS}' \
  --output json
```

**Checkpoint, and it decides whether step 1 is needed:**

- `Bound` is `null` **and** `LastModified` predates the PR #305 merge
  (2026-09-10) → the new binary is NOT deployed. Do step 1.
- `Bound` is `"120"` → already deployed and configured. **Skip step 1**, go
  to step 2.
- `Bound` is `null` but `LastModified` is recent → ambiguous, and this is
  exactly [[0141]]. The variable is optional, so its absence does not prove the
  binary is old. Treat as not-deployed and do step 1; a redundant deploy is
  cheap, a phantom induction is not.

⚠️ Do not skip this because the PR is merged. Merging is not shipping — 0091
merged the proto27 fix and production stayed frozen until 0094 deployed it.

### 1. [local machine, this repo] Deploy PR #305

⚠️ **Already merged** — `8726c76`, 2026-09-10. This step is the deploy only;
there is nothing left to merge.

Normal compute deploy. ⚠️ [[0141]] — confirm the deployed asset actually
changed; `deploy-production-compute` does not build.

### 2. [local machine, AWS CLI] Drop the bound below the statement duration

```bash
aws lambda update-function-configuration \
  --function-name prices-production-enrichment \
  --region eu-central-1 \
  --environment "Variables={$(aws lambda get-function-configuration \
      --function-name prices-production-enrichment --region eu-central-1 \
      --query 'Environment.Variables' --output json \
    | python3 -c 'import json,sys; v=json.load(sys.stdin); v["ENRICH_MAX_EXECUTION_TIME_SECS"]="1"; print(",".join(f"{k}={v}" for k,v in v.items()))')}"
```

⚠️ `update-function-configuration` **replaces** the whole `Environment` block —
read the current one and edit it, never pass a bare
`Variables={ENRICH_MAX_EXECUTION_TIME_SECS=1}`, which would wipe `CH_DOMAIN`,
`MTLS_SECRET_NAME` and the six enrichment vars.

### 3. [local machine, AWS CLI] Invoke and read the log

Wait for the next hourly run, or invoke directly. Expect in
`/aws/lambda/prices-production-enrichment`:

- a ClickHouse exception naming **`TIMEOUT_EXCEEDED` (code 159)**, with the
  query in the message
- ⛔ **NOT** `BadResponse("")`, and **NOT** a bare `Status: timeout` on the
  REPORT line

That contrast IS the acceptance: the same failure that was invisible for 26
days now arrives with an error code attached.

### 4. [local machine, AWS CLI] Restore

Same command as step 2 with `"120"`. Then confirm one clean `QueryFinish` pivot
pair on the next run before closing the AC.

⚠️ **Do not leave the bound at 1 s.** Every enrichment statement dies while it
is there, which re-creates the outage this task exists to fix — visibly this
time, but still. Restore in the same sitting.

### 5. [BE thread] Post our side of the two-sided check

The last unticked item after the induction, and the measurements are already
done — they are in the *"We confirm the errors stopped"* criterion above:
`BadResponse("")` 3/hour → **0**, `Status: timeout` every invocation → **0**,
and invocations/day **72 → 24**, which is the sharpest of the three because
async retries only exist when the attempt before them failed.

While you are there: nudge BE on **[[0277]]** — the `xdr-parser` bump to
`stellar-xdr 28`. Protocol 28 votes 2026-09-16 17:00 UTC and we cannot move
until they do.

## ✅ The `max_execution_time` MECHANISM is already proven — 2026-09-10

Measured before deploying anything, and it shrinks what the induction still has
to show. Re-read today from `system.settings_profile_elements`:

| profile | `max_execution_time` |
|---|---|
| `read_only` (`api_reader`, `dev_read`, `prices_reader`) | **30** |
| `prices_write_ddl` (**`prices_writer`** — the worker) | **absent → unbounded** |

`timeout_overflow_mode` is `throw` (unchanged from the default). So half 2's
premise still holds 20 days on.

**And ClickHouse demonstrably throws on this cluster.** `system.query_log`,
`exception_code = 159`, 30 days — 23 events across 9 days, every one a
`dev_read` query that outran the profile's 30 s:

```
Code: 159. DB::Exception: Timeout exceeded:
elapsed 30000.860859 ms, maximum: 30000 ms. (TIMEOUT_EXCEEDED)
```

That is exactly what half 2 buys, already happening: an error code, the elapsed
time and the bound that was crossed — against `BadResponse("")`, which carried
none of it and was indistinguishable from a network blip for 26 days.

⚠️ **The bound OVERSHOOTS.** It is checked between blocks, not preemptively:
one of these ran **43.9 s** against a 30 s limit before the check fired. So a
configured 120 s can kill at ~120-135 s, and the value must sit far enough under
the Lambda's 300 s to leave room for that. 120 s does.

### What this leaves for the induction

Not the mechanism — only the plumbing:

1. the worker's client actually **sends** the option, and
2. the worker **logs** the resulting exception rather than swallowing it.

⛔ Neither can be rehearsed as `dev_read`: `readonly = 1` refuses a settings
change outright (code 164), so the URL-parameter path only exists for
`prices_writer`. It needs the deploy.

## Acceptance Criteria

- [x] The source is shown correct — `pivot_ids() == [xlm, usdt]` and the peg
      excludes USDT, both green (2026-08-21). The defect is the artifact.
- [x] A test asserts `enrich_peg_pivot_step` issues TWO pivot statements, so a
      silently-narrowed pivot set fails the suite instead of the quote leg.
      **PR #304, merged 2026-09-10** (`b7496dd`). `plan_peg_pivot_step` splits
      planning from sending, and `plan_issues_one_peg_and_two_pivots` asserts one
      peg, two pivots, refs `[5, 7]` in order, and each pivot's ref id as the SQL
      literal that made this defect readable in `system.query_log` at all.
      ⚠️ It runs in CI; the end-to-end coverage that also catches this does NOT
      — every test in `ch_enrich_it.rs` is `#[ignore]` and needs a live
      ClickHouse. That is [[0275]].
- [x] After the fix, `system.query_log` shows `CAST(111 AS UInt32) AS
      ref_asset_id` running on the hourly schedule, outside any hand-run window.
      **Six runs from 15:20:31 UTC, 2026-08-21.**
- [x] `peg_insert : pivot_insert` reaches 1:2 on `price_ohlcv_1m`. **Verified
      2026-08-21 — every peg is followed by an XLM pivot and a USDT pivot.**
- [x] USDT-quoted `_1m` rows are measurably written — `written_rows > 0` on the
      USDT pivot, recorded before/after. **Before: 0 across all history. After:
      1,895 on the first batch, ~17/batch since.**
- [ ] `max_execution_time` is set per-caller on our client, and an exceeded bound
      produces a logged ClickHouse exception — verified by inducing, not inferred.
      **Code in PR #305** (2026-09-10): 120 s on the scheduled worker's client via
      `ENRICH_MAX_EXECUTION_TIME_SECS`; the operator CLIs stay unbounded on
      purpose, which is what "per-caller" means here. ⏳ **The induction is
      outstanding** and is the operator's to run against prod — though the
      section above narrows it to the plumbing; the mechanism is measured.
      ⚠️ **Scope decided 2026-09-10 — the worker only, not the CLIs.** After
      [[0111]] the worst statement on this path is ~3.3 s against the old 45.6 s,
      so 120 s is ~37x headroom and the number is NOT load-bearing. What earns it
      is the other end: it sits under the Lambda's 300 s, so a hang is a
      ClickHouse exception naming the query instead of a bare `Status: timeout`.
      ⛔ `max_execution_time = 0` is ClickHouse's UNLIMITED, not "instant". A
      zeroed knob silently restores the unbounded state; zero now sets no option
      and logs at `warn`, pinned by
      `a_zero_execution_bound_is_unlimited_not_instant`.
- [x] The bump is confirmed live **from Caddy's admin API**, not from the
      Caddyfile — their single-file bind mount desynced the two once already, and
      a file-only check cannot tell a real deploy from a phantom one.
      **Read from the running process 2026-09-10**, `GET /config/` on
      `app-caddy-1`'s `admin localhost:2019`:

      ```
      upstream clickhouse:8123
         dial_timeout             10s
         read_timeout             7200s
         response_header_timeout  7200s
         write_timeout            7200s
      ```

      ⚠️ **AC amended — was "BE confirm".** Read by us rather than BE, which
      satisfies the AC's stated reason in full: the reason was admin-API-vs-file,
      never who holds the terminal, and this IS the running process. BE's
      independent reading is no longer load-bearing.
      ⛔ Caddy stores durations in **nanoseconds**, so the raw field is
      `7200000000000` against a broken `30000000000` — 13 digits vs 11. Convert
      before reading it; this AC exists because a reading was wrong once.
      ⛔ BE's repo Caddyfile also says `7200s`. That proves nothing and is the
      trap: the container spent from 2026-07-06 reading a file that no longer
      existed at that path (host inode `16777224`, container `16777223`).
- [ ] We confirm the errors stopped from CloudWatch and `system.query_log`, and
      report it back in the thread. Two-sided, because neither side alone can see
      both halves.
      ✅ **Both measurement halves done 2026-09-10.** `system.query_log`: see the
      re-confirmation section above. CloudWatch, `ReadOnlyAccess`, 7-day window
      on `/aws/lambda/prices-production-enrichment`:

      | | baseline | now |
      |---|---|---|
      | `BadResponse("")` | 3/hour for 26 days (144 per 48 h) | **0** |
      | `Status: timeout` | every invocation, 2026-08-21 | **0** |
      | invocations/day | **72** (1 EventBridge + 2 async retries) | **24** |

      🔑 **The invocation count is the sharpest of the three.** Async retries
      exist only because the attempt before them failed, so 72/day → exactly
      24/day is an independent measurement of the same recovery — taken from
      Lambda's own metric rather than from either log — and it agrees with
      `system.query_log` to the hour.
      ⚠️ `Status: timeout` is a FIELD on the REPORT line on `provided:al2023`,
      not a `Task timed out` message; grepping the old string returns zero and
      reads as success. Filtered on the field.
      ⏳ Outstanding: posting it in the thread.
- [x] `CleanupRule` verified `DISABLED` before and after the deploy.
      **After, 2026-09-10:** `prices-production-cleanup` → `State: DISABLED`,
      `Schedule: cron(0 3 * * ? *)`. **Before** is carried by the readings of
      2026-08-25 and 2026-08-28, both `DISABLED`, either side of the 08-21 bump.
      ⚠️ A fresh "before" was unrecoverable by the time this was checked — the
      deploy was 20 days earlier. Recorded as what it is rather than implied.
- [x] A missing reference asset is **named in the logs** instead of silently
      narrowing `pivot_ids()`. `resolve_reference_ids` warns with the absent
      code (PR #304, merged 2026-09-10).
      ⚠️ **AC amended 2026-09-10 — was "fails loudly".** Decided against a hard
      failure: `stable_ids()`/`pivot_ids()` flattening a `None` away is the
      defect, and what made it a 26-day outage was the SILENCE, not the
      tolerance. Partial reference sets are legitimately legal — bootstrap
      discovers assets in arbitrary order — so a hard error would refuse to start
      on a fresh registry. The warn removes the silence without inventing a
      bootstrap/steady-state distinction nothing else in the worker draws.

## Out of scope

- The 1.56M peg-valued `_1m` rows already on prod — that is [[0212]].
- The full-table scan and the 556.78M XLM-quoted backlog — that is [[0111]].
  ⚠️ Fixing this task adds a **third** statement per batch to a pass that
  already reads 739.68M rows each time, so it makes 0111 worse. Sequence
  accordingly.
