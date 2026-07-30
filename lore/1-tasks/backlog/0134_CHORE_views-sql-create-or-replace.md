---
id: "0134"
title: "views.sql edits silently don't land — convert the remaining five views to CREATE OR REPLACE"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0072", "0076", "0061", "0133"]
tags: ["phase-future", "effort-small", "priority-medium", "clickhouse", "schema-drift"]
links: []
history:
  - date: 2026-07-29
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0072]] future work. Found while adding the new
      `current_prices` columns to `current_price_usd`: that view had to move to
      `CREATE OR REPLACE` because `CREATE VIEW IF NOT EXISTS` does not redefine
      a view that already exists. The other five views in `views.sql` still
      carry the same footgun.
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      Scope grew. The PR #158 review found that `CREATE OR REPLACE VIEW`
      requires a `DROP VIEW` grant on CH 26.3.10.60 — unconditionally, even
      when the view does not exist — so `prices-clickhouse-init` now aborts for
      any scoped applier, and the prod `prices_*` users are XML-managed in BE's
      `services.xml` and cannot be SQL-GRANTed by us. 0072 made that true for
      one view; this task would make it true for all six. Recorded as a
      prerequisite with three resolution options; deciding between them is now
      the substance of the task rather than the `sed`. Also carries the
      correction of the "every statement is CREATE … IF NOT EXISTS" doc claims
      that 0072 falsified.
---

# views.sql edits silently don't land on an already-provisioned target

## Summary

Five of the six views in `packages/prices-clickhouse/schema/views.sql` are
declared `CREATE VIEW IF NOT EXISTS`. On any target that already has them — i.e.
ch-prod-01 — editing a view's body and re-applying the file **silently no-ops**.
The apply reports success, the definition does not change, and nothing surfaces
the divergence between the repo and the live cluster.

Affected: `usd_reference`, `price_usd_series`, `usd_reference_1h`,
`price_usd_series_1h`, `identity_by_contract`. (`current_price_usd` was converted
by task 0072.)

## Context

Demonstrated on local CH 26.3.10.60 during 0072: with a v1-shaped
`current_price_usd` in place, applying a changed `CREATE VIEW IF NOT EXISTS`
definition left it at 6 columns; the `CREATE OR REPLACE` form took it to 13.

This is a plausible contributor to the class of drift [[0076]] had to reconcile
by hand. It is latent rather than active — these five views have not needed a
body change yet — but the failure mode is silent, which is the expensive part:
the next person to edit one will believe it deployed.

Plain views replace atomically, so there is no DROP window and no read-side
exposure. This is not true of the refreshable MVs (`current.sql`, `rollups.sql`),
which genuinely require DROP + re-CREATE — leave those alone.

## ⚠️ Prerequisite — `CREATE OR REPLACE VIEW` needs a `DROP VIEW` grant

Found in the PR #158 review (2026-07-29), **after** 0072 shipped the first
conversion. This is the real work of this task; the `sed` is trivial by
comparison.

Verified on CH 26.3.10.60: a user holding only `CREATE VIEW, SELECT ON prices.*`
gets

```
Code: 497. DB::Exception: … Not enough privileges.
(Missing permissions: DROP VIEW ON prices.current_price_usd)
```

— and it fails **even when the view does not exist yet**, so the grant is
unconditional, not a rollout-time-only need.

Consequences:

- `apply_sql` propagates the first error, and `prices-clickhouse-init` applies
  `VIEWS_SQL` unconditionally, so the init binary now **aborts** for any scoped
  applier. 0072 made that true for one view; this task makes it true for all six.
- The prod `prices_*` users are **XML-managed in BE's `services.xml`** and cannot
  be SQL-`GRANT`ed by us — the same constraint that moved the [[0133]] alarm to
  BE. Widening them is a BE-side change with its own coordination cost.
- The doc comments on `prices-clickhouse-init.rs:3` and `lib.rs:120` still claim
  *"Idempotent — every statement is `CREATE … IF NOT EXISTS`"*. 0072 falsified
  that and did not correct it; this task should.

Not currently biting: the 0072 rollout applies `views.sql` via
`docker exec … clickhouse-client`, which runs as the container's `default` user.
So the failure only appears the first time a scoped user runs the init path.

**Decide before converting** — the options are not equivalent, and picking one
is arguably the whole design content of this task:

1. Ask BE to add `DROP VIEW ON prices.*` to the prices users in `services.xml`.
   Cleanest end state, but a cross-team dependency, and it hands the applier a
   privilege it only needs for a no-op-shaped statement.
2. Split the view DDL out of the init path so scoped appliers never execute it,
   and apply `views.sql` only as a privileged operation (which is already how
   the runbook does it).
3. Make `apply_sql` tolerate `Code: 497` for view statements specifically. Cheap,
   but it re-introduces a silent no-op — the exact failure class this task
   exists to remove. Probably reject; recorded so it is not re-derived.

## Implementation

- Convert the five `CREATE VIEW IF NOT EXISTS prices.<v>` to
  `CREATE OR REPLACE VIEW prices.<v>`.
- Check whether `views_it.rs`'s `rewrite()` helper still works — it rewrites
  `"IF NOT EXISTS prices"` onto the scratch database name, and that branch
  becomes dead for converted views.
- `views_sql_has_six_create_view_statements` in `prices-clickhouse/src/lib.rs`
  asserts the statement list; extend it to assert the form, so a future view
  added with `IF NOT EXISTS` fails the build rather than shipping the footgun.
- Consider the same audit for `init.sql` — `CREATE TABLE IF NOT EXISTS` is
  correct there (tables must not be recreated), so this is views-only.
- Resolve the grant prerequisite above **first** — converting the other five
  before deciding just widens a known break.
- Correct the falsified idempotency claims in `prices-clickhouse-init.rs:3` and
  `lib.rs:120`, and record the grant requirement where an operator will meet it
  (the `views.sql` header and the init binary's doc comment).
- Reuse the upgrade-path test 0072 added — `views_sql_replaces_an_existing_v1_current_price_usd`
  in `views_it.rs` — as the pattern for the other five: seed the old shape, apply,
  assert it changed, with an `IF NOT EXISTS` control run to keep it non-vacuous.

## Acceptance Criteria

- [ ] The `DROP VIEW` grant question is decided and recorded (option 1/2/3
      above), and the chosen path is implemented — not just noted.
- [ ] All six views in `views.sql` use `CREATE OR REPLACE VIEW`.
- [ ] A unit test fails if any view in `views.sql` is declared `IF NOT EXISTS`.
- [ ] `prices-clickhouse-init` runs to completion as a **scoped** user (or the
      view DDL is provably no longer on its path).
- [ ] The "every statement is `CREATE … IF NOT EXISTS`" doc claims are corrected.
- [ ] `views_it.rs` passes against pinned CH 26.3.10.60.
- [ ] Re-applying `views.sql` to a target that already has the old definitions
      demonstrably updates them (verify on local CH before prod), with an
      `IF NOT EXISTS` control proving the assertion is non-vacuous.
