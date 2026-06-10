---
title: "Rollup-chain semantics under enriched _1m re-inserts — decision + proof plan"
type: generation
status: mature
spawns: []
tags: [clickhouse, materialized-views, rollups, replacingmergetree, aggregatingmergetree, refreshable-mv]
links:
  - "https://clickhouse.com/docs/materialized-view/incremental-materialized-view"
  - "https://clickhouse.com/docs/materialized-view/refreshable-materialized-view"
  - "https://clickhouse.com/docs/sql-reference/statements/create/view"
  - "../../../../docs/database-schema/database-schema-overview.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../blocked/0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
  - "../../backlog/0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
history:
  - date: 2026-06-09
    status: developing
    who: okarcz
    note: >
      Decision + proof plan authored without a live ClickHouse. Analyses
      the draft insert-trigger MV DDL in schema-overview §3.2 against CH
      insert-block MV mechanics; finds it under-counts independent of
      enrichment; recommends a refreshable / re-aggregate-from-FINAL
      rollup. Proof execution (local docker CH) deferred to a follow-up.
  - date: 2026-06-09
    status: mature
    who: okarcz
    note: >
      Proof EXECUTED against clickhouse-server 24.8.14 (proof/, run.sh,
      RESULTS.md). Confirmed all predictions plus two new findings: (1)
      the draft §3.2 MV does not even COMPILE — alias collision
      `sum(x) AS x` + re-`sum(x)` → ILLEGAL_AGGREGATION; (5) `max(version)`
      is an insufficient rollup version projection — enriching an early
      minute leaves bucket max unchanged, tying stale vs corrected rollup
      rows. Observed: draft under-counts 150→10; enrichment does not
      propagate (stays 0); re-aggregate-from-_1m-FINAL yields correct 150
      / 500. Refines recommendation toward a true Refreshable MV (atomic
      replace) over the scheduled-ReplacingMergeTree fallback.
  - date: 2026-06-10
    status: mature
    who: okarcz
    note: >
      Durability correction (doc-grounded). The default refreshable MV
      "atomically replaces the table's previous contents" (CREATE VIEW
      ref) — so replace-mode + a bounded WHERE window only ever holds the
      window, and clearing _1m would empty the rollup on the next refresh.
      Durable rollups therefore require APPEND ("inserts rows into the
      table without deleting existing rows"), which puts them back on
      ReplacingMergeTree version dedup → finding #5 (strictly-increasing
      version) applies after all. Corrected the §3 "atomic replace
      sidesteps finding #5" claim (only true with an unbounded window),
      added §3.1 durability subsection with the two ClickHouse doc
      citations, and added an APPEND-not-replace item to the §4 contract.
  - date: 2026-06-10
    status: mature
    who: okarcz
    note: >
      Expanded Option A′ into a full "A vs A′" decision-criteria section
      (refreshable MV vs external scheduled worker). Records that the two
      are the *same* INSERT…SELECT into the same independent table — the
      worker does NOT avoid any cascade/relation risk (there is none) — so
      the fork is decided on operability + CH refreshable-MV support, not
      data model. Adds the comparison table and the explicit decision rule;
      names the cluster CH version / refreshable-MV acceptability as the
      top BE-sync gate.
---

# Rollup-chain semantics under enriched `_1m` re-inserts — decision + proof plan

## 0. TL;DR

> **Status: proof executed** against `clickhouse-server 24.8.14` — see
> [`proof/RESULTS.md`](../proof/RESULTS.md) (`./proof/run.sh` reproduces).
> Every prediction below was observed, plus two findings the desk analysis
> missed: the draft **does not even compile** (alias collision), and
> `max(version)` is an **insufficient rollup version projection**.

The original draft insert-trigger materialised-view rollup that stood in
[`database-schema-overview.md` §3.2](../../../../docs/database-schema/database-schema-overview.md)
(`mv_ohlcv_1m_to_15m`) was **incorrect — and not only under enrichment.**
(§3.2 has since been **rewritten to the corrected pattern recommended by this
note**; what follows is the analysis that drove that rewrite.) A ClickHouse MV
fires on the *inserted block*, aggregating only
the rows in that one INSERT. Because the rollup target is
`ReplacingMergeTree(version)`, the failures below follow.

**Finding 0 (proof-only): the draft does not compile.** Transcribed verbatim it
raises `ILLEGAL_AGGREGATION` — `sum(volume_base) AS volume_base` shadows the
column, and the `vwap` line re-uses `sum(volume_base)`, nesting aggregate in
aggregate. So before any semantic argument, the DDL is non-functional. Fix:
`vwap = volume_quote_usd / nullIf(volume_base, 0)` (reference the aliases).

Even with that fixed, two runtime failures remain:

1. **Multi-block under-count (live path, no enrichment needed).** A 15-minute
   bucket is fed by ~15 separate per-minute INSERTs. Each fires the MV, each
   emits a *partial* `_15m` row with the **same** sort key
   `(asset_id, quote_asset_id, source, 15m-bucket-start)`. `ReplacingMergeTree`
   keeps exactly one — the `max(version)` partial — so the bucket collapses to
   roughly *the last minute's contribution*, not the sum of fifteen. Volumes
   read ~1/15 of truth.
2. **Enrichment double-/mis-count (the 0059 trigger).** When task 0026
   re-INSERTs a corrected `_1m` row with `version+1`, the MV fires again and
   emits another partial for that bucket. Whatever combine-or-replace rule the
   target uses, a *restated* contribution is indistinguishable from a *new*
   one.

The root cause is a fundamental tension: **incremental combine-on-insert and
correction-by-re-insert cannot both be served by a plain insert-trigger MV.**

**Decision (recommended): make the rollups re-aggregate from `_1m FINAL`
rather than sum the insert block** — implemented as a **Refreshable
Materialized View** (ClickHouse ≥ 23.12) or, if the cluster's CH version
can't be relied on, a scheduled `INSERT … SELECT … FROM _1m FINAL` re-aggregate.
Targets stay `ReplacingMergeTree(version)`. The refresh must run in **`APPEND`
mode** (default replace-mode + a bounded window destroys history — §3.1), and
because APPEND goes back through RMT dedup the projected version must be
**strictly increasing** (`sum(version)` / refresh epoch), **not**
`max(source.version)` (finding #5).
This is correct by construction on **both** failure modes, keeps rollups
"inside ClickHouse / no Rollup Lambda" (honouring ADR 0007 §3.4), preserves the
identical-shape target tables (no `AggregateFunction` columns → read path 0040
stays `FINAL`-simple), and yields the *more accurate* `volume_quote_usd`
(Σ of per-minute enriched values, not one coarse-bucket oracle multiply).

This finding is bigger than 0059's original framing: it means **0051 must not
ship the draft insert-trigger DDL as written.** Recommend an ADR-0007 amendment
(or a new ADR) recording the refreshable-rollup decision.

---

## 1. How a ClickHouse MV actually fires (the mechanic everything hinges on)

An insert-trigger MV in ClickHouse is **not** a maintained view of the source
table. It is an **INSERT trigger**: when a block of rows is inserted into the
source table, the MV's `SELECT` runs **over just that block**, and its output
is inserted into the target table. It does **not** read the target, and it does
**not** re-read the rest of the source bucket.

This is ClickHouse's own documented definition, not an inference. From the
[official incremental-MV docs](https://clickhouse.com/docs/materialized-view/incremental-materialized-view)
(retrieved 2026-06-09):

> "a ClickHouse materialized view is just a trigger that runs a query on blocks
> of data as they're inserted into a table."

> "When new rows are inserted into this table, ClickHouse executes the
> materialized view query *only* with those newly inserted rows."

> "the source table in the materialized view's query is replaced with the
> inserted block of data."

That third quote is the mechanically decisive one: inside the MV's `SELECT`,
`FROM price_ohlcv_1m` is substituted with *just the inserted block*, so a
`GROUP BY 15m-bucket` aggregates only the rows in that one INSERT — never a
re-read of the whole bucket. (The docs note the partial results are then sent
to the target table to be "updated and merged" by the target engine — the
"What the target engine does with N same-key rows" table below is exactly that
reconciliation step.)

> **Documented vs. deduced.** The block-only trigger behaviour above (and the
> `ReplacingMergeTree` "keep one row per sort key" rule) are documented facts.
> The specific *1/N under-count* conclusion in the table below is a sound
> deduction composed from those two facts — the §5 proof plan exists to confirm
> it empirically against a real ClickHouse before 0051 commits.

Consequences for a `GROUP BY 15m-bucket` rollup:

- If all 15 minutes of a bucket arrive in **one** INSERT block → one correct
  full-bucket partial. (This is the only case the draft DDL handles.)
- If they arrive in **N** blocks → **N** partial rows, same target sort key,
  each holding a sub-sum. The target engine now has to reconcile N partials.

What the target engine does with N same-key rows:

| Target engine | Reconciliation of N partial rows | OHLC correctness | Volume sum correct? |
|---|---|---|---|
| `ReplacingMergeTree(version)` (draft) | keep 1 (max version) | only last block's open/close survive | **No — keeps 1/N** |
| `SummingMergeTree` | sum *all* numeric cols | **No — sums open/high/low/close too → garbage** | yes for volumes |
| `AggregatingMergeTree` + `*State` cols | merge agg states | **Yes** (argMin/argMax/max/min/sum compose) | yes |

So even ignoring enrichment, the draft is wrong, and the naïve fixes each have
a catch. `AggregatingMergeTree` is the only insert-trigger option that is
*structurally* correct for OHLC — but it changes the target column types to
`AggregateFunction(...)`, breaking `CREATE TABLE _15m AS _1m` and forcing every
reader (0040) onto `-Merge`/`-State` semantics. Hold that thought.

## 2. Now add enrichment (the 0059-specific failure)

Enrichment (task 0026) corrects `_1m` by **re-INSERTing the row** with
`volume_quote_usd` filled and `version = original + 1`. The `_1m`
`ReplacingMergeTree` collapses old+new to the enriched winner. Good for `_1m`.

But the re-INSERT is *an insert into the MV's source table*, so the MV fires on
it. The emitted `_15m` partial carries the corrected row's
`volume_quote_usd` **and** its `volume_base` / `volume_quote` / `trade_count`
**again**:

- **`AggregatingMergeTree` (the "correct" insert-trigger engine):** `sumState`
  **adds** the re-inserted contribution to the already-aggregated bucket →
  `volume_base`, `volume_quote`, `trade_count` **double-count**;
  `volume_quote_usd` lands at `0 (original) + U (corrected)` which is *right by
  luck only because the original was 0* — it would be wrong for any
  re-correction.
- **`ReplacingMergeTree` (draft):** the `version+1` partial *wins* the bucket
  and the rollup becomes *only the enriched minute*, discarding the other 14.

**Conclusion:** a combine-on-insert MV cannot tell a *restatement* from a *new
contribution*. Correction-by-re-insert and incremental-combine are mutually
exclusive under plain insert-trigger MVs. You must break the tension.

## 3. Decision space

### Option A — Re-aggregate from `_1m FINAL` (Refreshable MV / scheduled) ✅ recommended

Rollups stop summing the insert block and instead **recompute the whole bucket
from the deduplicated source**:

```sql
-- Refreshable MV (CH >= 23.12) — runs on a schedule, inside ClickHouse.
CREATE MATERIALIZED VIEW prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE        -- coarser grains can refresh less often
TO prices.price_ohlcv_15m AS
SELECT
    toStartOfInterval(timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp) AS open,
    max(high) AS high,
    min(low)  AS low,
    argMax(close, timestamp) AS close,
    sum(volume_base)      AS volume_base,
    sum(volume_quote)     AS volume_quote,
    sum(volume_quote_usd) AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0) AS vwap,  -- ref aliases (finding #1)
    sum(trade_count)      AS trade_count,
    max(version)          AS version
FROM prices.price_ohlcv_1m FINAL          -- <-- post-dedup, post-enrichment
WHERE timestamp >= now() - INTERVAL 2 HOUR  -- bounded re-scan; coarser grains widen
GROUP BY timestamp, asset_id, quote_asset_id, source;
```

> **vwap must reference the aliases**, not re-sum (`sum(…)/sum(…)`) — the proof
> showed the re-sum form fails to compile (`ILLEGAL_AGGREGATION`, finding #1).

Why it's correct:

- Reads `_1m FINAL` → sees the **enriched, deduplicated** rows, never partials.
- Recomputes the **whole bucket** each refresh → no multi-block under-count.
- Enrichment propagates automatically on the next refresh — **no double-count.**
- A true Refreshable MV in **replace mode** atomically swaps the whole target,
  so it doesn't rely on `ReplacingMergeTree` version dedup — but that only
  preserves history if the query is **unbounded** (recompute all history every
  tick), which is too costly for coarse grains. With a **bounded** window
  (below) the MV must run in **APPEND** mode, which *does* go back through
  ReplacingMergeTree dedup → the version-projection trap (finding #5) applies.
  See the durability subsection below; this is the load-bearing detail for 0051.

Trade-offs:

- **Latency** = refresh interval (acceptable: 1m for `_15m`, widen for coarser).
- **Re-scan cost** per refresh, bounded by the `WHERE` window. At the project's
  ~0.48 GB/yr for `prices.*` (storage estimate doc) this is negligible.
- **CH version dependency.** Refreshable MVs are ≥ 23.12 and were experimental
  for a while. If the BE Hetzner cluster's version is uncertain, the *same
  query* runs as a scheduled job — see Option A′.

> **Version-projection trap (finding #5, proof-confirmed).** `version =
> max(source.version)` is **not** sufficient for a scheduled re-aggregate into a
> `ReplacingMergeTree(version)` target (Option A′). Enriching an *early* minute
> bumps that row's version (e.g. 7→8) but leaves the bucket `max(version)`
> unchanged (15), so the stale and corrected rollup rows **tie on version** and
> dedup degrades to insertion-order luck. Only *unbounded* replace-mode avoids
> this — and we reject that on cost, so under the durable **APPEND** design
> (§3.1) the trap **always** applies. Project a **strictly-increasing** version
> — e.g. `sum(version)` (observed 120→121 on the same enrichment) or a monotonic
> refresh epoch.

#### Durability & refresh mode — replace vs APPEND (doc-grounded)

The rollup tables must be **durable independent stores**, not ephemeral
projections of `_1m`. Each granularity is a separate physical table; an MV only
*writes into* its target — there is no cascade. So `TRUNCATE _1m` does **not**
directly delete rows already in `_15m`. **But the refresh mode decides whether
the *next* refresh wipes them**, and the default mode is dangerous here:

- **Default (replace) mode** — *"each refresh atomically replaces the table's
  previous contents."*
  ([CREATE VIEW ref](https://clickhouse.com/docs/sql-reference/statements/create/view))
  Combined with the bounded `WHERE timestamp >= now() - INTERVAL 2 HOUR`, the
  target then **only ever holds the last 2 h**, and if `_1m` is cleared the next
  refresh replaces the rollup with the (empty) result → **history loss.**
  Replace-mode is only safe with an *unbounded* recompute — too costly per grain.
- **APPEND mode** — *"If `APPEND` is specified, each refresh inserts rows into
  the table without deleting existing rows"*
  ([CREATE VIEW ref](https://clickhouse.com/docs/sql-reference/statements/create/view));
  *"The `APPEND` functionality allows you to add new rows to the end of the
  table instead of replacing the whole view"*
  ([Refreshable MV guide](https://clickhouse.com/docs/materialized-view/refreshable-materialized-view)).
  Rows outside the window are untouched; clearing `_1m` leaves historical rollup
  rows intact. **This is the durable choice** — and it is exactly the
  `INSERT … SELECT … FROM _1m FINAL` semantics the proof actually exercised
  (append into a ReplacingMergeTree), *not* atomic replace.

**Consequence:** durable + cost-bounded ⇒ **APPEND into a
`ReplacingMergeTree(version)` target**, which reintroduces finding #5 — so the
version projection **must** be strictly-increasing (`sum(version)` / refresh
epoch), regardless of whether it's a true refreshable MV (`REFRESH … APPEND`) or
the scheduled `INSERT … SELECT` (A′). The earlier "atomic replace sidesteps
finding #5" shortcut only holds for the unbounded-replace variant, which we are
not using.

**Retention corollary:** because the rollup is the *only* copy of its history,
`_1m`'s retention/TTL must be **≥ the widest refresh window** of any rollup that
reads it — otherwise a rollup bucket can never be rebuilt after `_1m` ages out.

ADR reconciliation: ADR 0007 §3.4 eliminated the **Rollup Lambda**. A
refreshable MV keeps rollups **inside ClickHouse with no external scheduler**,
so the ADR's intent holds; only the MV *flavour* changes (refreshable vs
insert-trigger). Recommend recording this as an ADR-0007 amendment.

### Option A′ — Scheduled re-aggregate worker (external orchestration of the *same* query)

Identical `INSERT INTO _15m SELECT … FROM _1m FINAL … GROUP BY …` driven by an
external scheduler (EventBridge → Lambda, or the existing **0039 `rollup-worker`**)
instead of ClickHouse's own `REFRESH` clause. **Caveat:** this resurrects a
scheduled rollup step, partially undoing ADR 0007 §3.4 — amend the ADR explicitly
if chosen, and fold it into 0039 rather than adding a new Lambda.

#### A vs A′ — this is the load-bearing fork; decide it on *operability + CH support*, not data-model risk

A common intuition is that an external worker "avoids the table-relations / cascade
risk of MVs." **It does not, because there is no such risk.** A ClickHouse MV
target is a fully independent table (an MV is a write-trigger, not a foreign-key
link); there is no cascade. In **APPEND** mode the refreshable MV executes *the
exact statement* the worker would —
`INSERT INTO _15m SELECT … FROM _1m FINAL … GROUP BY …` — into the same
independent `ReplacingMergeTree`, with the same finding-#5 version requirement.
**On the data model the two are identical.** The only real difference is *who
drives the schedule and where the SQL lives*: DB-native (`REFRESH … APPEND`) vs
external orchestration. There is **no third option** — ClickHouse has no built-in
cron for arbitrary `INSERT … SELECT` other than the refreshable MV, so the choice
is genuinely binary.

| Axis | A — Refreshable MV (APPEND) | A′ — External worker (0039) |
|---|---|---|
| The rollup query | identical | identical |
| Cascade / table relations | none | none (**not** an advantage) |
| Correctness (version, APPEND) | finding #5 applies | finding #5 applies (same) |
| Infra surface | one DDL object | Lambda + IAM + EventBridge + mTLS cert + DLQ + CloudWatch + deploy |
| Where it runs | inside the cluster; no external auth to break | external; needs a live mTLS path to Hetzner to even fire |
| Concurrency / leader | built-in (Replicated DB coordinates one refresh per tick) | hand-rolled "don't double-run" |
| ADR 0007 §3.4 fit | honours it (rollups stay in CH) | reopens it → amendment required |
| **CH version dependency** | **needs refreshable-MV support (≥ 23.12 + `allow_experimental_refreshable_materialized_view`)** | **none — runs on any CH version** |
| Observability / control | `system.view_refreshes` (more opaque) | own logs/metrics, ad-hoc window replay, pause |

**Where A′ genuinely wins** (the two axes that can flip the decision):

1. **CH version risk — the decisive one.** Refreshable MVs need ≥ 23.12 and were
   behind an experimental flag; the §5 proof did **not** exercise a true `REFRESH`
   MV for exactly this reason (it needs the flag even on 24.8). If the shared
   Hetzner cluster's version/flags don't reliably support them — or BE won't
   enable the flag — **A is simply unavailable and A′ becomes the primary
   choice, not a fallback.**
2. **Operational control.** Explicit retries, backfill replay, "recompute just
   this window," and first-class metrics are easier in a worker. Our rollups are
   pure re-aggregation today, so we don't *need* this — but it's a real edge if
   rollup logic ever grows beyond a `GROUP BY`.

**Where A wins:** least moving infra, no external auth/reliability surface, built-in
single-refresh coordination, and it honours ADR 0007 §3.4 without an amendment.

**Decision rule:** prefer **A** *iff* the Hetzner cluster supports refreshable MVs
**and** BE is comfortable with scheduled internal refreshes on their cluster;
otherwise take **A′**, folded into 0039, with an ADR-0007 amendment. Either way
the SQL, the target engine, and the version projection are unchanged — so 0051 can
write the query once and bind it to whichever driver BE's answer dictates. This
hinges on the §6 open question (cluster CH version + refreshable-MV acceptability),
which is therefore the **top BE-sync item** for this task.

### Option B — `AggregatingMergeTree` insert-trigger MVs

Structurally correct for multi-block aggregation, but (a) **double-counts under
enrichment re-inserts** (§2), (b) changes target tables to `AggregateFunction`
columns — breaks `CREATE TABLE _Ng AS _1m`, the backfill direct-write path
(§3.2 "backfill writes directly to higher granularities"), and forces 0040
readers onto `-Merge`. Rejected: solves the easy half, fails the enrichment
half, and taxes every reader.

### Option C — Enrich each granularity independently (no propagation)

Run the oracle ASOF join against `_15m`, `_1h`, … directly; nothing propagates.
Rejected: (a) still needs correct `volume_quote` rollups (same multi-block
problem one column over), (b) **less accurate** — one oracle price per coarse
bucket vs Σ of per-minute enriched values, (c) N× the enrichment passes.

### Option D — `VersionedCollapsingMergeTree` with cancel rows

Enrichment emits a `-1` cancel of the original plus a `+1` corrected row so the
sum nets out. Rejected: the cancel row must reproduce the original bucket
contribution *exactly*; fragile, hard to make idempotent, heavy operational
burden.

**Chosen: A** (with A′ as the version-constrained fallback).

## 4. Contract handed to task 0051

0051 owns the DDL; this note fixes the semantics it must implement:

- [ ] Rollup MVs **re-aggregate from `<source>_Ng FINAL`**, not from the insert
      block. Refreshable MV (Option A) preferred; scheduled re-aggregate (A′)
      only if the cluster CH version forces it.
- [ ] **Refresh in `APPEND` mode, never default replace.** The default
      *"atomically replaces the table's previous contents"*
      ([CREATE VIEW ref](https://clickhouse.com/docs/sql-reference/statements/create/view));
      combined with a bounded window that **destroys all history outside the
      window** (and empties the rollup if `_1m` is cleared). `REFRESH … APPEND`
      *"inserts rows … without deleting existing rows"* — durable. The scheduled
      `INSERT … SELECT` (A′) is APPEND by nature. See §3.1.
- [ ] **`_1m` retention/TTL ≥ the widest rollup refresh window.** The rollup is
      the only copy of its history; a bucket can't be rebuilt once `_1m` ages out
      past the window.
- [ ] **`vwap` references the summed aliases** (`volume_quote_usd /
      nullIf(volume_base, 0)`), never `sum(…)/sum(…)` — the latter fails to
      compile (`ILLEGAL_AGGREGATION`, finding #1).
- [ ] Each rollup `SELECT` projects `sum(volume_quote)` **in addition to**
      `sum(volume_quote_usd)` (native quote volume available at every grain —
      task 0058).
- [ ] **Version projection (always needed, given APPEND).** Because durability
      forces APPEND into a `ReplacingMergeTree(version)` target (above), the
      version trick is **not** optional: `version = max(source.version)` **must
      not** be used — it ties pre/post early-minute enrichment (finding #5). Use
      a strictly-increasing projection (`sum(version)` or a refresh epoch). (The
      "atomic replace needs no version trick" shortcut only applies to unbounded
      replace-mode, which we reject on cost — see §3.1.)
- [ ] Rollup targets remain `ReplacingMergeTree(version)`, identical shape to
      `_1m` (no `AggregateFunction` columns).
- [ ] Refresh windows bounded per grain (`_15m` ~minutes, `_1M` ~hourly/daily);
      document each.
- [x] The draft insert-trigger `mv_ohlcv_1m_to_15m` in schema-overview §3.2 is
      **superseded** (and doesn't compile as written). **Done 2026-06-09** —
      §3.2 now shows the corrected refreshable / re-aggregate-from-`_1m FINAL`
      pattern; the three insert-trigger phrasings elsewhere in the doc were
      updated to match. 0051 finalises refreshable-MV vs. scheduled against the
      cluster CH version.
- [ ] ADR-0007 amendment (or new ADR) records refreshable-vs-insert-trigger.

## 5. Proof — EXECUTED ✅

> **Ran 2026-06-09 against `clickhouse-server 24.8.14`.** Artifacts:
> [`proof/run.sh`](../proof/run.sh) (one-command repro),
> [`proof/01_schema.sql`](../proof/01_schema.sql),
> [`proof/02_seed.sql`](../proof/02_seed.sql),
> [`proof/03_enrich_and_fix.sql`](../proof/03_enrich_and_fix.sql),
> [`proof/RESULTS.md`](../proof/RESULTS.md). Headline numbers:
>
> | | `_1m FINAL` (truth) | draft `_15m` | re-aggregate from `_1m FINAL` |
> |---|---|---|---|
> | `volume_base` | **150** | **10** ❌ (1/15) | **150** ✅ |
> | `volume_quote_usd` after enrich | **500** | **0** ❌ (no propagation) | **500** ✅ |
> | `trade_count` | 15 | 1 | 15 |
>
> Plus finding #1 (draft won't compile) and finding #5 (`max(version)`
> 15→15 ties; `sum(version)` 120→121 strictly increases). The plan that
> produced these results follows.

Goal: empirically demonstrate the draft's under-count, the enrichment
mis-propagation, and the Option-A fix, on a minimal `_1m → _15m` chain. No live
infra; a throwaway `clickhouse/clickhouse-server` container.

**Fixtures.** One `(asset_id, quote_asset_id, source)` series; a single 15-minute
bucket of 15 one-minute rows; per-minute `volume_base = 10`, `volume_quote = 50`,
`volume_quote_usd = 0`; versions `v = minute_index` (monotonic).

**Experiment 1 — reproduce the draft bug.**
1. Create `_1m`, `_15m` (`ReplacingMergeTree(version)`) and the **draft
   insert-trigger** `mv_ohlcv_1m_to_15m`.
2. Insert the 15 rows in **15 separate INSERT statements** (simulating live
   per-minute arrival).
3. `SELECT … FROM _15m FINAL`. **Expect** `volume_base ≈ 10` (one minute), not
   `150` → confirms multi-block under-count.

**Experiment 2 — enrichment on the draft.**
4. Re-INSERT minute-7 with `volume_quote_usd = 500`, `version = 7_000+1`.
5. `SELECT … FROM _15m FINAL`. Record the (wrong) result → confirms restatement
   is mishandled.

**Experiment 3 — Option A fix.**
6. Drop the insert-trigger MV; create the **refreshable** MV reading
   `_1m FINAL` (or run the equivalent `INSERT … SELECT … FROM _1m FINAL` to
   simulate a refresh tick deterministically).
7. Re-run inserts (Exp 1) + the enrichment re-insert (Exp 2), trigger a refresh.
8. **Assert** `_15m FINAL` shows `volume_base = 150`, `volume_quote = 750`,
   `volume_quote_usd = 500`, `open = minute_1.open`, `close = minute_15.close`,
   `version = max(version)` — correct full bucket **and** enriched.

**Experiment 4 — chain depth.** Extend `_15m → _1h` to show the refresh pattern
composes across ≥ 2 hops (each grain reads the previous grain `FINAL`).

**Deliverables of the proof step (separate follow-up):** a
`proof/` dir with `schema.sql`, `seed.sql`, `assert.sql`, a `docker-compose` or
`run.sh`, and a short results table pasted back into this note (promoting it
`developing → mature`). This becomes the basis for AC #3's integration test.

## 6. Open questions for the BE cross-team sync

- What ClickHouse version runs on the shared Hetzner cluster? (Gates A vs A′.)
- Are refreshable MVs enabled / acceptable on the shared cluster, or is a
  scheduled re-aggregate (folded into 0039 `rollup-worker`) preferred operationally?
- Refresh cadence per grain that BE is comfortable with vs. read-freshness SLA.
