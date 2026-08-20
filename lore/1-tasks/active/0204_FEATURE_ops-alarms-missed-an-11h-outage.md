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
  - date: 2026-08-19
    status: active
    who: okarcz
    note: >
      Gap 4 BUILT, not deployed. New usd_sanity.rs module on
      rollup-freshness-probe publishing two counts over a rolling 7-day window
      (UsdtPegAppliedCandles, UsdtStrandedCandles), plus two alarm ladders at
      [1, 100, 10000] with alarm AND OK actions on every rung. The ladder is
      what unblocked gap 4 ahead of gap 3: gap 3's own analysis says drift is
      binary so gap 2's trick cannot transfer, but a COUNT of wrong candles has
      depth — a regressed writer keeps adding to it — so it transfers unchanged
      and gap 4 never needed the re-notification decision gap 3 is still waiting
      on. Three design choices worth knowing: the USD read runs LAST in the
      invocation so a correctness check can never blind the liveness checks
      beside it; the USDT leg is resolved by code+issuer rather than a
      hard-coded asset_id, with resolved_legs carried in the same row so an
      unresolvable identity is refused rather than reported healthy; and the
      stranded direction has a 48 h grace matching BE's own TVL window, because
      enrichment fills close_usd asynchronously and an ungraced metric would
      never read zero. 🔴 Running the IT against a real ClickHouse found a
      silent deserialization corruption that every unit test and all four clippy
      feature combinations missed: a scalar subquery types as Nullable(UInt64),
      RowBinary prefixes a null flag byte, and reading it into u64 returned 256
      for a true count of 1 with no error — the probe would have refused every
      healthy run. Fixed with toUInt64(ifNull(...)) and pinned. 31 unit tests,
      11 ITs, clippy clean on all four feature combinations.
  - date: 2026-08-19
    status: active
    who: okarcz
    note: >
      Gap 3 BUILT, not deployed — the last gap. 0142's drift.rs is called as a
      library from the same probe, so no new Lambda, no new EventBridge rule, no
      new grant, nothing outside observability-stack. Three alarms:
      mv-drift-critical, mv-drift, mv-drift-unreadable. The re-notification
      question that had blocked gap 3 since 2026-08-17 was DECIDED by the
      operator: one Slack message is enough. What made that reasonable rather
      than a corner cut is that the gap 2 analogy does not transfer — the DLQ was
      DETERIORATING while its alarm was quiet (1 became 91), whereas one drifted
      MV stays one drifted MV until a person fixes it, so latching costs
      "somebody may forget" rather than "we are blind to an escalation". Both
      alarm descriptions say explicitly that they latch. The one severity that
      DOES compound — an MV that lost APPEND, which deletes pre-rolled history on
      every refresh (the 0090/0095 loss) — is split onto its own metric and its
      own alarm for exactly that reason. Also added MvDriftUnreadable: this
      task's own findings noted that all-six-MISSING is a grant gap rather than
      six dead tiers but left it as a warning, and a warning is not enough when
      the alarm would page at maximum urgency with the wrong diagnosis;
      system.tables is grant-FILTERED, so the probe counts visible prices
      objects and suppresses the drift counts when it can see none. Two comments
      in main.rs and disk.rs claiming "the probe touches no system.* table" were
      made false by this and are corrected. 40 unit tests, 15 ITs including a
      negative control against the 7 real MVs and an induced non-APPEND MV,
      clippy clean on all four feature combinations. ⚠️ All four gaps are now
      BUILT and NONE are deployed; deploy Prices-production-Observability only.
      Task stays active until the deploy and the gap 1/2 induction.
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
      ✅ **Met for gaps 3 and 4** — both induce their real defect against a live
      ClickHouse (a par-valued candle, an aged zero, an edited MV declaration, a
      live MV without `APPEND`), and doing so for gap 4 found a silent
      deserialization corruption no unit test could reach.
      ⛔ **Still NOT met for gaps 1 and 2**, and for gap 1 it cannot be before
      the deploy.
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
- [x] **Gap 3** — `prices-clickhouse-drift` runs on a schedule and reports
      somewhere a person reads, with `CRITICAL` separated from `DRIFT` rather
      than collapsed into "exit 1" — ✅ **built 2026-08-19, not deployed.**
      Three alarms: `-mv-drift-critical`, `-mv-drift`, `-mv-drift-unreadable`.
      ⚠️ **AMENDED, not met as originally written.** The criterion said
      *"re-notifying while drift persists"*; the operator decided on 2026-08-19
      that one Slack message is enough, on the argument that drift does not
      deteriorate the way the DLQ did — so a latched alarm costs "somebody may
      forget", not "we are blind to an escalation". Both alarm descriptions
      state that they latch. The one severity that *does* compound, a lost
      `APPEND`, is split onto its own alarm precisely because that argument does
      not cover it. See "Implementation — gap 3", design decisions 1 and 2.
- [x] **Gap 4** — a data-level USD-correctness check runs on a schedule and
      alarms on **both** directions: no USDT-quoted candle at
      `close_usd / close ≈ 1.0` in the post-break era, and none at
      `close_usd = 0` with a `close` above the `Decimal(38, 14)` underflow
      bound. Scoped to the quote leg, so exotic-quoted zeros do not breach it.
      — ✅ **built 2026-08-19, not deployed.** `usd_sanity.rs` + two alarm
      ladders at `[1, 100, 10000]`. See "Implementation — gap 4"
- [x] **Gap 4 verified by inducing it** — ✅ 2026-08-19.
      `usd_sanity_counts_both_induced_defects` writes a par-valued candle and an
      aged zero into a real ClickHouse and asserts each is counted; six further
      ITs induce the grace boundary, dust, an exotic leg, a `version + 1` repair
      and an unresolvable identity. ⚠️ Doing this **found a silent
      deserialization corruption** no unit test could reach — see "Issues
      encountered" under gap 4. Closes [[0182]]'s last acceptance criterion.

## Implementation — gap 3 (2026-08-19)

Same branch. Rides the existing probe: no new Lambda, no new EventBridge rule,
no new grant, and nothing outside `observability-stack.ts`.

**The re-notification question was DECIDED by the operator: one Slack message is
enough.** See design decision 1 for the argument that made that reasonable
rather than a compromise.

1. **`packages/rollup-freshness-probe/src/mv_drift.rs`** — new module.
   `drift_metrics()`, `visible_objects_query()`, `describe()`, feature-gated
   `publish_drift()`. All the comparison work is [[0142]]'s `drift.rs`, called
   as a library; this module only splits the report into severities.
2. **`src/main.rs`** — runs **last** in the invocation. See design decision 3.
3. **`infra/src/lib/stacks/observability-stack.ts`** — three alarms:
   `prices-{env}-mv-drift-critical`, `-mv-drift`, `-mv-drift-unreadable`.
   `Maximum` over 15 min, 1-of-2, `>= 1`, `NOT_BREACHING`, alarm **and** OK
   actions. No new config: `>= 1` is the only sensible threshold for "an MV is
   wrong", so there is nothing to tune and nothing to drift out of sync.

### Design decisions

**From plan**

1. **One alarm transition, accepted deliberately — and the gap 2 analogy is
   what does NOT transfer.** This task exists partly because the DLQ alarm
   notified once and went quiet while the queue climbed to 91. The instinct is
   that drift has the same defect. It does not, quite: **the DLQ was
   deteriorating** and the silence hid an escalation, whereas one drifted MV
   stays one drifted MV until a person fixes it. So a latched alarm here costs
   *"somebody may forget"*, not *"we are blind while it gets worse"* — a much
   cheaper failure, and one a ticket closes.

   That is what makes option 1 defensible rather than a corner cut, and it is
   why the stored `first_seen` state (a new table, an admin step, and a
   read-only check turned into a writer) was not worth buying.

   ⚠️ **Both alarm descriptions say so explicitly** — *"fires once and stays
   latched; silence means still wrong, never resolved"* — because an operator
   who does not know that will read quiet as fixed.

**Emerged**

2. **The critical severity is split out precisely because it IS the exception to
   decision 1.** An MV that has lost `APPEND` replaces its whole target table on
   every refresh, so with these MVs' bounded `WHERE timestamp >= now() - window`
   it deletes pre-rolled history every tick — the [[0090]]/[[0095]] data loss.
   **That one does compound while nobody looks.** Giving it its own metric and
   its own alarm means the one case where latching is genuinely costly is never
   buried inside the count of cases where it is not, and it can carry emergency
   wording without making routine drift shout.
3. **The drift read runs last in the invocation.** Same rule `main.rs` already
   documents for the disk and USD reads, and here it has a specific reason: this
   is the **only** read in the probe that touches `system.*`. A narrowed grant
   or a metadata hiccup must not abort the rollup, disk and USD publishes, all
   of which sit behind `NOT_BREACHING` alarms that score healthy on missing data.
4. **An MV that is both drifted and non-`APPEND` counts once, as critical.**
   Double-counting would let one object inflate a number the operator reads as
   "how many MVs are affected".
5. **`MvDriftUnreadable` and the visible-object discriminator.** This task's own
   findings say all-six-`MISSING` is a grant gap rather than six dead tiers, but
   left it as a warning. A warning is not enough when the alarm would page at
   maximum urgency with the wrong diagnosis. `system.tables` is grant-**filtered**
   (not denied), so a narrowed grant silently makes every MV look missing. The
   probe counts visible `prices` objects: **none visible** means it cannot see
   the schema, so the counts are suppressed and a separate alarm fires that names
   the grant as the first thing to check. ⚠️ If objects *are* visible and the MVs
   are still gone, they really are gone and the ordinary alarm is correct — the
   discriminator does not swallow a real catastrophe.
6. **No new config value.** `>= 1` is the only meaningful threshold for "an MV
   is wrong", unlike the disk percentage and the DLQ/USD ladders. Adding a knob
   would create a second place for the truth to live, which is the drift hazard
   `DISK_FREE_PERCENT_BOUND` already carries.

### Issues encountered

- Two comments in `main.rs` and `disk.rs` asserted **"the probe touches no
  `system.*` table"**. Gap 3 makes that false. Both corrected rather than left,
  with the distinction that matters spelled out: `system.disks` is grant-**denied**
  and cannot be granted (which is why gap 1 reads filesystem *functions*), while
  `system.tables` is grant-**filtered** and needs nothing new. A stale invariant
  comment is how the next person concludes gap 3 needs a grant it does not.
- The first unit-test fixtures were written against guessed field names
  (`MvFingerprint { append, select }`, `Difference { expected, actual }`); the
  real shapes are `refresh: String` / `body` and `declared` / `live`. Compile
  error, caught immediately — noted only because it is the same class of
  mistake as the `assets` fixture in gap 4.

### Tests

**Unit ×9** in `mv_drift.rs`: clean chain, lost-`APPEND` → critical, both-at-once
counted once, every non-destructive status counted as ordinary drift, the
invisible-schema suppression, missing-MVs-in-a-visible-schema really missing, the
scoped visible-objects query, and `describe()`.

**IT ×4** on the 26.3.10.60 pin, all inducing the real condition:

| test | what it induces |
|---|---|
| `a_freshly_applied_schema_reports_no_drift` | the **negative control** — 7 real MVs, all in sync, must read 0/0/0 |
| `an_edited_declaration_is_detected_as_drift` | a modified `rollups.sql` fed to `check_mv_drift`, live untouched |
| `a_live_mv_without_append_is_detected_as_critical` | a throwaway MV created **without** `APPEND`, dropped after |
| `an_invisible_database_suppresses_the_counts_instead_of_paging` | a database with nothing visible |

⚠️ The negative control is what makes the critical test meaningful: `is_append`
reads the *live* fingerprint, so without a case proving healthy MVs read 0 the
critical metric could have been always-on.

40 unit tests and 15 ITs green; clippy clean on all four feature combinations.

⚠️ **Built, NOT deployed** — `Prices-production-Observability` only. `cdk synth`
confirms 34 alarms total, 3 of them gap 3's, each with one alarm and one OK
action.

## Implementation — gap 4 (2026-08-19)

Same branch as gaps 1 and 2. Rides the existing probe, so no new Lambda, no new
EventBridge rule, and nothing outside `observability-stack.ts`.

1. **`packages/rollup-freshness-probe/src/usd_sanity.rs`** — new module.
   `sanity_query()`, `SanityCounts`, `sanity_metrics()`, feature-gated
   `publish_sanity()`. Same split as `disk.rs`: query construction and metric
   shaping compile in every build and are unit-tested; the AWS SDK stays behind
   `--features lambda`.
2. **`src/main.rs`** — reads the USD counts **last**, after both existing
   publishes. See design decision 2.
3. **`infra/src/lib/stacks/observability-stack.ts`** — two ladders,
   `prices-{env}-usd-peg-applied-{n}` and `prices-{env}-usd-stranded-{n}`,
   `Maximum` over 15 min, 1-of-2, GREATER_THAN_OR_EQUAL, `NOT_BREACHING`, alarm
   **and** OK actions on every rung.
4. **`infra/src/lib/types.ts`** — `opsAlarms.usdSanityEscalationCounts` with
   validation (strictly increasing positive integers, **non-empty**).
   **`infra/envs/production.json`** — `[1, 100, 10000]`.

### Design decisions

**From plan**

1. **A ladder, because the count has depth.** Gap 3's analysis says drift is
   binary and therefore cannot use gap 2's trick. A count of wrong candles is
   not binary — a regressed writer keeps adding to it — so the ladder transfers
   unchanged, and gap 4 is not blocked on the re-notification question gap 3 is
   still waiting on. ⚠️ Rungs re-notify on **growth**; a frozen historical
   population would still latch. Accepted: this is a re-introduction guard, and
   a re-introduction grows.

**Emerged**

2. **The USD read runs last in the invocation, and the ordering is
   load-bearing** — the same rule `main.rs` already documents for the disk read,
   extended one step. This is the newest and least proven of the three reads, it
   scans OHLCV data rather than calling a function, and it has the most ways to
   fail (unresolvable identity, registry change, slow `FINAL`). Any of those
   running *first* would abort the invocation before the rollup and disk data
   landed, and those alarms are `NOT_BREACHING` — so a correctness check could
   blind the seven liveness checks beside it. It must not be able to.
3. **The leg is resolved by code + issuer at query time, not by a hard-coded
   `asset_id`.** The numeric id is not a stable contract while [[0139]] is open,
   and a probe silently watching the *wrong* leg is worse than one that fails.
   `resolved_legs` travels in the same row and `sanity_metrics` refuses anything
   but exactly 1 — otherwise an unresolvable identity matches no candles, both
   counts read 0, and `NOT_BREACHING` scores a check that never ran as healthy.
4. **A 48-hour grace on the stranded direction, matching BE's own window.**
   Enrichment fills `close_usd` asynchronously, so recent candles are
   legitimately zero and an ungraced metric would never read zero. 48 h is not a
   round number picked for comfort: it is the window BE actually read (last
   `close_usd > 0` within 48 h, else `--`), so the alarm fires exactly when a
   consumer has begun losing a value. ⚠️ It must also clear real enrichment lag,
   measured at 17 h+ on 2026-08-19 — if [[0209]] turns out to be a widening gap
   rather than ordinary lag, re-check this bound rather than raising it.
5. **The scan is bounded to 7 days.** An unbounded OHLCV scan every 15 minutes
   is [[0111]]'s outage re-introduced as a health check.
6. **`price_ohlcv_1d`, not `_1h`.** The defect appeared on all five tiers, so
   any one is diagnostic; `_1d` is ~24× cheaper and still reacts within one probe
   interval because a day's bucket exists from its first trade.

### Issues encountered

- 🔴 **A scalar subquery returns `Nullable(UInt64)`, and deserializing that into
  `u64` corrupts silently rather than failing.** `(SELECT count() FROM usdt)`
  typed as nullable; RowBinary prefixes a nullable column with a one-byte null
  flag, so the field read **256** for a true count of 1 — no error, no warning.
  The probe would then have refused every healthy run as "ambiguous identity"
  and the gap-4 alarms would never have published a datum.

  ⚠️ This is worse than the PR #97 regression the IT file was written for, which
  at least failed loudly. Fixed with `toUInt64(ifNull(…, 0))` and pinned by
  `the_resolved_leg_count_is_forced_non_nullable`.

  **It was caught only by running the integration test against a real
  ClickHouse.** Every unit test passed, clippy was clean on all four feature
  combinations, and the query string was correct SQL. Nothing short of executing
  it would have found this — which is the AC-4 lesson ("verified by inducing the
  condition, not by reading the CDK") arriving before the alarm was even
  deployed.
- The `prices.assets` fixture in the IT was written with a `version` column,
  which that table does not have (it is `ReplacingMergeTree(updated_at)`). Also
  caught by running it.

### Tests

**Unit ×13** in `usd_sanity.rs`: quote-leg scoping, issuer-based resolution,
both failure directions, the grace period, the bounded window, `FINAL` on both
tables, the underflow floor being the arithmetic bound rather than a round
number, the non-nullable `resolved_legs` pin, zero/ambiguous leg refusal, and
that the peg tolerance cannot reach a real USDT price.

**IT ×7** in `rollup_freshness_it.rs`, all against the 26.3.10.60 pin —
including **AC "verified by inducing it"**:

| test | what it induces |
|---|---|
| `usd_sanity_query_executes_and_reads_a_healthy_leg_as_zero` | the control — a correctly priced leg reads 0/0 |
| `usd_sanity_counts_both_induced_defects` | writes a par-valued candle **and** an aged zero; both counted |
| `a_freshly_written_zero_is_not_yet_stranded` | the same row inside and outside the grace |
| `dust_below_the_underflow_bound_is_not_counted_as_stranded` | a `1e-14` close |
| `an_exotic_quoted_zero_is_ignored_because_it_is_by_design` | a non-USDT leg at zero |
| `a_repaired_candle_stops_counting_once_a_higher_version_supersedes_it` | a `version + 1` repair, so `FINAL` is proven |
| `an_unresolvable_usdt_leg_reads_as_zero_and_is_therefore_refused` | the silent all-clear |

31 unit tests and 11 ITs green; clippy clean on all four feature combinations
(`""`, `aws-mtls`, `lambda`, `--all-features` — `--all-targets` is not
`--all-features`, and this crate has feature-gated entrypoints).

⚠️ **Built, NOT deployed** — same status as gaps 1 and 2, and the same
constraint: deploy `Prices-production-Observability` **only**. `cdk synth`
confirms 6 new alarms, each with one alarm action and one OK action.

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
   **`infra/envs/production.json`** — set to `20`. ⚠️ 15 was proposed and reversed on 2026-08-20 — see design decision 6.
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
   still free. ⚠️ The 2026-08-17 measurement is 430.6 GiB free = **24.45%**, so a
   25% bound would have been in ALARM from the moment it shipped. Unit-tested
   against both the measured steady state and a replay of the incident.

   ⚠️ **15 was proposed by the operator on 2026-08-20 and reversed the same day
   once the cost was measured. The measurement is the durable part:**

   | bound | fires after N GiB consumed | catches a repeat of 2026-08-13? |
   |---|---|---|
   | 25% | ~0 (already breached) | — in ALARM on day one |
   | **20%** | **78 GiB** | **yes**, about half-way through |
   | 15% | 166 GiB | **no** — 150 GiB lands at 15.93%, 16 GiB short |

   The event this alarm exists for consumed ~150 GiB, which sits *between* the
   two thresholds. So the choice is not a sensitivity dial: **five percentage
   points is the entire margin between catching that incident and missing it
   entirely.** ⛔ Re-measure before moving this value; it is far more sensitive
   than a percentage looks.

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

## Code review — four defects found and fixed (2026-08-19)

A review of PR #221 after all four gaps were built. Seven findings; four were
real defects in code written for this task and are fixed below. Two are recorded
as open questions and one was rejected — see "Not fixed" at the end.

⚠️ **Three of the four are the same failure in different clothes: a check that
does not run, scoring healthy.** Every alarm in this crate treats missing data
as OK, so *any* path that stops a datum being published is indistinguishable
from "nothing is wrong". That is the exact defect this task exists to close, and
it was reintroduced three separate times while closing it.

### 1. 🔴 Gap 4 could silently disable gap 3

The four checks ran with `?`, so each aborted every check below it, and gap 4's
comment claimed to run **last** while gap 3 had been inserted after it — both
comments said "LAST", which cannot be true of two things.

The consequence was not theoretical. `sanity_metrics()` refuses an unresolvable
USDT identity (a documented, plausible state), which failed the invocation
**before** the MV-drift read ran at all. With `MvDriftCritical` publishing no
datum and `NOT_BREACHING` in force, **an MV that had lost `APPEND` and was
destroying history every 15 minutes would have shown OK in Slack** for as long as
the USDT lookup stayed broken. Gap 4's design decision 2 asserts the opposite
property in writing.

**Fixed by inverting the design rather than reordering it.** Careful ordering was
the wrong tool: it makes correctness depend on a comment nobody re-reads when
adding a fifth check. Each check now records its own failure into a `failures`
vec and the next one still runs; the invocation fails at the **end** if any did,
so the probe's `-errors` alarm still carries it — but only after everything that
*could* publish has published.

### 2. A resolved leg that matches nothing read as healthy

`resolved_legs` guards an identity that cannot be found. It does not guard an
identity that resolves to an `asset_id` the candles no longer carry — precisely
the [[0139]] renumber risk that made us resolve by issuer in the first place.
That passes the guard, matches zero rows, publishes two zeros, and every gap-4
alarm reads healthy forever.

**Fixed** with a second guard on `scanned`. `sanity_metrics` now returns
`Result<_, SanityRefusal>` with two variants rather than a bare `Option`, because
the two refusals send the operator to different places — one to `prices.assets`,
one to the candles — and an alarm naming the wrong one costs an hour.

### 3. The drift alarms could announce a repair that never happened

All three carry an OK action with `treatMissingData: NOT_BREACHING` and
`evaluationPeriods: 2`. Two consecutive missed publishes transition a latched
ALARM back to OK and post an explicit **"resolved"** message to Slack — while the
MV is still drifted and nobody has touched it.

⚠️ That is a stronger form of the 2026-08-13 false-recovery signal this task was
filed over, and it made the descriptions' own "silence means still wrong"
guidance wrong in the one case it is written for.

**Fixed** with `treatMissingData: MISSING` on the three drift alarms only, which
retains the last state across a gap. Nothing is lost: a probe that stops
publishing is already covered by its own `-errors` alarm. The liveness alarms
keep `NOT_BREACHING` deliberately — for those, "no data" genuinely is the absence
of a breach.

### 4. The IT file understated what it destroys, and leaked a user

The header said "truncates `prices.price_ohlcv_*`". The tests also truncate
**`prices.assets`** and create/drop a real MV, a target table and a database.
`ch_url()` honours `CLICKHOUSE_URL`. Header corrected to say all of it.

Separately, `restricted_user_can_read_disk_headroom_but_not_system_disks` created
a passwordless user with `SELECT ON prices.*` and dropped it at the end — so any
failing `.expect()` before that line unwound past the cleanup and left the
account behind. Both reads are now collected before anything can panic and the
user is dropped **before** the assertions run.

### Not fixed — two open, one rejected

- ✅ **RESOLVED 2026-08-19 by changing the TIER, not the threshold.** The review
  said rung 1 would latch on healthy data. Its mechanism was wrong (not a
  `no_reference` floor) and — once measured properly — so was the conclusion, but
  only because the fix turned out to be somewhere neither of us was looking. The
  check moved from `price_ohlcv_1d` to `price_ohlcv_1h`; the grace stays 48 h and
  rung 1 stays 1. See "Prod baseline" below.
- ⏳ **The probe's 1-minute / 256 MB Lambda config was never revisited.** Its
  stale justifying comment ("seven metadata-only `max()` reads … trivially
  fast") **is now corrected** in `eventbridge-stack.ts`, which enumerates what
  each added read costs and names the drift check's ~20 sequential round trips
  as the only one that scales with latency rather than data volume. ⚠️ The
  comment change is **comments only** — `cdk synth Prices-production-EventBridge`
  is byte-identical before and after, verified twice (once after the pre-commit
  hook reformatted the file), so nothing about `CleanupRule` moved.
  The **config itself is deliberately untouched**: changing it means deploying
  that stack, which is the [[0200]] hazard. Today's measurement defuses most of
  the concern — the USD scan matches **91 rows** over 7 days, i.e. negligible.
  ⚠️ A hard Lambda timeout is not a Rust `Err`, so it publishes nothing while
  every alarm the probe feeds except the MV-drift ones scores missing data as
  healthy. **Confirm headroom by reading the function's `Duration` metric after
  the observability deploy** — that needs no change to `eventbridge-stack.ts` at
  all, which is the point.
- ⛔ **Rejected: tightening the grant discriminator to "no MVs visible".** The
  review is right that `visible_objects == 0` only catches a *total* loss of
  visibility, so a grant narrowed to one table still pages with the wrong
  diagnosis. But the proposed fix — suppress when no `MaterializedView` rows are
  visible — reads identically when the MVs really have all been dropped, which is
  the catastrophe design decision 5 explicitly requires the discriminator **not**
  to swallow. The partial-grant case stays uncovered on purpose.

### Prod baseline, measured 2026-08-19 before deploy

Run because the review said rung 1 might be in ALARM on day one. It is the same
test that set gap 1's disk bound, and it was right to run: **one of the two
thresholds is fine and the other is not.**

**Disk — deploy as configured.** `filesystemAvailable()` reads **431.60 GiB of
1.72 TiB = 24.55% free**, above the 20% bound, and essentially flat against the
~430 GiB measured on 2026-08-17, so BE is not currently filling the volume.
⚠️ It also **re-confirms decision 6 two days on**: at 24.55%, a 25% bound would
be in ALARM *right now*. That choice was made from a single Sunday reading; it
holds.

**USD peg-applied — clean.** `peg_applied = 0`, `resolved_legs = 1`,
`scanned = 91`. Three things at once: [[0182]]'s repair is holding with no
re-introduction, the identity resolves on prod (the field the
`Nullable(UInt64)` corruption would have read as 256), and the new `EmptyScan`
guard will not false-fire.

**USD stranded — 0 today, but that is a snapshot, not a verdict.**

| day | priced | unpriced |
|---|---|---|
| 08-13 → 08-17 | all | 0 |
| 08-18 | 8 | 8 |
| 08-19 | 0 | 8 |

`stranded` reads 0 only because both dark days sit inside the 48 h grace. The
08-18 rows cross it at **2026-08-20 00:00**.

#### What the investigation actually found — and it is NOT the reviewer's mechanism

Four measurements, in the order that narrowed it. ⚠️ **The first two conclusions
were wrong and are recorded because the wrong turns are instructive**, not to be
repeated:

1. The 16 unpriced rows are a near-identical roster two days running (CNY, GYEN,
   LINK, PL, XLM, yUSDC, yXLM), all `sdex`. ❌ Read as *asset-specific*. Wrong.
2. Four sampled assets priced cleanly every day 08-06 → 08-17, then zero on
   08-18 and 08-19 — a clean cut-off on the day [[0182]]'s repair ran.
   ❌ Read as *enrichment stopped*. Wrong.
3. **Split by quote leg, which is what settled it.** On 08-18 the USDC leg ran
   759 priced / 8 unpriced and the XLM leg 3,446 / 49 — both healthy. Only the
   USDT leg is dark. **Enrichment is alive; the failure is leg-specific.**
   ⚠️ This is exactly [[0209]]'s first acceptance criterion, and it is now met.
4. `prices.usd_rate` holds **only USDC** — 1,440 rows in 5 days, fresh to
   `2026-08-19 16:30`. USDT has no direct rate at all, so it must be derived.
   The USDT/USDC reference market carries **exactly one candle per day**, present
   every day, priced through 08-18 and not yet for 08-19.

**The mechanism that fits: a two-hop chain.**
`usd_rate(USDC)` → the USDT/USDC reference candle's `close_usd` → every
USDT-quoted candle. A USDT-quoted candle cannot be priced until its own day's
reference has been priced first, so the USDT leg is structurally one hop behind
every other leg. That is why 08-19 is uniformly dark and why USDC and XLM, which
are one hop shorter, are not.

⚠️ **This is a steady state, not a degradation.** At any moment the most recent
day-and-a-bit of USDT-quoted candles is unpriced and everything older is filled;
the same query on 08-17 would have shown 08-16 and 08-17 dark. [[0209]] was filed
yesterday for the same observation seen a day earlier.

⚠️ **The residual the chain does NOT explain, left open honestly:** on 08-18 the
reference *was* priced, yet only half that day's dependants filled. The chain
accounts for 08-19 completely and 08-18 only partially. Do not treat the
mechanism as fully proven.

#### ✅ RESOLVED — the tier was the problem, not the threshold

⚠️ **Everything from here to the end of this subsection was the state on the
evening of 2026-08-19 BEFORE the age distribution was measured. It is kept
because the reasoning was sound and the conclusion was still wrong**, which is
the point: the collision is real on `_1d` and does not exist on `_1h`, and
nothing short of bucketing by age could tell those apart.

**The measurement that resolved it.** Bucketing every USDT-quoted `_1h` candle by
age: unpriced rows appear only in the 0-30 h bands, and **every band from 30 h out
to 162 h is 100% priced**. The real enrichment ceiling is ~30 h, so a 48 h grace
carries ~18 h of headroom. There is no collision on the hourly tier.

The `_1d` tier is a different story on the same day: its 08-18 bucket was still
**half unpriced at ~41 h**. Two reasons, and the second is the one that was
invisible until now:

1. A bucket's `timestamp` is its **START**, so a `_1d` candle stamped `00:00` is
   not complete until 24 h later — **half of a 48 h grace is gone before there is
   anything to enrich**. On `_1h` that cost is one hour.
2. `_1d` is downstream of `_1h` in the rollup chain, so it is additionally behind
   in wall-clock terms.

⚠️ **We had picked the tier that sits closest to the line it is measured
against**, and picked it for a cost reason that also turned out to be false:

| tier | rows read | bytes | duration | returned |
|---|---|---|---|---|
| `_1d` | 984,706 | 50.5 MiB | 44-62 ms | 91 |
| `_1h` | ~1.37 M | ~70 MiB | 41-50 ms | 423 |

**1.4×, not the "~24×" the original decision 6 asserted** — that figure was about
how many rows the *tables hold*, not what a query scoped to one quote leg and 7
days actually touches. Most of the work is the `FINAL` merge and the `assets`
lookup, which both tiers pay identically. ⚠️ Note also that the check is not as
cheap as "91 rows" suggested; **measure it by what it reads, not what it
returns**.

The last objection died too: the USDT/USDC reference **does** exist hourly —
9-18 buckets/day over the week, essentially all priced — so hourly dependants
have something to price against. `_1h` is also a forever-table, so the 7-day
window can never outrun retention.

**Result: `SANITY_TABLE` is now `price_ohlcv_1h`. Grace unchanged at 48 h, so it
keeps meaning what design decision 4 says it means (BE's real loss window). Rung
1 unchanged at 1, so it stays a small-count re-introduction guard.** Both
alternatives below were rejected because each gave up one of those properties.

Pinned by two unit tests — `the_check_reads_the_hourly_tier_so_the_grace_is_not_eaten_by_bucket_width`
and `the_grace_clears_the_measured_enrichment_latency` — so a future "use the
cheaper coarse table" optimisation fails locally instead of on prod.

#### The superseded analysis, and the consequence it predicted

**The 48 h grace is sized about the same as the latency it exists to clear.** The
USDT leg's normal latency is one reference cycle plus a sweep pass, landing
somewhere near 24-48 h. So rung 1 at a threshold of 1 fires whenever the sweep
runs slightly long — on ordinary operation, not on damage. A permanently-firing
alarm gets muted, which is the outcome this whole task exists to end.

⚠️ **This is a genuine trade, not a retune.** Design decision 4 chose 48 h
*because* it is the window BE actually read, so the alarm fires exactly when a
consumer begins losing a value. On the USDT leg that now happens routinely, so
widening the grace past the real latency means the alarm no longer means what
decision 4 says it means. Two options, both costing something:

- **Widen `STRANDED_GRACE_SECONDS`** past the leg's measured latency — honest
  about the chain, but decouples the alarm from BE's actual loss window;
- **Lift rung 1** above the normal unpriced population (~8-16/day) — keeps the
  48 h meaning, but stops it being a small-count re-introduction guard.

✅ **Neither was taken.** Changing the tier removed the collision without giving
up either property. ⚠️ Both remain the right answers *if* the ~30 h ceiling ever
widens — re-measure the age distribution first, and do not simply raise the grace
to silence the alarm.

⚠️ The cheap confirmation, worth running before deciding: re-read the per-day
priced/unpriced counts after **2026-08-20 00:00**. If 08-18's eight have filled,
the chain explanation holds and this is latency. If they are still zero past the
48 h line, something is genuinely stuck and it is a different problem.

### Verification

42 unit tests (was 40) and **15 ITs green against the 26.3.10.60 pin**; clippy
clean on all four feature combinations. `cdk synth` still emits 34 alarms, the
three drift alarms now `TreatMissingData: missing`, each retaining one alarm and
one OK action; the other 26 `notBreaching` alarms are untouched.

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

## ⚠️ Pre-deploy re-check, 2026-08-20 — gap 4 found a live outage before shipping

The cheap confirmation the baseline section asks for ("re-read the per-day
counts after 2026-08-20 00:00") was run. **The 08-18 rows had not filled**, and
following that thread turned a threshold question into a root-cause
investigation. Spawned [[0209]]'s root cause and [[0212]].

### What was measured

| finding | measurement |
|---|---|
| unpriced frontier | **47 h**, not the ~30 h recorded on 08-19 — ~1 h inside the grace |
| every band 48 h → 162 h | 100% priced, 20 consecutive clean bands |
| USDT leg, `_1m` | **0 priced since 2026-08-13**, both USD columns untouched |
| USDC / XLM legs | current to 08-20 08:00 — the worker is alive |
| USDT/USDC reference | priced 18/18 on 08-19, current to 07:00 — inputs are healthy |
| `pivot_written` on `_1m`, all history | **0**, against `peg_written` = 1,564,045 |

**The USDT pivot has never priced a `_1m` row.** [[0172]] removed the $1 peg on
08-13 and its replacement never functioned; the leg has been dark since.
[[0111]] is the blocking dependency — `pivot_sql` is `ORDER BY timestamp` ASC
behind a 657 M-row backlog draining ~9,800/step and *rising*.

### What this changes here

1. ✅ **The stranded ladder is vindicated, not retuned.** Twenty clean bands ≥ 48 h
   is a well-behaved baseline, so rung 1 at 1 is correct. It breached at ~14:00
   on 2026-08-20 as a **true positive**. ⛔ The [[0209]] fix is to repair the
   pipeline, never to widen `STRANDED_GRACE_SECONDS`.
2. 🔴 **The peg-applied direction is pointed one tier above the defect.** It reads
   `_1h`, which [[0182]]'s repair wrote; the peg values live in `_1m`, which the
   repair never touched. On prod it would have published a confident **0 over
   1,564,045 wrong rows**. This is the *same* failure the task is named for —
   a check scoring healthy because it looked at the surface least able to show
   the defect — reintroduced a fourth time, and this time inside the guard built
   against it. ⚠️ Recorded in `SANITY_TABLE`'s doc comment and spawned as [[0213]]; **not fixed here**,
   because pointing `SANITY_TABLE` at `_1m` trades a blind spot for a
   permanently-breaching alarm (1.5 M is above every rung) and re-introduces the
   retention interaction the forever-table choice avoids. It needs its own scoped
   `_1m` query and ladder.
3. **Two false claims corrected in code**, both from the 08-19 baseline: the
   `STRANDED_GRACE_SECONDS` "~30 h ceiling / ~18 h headroom" note, and the unit
   test pinning it (`the_grace_clears_the_measured_enrichment_latency`, now
   `the_grace_is_bes_loss_window_not_a_lag_estimate`). A headroom assertion over
   a broken pipeline measures nothing. 44 unit tests green, clippy clean.

⚠️ **The "superseded analysis" and "RESOLVED — the tier was the problem"
subsections above are now themselves partly superseded.** The `_1d` → `_1h`
change was still right, and for the right reason (bucket width). But its
supporting measurement — the ~30 h ceiling — was an artefact of 0182's repair
coverage, not a property of enrichment. Read those sections for the bucket-width
argument, not for the latency figures.

## 🔴 Deploy attempt 1 FAILED — CloudWatch caps AlarmDescription at 1024 chars

2026-08-20, first `deploy-production-observability`. Three `usd-stranded` rungs
carried ~1250-character descriptions and CloudWatch rejected them **mid-deploy**,
after other alarms in the stack had already been created.

⚠️ **Nothing local catches this.** `cdk synth` renders an over-long description
happily, the template is valid CloudFormation, `cdk diff` showed the expected 12
additions and 0 deletions, CI was green, and the limit is enforced only by the
CloudWatch API. **This is AC 4's lesson yet again — "verified by inducing the
condition, not by reading the CDK" — and this time it bit the deploy itself.**

Two fixes, and the second matters more than the first:

1. **The three descriptions were shortened to ≤ 937 characters.** ⚠️ The text
   that pushed them over was text **this same day had falsified**: the "MEASURED
   BASELINE … priced by 30 h of age" claim and the "TWO-HOP chain … check that
   chain in order rather than assuming the sweep" instruction. Both would have
   sent a responder down a dead end — the hops were measured healthy while the
   leg was dark. The rewrite names the real cause (the pivot has never priced a
   `_1m` row), says the alarm stays latched until [[0209]] is fixed, and warns
   to verify on `_1m` rather than a coarse tier. **The length limit forced a
   correction that was owed anyway.**
2. **`assertAlarmDescriptionsFitCloudWatch()` now runs at synth**, walking the
   construct tree and throwing with the offending alarm names and lengths. It
   reads the *resolved* CloudFormation property, so token- or `Fn::Join`-built
   descriptions are measured as CloudWatch will see them. ⚠️ These alarms carry
   deliberately long runbook-style descriptions because an operator reading
   Slack at 03:00 has nothing else — that is worth keeping, which is exactly why
   the ceiling needs a local guard rather than discipline.

**Verified by inducing it**: padding one description to 2533 chars makes synth
fail with all three names and lengths; restoring produces a template
byte-identical to the verified-good one. 34 alarms, max description 937.

## Future Work

- **Point the peg-applied check at `_1m`** with its own ladder and scan bound —
  spawned as [[0213]]. The only gap-4 direction still blind on prod, and it must
  land after [[0212]] or it ships permanently breached.
- The two integration tests never run in CI (no ClickHouse service, no
  `--ignored`). Pre-existing and wider than this task.
- Move the disk metrics to their own `Prices/ClickHouse` namespace once the
  0182/0201 campaign has landed and `eventbridge-stack.ts` is safe to deploy.
