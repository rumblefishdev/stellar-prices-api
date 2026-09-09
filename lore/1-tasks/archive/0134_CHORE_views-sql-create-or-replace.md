---
id: "0134"
title: "views.sql edits silently don't land — convert the remaining five views to CREATE OR REPLACE"
type: CHORE
status: completed
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
  - date: 2026-08-03
    status: completed
    who: okarcz
    note: >
      All six views in `views.sql` now use `CREATE OR REPLACE VIEW`. Two new
      tests (unit 8→9, views_it 3→4): a build-breaking form guard, proven
      non-vacuous by reverting a view, and a stub-based integration test that
      rewinds all six views and asserts the shipped form replaces them while the
      `IF NOT EXISTS` control leaves them standing. 13/13 integration tests green
      on local CH pinned to 26.3.10.60. Five files touched plus two stale doc
      sites the plan had not listed (schema-overview sample DDL, crate README).
      Two plan premises turned out wrong and are recorded: `views_it.rs`'s
      `rewrite()` branch is NOT dead — it serves `init.sql`'s unqualified
      `CREATE DATABASE` and removing it would target the real `prices` database —
      and `lib.rs:120` was never falsified, being scoped to `INIT_SQL`. Prod
      needs no re-apply: the converted bodies are identical to what is live, so
      only future edits change behaviour. Spawned [[0142]] — `rollups.sql` has
      the same footgun with no `OR REPLACE` escape.
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
- [x] All six views in `views.sql` use `CREATE OR REPLACE VIEW`.
- [x] A unit test fails if any view in `views.sql` is declared `IF NOT EXISTS` —
      `views_sql_uses_create_or_replace_for_every_view`, proven non-vacuous by
      reverting `usd_reference_1h` and confirming the failure.
- [x] `views.sql` is provably off the scoped-user path, and the privileged-applier
      requirement is documented where the init path is described (views.sql
      header, `VIEWS_SQL` const, init binary, crate README, schema overview).
- [x] The "every statement is `CREATE … IF NOT EXISTS`" doc claims are corrected.
- [x] `views_it.rs` passes against pinned CH 26.3.10.60 — 4/4, plus all 13
      `prices-clickhouse` integration tests green on the same engine.
- [x] Re-applying `views.sql` to a target that already has the old definitions
      demonstrably updates them (verified on local CH), with an
      `IF NOT EXISTS` control proving the assertion is non-vacuous.

## Implementation Notes

Landed as planned; the option-2 decision needed no revisiting.

**Schema** — `packages/prices-clickhouse/schema/views.sql`:

- Five conversions: `usd_reference`, `price_usd_series`, `usd_reference_1h`,
  `price_usd_series_1h`, `identity_by_contract`. All six statements are now
  `CREATE OR REPLACE VIEW`.
- New file-header sections: **"Statement form"** (why `IF NOT EXISTS` is banned
  here, why the refreshable MVs keep their own form) and **"⚠️ This file requires
  a PRIVILEGED applier"** (the `Code: 497` text, the measured grants, the
  `docker exec` loopback path, and that option 1 was rejected).
- The per-view `CREATE OR REPLACE` note on `current_price_usd` said "which the
  other views here still use" — now stale, rewritten to point at the header.

**Rust** — `packages/prices-clickhouse/`:

- `src/lib.rs`: new test `views_sql_uses_create_or_replace_for_every_view`
  asserting every statement is `CREATE OR REPLACE VIEW` and none contains
  `IF NOT EXISTS`, with a `stmts.len() == 6` non-vacuity guard. Unit tests 8 → 9.
- `src/lib.rs`: the `VIEWS_SQL` const doc now states the form and the
  privileged-applier consequence.
- `src/bin/prices-clickhouse-init.rs`: the "every statement is
  `CREATE … IF NOT EXISTS`" claim replaced with the true split (tables
  `IF NOT EXISTS`, views `OR REPLACE`, so a re-run re-applies view definitions
  rather than skipping them), plus a privileged-user warning.
- `tests/views_it.rs`: new `views_sql_replaces_every_existing_view` — rewinds all
  six views to a `SELECT 1 AS stub_sentinel` stub, runs the `IF NOT EXISTS`
  control (asserting all six stubs SURVIVE), then applies the shipped form and
  asserts every stub is gone and each view exposes a column only the real
  definition has. New `view_columns` helper. Integration tests 3 → 4.

**Docs** — two stale sites the plan had not listed:

- `docs/database-schema/database-schema-overview.md` §3.2 showed the sample DDL
  for `price_usd_series` and `usd_reference` as `CREATE VIEW IF NOT EXISTS` —
  both corrected, plus a blockquote on the privileged-applier requirement.
- `packages/prices-clickhouse/README.md` opened with "applied idempotently by
  the `prices-clickhouse-init` binary" with no mention of the DDL grants needed
  — blockquote added.

**Verification** — local CH pinned to 26.3.10.60 (the exact ch-prod-01 version):

```
cargo test -p prices-clickhouse            →  9 passed
cargo test -p prices-clickhouse -- --ignored → 13 passed (4 views_it, 3 current_mv,
                                               3 rollup_append, 2 preroll, 1 seed)
cargo fmt --check → clean;  cargo clippy → no new warnings
```

Nothing was applied to ch-prod-01. Prod is unchanged by this task and does not
need a re-apply: the five converted views' **bodies** are byte-identical to what
is live, so the conversion only changes what a *future* edit does.

## Issues Encountered

- **The plan's `rewrite()` premise was wrong, and the branch is still
  load-bearing.** The plan said `views_it.rs`'s `.replace("IF NOT EXISTS
  prices", …)` "becomes dead for converted views". It was never about the views:
  the *first* replace (`"prices."` → `"{db}."`) already catches every view, since
  all six are qualified `prices.<name>`. The only statement reaching the second
  replace is `init.sql`'s unqualified `CREATE DATABASE IF NOT EXISTS prices` —
  and deleting it would make `setup_scratch` create the REAL `prices` database
  instead of the scratch one. Left in place, with a comment recording why, so it
  is not "cleaned up" by the next reader.

- **`lib.rs:120` was not actually falsified.** The plan listed it alongside
  `prices-clickhouse-init.rs:3` as a claim 0072 broke. It reads "Apply
  [`INIT_SQL`] … every statement is a `CREATE … IF NOT EXISTS`" — scoped to
  `INIT_SQL`, which *is* uniformly `IF NOT EXISTS`, so it was and remains
  correct. Left unchanged; the doc correction went to the `VIEWS_SQL` const
  (where a reader actually looks for view semantics) and to the init binary,
  whose claim genuinely did cover the whole apply.

- **`init.sql` audit: nothing to do**, as the plan expected. All 15 statements
  are `CREATE DATABASE/TABLE IF NOT EXISTS`, which is correct — tables must not
  be recreated. Views-only confirmed.

## Design Decisions

### From Plan

1. **Option 2 — `views.sql` is a privileged, operator-applied artifact.**
   Decided 2026-07-30 on measured grants; unchanged by implementation. Documented
   rather than worked around.

### Emerged

2. **The guard test asserts the form for ALL statements, not a per-view
   allowlist.** A future view added to the file is covered automatically; an
   allowlist would have to be remembered. The `len() == 6` assertion keeps it
   non-vacuous, and doubles as the tripwire for anyone adding a seventh view
   without reading the header.

3. **Stub-based integration test instead of per-view "v1 shapes".** The plan
   suggested reusing 0072's pattern — seed the genuine old definition, re-apply,
   assert it changed. For five views with no meaningful "v1", a
   `SELECT 1 AS stub_sentinel` stub is a stronger assertion (it shares NO column
   with the real view, so a partial replace cannot pass) and does not encode
   historical shapes that would rot. 0072's own test keeps its real v1 shape,
   since ch-prod-01 genuinely held that definition.

4. **Kept 0072's per-view `CREATE OR REPLACE` assertion** in
   `views_sql_current_price_usd_forwards_every_current_prices_column` even though
   the new test subsumes it — it is the anchor explaining *why* 0072 needed the
   form, and deleting it would lose that reasoning at the site that motivates it.

5. **Fixed two doc sites the plan did not list.** The schema-overview sample DDL
   was the higher-value one: it is the document BE reads, and it would have kept
   teaching the banned form after the repo stopped using it.

6. **Did not convert the task file to a directory** despite exceeding the ~150
   line guidance in `1-tasks/CLAUDE.md` — no notes, benchmarks or sources were
   produced, and the repo has ample precedent for long single-file tasks.

## Future Work

- [[0142]] — `rollups.sql` carries the identical silent no-op
  (`CREATE MATERIALIZED VIEW IF NOT EXISTS`, no `DROP` anywhere in the file), and
  a refreshable MV has no `OR REPLACE` escape, so the fix is a DROP + re-CREATE
  procedure with real data-loss and freshness exposure rather than a one-word
  change. Deliberately out of scope here; spawned to backlog.
