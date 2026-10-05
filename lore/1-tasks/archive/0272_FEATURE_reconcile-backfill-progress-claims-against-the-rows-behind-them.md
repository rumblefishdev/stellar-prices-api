---
id: "0272"
title: "Nothing compares backfill_progress's claims against the rows behind them — reconcile the stored watermarks on a schedule, not at the end of a run"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0264", "0263", "0176", "0243", "0127", "0200"]
tags: [layer-backend, priority-medium, effort-small, milestone-M2, observability, backfill, data-correctness]
milestone: 2
links:
  - "../../../packages/backfill-freshness-probe/src/lib.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-09-08
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0264]] acceptance criterion 4, which is **deferred here
      rather than met**. Scoped deliberately as a periodic sweep and NOT as the
      end-of-run check first proposed — see Design below for why that version is
      vacuous.
  - date: "2026-09-29"
    status: active
    who: akot
    note: >
      Activated. Prod measured read-only first (both streams reconcile to 0;
      a weekly `_1h` scan costs ~311 ms), then planned via GSD quick task
      260929-k21 — plan in `.planning/quick/`, binding brief BRIEF-0272.
  - date: "2026-09-29"
    status: active
    who: akot
    note: >
      Sweep built on branch `feat/0272_reconcile-backfill-progress-claims`:
      a weekly `{"check":"reconcile"}` rule on backfill-freshness-probe, the
      `EarliestOverclaimSeconds` metric and one IGNORE alarm per stream. Proven
      locally on ClickHouse 26.3.10.60 (9 IT scenarios, 7 defects shown red) and
      by synth. Not deployed; in review as PR #371. Created
      the wiki note `lore/3-wiki/project/amm-history-is-not-in-price-ohlcv-1m.md`,
      so the existing `[[amm-history-is-not-in-price-ohlcv-1m]]` links resolve.
  - date: "2026-10-05"
    status: completed
    who: akot
    note: >
      Closed by Adam's decision. PR #371 merged 2026-09-30, deployed; the
      first weekly reconcile datum (2026-10-05) was published for both
      streams and both overclaim alarms went to OK. The induced alarm-to-Slack
      check (post-deploy runbook steps 2-3) was not run, so criteria 1 and 5
      are met locally and live only on the OK path. No follow-up task.
---

# `backfill_progress` holds claims nobody compares to the data

## Summary

`prices.backfill_progress` stores assertions about coverage — how far back the
data goes, how far a stream has reached — and `GET /v1/backfill/status` publishes
them to consumers. Until [[0127]] nobody had ever compared those assertions to
the rows they describe. Two of the columns turned out to overstate.

Both causes are now fixed at the writer ([[0263]], [[0264]]). Nothing detects the
**general** case: a stored claim that outruns the data behind it.

## Context

The class, from the two known instances:

| column                    | claimed                     | reality                     | fixed by |
| ------------------------- | --------------------------- | --------------------------- | -------- |
| `earliest_data_available` | AMM data from 2024-02-20    | first AMM candle 2024-03-08 | [[0264]] |
| `current_ledger`          | a genesis floor             | a chunk that stopped short  | [[0263]] |

The AMM overclaim stood for roughly eighteen months and was invisible: the
endpoint kept returning a plausible number, nothing errored, and no alarm covers
it. It surfaced only because a human went looking.

## Design — why a periodic sweep, and NOT an end-of-run check

⚠️ **An end-of-run check was proposed first and is vacuous. Do not build it.**

After [[0264]], the claim a run writes *is* the earliest minute that run landed,
derived from the same `source` string handed to `Sink::write_candles`. Comparing
the run's observation against the run's own claim compares a value to itself; it
can only pass. The one legitimate way they differ is `merge_min` pulling in an
older stored value from a prior run, which is correct behaviour.

A real check compares the **stored** row against `min(timestamp)` in the candle
tables. That is a database query either way, so end-of-run and periodic are the
same query differing only in trigger — and the triggers are not equally useful:

| side of the comparison | moves when                                                     |
| ---------------------- | -------------------------------------------------------------- |
| the claim              | a backfill runs — now rare, and correct by construction         |
| the reality            | candles are deleted — partition drops, retention, a repair job  |

Deletion under a frozen claim is the live drift path, and **only a timer catches
it**. `sdex-backfill`'s `sink.rs` is the sole production writer of this table, so
with the archive complete and the AMM stream resting, an end-of-run trigger fires
approximately never — and fires precisely when the writer is most trustworthy.

The deletion path is not hypothetical: `price_ohlcv_1m` carries a nominal 7-day
retention in `cleanup-worker`'s `RETENTION`, and [[0200]] asks whether that worker
should be re-enabled.

## Implementation

- One query per stream: stored `earliest_data_available` vs `min(timestamp)` for
  that stream's sources. **Query `price_ohlcv_1h`, not `_1m`** — see
  [[amm-history-is-not-in-price-ohlcv-1m]]; AMM history is not in `_1m` at all,
  and a `_1m` query returns a false pass. ⚠️ **Overridden 2026-09-29: `_1h`,
  not the `_1d` this bullet first named** (decision D2). The regressed AMM claim
  2024-03-08 00:00 against the true first candle 19:00 is a 19-hour overclaim
  that is invisible at day resolution. The `_1h` scan was measured on prod at
  215.7 M rows / 1.08 GB / 311 ms / 3 MB of memory against a 3.73 GiB limit,
  run weekly (AC 3). The fact is enforced in the `RECONCILE_QUERY` doc comment
  (`packages/backfill-freshness-probe/src/reconcile.rs`) and the
  `reads_1h_not_1m` IT case.
- Overclaim in seconds (`reality − claim`, positive when the claim is too early)
  published per stream; alarm above zero.
- Home: extend `backfill-freshness-probe`. It already runs on a schedule, already
  reads this table, and already publishes a `Prices/Backfill` metric with an
  alarm wired to Slack — so this is a query and an alarm, not a new service.
  Decide against standing alone with [[0243]], and record the reason.
- **Once a week, not every 15 minutes.** The value changes only when a backfill
  lands older data or candles are removed. `earliest_data_available` is stored
  precisely because computing it live is a full scan (`timestamp` is not the sort
  key) — the scan is cheap on `_1d` (~125k AMM rows) and must not be put on the
  probe's normal cadence.
- Extend to `current_ledger` against `backfill_sdex_ledgers` if the same shape
  fits; [[0263]] left the floor unprovable from the column alone.

## Acceptance Criteria

- [ ] A stored watermark that precedes the data behind it is detected on **either**
      stream, without a human going looking. ⏳ **Met locally, live half pending
      the owner's deploy:** the IT proves both streams' detection and the
      synthesized template carries the weekly rule and both alarms; it is live
      only once deployed and runbook step 1 has run.
- [x] The check runs against the durable tables; a `_1m`-only query is rejected in
      review as a false pass. It reads `price_ohlcv_1h`; the `reads_1h_not_1m`
      IT case and red proof (iii) show a `_1m` query passes falsely.
- [x] Cadence is weekly or slower, with the scan cost measured rather than assumed.
      `cron(47 5 ? * MON *)`; 215.7 M rows / 1.08 GB / 311 ms / 3 MB measured.
- [x] Home decided between the existing probe and standing alone, with the reason
      recorded. The existing probe plus a second rule — see Design Decisions D1.
- [ ] Verified by inducing: point it at a deliberately wrong stored value and
      confirm it fires, per the repo's standing practice of proving an alarm by
      breaking something. ⏳ **Met locally, live half pending the owner's
      deploy:** the ClickHouse IT points the exact production query at
      deliberately wrong stored claims and each of seven restored defects turns it
      red (Implementation Notes); the alarm → Slack half is runbook steps 2–3
      below, with a published datum, never a wrong claim in prod.
- [x] [[0264]]'s AC 4 is closed by reference to this task's outcome.

## Implementation Notes

Built 2026-09-29 on `feat/0272_reconcile-backfill-progress-claims` (PR #371).

- **Code.** `packages/backfill-freshness-probe/src/reconcile.rs` holds
  `RECONCILE_QUERY` and the metric shaping; `check_from_event` (`lib.rs`) routes
  `{"check":"reconcile"}` to it, and an event with no `check` still runs
  push-age. Infra: `scheduleExpressions.backfillReconcileProbe`, rule
  `prices-{env}-backfill-reconcile` targeting the freshness function, alarms
  `prices-{env}-backfill-earliest-overclaim-sdex` / `-amm`.
- **D1 — home (AC 4).** The existing probe already has the ClickHouse client, the
  `Prices/Backfill` publish grant and a Slack-wired `-errors` alarm. A standalone
  Lambda would be a tenth scheduled worker needing a health-alarm exemption for
  about 1 s of work a week. A second rule, not clock-gating inside the 15-minute
  handler.
- **D3 — metric.** `first _1h bucket − claim`, signed, not clamped. Every
  positive value is a real overclaim (lower bound); (−3600, 0] is bucket slack.
  Expected prod readings: about −2820 for `sdex_archive`, 0 for `soroban_amm`.
- **D5 — edge cases.** A claim with no `_1h` rows reads `now() − claim`
  (`LEFT JOIN` + `join_use_nulls = 1`, otherwise 1970 is filled in and it passes
  silently). A NULL claim or missing row publishes nothing and logs a `WARN`.
  Every `status` is reconciled.
- **D6 — `current_ledger` vs `backfill_sdex_ledgers` not built.** The markers are
  backfill-only (max 63 352 611, 1.3 M below the target), the minimum marker is 3
  while `current_ledger` is 1 (false fire on day one), re-ingests delete markers
  on purpose (would page during planned work), and the claim is contiguity, not
  a minimum — a different query and alarm. [[0263]] already fixed the writer.
  🔎 For the owner: `sdex_archive` reads `completed` while the April-2019
  re-ingest hole (23 487 993–23 665 284) existed in `backfill_sdex_ledgers`.
- **Red→green (AC 5, local half).** `backfill_reconcile_it` on ClickHouse
  26.3.10.60: 8 table cases plus `claim_without_rows`. Each defect restored in
  turn turned it red: INNER JOIN, no `join_use_nulls`, `_1m`, a venue list, a
  status filter, no `FINAL`, and the claim filter as PREWHERE (before `FINAL`).
  On 26.3 `optimize_move_to_prewhere_if_final = 1` alone does not move that
  predicate; the setting stays pinned to 0 as a guard.
- **Prod, read-only (`dev_read`, 2026-09-29).** Both streams reconcile to ≤ 0.
  `_1h` scan: 215.7 M rows, 1.08 GB, about 0.3 s. No `_1h` rows before
  2015-01-01 (a junk early timestamp would mask overclaims for good); earliest
  bucket per source: `sdex` 2015-11-18, `soroswap` 2024-03-08, `phoenix`
  2024-03-21, `aquarius` 2024-04-18, `sushiswap` 2026-09-22, `comet` 2026-09-24.

## Design Decisions

1. **D1 — home: the existing probe plus a second weekly rule** (reasons in
   Implementation Notes).
2. **D2 — `price_ohlcv_1h`, overriding this task's original `_1d`.** `_1m` is a
   false pass for AMM history ([[amm-history-is-not-in-price-ohlcv-1m]]); `_1d`
   hides a 19-hour regression of the AMM claim; `_1h` and `_1d` are both kept
   forever and the `_1h` scan is cheap once a week.
3. **Deviation from CONTEXT D1: an unknown `check` value fails the invocation**
   instead of running push-age. Only an event with no `check` key runs
   push-age; the scheduled 15-minute event never carries one, so D1's
   backward-compatibility goal holds. Reason: under D1's literal wording a typo
   in the reconcile payload (`"reconcil"`) would silently run push-age and
   publish no overclaim datum, and under IGNORE that looks exactly like "no
   news" — the silent failure this task exists to prevent. The one-line
   reversal is to map unknown values to `Check::PushAge` in `check_from_event`.
4. **`treatMissingData: IGNORE`, not NOT_BREACHING (D4).** Coverage-sweep emits
   only on breach, so for it "missing" means healthy. This probe always
   publishes, so "missing" means "no news", and six silent days must not resolve
   a latched ALARM (0243 decision 6).
5. **Alarm period is 1 hour — the owner's choice (2026-09-29) over CONTEXT D4's
   1 day.** A ≤ 0 datum at least 1 h after a positive one returns the alarm to
   OK quickly after a fix or after the runbook's induced datum, where a 1-day
   `Maximum` window would hold it in ALARM for up to about 24 h. IGNORE keeps a
   real positive datum latched across the empty hourly windows whatever the
   period, so the shorter period loses nothing.
6. **The `amm-history-is-not-in-price-ohlcv-1m` wiki note was created** by the
   owner's decision, overriding CONTEXT D8 ("do not create"); the existing
   links in 0101, 0264, 0271 and this task were kept, not repointed.
7. **Alarm only.** Nothing writes a corrected value back to `backfill_progress`;
   a fix is a deliberate operator write, because `merge_min` never moves the
   stored claim later ([[0264]]'s operator-correction SQL is the pattern).

## Post-deploy verification (owner runs; agent ran none)

No step writes to prod `backfill_progress`.

1. After deploy, invoke the probe with the reconcile payload:
   `aws lambda invoke --function-name <backfill-freshness-probe function> --payload '{"check":"reconcile"}' --cli-binary-format raw-in-base64-out out.json`.
   Expect `published` with both streams ≤ 0 (about −2820 for `sdex_archive`,
   0 for `soroban_amm`) and the `backfill reconcile complete` line in the
   function's CloudWatch Logs.
2. Publish a positive test datum, **outside 05:00–05:59 UTC on a Monday** (the
   real run's datum would share that clock hour):
   `aws cloudwatch put-metric-data --namespace Prices/Backfill --metric-name EarliestOverclaimSeconds --dimensions Environment=production,Stream=soroban_amm --value 3600 --unit Seconds`.
   Expect `prices-production-backfill-earliest-overclaim-amm` to go to ALARM at
   its next evaluation (within about an hour) and the Slack/SNS message to
   arrive. It stays in ALARM, because the empty hours are ignored.
3. Return to OK: publish `--value 0` for the same dimensions **at least 1 h
   after** step 2 (a later clock hour), or rerun step 1 at that time. Expect OK
   and the OK notification in Slack. A 0 inside the same clock hour as step 2
   shares the `Maximum` window and correctly leaves the alarm in ALARM.
   `aws cloudwatch set-alarm-state` is only a temporary override, re-evaluated
   at the next evaluation, so it is not proof of the OK path.
4. Rejected alternative: writing a wrong claim into prod `backfill_progress`. It
   is served publicly by `/v1/backfill/status` and made sticky by `merge_min`.

### What the alarms will and will not tell you

- **First run after deploy:** both new alarms go INSUFFICIENT_DATA → OK on
  their first ≤ 0 datum, so expect two OK notifications in Slack. Not an
  incident.
- **A stream without a claim:** if a stream's `earliest_data_available` goes
  NULL or its `backfill_progress` row disappears, no datum is published and,
  under IGNORE, that stream's alarm keeps its last state. The only trace is a
  `WARN` line `no claim to reconcile; no datum published` in
  the probe's logs.
- **A failing weekly run is quiet.** Lambda's default async retries (2) apply,
  so one failure means up to 3 scans of about 1.08 GB each on the shared
  ClickHouse. The shared `-errors` alarm (15-minute Sum, NOT_BREACHING) goes to
  ALARM and back to OK within about 30 minutes, and the overclaim alarms keep
  their stale state. Nothing signals "no successful reconcile in N weeks". This
  is accepted by design (D4); a staleness check on the metric is the follow-up
  if that becomes a problem.
- **Re-running the query by hand as `dev_read`:** that user is `readonly=1`
  and ClickHouse rejects any `SETTINGS` clause (`Code: 164 … READONLY`). Strip
  the `SETTINGS` line; `join_use_nulls = 1` is then lost, so read a missing
  candle side as the 1970 default, not as "no rows". The probe's own user
  accepts settings: the push-age `AGE_QUERY` has run with `SETTINGS` in prod
  every 15 minutes.

## Notes

- 🔑 **The general lesson this task exists to institutionalise:** `backfill_progress`
  holds *claims*. Two columns were found asserting more than the data supported,
  by different mechanisms, eighteen months apart in origin and one week apart in
  discovery. Fixing both writers removes those two mechanisms; it does not make
  the table self-checking.
- A one-off manual reconciliation should be run when [[0264]]'s data correction
  lands, independently of this task — that confirms the correction took, and does
  not wait on this being built.
