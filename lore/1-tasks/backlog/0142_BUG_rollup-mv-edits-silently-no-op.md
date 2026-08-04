---
id: "0142"
title: "rollups.sql edits silently don't land either — the refreshable MVs carry 0134's footgun with no OR REPLACE escape"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0134", "0136", "0095", "0090", "0104", "0143"]
tags: ["priority-medium", "effort-medium", "clickhouse", "schema-drift", "footgun"]
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

## Acceptance Criteria

- [ ] A drift between `rollups.sql` and the live MV definitions is detectable
      rather than silent.
- [ ] A documented, tested procedure exists for changing a rollup MV body on a
      provisioned target, preserving the [[0095]] APPEND invariants.
- [ ] The procedure states its freshness-gap exposure and pairs with [[0137]].
- [ ] `preroll.sql` audited for the same defect.
