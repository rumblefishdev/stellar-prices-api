---
id: "XXXX"
title: "Rollup MVs run in dependency order and reconcile against their source tier"
status: accepted
deciders: [akot]
related_tasks: ["0143", "0203", "0142", "0202", "0200", "0287"]
related_adrs: ["0007", "0287"]
tags: [clickhouse, rollups, refreshable-mv, depends-on, reconciliation, data-correctness, observability]
links:
  - "../1-tasks/active/0143_BUG_rollup-mv-refresh-ordering-race.md"
  - "../1-tasks/active/0203_FEATURE_rollups-self-heal-by-event-time-completeness.md"
  - "../../packages/prices-clickhouse/src/rollup_sql.rs"
  - "../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../packages/prices-clickhouse/src/drift.rs"
  - "../../packages/rollup-freshness-probe/src/refresh_waits.rs"
  - "../../packages/rollup-freshness-probe/src/reconcile_mismatch.rs"
  - "../../infra/src/lib/stacks/observability-stack.ts"
  - "../../docs/runbooks/0142-rollup-mv-reapply.md"
  - "../../docs/runbooks/0286-reingest-history.md"
history:
  - date: "2026-09-28"
    status: accepted
    who: akot
    note: >
      Written with tasks 0143 and 0203, which ship on one branch. The
      architecture was approved on 2026-09-28 after a spike and research on
      ClickHouse 26.3.10.60. The in-place MODIFY REFRESH rollout was decided the
      same day. Every behaviour below is pinned by an IT on the 26.3.10.60 pin
      that was seen to fail with its defect restored. The reconcile pass's peak
      memory was measured against the prod per-query quota. Not deployed yet:
      the rollout waits for task 0286 phase 3.
---

# ADR XXXX: Rollup MVs run in dependency order and reconcile against their source tier

**Related:**

- [Task 0143: the rollup MV cascade has no DEPENDS ON](../1-tasks/active/0143_BUG_rollup-mv-refresh-ordering-race.md)
- [Task 0203: rollups self-heal by event-time completeness](../1-tasks/active/0203_FEATURE_rollups-self-heal-by-event-time-completeness.md)
- [Task 0202: coarse tiers holed by the disk-full ingest stall](../1-tasks/archive/0202_BUG_coarse-tiers-holed-by-disk-full-ingest-stall.md)
- [Task 0142: rollup MV edits silently no-op](../1-tasks/archive/0142_BUG_rollup-mv-edits-silently-no-op.md)
- [Task 0200: is the cleanup worker still needed](../1-tasks/backlog/0200_RESEARCH_is-the-cleanup-worker-still-needed/)
- ADR 0007 (rollups live in ClickHouse), ADR 0287 (one candle definition on every tier)

---

## Context

The six coarse candle tiers are refreshable `APPEND` MVs:
`1m → 15m → 1h → 4h → 1d → {1w, 1M}`. Each re-rolls a window measured from
`now()` out of the tier directly below, into a `ReplacingMergeTree(version)`,
projecting `sum(version)`. Each MV fires on its own wall clock. Two different
failures follow from that.

**1. Same-slot race (task 0143, observed 2026-08-04).** Nothing orders a tier
after the tier it reads. On 2026-08-04 at 00:00 `mv_ohlcv_1w_to_1M` read
`price_ohlcv_1w` while `mv_ohlcv_1d_to_1w` was still writing the new week. The
month re-appended July, and `price_ohlcv_1M` stayed 34 days stale with every MV
reporting success. Since task 0286 both dailies read `price_ohlcv_1d`, so the
race is now `mv_ohlcv_4h_to_1d` (which fires at 00:00) against the two dailies
(also at 00:00).

**2. Holes the clock window never revisits (task 0203, observed 2026-08-13).**
A disk-full stall held ingest for 11.5 h (task 0202). The ledger-processor's
durable cursor then back-filled `price_ohlcv_1m`. At 07:56 it wrote rows
labelled 21:00, 22:00, 23:00 and later. By then those buckets were older than
the 2-hour window of `mv_ohlcv_1m_to_15m`, and every tier reads only the tier
below it, so the hole propagated up the whole chain and was never repaired.
The tip stayed current, so task 0137's freshness alarm saw nothing: a hole
behind a healthy tip is invisible to a staleness check.

The obvious fix for failure 2, rebuilding from the target's own tip
(`least(now() - window, max(target))`), would have caught that one incident.
It is wrong in general. Outages are intermittent, so a bucket is often built
from a partial source, the tip moves past it, and the late rows for that bucket
land behind the tip. That is 0202's partial bucket, which reads as a real but
quiet hour.

---

## Decision

1. **The fast chain runs in dependency order (0143).** Every fast MV except
   `mv_ohlcv_1m_to_15m` carries a fully qualified `DEPENDS ON` the MV that
   writes its source table: `15m_to_1h → 1m_to_15m`, `1h_to_4h → 15m_to_1h`,
   `4h_to_1d → 1h_to_4h`, `1d_to_1w → 4h_to_1d`, `1d_to_1M → 4h_to_1d`. A
   dependent's slot waits until its dependency has refreshed for the same slot.
   Bodies, windows and cadences are unchanged. The dependency is derived from
   `TIERS` (`rollup_sql::dependency`), never kept by hand. Names are qualified
   because ClickHouse stores them qualified and the drift check compares the
   stored text.

2. **Six hourly reconciliation MVs (0203)**, `mv_reconcile_1m_to_15m` …
   `mv_reconcile_1d_to_1M`, one per coarse tier. Each one:
   - is `REFRESH EVERY 1 HOUR … APPEND`, `TO` the same table as the tier's fast MV;
   - reads a bucket-aligned window, `>= toStartOfInterval(now() - INTERVAL 7 DAY, <tier interval>)`,
     so every compared bucket is whole;
   - wraps `rollup_select` verbatim (no second copy of the aggregation, which
     keeps ADR 0287 §4's one definition), and emits a rollup row only where
     `(timestamp, asset_id, quote_asset_id, source, trade_count, volume_base)`
     is `NOT IN` the target `FINAL` over the same bound. That covers both a
     missing bucket and a disagreeing one;
   - **never compares `version`.** Enrichment re-inserts `_1m` at `version + 1`,
     and the coarse sweep bumps coarse rows `+1`, so version is not a
     completeness test;
   - **rewrites only closed buckets** whose end is at least `MISMATCH_GRACE`
     (2 h) old. The open bucket of every tier stays the fast MV's alone, so the
     reconcile pass never becomes the live writer of 1d/1w/1M, and on a live
     system a pass that finds nothing missed writes nothing. Two hours is the
     fast 15m window, so every 15m bucket has one writer or the other; every
     coarser fast window is wider and carries a repaired child into the open
     parent on its own next slot.

   The reconcile MVs are chained among themselves the same way
   (`mv_reconcile_15m_to_1h DEPENDS ON mv_reconcile_1m_to_15m`, …), so one
   hourly pass repairs 15m → 1h → 4h → 1d → {1w, 1M}.
   `mv_reconcile_1m_to_15m` depends on nothing. **There are no cross edges:**
   no fast MV depends on a reconcile MV (fresh data must never wait for the
   backstop), and no reconcile MV depends on a fast one (a stopped or failing
   fast MV must not silently block the backstop that exists for that case).
   A unit test pins this.

3. **Window and cadence are generator constants**, in one place:
   `RECONCILE_WINDOW = "INTERVAL 7 DAY"`, `RECONCILE_REFRESH = "EVERY 1 HOUR"`,
   and `MISMATCH_GRACE = "INTERVAL 2 HOUR"`, which bounds both the reconcile
   pass and the signal below, so the signal counts exactly what the next pass
   would write.

4. **Completeness is observable.** `rollup-freshness-probe` publishes:
   - `RollupMismatchBuckets` per coarse table. This is the reconcile SELECT
     wrapped in a `count()`, so it covers the same closed buckets that ended at
     least 2 h ago.
     The SQL comes from the generator. One alarm per table fires at 6 of 6
     fifteen-minute periods, which is longer than one reconcile cycle.
   - `MvRefreshWaitingCount`: any of the 12 MVs in `WaitingForDependencies` for
     longer than one of its own periods.
   - `MvRefreshDisabledCount`: any of the 12 stopped.
   - `MvRefreshFailingCount`: any of the 12 not stopped whose last refresh left
     an error in `exception`, or that has not succeeded for more than 2 of its
     own periods while not waiting. Nothing depends on the four 1w/1M leaves,
     so a leaf that fails on every slot makes nothing wait. The reconcile MVs
     repair a dead fast leaf's closed buckets, so the freshness alarm sees it
     only once the open bucket it never writes ages past that tier's bound,
     which takes days for 1w/1M.
   - `MvRefreshUnreadable`: `system.view_refreshes` is denied, or none of the 12
     views is visible. In that case it publishes no count, never a 0.

   The drift check treats a missing or wrong `DEPENDS ON` as `refresh` drift,
   and two declared writers per coarse table as normal.

5. **Rollout in place.** The five fast dependents change with
   `ALTER TABLE … MODIFY REFRESH <full clause>`, which leaves no exposure
   window. The six reconcile MVs are `CREATE`d. DROP + CREATE per runbook 0142
   stays documented as the fallback. The statements are generator output, and
   a unit test pins them into runbook 0142.

---

## Rationale

- **Reconciliation is selected by event time, not arrival time.** A bucket is
  rebuilt because its source disagrees with it, whatever order the rows arrived
  in. The ITs pin reversed arrival, a back-dated hole behind a healthy tip, and
  a partial bucket completed later. That is the property the clock window and
  the tip bound both lack.
- **The overwrite safety already exists.** A superset of children has a higher
  `sum(version)` than a subset, so a complete re-roll outranks a partial row in
  the RMT. Reconciliation changes which closed buckets are re-rolled, not how
  a row wins, and it leaves the open bucket to the fast MVs. See the version
  analysis below.
- **It stays inside ClickHouse**, where ADR 0007 put the rollups. There is no
  new runtime, no network hop and no second copy of the aggregation.
- **Cost is bounded.** A second pass that finds nothing appends nothing (pinned
  on `system.view_refreshes.written_rows = 0`), and the heaviest pass stays well
  inside the per-query quota (below).

---

## Alternatives Considered

### Insert-triggered MVs

**Description:** Classic MVs that fire on each insert into the tier below.

**Cons:** Into an aggregating target they double-count the RMT rewrites that
enrichment and the sweep make. The spike reproduced 350 against a true 250,
the same finding as tasks 0059 and 0282. They also only see new inserts, so
they cannot repair a bucket whose source is already there.

**Decision:** REJECTED — they give wrong totals.

### Timescale-style invalidation log

**Description:** Every writer records the (series, bucket) ranges it touched,
and a job re-rolls only those.

**Cons:** It needs a hook in every writer (ingest, backfill, enrichment, the
events path) and a new table. A single writer that forgets the log silently
reintroduces the hole. Ingest and enrichment changes are out of scope, and the
comparison gets the same answer without trusting the writers.

**Decision:** REJECTED — too many trusted writers; comparing with the source
needs none.

### Wider `now()` windows on the fast MVs

**Description:** Raise the 2-hour window of `mv_ohlcv_1m_to_15m`, and the others.

**Cons:** It is still a clock window. Any stall longer than the new width holes
every tier exactly as before, and every per-minute pass re-rolls the whole
width, paying the cost where latency matters.

**Decision:** REJECTED — it moves the cliff and makes the hot path heavier.

### A Lambda outside ClickHouse

**Description:** A scheduled job reads both tiers and writes the repairs.

**Cons:** ADR 0007 eliminated the rollup Lambda. Bringing one back adds a
runtime, credentials, a network path over mTLS, timeouts, and a second
implementation of the aggregation that could drift from `rollup_select`.

**Decision:** REJECTED — conflicts with ADR 0007, with no gain.

### Tip-based lower bound

**Description:** Rebuild from `least(now() - window, max(target.timestamp))`.

**Cons:** It assumes data arrives in order. An intermittent outage builds a
partial bucket, the tip moves past it, and the back-fill for that bucket is
never read (0202's partial-bucket failure).

**Decision:** REJECTED — it would have caught 2026-08-13, but not the
intermittent shape these outages normally take.

---

## Version analysis (overwrite safety)

- **The superset wins by a wide margin.** `_1m.version` is
  `ledger · 1000 + operation_index`, about 6·10¹⁰ per row. A re-roll that adds
  even one missing child exceeds a sweep-bumped (`+1`) partial by about 6·10¹⁰.
  The ITs use ledger-scale versions and pin that a complete re-roll beats a
  sweep-bumped partial in every tier.
- **An equal-version tie goes to the last insert.** ClickHouse documents it
  for `ReplacingMergeTree(ver)`: among rows with the same sorting key and the
  same maximum `ver`, the last inserted one is kept. A reconcile row written
  after the stale row it ties with therefore wins, until a later writer
  (a fast MV, the sweep) replaces it.
- **Adding a missing child always wins; replacing one may not.** A re-roll
  that adds a child outranks the partial by that child's whole version. A
  re-roll that only REPLACES children (same minutes, different `trade_count`
  or `volume_base`) outranks the stale row only by the replaced children's
  version delta, and the stale row may carry coarse bumps on top of its old
  sum: `+1` per sweep run, `+2` per `coarse-repair` run. So with real
  versions:
  - delta greater than the bumps: the re-roll wins, as intended;
  - delta equal to the bumps: a tie, which the re-roll wins as the last
    insert;
  - delta smaller than the bumps: **the re-roll loses on every pass.** An
    equal-version whole-minute rewrite of `_1m` under a bumped coarse row is
    one such case. The bucket is re-emitted every hour and never converges,
    so its mismatch alarm fires permanently.

  This needs existing minutes rewritten at nearly the same version under a
  bumped coarse row, for example the same ledgers re-processed with a
  different candle definition. A back-fill that adds fills does not produce
  it. It is unlikely, but not impossible with real versions. The permanent
  mismatch is the signal. A pre-roll of that bucket loses the same way, so
  the repair is manual: write the child at a higher version, or delete the
  stale coarse row. The ITs pin the "adds a child" case with ledger-scale
  versions and do not use toy versions, where small sums make ties common.
- **The loss case is a re-ingest.** When the source is rewritten at a lower
  `sum(version)` than the enriched coarse rows (task 0286's re-ingest drops a
  `_1m` month and refills it), the reconcile rows lose in the RMT. They are
  re-emitted every hour, the mismatch never converges and the alarm fires. That
  is the correct signal, but it is noise during planned work. **The runbooks
  therefore STOP the six reconcile MVs for any re-ingest or coarse rebuild**
  (0286-reingest-history §1a/§7f; the other runbooks point there).
- **`close_usd` can briefly go to 0.** A re-emitted coarse row re-derives
  `close_usd` from its children. If they are not priced yet, a sweep-priced
  coarse `close_usd` reads 0 until the next sweep pass. The sweep covers the
  current and previous month, which contains the window. The fast MVs already
  do the same inside their windows, so this is not new.
- **Enrichment-only changes are not propagated, by design.** Enrichment does
  not change `trade_count` or `volume_base`, so a `_1m` `version + 1` bump
  triggers no reconcile write and loses no bucket (pinned). Pricing the coarse
  tiers stays the sweep's job.

---

## Consequences

### Positive

- A hole younger than 7 days heals with no operator action, whatever order the
  back-fill arrives in. Closed buckets heal within one to two hourly passes
  once they are 2 h past their end. A still-open coarse bucket (today, this
  week, this month) takes the repaired child on its fast MV's next slot.
- The 00:00 race is gone: the dailies include the day `mv_ohlcv_4h_to_1d`
  writes in the same slot (pinned).
- A hole behind a healthy tip is now visible (`RollupMismatchBuckets`). A `_1m`
  hole shows on `price_ohlcv_15m` first, because each tier is compared only with
  the tier directly below. Until the pass propagates the repair, the coarser
  tiers agree with their equally holed source. A coarser table mismatching on
  its own means the chain broke part-way.
- The rollout has no exposure window: `MODIFY REFRESH` in place, and
  ClickHouse refuses to add or remove `APPEND` that way, so replace mode cannot
  be reintroduced by it.

### Negative

- **An outage longer than 7 days is still manual.** Buckets outside the window
  are never compared. Pre-roll them with `schema/preroll-live-gap.sql`
  (runbook 0142).
- **The window is bounded by `_1m` retention (task 0200).** Nothing heals from
  data that was dropped. **A `_1m` retention shorter than 7 days makes the 15m
  mismatch alarm fire permanently** unless `RECONCILE_WINDOW` is lowered with
  it. The 15m bucket that straddles the cleanup cutoff has only part of its
  source left. Its re-roll has a lower `sum(version)` than the stored row, so
  it loses on every hourly pass and is re-emitted, and
  `RollupMismatchBuckets{price_ohlcv_15m}` never reaches 0. Cleanup moves the
  cutoff every run, so there is always such a bucket. Whoever turns cleanup
  back on lowers `RECONCILE_WINDOW` below the retention, less one 15m bucket
  and one cleanup interval, in the same change.
- **Every re-ingest must STOP the reconcile MVs, and STOP is lost on a server
  restart.** A forgotten STOP is alarmed (`MvRefreshDisabledCount`), which
  means that alarm also fires, as expected, during every planned re-ingest.
- **A blocked dependency stalls the chain silently.** A stopped, failing or
  misspelled dependency leaves its dependents in `WaitingForDependencies` with
  no error, and `SYSTEM REFRESH VIEW` ignores dependencies. Hence the waiting,
  disabled and unreadable alarms.
- **The probe's view of this depends on a grant.** `system.view_refreshes` is
  denied (Code 497), not filtered, to an identity with only
  `SELECT ON prices.*`. Prod needs `SELECT ON system.view_refreshes` for the
  probe identity (`prices_writer`, XML-managed by BE, BE task 0477). Until then
  the unreadable alarm fires by design.
- **Reconciliation and the mismatch count are one-sided.** Both start from
  the source aggregate, so they see a target bucket that is missing or
  disagrees with its source, never an extra one. A target row with no source
  rows in the window is neither counted nor repaired: one left by a removed or
  rogue writer, or one whose source was deleted. The drift check catches an
  undeclared writer; a stale row whose source was deleted needs a manual
  delete.
- **A repair reaches an open coarse bucket only at its fast MV's cadence**:
  within 4 h for the open day, and within a day for the open week and month.
  The reconcile pass never writes an open bucket, by design (it must not
  become the live writer of the coarse tiers).
- **The first CREATE of each reconcile MV runs a full 7-day comparison at
  once**, because a new refreshable MV refreshes at creation even with
  `DEPENDS ON`. The runbook creates them one at a time, off-peak.
- **`system.view_refreshes.written_rows` is not a literal row count** on
  26.3.10.60. It read 256 × the buckets appended. It is exactly 0 when nothing
  was written, and that is the only way to read it.
- **Memory.** Peak memory of the heaviest pass (`mv_reconcile_1m_to_15m`, the
  only one reading `_1m`), measured locally on 26.3.10.60 from
  `system.query_log.memory_usage`. About 3 M `_1m` rows over the 7-day window
  (prod's busiest day is 413 k rows), ledger-scale versions, `max_threads` 8.
  The gate was 50 % of the 5.59 GiB prod per-query quota:

  | shape                         | (series, 15m) groups | op               | peak       | % of 5.59 GiB |
  | ----------------------------- | -------------------- | ---------------- | ---------- | ------------- |
  | prod-like dense, 300 series   | 200,100              | full emit        | 238.15 MiB | 4.2 %         |
  | prod-like dense, 300 series   | 200,100              | count, all agree | 105.24 MiB | 1.8 %         |
  | prod-like sparse, 2000 series | 1,200,000            | full emit        | 1.19 GiB   | 21.3 %        |
  | prod-like sparse, 2000 series | 1,200,000            | count, all agree | 439.63 MiB | 7.7 %         |
  | worst case, 1 row per group   | 2,997,000            | full emit        | 2.26 GiB   | 40.4 %        |
  | worst case, 1 row per group   | 2,997,000            | count, all agree | 1.02 GiB   | 18.3 %        |

  Memory scales with group cardinality, not rows, and a whole month in `_1m`
  adds read time, not group memory. The rollout checklist therefore starts with
  a read-only cardinality count on prod. A "full emit" is the first CREATE
  against an empty target, which is the worst a pass can do.

---

## References

- `packages/prices-clickhouse/src/rollup_sql.rs` — `TIERS`, `dependency`,
  `mv_ddl`, `mv_modify_refresh`, `reconcile_select`, `reconcile_mv_ddl`,
  `reconcile_mismatch_select`, the three constants
- `packages/prices-clickhouse/schema/rollups.sql` — the twelve generated statements
- `packages/prices-clickhouse/tests/rollup_reconcile_it.rs`,
  `rollup_order_it.rs`, `rollup_drift_it.rs` — the behaviour on 26.3.10.60
- `docs/runbooks/0142-rollup-mv-reapply.md` — rollout, fallback, verification, rollback
- `docs/runbooks/0286-reingest-history.md` §1a / §7f — stopping reconciliation
  during a re-ingest
- ClickHouse refreshable MVs: `DEPENDS ON`, `APPEND`, `MODIFY REFRESH`,
  `SYSTEM STOP/START/WAIT VIEW`, `system.view_refreshes`
