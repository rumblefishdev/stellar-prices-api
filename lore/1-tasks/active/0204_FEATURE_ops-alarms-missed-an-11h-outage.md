---
id: "0204"
title: "Ops alarms missed an 11.5 h outage — no free-space alarm on the shared CH volume, and the DLQ alarm fires once then goes quiet"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0202", "0203", "0137", "0142", "0056", "0201", "0182", "0172", "0196"]
tags:
  ["priority-high", "effort-small", "observability", "alarms", "resilience", "milestone-M2"]
milestone: 2
links:
  - "../../../apps/infra/src"
history:
  - date: 2026-08-14
    status: backlog
    who: okarcz
    note: >
      Spawned from 0202. The 2026-08-13 disk-full stall ran 11.5 h and was
      discovered from three Lambdas panicking, not from any alarm that watches
      the actual condition. Two concrete gaps, both cheap to close.
  - date: 2026-08-17
    status: backlog
    who: okarcz
    note: >
      Added gap 3 — [[0142]] shipped `prices-clickhouse-drift` but nothing runs
      it, and it watches a condition no existing alarm sees (0137 measures
      whether the rollups produce data; a drifted MV produces data perfectly
      well while producing the wrong numbers). Recorded the connection
      constraint that decides where it can run, and the three design traps:
      drift is a standing condition not an event, exit 1 carries two different
      severities, and all-six-MISSING is a grant gap rather than an outage.
  - date: 2026-08-17
    status: active
    who: okarcz
    note: >
      Activated for **gap 1 only** — the free-space alarm — ahead of the
      0201/0182 repair campaign scheduled for the morning of 2026-08-18. This
      task's own gap-1 note makes it a precondition for that run ("it should not
      start without this alarm in place and a word with BE").
      ⚠️ The premise of that warning has moved and the task text is now stale:
      it was written at ~91% used, and the volume measured **430.6 GiB free /
      75.5% used** on 2026-08-17. So the campaign has headroom it did not have,
      and the alarm is no longer "the run cannot start" but "the run is
      unwatched". Still worth having before the morning, because the failure
      mode is BE filling the volume independently — which is exactly what
      happened on 2026-08-13 and is not something we control.
      Gap 3 (scheduled drift check) is NOT in this activation.
  - date: 2026-08-17
    status: active
    who: okarcz
    note: >
      Gap 2 pulled into the same activation on the operator's call. Its link to
      the campaign is real but indirect: if the shared volume fills during the
      10-15 h run, ingest Lambdas fail and the DLQ fills — and the gap-2 defect
      is precisely that Slack cannot distinguish 1 from 91. Gap 1 and gap 2 are
      two links in one chain, so closing only one was the weaker call.
      Delivered as a threshold ladder ([10, 50] above the existing >= 1 alarm)
      plus the AC-3 runbook note. Gap 3 remains, so this task stays active.
  - date: 2026-08-17
    status: active
    who: okarcz
    note: >
      Gap 3 scoped but deliberately NOT built. It has no bearing on the
      0182/0201 campaign — 0142's prod drift run came back clean the same day,
      and the campaign writes data, not MV definitions, so it cannot introduce
      drift. Two findings recorded against gap 3 that invert its cost estimate:
      the mTLS/Lambda objection is already paid for by gap 1, and `system.tables`
      is grant-FILTERED rather than denied (verified: 32 tables and 7
      `create_table_query` values readable by a prices-only user), so no new
      grant and no BE dependency. ⚠️ The "Hetzner cron is cheapest" paragraph in
      the gap 3 section is now marked stale.
      Left open on purpose: re-notification for a standing condition has no
      cheap answer — drift is binary, so gap 2's ladder trick does not transfer,
      and the alternatives all cost something. That decision should be taken
      before any code, not during it.
  - date: 2026-08-19
    status: active
    who: okarcz
    note: >
      Added gap 4, absorbed from 0182's last open acceptance criterion — a
      data-level check that close_usd is RIGHT, not merely present. close_usd
      has been wrong on prod through three different doors (0172's peg, 0196's
      oracle mis-attribution, 0182's epoch boundary) and two of the three never
      touch the writer, so 0172's writer tests cannot be the guard. Same
      category as gap 3 rather than gaps 1-2: correctness, not liveness — gap
      3's own sentence about a drifted MV producing data perfectly well while
      producing wrong numbers transfers directly. Lands on
      rollup-freshness-probe alongside gap 1, which has already paid the
      aws-mtls wiring that gap 3 identifies as the blocker for anything
      AWS-side, and keeps it off eventbridge-stack and its CleanupRule. Checks
      both directions because 0182's own repair caused the inverse failure on
      2026-08-19 — 157 candles zeroed with nothing to refill them. Three traps
      recorded: it is a standing condition so the alarm must not latch; it must
      be scoped to the quote leg because exotic-quoted zeros are by design
      (~74M on _1h alone); and the pre-epoch par window is legitimately ~1.0 so
      the ratio check must be bounded to the post-break era.
---

# Ops alarms missed an 11.5 h outage

## Summary

The 2026-08-13 stall ([[0202]]) was found by reading Lambda panic logs after the
fact. Every alarm that fired was a **downstream symptom**; nothing watched the
condition itself, and the one alarm that returned to OK did so for the wrong
reason. Three gaps, all small.

## Gap 1 — no free-space alarm on the ClickHouse host

`system.disks` had the answer the entire time. We learned about a full disk from
`asset-discovery`, `supply` and `ledger-processor` failing.

⚠️ **This matters more here than on a dedicated host: the volume is SHARED with
BE and we are 3.3% of it** (58.93 GiB of 1.72 TiB; BE's `default` is 951 GiB).
We cannot control what fills it and cannot free meaningful space ourselves — so
**warning time is the only lever we have**. It sat at 91.4% used after recovery,
meaning the next comparable event repeats this.

- Alarm on free space with enough headroom to act (the incident consumed ~150
  GiB, so a threshold at ~15-20% free would have given hours of warning).
- ⚠️ **[[0201]] writes to this volume for 10-15 h.** It should not start without
  this alarm in place and a word with BE.

## Gap 2 — the DLQ alarm fires once and never re-notifies

Slack showed `ApproximateNumberOfMessagesVisible >= 1`. By morning the DLQ held
**91**. Nobody reading Slack could tell 1 from 91.

- Re-notify on growth, or alarm on a rate/threshold ladder rather than a single
  `>= 1` edge.

## ⚠️ And the recovery signal was actively misleading

The lag alarm returned to **OK** at 07:56 — truthfully, the queue *was* empty.
But it emptied partly by messages **being given up on**, not processed: the age
series eased (26,155 → 26,117 → 25,969) exactly as the DLQ filled.

**An empty queue is not a processed queue.** Recovery must be verified on the
**data** (`max(timestamp)` on `price_ohlcv_1m`), never on alarm state — the same
lesson [[0137]] already records for the rollup alarm, arriving through a new
door.

⚠️ Note that even the data check is insufficient alone: on 2026-08-13
`max(timestamp)` was 63 s behind while **eight hourly buckets were missing**. A
completeness signal is [[0203]]'s scope; this task covers the disk and the DLQ.

## Gap 3 — the rollup drift check is manual and nothing runs it

[[0142]] built `prices-clickhouse-drift`: read-only, exits 0 when all six MVs
match `rollups.sql` and are `APPEND`, 1 otherwise. **Nothing runs it.** A check
nobody runs is a check that does not exist, and this one covers a condition no
other alarm sees — [[0137]] watches whether the rollups are *producing data*,
which a drifted MV does perfectly well while producing the wrong numbers.

Cheapest home is a cron on the Hetzner host against `localhost:8123`, which
sidesteps the connection problem entirely: prod's HTTP endpoint is mTLS-only
behind Caddy and `prices_clickhouse::client()` builds a plaintext client, so
anything running from AWS needs the crate's `aws-mtls` feature wiring first.
Folding it into 0137's Lambda is the tidier end state and the more expensive one.

### ⚠️ The paragraph above is STALE as of 2026-08-17 — the Lambda route now wins

Two findings from the gap 1 / gap 2 work invert that cost comparison. Neither is
re-derivable by reading the task, so do not act on the "Hetzner cron is cheapest"
line without reading these first.

**1. The mTLS objection is already paid for.** Gap 1 put a ClickHouse-reading,
CloudWatch-publishing path into `rollup-freshness-probe`, which runs
`client_from_lambda_env("prices")` on a 15-minute EventBridge schedule. The
`aws-mtls` wiring the paragraph above treats as unbuilt work now exists and is in
production use. `drift.rs` is already a library module in `prices-clickhouse`
(the CLI in `bin/` is a thin wrapper), so the probe can call it directly.

**2. Folding it in needs NO new EventBridge rule, and that is a safety property,
not just convenience.** A new rule means deploying `eventbridge-stack.ts`, which
is where `CleanupRule` lives — synth confirms that template still emits
`State: ENABLED` while the live rule is DISABLED, so any deploy of it can
silently re-enable cleanup ([[0200]]). Reusing the probe's existing schedule
keeps gap 3 inside `observability-stack.ts`, exactly as gaps 1 and 2 were kept.
⚠️ A Hetzner cron avoids that hazard too, but trades it for touching the prod
host directly and for a check that lives outside CDK, invisible to every alarm
and review path we have.

**3. The privilege question is answered, and it is NOT the `system.disks`
situation.** Measured on 26.3.10.60 (2026-08-17) against a user holding exactly
`GRANT SELECT ON prices.*` — the probe's identity:

| read | prices-only user | note |
|---|---|---|
| `system.disks` | ⛔ `ACCESS_DENIED` | and cannot be granted — see gap 1 |
| `count() FROM system.tables WHERE database='prices'` | ✅ `32` | same as `default` |
| `create_table_query` for the 7 MVs | ✅ `7` | the column drift actually compares |

`system.tables` is **grant-filtered, not denied**, and our grant covers the whole
`prices` database — so the filtering removes nothing we need. Gap 3 requires **no
new grant and no BE dependency**. (This is also why the `MISSING` note below
matters: filtering is real, it just does not bite *this* identity.)

### The one genuinely open design question — re-notification

⚠️ **This is the reason gap 3 was not built alongside gaps 1 and 2**, and it does
not have a cheap answer. Drift is a standing condition, so it hits the same
CloudWatch wall gap 2 hit: an alarm notifies on a **state transition**, latches,
and then says nothing while the condition persists.

Gap 2 escaped that with a threshold ladder because a DLQ has **depth** to climb.
Drift has no depth — it is binary. To make it re-notify you would need to publish
something that keeps rising, e.g. *hours since drift was first detected*, and
that requires **state the probe does not keep** (each invocation is
independent). Options, none obviously right:

- persist "first seen" in a ClickHouse table and derive the age from it — real
  state, but it makes a read-only check a writer;
- accept one latched alarm plus a separate daily digest;
- alarm on the transition only, and rely on drift being rare and on the runbook.

**Decide this before writing code.** Picking it implicitly while implementing is
how gap 2's defect got shipped in the first place.

Three things this task's own findings say to design in:

- ⚠️ **Drift is a standing condition, not an event.** It stays wrong until a
  person fixes it, so a single-edge alarm is the gap 2 failure again — the DLQ
  alarm that fired once while the queue grew to 91.
- ⚠️ **Exit 1 is not one severity.** `CRITICAL` (an MV that lost `APPEND`) means
  history is being destroyed on every refresh and should page; `DRIFT` means
  someone's edit silently did not land and can wait for morning. The severity is
  in the output, not the status code — an alarm that reads only the exit code
  throws that distinction away.
- ⚠️ **All six reporting `MISSING` is a grant gap, not six dead tiers.**
  `system.tables` is filtered by grant. The binary prints a note when it sees
  that shape; an alarm must not page as if the chain were gone.

## Gap 4 — nothing watches whether the USD values are RIGHT

Added 2026-08-19 from [[0182]], which needed a "guard against re-introduction"
and found this task already holds the shape of it.

`close_usd` has now been wrong on prod through **three different doors**, and no
alarm saw any of them:

| how it broke | task | what the writer test would have caught |
|---|---|---|
| USDT priced at a $1 peg it no longer held | [[0172]] | ✅ the writer |
| Reflector rows mis-attributed to the USDT identity | [[0196]], and [[0168]] before it | ❌ never touches the writer |
| a reset epoch 19 h below its pivot reference | [[0182]] | ❌ never touches the writer |

Two of the three bypass the writer entirely, so a unit test on the writer — which
[[0172]] already has — cannot be the guard. **The condition is in the data, so
the check has to be in the data.**

This is **the same category as gap 3, not the same as gaps 1-2.** Gaps 1-2 and
[[0137]] watch liveness and capacity: is data arriving, is there disk, is the
queue growing. Gap 3 and gap 4 watch correctness — and gap 3's own sentence
transfers with one noun changed: *0137 measures whether the rollups produce data;
a drifted MV produces data perfectly well while producing the wrong numbers.* A
USDT candle valued at par produces data perfectly well too.

### What it asserts — both directions, and the second one is new

- **No USDT-quoted candle carries `close_usd / close ≈ 1.0`.** The original
  defect: the peg re-applied, by any writer.
- **No USDT-quoted candle sits at `close_usd = 0` with a representable `close`.**
  ⚠️ Added because [[0182]]'s repair *caused* this on 2026-08-19 — 157 candles
  zeroed with nothing to refill them. The inverse failure is as real as the
  original, and an asymmetric check would have passed while the damage stood.
  The `close` bound is the arithmetic one (`rate × close` rounding to zero at
  `Decimal(38, 14)`, i.e. ~`5e-14`), **not** a round number that looks small —
  0182's runbook records a first attempt at `1e-11` that counted dust which had
  priced perfectly well.

### Where it runs

`packages/rollup-freshness-probe`, alongside gap 1. Same reasoning as gap 1, and
the three costs are already paid there: the `aws-mtls` wiring that gap 3's note
identifies as the blocker for anything AWS-side, an EventBridge schedule and a
`PutMetricData` grant scoped by `cloudwatch:namespace`, and dead-probe cover from
`addWorkerHealthAlarms`. The crate is already split so the query construction and
the metric shaping are unit-testable without the AWS SDK.

⚠️ **Reusing `Prices/Rollup` keeps this off `eventbridge-stack`** — the same
reason gap 1 landed there. That stack owns `CleanupRule`, and every deploy
touching it can silently re-enable cleanup ([[0200]]).

### Design traps — two inherited, one specific

- ⚠️ **Standing condition, not an event** — identical to gap 3 and to gap 2's
  structural defect. A wrong `close_usd` stays wrong until a person repairs it,
  so the alarm must keep its OK action on every rung or it latches and goes
  silent while the population grows.
- ⚠️ **Scope it to the quote leg, not to "all candles".** Exotic-quoted rows sit
  at `close_usd = 0` **by design** — there is no USD reference and no tier can
  price them ([[0182]] measured ~74M such rows on `_1h` alone). A check that
  counts every zero would breach permanently on healthy data.
- ⚠️ **The pre-epoch window is a legitimate `≈ 1.0`.** USDT was at *measured*
  par from 2021-02 until the June 2022 break ([[0172]]), and rows below
  `2021-02-07 19:00` deliberately keep `close × $1`. So the ratio check must be
  bounded to the post-break era or it fires on correct history — the mirror of
  the epoch mistake that made [[0182]] necessary twice.

## Acceptance Criteria

- [x] Free-space alarm on the CH host, threshold chosen to give hours of
      warning, routed to the same Slack channel as the existing ops alarms —
      **built, not deployed**. `prices-{env}-ch-disk-free` at 20% free, on the
      existing `snsAction` (so the same `#stellar-prices-api-bot` channel as
      every other ops alarm). See "Implementation — gap 1" below
- [x] DLQ alarm distinguishes 1 from 91 — re-notifies on growth or uses a
      threshold ladder — **built, not deployed**. Threshold ladder: rungs at 10
      and 50 above the existing `>= 1` alarm. See "Implementation — gap 2"
- [x] Runbook note: an ingest stall is verified recovered on the DATA, never on
      alarm state, and freshness alone does not prove completeness — added to
      `docs/runbooks/running-ingestion-components.md` as
      "Verifying recovery after an ingest stall", with both queries and the
      redrive/cleanup traps
- [ ] ⚠️ Alarms verified by inducing the condition, not by reading the CDK — the
      0137 lesson that an alarm must be tested against the failure it exists for.
      **NOT met for either gap, and for gap 1 it cannot be before the deploy.**
      - *Gap 1:* what **is** verified by inducing the condition is the
        **privilege** constraint — an IT creates a least-privileged user and
        asserts it really is denied `system.disks` and really can call the
        filesystem functions. The disk condition itself is only exercised in unit
        tests against measured numbers. Filling a shared 1.72 TiB volume to prove
        an alarm is not something to do to BE; the honest test is to raise the
        threshold above current free space after deploy, confirm it fires into
        Slack, and put it back.
      - *Gap 2:* the ladder **is** inducible cheaply and without touching prod
        data — send N dummy messages to the DLQ, watch the rungs fire in order,
        then purge. Worth doing on the first deploy. Not done yet.
- [ ] `prices-clickhouse-drift` runs on a schedule and reports somewhere a person
      reads, re-notifying while drift persists, with `CRITICAL` separated from
      `DRIFT` rather than collapsed into "exit 1" *(gap 3 — not in this
      activation)*
- [ ] **Gap 4** — a data-level USD-correctness check runs on a schedule and
      alarms on **both** directions: no USDT-quoted candle at
      `close_usd / close ≈ 1.0` in the post-break era, and none at
      `close_usd = 0` with a `close` above the `Decimal(38, 14)` underflow
      bound. Scoped to the quote leg, so exotic-quoted zeros do not breach it.
      *(gap 4 — not in this activation)*
- [ ] **Gap 4 verified by inducing it** — write a par-valued USDT candle and a
      zeroed one with a representable `close` into a test fixture and confirm
      each breaches; the [[0137]] lesson applied to this alarm too. Closes
      [[0182]]'s last acceptance criterion.

## Implementation — gap 1 (2026-08-17)

Branch `feat/0204_ch-disk-freespace-alarm`. Named for the gap rather than the
task slug, so gaps 2 and 3 can each take their own branch without reuse.

1. **`packages/rollup-freshness-probe/src/disk.rs`** — new module.
   `disk_query()`, `DiskUsage`, `free_percent()`, `disk_metrics()`, and a
   feature-gated `publish_disk()`. Same split as the rollup half: pure shaping
   and query construction compile in every build and are unit-tested; the AWS
   SDK stays behind `--features lambda`.
2. **`src/main.rs`** — reads the disk **after** the rollup metrics are already
   published. See design decision 3; the ordering is load-bearing.
3. **`infra/src/lib/stacks/observability-stack.ts`** — `ChDiskFreeAlarm`
   (`prices-{env}-ch-disk-free`), `Minimum` over 15 min, 1-of-2, LESS_THAN,
   `NOT_BREACHING`, alarm + OK actions on the existing ops SNS topic.
4. **`infra/src/lib/types.ts`** — `opsAlarms.chDiskFreePercent` with validation
   (a number in `(0, 100)` exclusive) and the threshold rationale.
   **`infra/envs/production.json`** — set to `20`.
5. The `rollup-freshness-probe` dead-probe `impact` string now says the disk
   alarm goes dark with it too, because it does.

**Tests:** 18 unit (7 new) + 4 integration (2 new), all green. `cargo clippy
--all-targets -D warnings` clean on both the default and `lambda` feature sets;
the `lambda` bin was force-rebuilt to confirm, since `main.rs` is entirely behind
that feature and the default build never compiles it.

⚠️ **The integration tests do not run in CI** — there is no ClickHouse service in
`.github/workflows/ci.yml` and nothing passes `--ignored`. This is pre-existing
(the 0137 IT has always been in the same position), not introduced here, but it
means "CI green" says nothing about the two ITs. They were run locally against
26.3.10.60. Worth its own task.

### Design decisions

**From plan**

1. **Ride on `rollup-freshness-probe` instead of a new probe.** It already holds
   the mTLS ClickHouse client, a CloudWatch publish path, a 15-minute schedule
   and dead-probe alarm cover. A new probe means a new EventBridge rule, and
   `CleanupRule` lives in that stack — see decision 2.

**Emerged**

2. **Publish into the existing `Prices/Rollup` namespace, though these are not
   rollup metrics.** The probe role's `PutMetricData` grant is conditioned on
   `cloudwatch:namespace == Prices/Rollup` in `eventbridge-stack.ts`, so a
   `Prices/ClickHouse` namespace would have required editing that stack — and
   ⚠️ **that is the stack that owns `CleanupRule`**. Confirmed by synth that the
   template still carries `State: ENABLED` while the live rule is DISABLED, so
   any deploy of it can silently re-enable cleanup, and cleanup during the
   0182/0201 campaign shreds the campaign's output. Reusing the namespace keeps
   the whole change inside `observability-stack.ts`. Revisit after the campaign.
3. **The disk read runs AFTER the rollup publish, and must not be moved.** Both
   halves propagate errors, so whichever runs second can only cost itself.
   Reading the disk first would mean a disk-side failure aborts the invocation
   before any `RollupLagSeconds` datum lands — and those alarms are
   `NOT_BREACHING`, so all seven would score healthy while a rollup sat frozen.
   That is the 0136 blind spot, reintroduced by an unrelated feature. Commented
   as load-bearing at the call site.
4. **`filesystemAvailable()`, not `filesystemUnreserved()`.** Available is what
   an unprivileged writer can still consume (root-reserved blocks already
   excluded), which is the question the alarm asks. Unreserved additionally
   subtracts ClickHouse's in-flight merge reservations, so it moves with merge
   activity and would make the alarm jitter for reasons unrelated to the volume
   filling up.
5. **Capacity reading as zero fails the invocation rather than publishing.**
   Zero capacity is a broken reading, not a full disk. Publishing `0.0` would
   page falsely; publishing nothing would let `NOT_BREACHING` score it healthy.
   Neither is acceptable in a task about false-OK, so it errors and the probe's
   own `-errors` alarm carries it.
6. **Bound at 20%, not 15% and not 25%.** 20% of 1.72 TiB is ~352 GiB against an
   incident that consumed ~150 GiB, so it fires with roughly twice the incident
   still free. ⚠️ The 2026-08-17 measurement is 430.6 GiB free = **25.0%**, so a
   25% bound would have been in ALARM from the moment it shipped. Unit-tested
   against both the measured steady state and a replay of the incident.

## Implementation — gap 2 (2026-08-17)

Same branch. A **threshold ladder**, not a re-notification mechanism.

1. **`observability-stack.ts`** — `ledgerProcessorDlqEscalationAlarms`, one
   `prices-{env}-ledger-processor-dlq-{depth}` alarm per configured depth on the
   same `AWS/SQS ApproximateNumberOfMessagesVisible` metric.
2. **`types.ts`** — `opsAlarms.dlqEscalationDepths`, validated as strictly
   increasing integers above 1. **`production.json`** — `[10, 50]`.
3. **`running-ingestion-components.md`** — the recovery-verification section
   (AC 3), covering the misleading OK and the freshness-≠-completeness trap.

Verified by synth: both rungs present, `GreaterThanOrEqualToThreshold`,
`notBreaching`, alarm **and** OK actions on the ops topic, and rung 1 keeps its
logical id `LedgerProcessorDlqAlarmD32FFD0F` — the change is purely additive.

### Design decisions — gap 2

**Emerged**

7. **A ladder, not a smarter single alarm.** ⚠️ The defect is structural: a
   CloudWatch alarm notifies on a **state transition**, so once the `>= 1` alarm
   is latched in ALARM it is silent however far the queue climbs. No threshold on
   one alarm fixes that. Separate alarms have separate transitions, so growth
   produces new messages.
8. **Every rung keeps its OK action, and this is not optional.** A rung with no
   route back to OK latches on first breach and is then permanently silent — it
   would reproduce this exact defect one level up. The price is one OK per rung
   on a redrive to empty; that noise is deliberate.
9. **Rung 1 is untouched — same logical id, same alarm name.** Only its
   description changed (an in-place update). Renaming it would force a
   replace/recreate and discard its alarm history, which is not something to do
   to a live alarm the day before a repair campaign.
10. **Depths 10 and 50.** 1 = a ledger was dropped, always worth a look; 10 = not
    a lone poison pill, something systemic; 50 = an outage in progress. The
    2026-08-13 event reached **91**, so it would have lit every rung — which is
    the readable signal that was missing.
11. **Rung 1 stayed out of the config array.** `dlqEscalationDepths` lists only
    the rungs *above* it, so the existing alarm cannot be accidentally retuned or
    removed by a config edit.

## Issues Encountered

- 🔴 **`system.disks` is unusable from this probe, and it would have deployed
  green and failed on every prod invocation.** The obvious query is `SELECT
  free_space, total_space FROM system.disks`. Measured on 26.3.10.60 against a
  user holding exactly `GRANT SELECT ON prices.*` — the shape of the `ingestion`
  identity (`prices_writer`) the probe connects as:

  ```text
  Code: 497. DB::Exception: Not enough privileges. To execute this query,
  it's necessary to have the grant SELECT ON system.disks. (ACCESS_DENIED)
  ```

  And the grant cannot be added: `prices_writer` is XML-defined and that access
  storage is read-only (`ACCESS_STORAGE_READONLY`) — the same wall [[0182]] hit
  trying to get it `ALTER FREEZE PARTITION`. Fixing it that way means an edit to
  BE's `users.xml` plus a reload, i.e. a cross-team dependency.

  `filesystemAvailable()` / `filesystemCapacity()` are **functions**, carry no
  table grant, and return the same numbers for the default disk (256 786 214 912
  vs 256 786 149 376 — the drift is concurrent writes between the two reads).
  This also preserves the property `main.rs` already documented: the probe
  touches no `system.*` table. Pinned by an IT that creates a least-privileged
  user and asserts **both** halves, so a future "simplification" back to
  `system.disks` fails locally instead of on prod.

- **`filesystemFree()` does not exist** on 26.3.10.60 (`UNKNOWN_FUNCTION`). The
  three that do are `filesystemAvailable`, `filesystemUnreserved`,
  `filesystemCapacity`. Recorded so the next reader does not try it.

- **`cdk synth` fails on a clean checkout** with `«CannotFindAsset» Cannot find
  asset at web/portal/dist` — `PortalHostingStack` ([[0185]], merged as #218)
  needs the portal bundle on disk. `npx nx build portal` first. Unrelated to this
  task, but it blocks any synth and will catch the next person out.

- **The pre-commit hook fails on a tree that has not been re-installed since
  #218.** `@nx/vite`, `@nx/react` and `@nx/web` entered `package.json` with that
  merge; without `npm ci` the hook dies on `Unable to resolve local plugin with
  import path @nx/vite/plugin` and the commit is refused.

## Future Work

- The two integration tests never run in CI (no ClickHouse service, no
  `--ignored`). Pre-existing and wider than this task.
- Move the disk metrics to their own `Prices/ClickHouse` namespace once the
  0182/0201 campaign has landed and `eventbridge-stack.ts` is safe to deploy.
