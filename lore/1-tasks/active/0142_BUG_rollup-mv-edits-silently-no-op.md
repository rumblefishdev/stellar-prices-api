---
id: "0142"
title: "rollups.sql edits silently don't land either — the refreshable MVs carry 0134's footgun with no OR REPLACE escape"
type: BUG
status: active
related_adr: []
related_tasks: ["0134", "0136", "0095", "0090", "0104", "0143", "0144", "0146"]
tags: ["priority-high", "effort-medium", "clickhouse", "schema-drift", "footgun"]
links: []
history:
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0134]]. That task converted the six plain views in
      `views.sql` to `CREATE OR REPLACE VIEW` and deliberately scoped out the
      refreshable MVs. Confirmed while doing so that `rollups.sql` declares all
      six MVs `CREATE MATERIALIZED VIEW IF NOT EXISTS` with **no** preceding
      `DROP` — so the identical silent no-op applies there, and unlike a plain
      view a refreshable MV cannot use `OR REPLACE` as the escape.
  - date: 2026-08-14
    status: active
    who: okarcz
    note: >
      Promoted to active. Selected specifically because it needs **no Hetzner
      access** — the BE team are working on the shared volume after the
      2026-08-13 disk-full incident ([[0202]]), which rules out the two tasks
      that were otherwise next ([[0182]]'s repair run and [[0201]]), and
      [[0111]] cannot choose among its four fixes without prod `query_log`
      measurements. This task's cheapest win — comparing `create_table_query`
      in `system.tables` against `rollups.sql` — is written and tested entirely
      against a local ClickHouse pinned to the prod version 26.3.10.60.
      It also unblocks [[0146]] and [[0203]].
  - date: 2026-08-14
    status: active
    who: okarcz
    note: >
      Tool + runbook implemented on branch
      `fix/0142_rollup-mv-edits-silently-no-op`. Read-only drift check
      (`prices-clickhouse-drift`) comparing `rollups.sql` to a target's live
      definitions, plus `docs/runbooks/0142-rollup-mv-reapply.md`. 11 new unit
      tests (23 in the crate), 5 new ITs on the 26.3.10.60 pin, all verified
      non-vacuous; 24 crate ITs and the whole workspace unit suite green.
      Three of four ACs met. **The MVs themselves are unchanged and the check
      has never been run against ch-prod-01** — deferred while BE work the
      shared volume ([[0202]]); it is read-only but its result would want
      acting on. Stays active for that run.
---

# `rollups.sql` edits silently no-op on a provisioned target

## Summary

All six rollup MVs in `packages/prices-clickhouse/schema/rollups.sql` are
declared:

```sql
CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE APPEND
TO prices.price_ohlcv_15m AS
SELECT …
```

`IF NOT EXISTS` does not redefine an object that already exists. On ch-prod-01 —
which holds all six — **editing an MV body and re-applying `rollups.sql` changes
nothing, and the apply reports success.** This is exactly the failure [[0134]]
removed from `views.sql`; it is untouched here.

`current.sql` already does the right thing (an explicit `DROP` then `CREATE`);
`rollups.sql` has no `DROP` anywhere in the file.

## Why this is harder than 0134

A plain view replaces atomically, so 0134's fix was a one-word change per
statement. A refreshable `TO`-table MV has no `CREATE OR REPLACE` form, so the
only route is `DROP` + re-`CREATE` — and that is not free:

- **The DROP window is a data-loss window.** [[0090]] / [[0095]] are the
  precedent: replace-mode MVs over a `TO` table wiped the coarse history, and
  the recovery was expensive. Any re-CREATE must preserve the APPEND +
  `sum(version)` + aligned-window shape [[0095]] landed, or it silently
  reintroduces that bug.
- **A dropped MV stops rolling up while it is gone.** [[0136]] is the precedent
  for how long a starved rollup can go unnoticed — nine days, no alarm. Any
  procedure here should be paired with [[0137]]'s freshness alarm.
- **Cadence/window interactions** are [[0104]]'s open question; a re-CREATE is
  the moment those get re-decided by accident.

So the deliverable is probably **not** "convert them all" but a safe, documented
re-apply procedure plus a guard that makes the drift visible.

## Implementation

Roughly, in preference order:

- **Make the divergence detectable.** Compare the `create_table_query` in
  `system.tables` for each `mv_ohlcv_*` against the statement in `rollups.sql`
  and report a mismatch. This turns a silent no-op into a loud one without
  touching production objects, and is the cheapest real win.
- **Write the re-apply runbook**: DROP + re-CREATE per MV, one at a time, with
  the APPEND / `sum(version)` / aligned-window invariants stated as a
  pre-flight checklist, and the expected freshness recovery after each step.
  Model it on the [[0136]] recovery runbook, which already documents per-table
  surgery on these objects.
- **Consider a guard test** in the same shape as 0134's
  `views_sql_uses_create_or_replace_for_every_view` — but note the assertion
  here is the *opposite* (these must NOT be `OR REPLACE`), so it should assert
  the file's intended form and that a re-apply procedure is referenced.
- Audit `preroll.sql` for the same pattern while in the file.

## Implementation (2026-08-14)

Branch `fix/0142_rollup-mv-edits-silently-no-op`. Both halves the task asked
for: the divergence is now **detectable**, and changing an MV is now a
**documented procedure**. The MVs themselves are unchanged — see "what is NOT
done" below.

1. **`src/drift.rs`** — read-only comparison of `rollups.sql` against a target's
   live definitions. `check_rollup_drift(client, database)` returns an `MvReport`
   per declared MV: `InSync` / `Missing` / `Drifted(Vec<Difference>)`, each
   difference naming the field (refresh clause / target table / select body) and
   carrying both renderings.
2. **`src/bin/prices-clickhouse-drift.rs`** — the operator entrypoint. Exit 0
   when all six are present, match and are APPEND; exit 1 otherwise. `--verbose`
   prints full definitions.
3. **`schema/rollups.sql`** — header now opens with the `IF NOT EXISTS` warning,
   the runbook path and the drift command.
4. **`docs/runbooks/0142-rollup-mv-reapply.md`** — the DROP + re-CREATE
   procedure, with the four invariants as a pre-flight checklist.
5. **Guard test** `rollups_sql_keeps_if_not_exists_and_references_the_reapply_runbook`
   — asserts the file's intended form (the *opposite* of [[0134]]'s guard) and
   that the runbook pointer survives.

### The comparison is not a string compare — and could not be

ClickHouse does not store submitted text. `system.tables.create_table_query` is
re-serialised from the AST and differs from the file four deterministic ways
(measured on the 26.3.10.60 pin): `IF NOT EXISTS` dropped, the target's column
list injected, `DEFINER = default SQL SECURITY DEFINER` injected, and syntax
normalised (`INTERVAL 15 MINUTE` → `toIntervalMinute(15)`).

A naive text compare therefore reports drift on a **freshly applied, unedited**
chain. That is worse than no check: a permanently-red signal gets ignored and
the real drift arrives unnoticed inside it.

The fix is to let ClickHouse normalise both sides — `formatQuerySingleLine`
renders a submitted statement through the same AST serialiser that produced
`create_table_query`, which kills differences 1 and 4. Differences 2 and 3 are
skipped structurally: both sit strictly between the `TO <target>` token and
` AS SELECT`, so comparing a `(name, refresh, target, body)` fingerprint never
sees them. That is correct rather than merely convenient — the column list is a
property of the target *table*, not of the MV definition this file owns.

### Design decisions

**From plan**

1. **Detect, do not auto-fix.** The task's own preference, and it survives
   contact: the `DROP` is a live exposure window and re-CREATE re-opens every
   [[0095]] invariant, so it stays an operator action under a runbook. The apply
   path must never be able to take a rollup tier offline — pinned by the guard
   test asserting no `DROP` in `rollups.sql`.

**Emerged**

2. **A separate binary, not a flag on `prices-clickhouse-init`.** The check must
   be runnable against ch-prod-01 by an account with **no DDL grants** —
   `prices_writer`/`prices_reader` hold none and cannot be granted any (XML-managed,
   `ACCESS_STORAGE_READONLY`). A flag on the init CLI would have inherited that
   binary's privileged-operator requirement for no reason, and left open the
   possibility of applying something by mistyping a flag. Read-only by
   construction beats read-only by intent.
3. **Replace mode is reported independently of drift.** A refreshable MV that
   lost `APPEND` is not stale, it is *destroying history on every tick*
   ([[0090]]/[[0095]]). `MvReport::needs_attention()` therefore fires on a live
   non-APPEND MV **even when the file agrees with it** — an in-sync file is no
   defence if what both sides hold is the destructive form. Unit-pinned by
   `an_in_sync_but_replace_mode_mv_still_needs_attention`.
4. **An empty report is a failure, not an all-clear.** The binary exits 1 if the
   check produced no reports at all. This tool exists because a false all-clear
   is the failure mode; a report listing nothing looks identical to a clean one
   at a glance. Same reasoning as [[0114]]'s "silent no-op" pre-registration.
5. **An unparseable definition raises rather than being skipped.**
   `SchemaError::UnparsableDdl`. A statement silently dropped from the report
   shortens it, and a shorter report reads exactly like a cleaner one.
6. **The default report excerpts around the FIRST divergence, not the start.** A
   rollup body is ~700 characters that agree for the first several hundred;
   head-truncation printed two *identical* lines and told the operator nothing.
   Caught by running the tool against seeded drift, not by reading the diff.

### Tests

- **Unit ×11 new** (`drift.rs` ×8, drift binary ×3), 23 total in the crate.
  Key ones: `the_server_injections_do_not_register_as_drift` (the whole premise —
  the two renderings of one unedited MV must fingerprint identically),
  `an_edited_body_does_not_fingerprint_as_in_sync`,
  `a_replace_mode_mv_is_not_append`,
  `the_excerpt_is_centred_on_the_divergence_not_on_the_start`.
- **`tests/rollup_drift_it.rs` ×5** on the 26.3.10.60 pin:
  - `a_freshly_applied_chain_reports_no_drift` — the baseline that a naive
    comparison fails.
  - `an_edited_body_is_reported_as_drift_because_the_reapply_silently_no_ops` —
    the centrepiece. Applies the chain, edits a body (the real [[0146]] change:
    `argMax(close_usd,…)` → `argMaxIf(…, close_usd > 0)`), **re-applies and
    asserts the apply reports success while `create_table_query` is byte-identical
    to before**, then asserts the check reports drift anyway. It is simultaneously
    a reproduction of the defect and proof of the detector. Carries a control:
    the same target against the *unedited* file must still be `InSync`, so it
    cannot pass for a check that reports drift unconditionally.
  - `a_dropped_mv_is_reported_as_missing`, `a_replace_mode_mv_is_reported_as_drift_and_as_not_append`,
    `a_hand_edited_window_is_reported_as_drift`.

**Verified non-vacuous.** Disabling the body comparison (`body: String::new()`)
failed exactly the two body-drift ITs and left the missing / replace-mode /
clean cases passing — i.e. each test fails for its own reason, not a shared one.

⚠️ **`IF NOT EXISTS` swallowing the edit is now asserted, not assumed.** If a
future ClickHouse ever makes the rollup MVs re-appliable, that IT fails loudly
and this task's premise has changed.

## What is NOT done

**No MV body has been changed, and nothing has been run against ch-prod-01.**
This task makes the drift *visible* and the change *safe to perform*; it does
not perform one. The live chain on prod has never been checked with this tool —
the first run is an operator action and it is read-only.

⚠️ The drift check has only ever run against a scratch/local target. It is
plausible ch-prod-01's live definitions already differ from `rollups.sql` (the
[[0136]] recovery re-created MVs by hand, and [[0090]]/[[0095]] re-created them
again). **Running it on prod is the obvious next step and may itself be a
finding.** Deliberately not done here: BE are working on the shared Hetzner
volume ([[0202]]), and while the check is read-only, the result would want
acting on.

## Acceptance Criteria

- [x] A drift between `rollups.sql` and the live MV definitions is detectable
      rather than silent — `prices-clickhouse-drift`, read-only, exit 1 on
      drift. Proven against a real edit that `IF NOT EXISTS` swallowed.
- [x] A documented, tested procedure exists for changing a rollup MV body on a
      provisioned target, preserving the [[0095]] APPEND invariants —
      `docs/runbooks/0142-rollup-mv-reapply.md`, four invariants as a pre-flight
      checklist, coarse-to-fine order, verification on the data as well as the
      DDL.
- [x] The procedure states its freshness-gap exposure and pairs with [[0137]].
      **Measured 2026-08-14 rather than reasoned:** a re-created refreshable MV
      runs its initial refresh **immediately at `CREATE`**, not at the next
      scheduled boundary (`last_success_time` = create time on two independent
      samples), and `next_refresh_time` realigns to the clock. Combined with each
      MV re-aggregating a bounded *window*, the gap is self-healing provided the
      outage stays inside that window (tightest is 2 h on `_1m_to_15m`, against a
      DROP+CREATE measured in seconds). So the exposure is **not** the length of
      the gap — it is a botched re-CREATE, which is what the checklist is for.
- [x] `preroll.sql` audited for the same defect — **not susceptible.** All four
      pre-roll scripts (`preroll.sql`, `-incremental`, `-live-gap`,
      `-amm-reprice`) contain **zero** object DDL: no `CREATE`, no
      `IF NOT EXISTS`, no view or table creation of any kind. They are
      `INSERT`/`ALTER … DELETE` only, so there is no object for `IF NOT EXISTS`
      to decline to redefine. Nothing to change.
- [ ] Run the check against ch-prod-01 — deferred, see "What is NOT done".
