---
id: "0134"
title: "views.sql edits silently don't land — convert the remaining five views to CREATE OR REPLACE"
type: CHORE
status: active
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
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      Grants measured on ch-prod-01 — the prerequisite is smaller than the
      review implied. `prices_writer` holds only SELECT/INSERT/ALTER
      DELETE/OPTIMIZE on prices.* and `prices_reader` only SELECT; neither has
      DROP VIEW, and neither has CREATE VIEW / CREATE TABLE / CREATE DATABASE
      either. So `prices-clickhouse-init` was never runnable as a scoped user —
      0072 did not introduce a break, and schema DDL has always been an operator
      action as the container's `default` user (the docker-exec loopback path,
      which bypasses Caddy and the mTLS CN map). **Decision: option 2** — keep
      view DDL off the scoped path and document it, rather than request a broad
      DDL grant from BE (option 1, now known to be insufficient as well as
      over-broad) or swallow Code: 497 (option 3, re-introduces the silent
      no-op). No BE dependency; the conversion work itself is unstarted.
  - date: 2026-08-03
    status: active
    who: okarcz
    note: >
      Activated. The prerequisite decision (option 2) was already settled on
      measured ch-prod-01 grants, so the remaining work is local and
      dependency-free: the five conversions, a form assertion in the unit test,
      the falsified idempotency doc claims, and the `views_it.rs` `rewrite()`
      branch. The [[0072]] step-4 coupling that held this back is resolved —
      0072 completed and archived earlier today.
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
  `VIEWS_SQL` unconditionally, so the init binary **cannot complete** as a scoped
  applier. 0072 made that true for one view; this task would make it true for all
  six.
- The doc comments on `prices-clickhouse-init.rs:3` and `lib.rs:120` still claim
  *"Idempotent — every statement is `CREATE … IF NOT EXISTS`"*. 0072 falsified
  that and did not correct it; this task should.

### Measured on ch-prod-01, 2026-07-30 — smaller than it first looked

The initial framing assumed a scoped applier existed that this would break. It
does not. Actual grants:

```
prices_writer:  SELECT, INSERT, ALTER DELETE, OPTIMIZE ON prices.*
                SELECT ON system.parts
prices_reader:  SELECT ON prices.*
```

Both `storage = users_xml` (BE's `services.xml`, as expected), and the
`system.grants` DROP/CREATE filter returned **zero rows**.

So neither runtime user has `DROP VIEW` — **nor `CREATE VIEW`, `CREATE TABLE`,
or `CREATE DATABASE`.** `prices-clickhouse-init` could therefore never have run
as `prices_writer` at any point, including before 0072: `init.sql` opens with
`CREATE DATABASE IF NOT EXISTS prices` and would have failed on its first
statement. The review finding is real about the `DROP VIEW` requirement, but
wrong that 0072 *introduced* a break — there is no scoped applier to break.

Schema DDL on ch-prod-01 has always been an operator action as the container's
`default` user over the loopback native port (`docker exec … clickhouse-client`,
no `--user`), which bypasses Caddy and the mTLS CN map entirely. That is how
[[0076]] applied the 0039/0053 schema and how the 0072 runbook applies both
`current.sql` and `views.sql`. The runtime users' DDL-free grants are the
intended design, not an oversight.

### Decision (2026-07-30) — option 2

1. ~~Ask BE to add `DROP VIEW ON prices.*` to the prices users in
   `services.xml`.~~ **Rejected — insufficient and over-broad.** The measurement
   above shows they would also need `CREATE VIEW`, plus `CREATE TABLE` /
   `CREATE DATABASE` for `init.sql` to run at all. That is a large DDL grant to
   the ingestion writer for no operational gain, on top of a cross-team change.
2. **CHOSEN — keep view DDL off the scoped path** and treat `views.sql` as a
   privileged, operator-applied artifact. This is already the de facto reality
   on every path that exists; the task is to make it explicit rather than to
   change behaviour, so it carries no rollout risk and no BE dependency.
3. ~~Make `apply_sql` tolerate `Code: 497` for view statements.~~ **Rejected** —
   re-introduces a silent no-op, the exact failure class this task exists to
   remove. Recorded so it is not re-derived.

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
- Per the option-2 decision: keep `views.sql` off any scoped-user path, and say
  so where someone would otherwise assume it is runnable — the `views.sql`
  header, the init binary's doc comment, and wherever the init path is
  documented. No BE grant request; no `services.xml` change.
- Correct the falsified idempotency claims in `prices-clickhouse-init.rs:3` and
  `lib.rs:120`, and record there that view DDL requires a privileged user
  (`DROP VIEW` in particular), so the next person meets the constraint before
  the error rather than after.
- Reuse the upgrade-path test 0072 added — `views_sql_replaces_an_existing_v1_current_price_usd`
  in `views_it.rs` — as the pattern for the other five: seed the old shape, apply,
  assert it changed, with an `IF NOT EXISTS` control run to keep it non-vacuous.

## Acceptance Criteria

- [x] The `DROP VIEW` grant question is decided and recorded — **option 2**,
      2026-07-30, on measured ch-prod-01 grants. *(Decision only; the code
      changes it implies are the unchecked items below.)*
- [ ] All six views in `views.sql` use `CREATE OR REPLACE VIEW`.
- [ ] A unit test fails if any view in `views.sql` is declared `IF NOT EXISTS`.
- [ ] `views.sql` is provably off the scoped-user path, and the privileged-applier
      requirement is documented where the init path is described.
- [ ] The "every statement is `CREATE … IF NOT EXISTS`" doc claims are corrected.
- [ ] `views_it.rs` passes against pinned CH 26.3.10.60.
- [ ] Re-applying `views.sql` to a target that already has the old definitions
      demonstrably updates them (verify on local CH before prod), with an
      `IF NOT EXISTS` control proving the assertion is non-vacuous.
