---
id: "0134"
title: "views.sql edits silently don't land — convert the remaining five views to CREATE OR REPLACE"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0072", "0076", "0061"]
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

## Acceptance Criteria

- [ ] All six views in `views.sql` use `CREATE OR REPLACE VIEW`.
- [ ] A unit test fails if any view in `views.sql` is declared `IF NOT EXISTS`.
- [ ] `views_it.rs` passes against pinned CH 26.3.10.60.
- [ ] Re-applying `views.sql` to a target that already has the old definitions
      demonstrably updates them (verify on local CH before prod).
