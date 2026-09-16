# Runbook — rolling out the ADR 0287 candle definitions (task 0286, phase 1)

How to land the price-forming candle definitions on ch-prod-01 and the worker
fleet without producing a coarse estate that means two different things at once.

**What changes on the wire.** A candle's prices come only from the
price-forming trades of its own bucket. A DUST-ONLY minute — volume, but not one
fill whose price means anything — is written `open = high = low = close = 0` with
`pf_trade_count = 0`, and contributes to no coarse `open`/`high`/`low`/`close`.
Coarse `close_usd` becomes the bucket's own close re-priced by the latest priced
child's rate. The month rolls from the **day**, under a renamed MV
`mv_ohlcv_1d_to_1M`.

**Applies to:** `prices.price_ohlcv_{1m,15m,1h,4h,1d,1w,1M}` and the six
`mv_ohlcv_*` MVs on ch-prod-01; the enrichment worker, the coarse sweep, the
prices-api and the ingest binaries.

**Read first:** [`0142-rollup-mv-reapply.md`](0142-rollup-mv-reapply.md) — the
per-MV DROP + re-CREATE procedure, the privileged-user note and the "where these
commands run" warning all apply unchanged. This runbook is the ORDER around it,
plus the two things 0142 does not cover: the FREEZE before the first re-CREATE,
and the month's rebuild.

## 1. Preconditions

| #   | Precondition                                                                                                                                                                                                                                                       | Check                                                                              |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| 1   | **Task 0282's fix is deployed.** Candle writes are summed, not replaced, across a reconcile run. Under the new definition a later dust-only write of a minute REPLACES a priced one outright, so 0282 stops being a volume defect and starts being a price defect. | The ledger-processor running on prod carries 0282's reconcile fix.                 |
| 2   | **The S2 branch is merged** and the workers being deployed are built from it.                                                                                                                                                                                      | `git log --oneline origin/develop \| head`                                         |
| 3   | **The drift binary is green before you start.** A target already drifted for some other reason makes step 7's expected ALARM unreadable.                                                                                                                           | `./prices-clickhouse-drift` on the host (0142, "Where these commands run") exits 0 |
| 4   | **The 0142 / 0137 alarms are known-green.** `prices-production-mv-drift` OK, `MvDriftCritical` 0, the rollup freshness alarm OK.                                                                                                                                   | CloudWatch                                                                         |
| 5   | **Disk headroom for the FREEZE snapshots** (hardlinks under `shadow/`, cheap but non-zero).                                                                                                                                                                        | `ssh … 'df -h /var/lib/docker'`                                                    |

## 2. Order, and why it is not negotiable

```
schema  →  enrichment + coarse sweep + prices-api  →  MV re-CREATE  →  ingest LAST
```

The same statement is in `packages/prices-clickhouse/schema/init.sql` and in the
header of `schema/rollups.sql`; this is the operational reading of it.

- **Schema first.** Everything else refers to the three pf columns. The
  migration is idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS` per table,
  and the DEFAULT expressions (`pf_trade_count DEFAULT trade_count`, and so on)
  give every pre-existing row the pre-0286 meaning — every fill formed price —
  which is exactly what history should keep saying until phase 3.
- **Enrichment, the coarse sweep and prices-api before the MVs.** Pre-0286
  enrichment re-inserts a candle with a FIFTEEN-column list. A named list that
  omits a column is not an error: ClickHouse fills it from its DEFAULT, so
  `pf_trade_count` silently becomes `trade_count` and a dust-only minute comes
  back claiming to be fully price-forming. That corruption is invisible and
  lands on whatever rows the pass happens to touch.
- **The ingest LAST.** A pre-0286 MV meeting a post-0286 ingest reads a
  dust-only minute's `low = 0` with `min(low)` and writes a zero `low` across
  the whole coarse bucket, on every tier above it.

Between the MV re-CREATE and the ingest deploy nothing is written in the new
shape, so the window between them is safe to keep open as long as you like.

## 3. FREEZE the coarse partitions before the first re-CREATE

The rollback in section 6 is `DROP PARTITION` + `ATTACH PARTITION FROM`, which
needs the snapshots to exist BEFORE anything writes in the new shape. Freeze
every coarse partition the six MV windows cover — the widest window is 400 days,
so in practice the last ~14 monthly partitions of each coarse table, and every
partition of `price_ohlcv_1M`.

```sql
-- One per (table, partition). `toYYYYMM` partitioning means the partition id IS
-- the numeric YYYYMM. Run as the privileged user (see 0142).
ALTER TABLE prices.price_ohlcv_15m FREEZE PARTITION 202609
  WITH NAME 'rollout_0286_prices_price_ohlcv_15m_202609';
```

Enumerate what to freeze rather than typing it out:

```sql
SELECT table, partition, formatReadableSize(sum(bytes_on_disk)) AS size
FROM system.parts
WHERE database = 'prices' AND active
  AND table IN ('price_ohlcv_15m','price_ohlcv_1h','price_ohlcv_4h',
                'price_ohlcv_1d','price_ohlcv_1w','price_ohlcv_1M')
  AND toDate(concat(substring(partition, 1, 4), '-',
                    substring(partition, 5, 2), '-01')) >= toStartOfMonth(now() - INTERVAL 400 DAY)
GROUP BY table, partition
ORDER BY table, partition;
```

The parts land under
`/var/lib/clickhouse/shadow/rollout_0286_prices_<table>_<partition>/`. Confirm
they exist before continuing — `repair-coarse-usd-values.md` §"Rollback" has the
same idiom and the `SYSTEM UNFREEZE WITH NAME …` cleanup.

```bash
ssh … 'docker exec app-clickhouse-1 ls /var/lib/clickhouse/shadow/ | grep -c rollout_0286_'
ssh … 'docker exec app-clickhouse-1 du -sh /var/lib/clickhouse/shadow/'
```

## 4. Re-CREATE the five unchanged-name MVs, fine to coarse

Under [`0142-rollup-mv-reapply.md`](0142-rollup-mv-reapply.md)'s checklist,
one at a time, in this order:

```
mv_ohlcv_1m_to_15m → mv_ohlcv_15m_to_1h → mv_ohlcv_1h_to_4h
→ mv_ohlcv_4h_to_1d → mv_ohlcv_1d_to_1w
```

Fine to coarse so each MV is re-created only once its own source is already
correct. Every re-created body must keep the task 0095 invariants — `APPEND`,
`sum(version)`, a bucket-ALIGNED window — and the statements in
`schema/rollups.sql` carry all three. Take them from that file verbatim; it is
generated from `src/rollup_sql.rs` and pinned to it by unit tests, so a body
retyped by hand is the only way to get a wrong one.

⚠️ **The rate-form `close_usd` supersedes task 0146.** 0146 is the open task to
replace the MVs' unguarded `argMax(close_usd, t.timestamp)` with the 0145 guard.
Do not do both: the bodies you are about to create carry
`close × argMaxIf(close_usd / close, t.timestamp, close_usd > 0 AND close > 0)`,
which skips the un-enriched sentinel exactly as the guard did AND keeps
`close_usd` on the same bucket as `close`. **The six MVs are re-created ONCE,
with both changes**, and 0146 closes as delivered here.

## 5. Rebuild the month

The month's MV changes NAME as well as body, because it changes source: a week
belongs wholly to the month it STARTS in, so a week-fed month took its close and
extremes from whichever month owned the straddling week. `price_ohlcv_1M` is
therefore not merely stale, it is wrong at every boundary, and re-pointing the MV
does not repair the rows already in it.

```
DROP the old view → CHECK 1d coverage → TRUNCATE 1M → re-roll from 1d → CREATE the new view
```

**5a. Drop the old view.**

```sql
DROP VIEW prices.mv_ohlcv_1w_to_1M;
```

**5b. The coverage check — BEFORE the TRUNCATE, and it is a STOP.** Every month
present in `price_ohlcv_1M` must be covered by `price_ohlcv_1d`, or the TRUNCATE
throws away months the re-roll cannot rebuild.

```sql
SELECT m.month, m.rows_1M, d.days
FROM (
    SELECT toStartOfMonth(timestamp) AS month, count() AS rows_1M
    FROM prices.price_ohlcv_1M FINAL GROUP BY month
) AS m
LEFT JOIN (
    SELECT toStartOfMonth(timestamp) AS month, count() AS days
    FROM prices.price_ohlcv_1d FINAL GROUP BY month
) AS d USING (month)
WHERE d.days = 0 OR isNull(d.days)
ORDER BY m.month;
```

**Zero rows, or STOP.** A non-empty result means `price_ohlcv_1d` does not cover
some month the 1M table holds. Do not truncate. Either pre-roll the missing days
first (`schema/preroll.sql`, or `preroll-live-gap.sql` for a bounded range) or
escalate — phase 3's re-ingest is the durable answer and this rollout does not
depend on the month being rebuilt today.

**5c. Truncate and re-roll.**

```sql
TRUNCATE TABLE prices.price_ohlcv_1M;
```

Then run the 1M statement of `schema/preroll.sql` (the last one — full range,
`FROM prices.price_ohlcv_1d AS t FINAL`). It names all eighteen columns and
projects the pf aggregates, so the rebuilt months mean what the days mean.

**5d. Create the new view**, the last statement of `schema/rollups.sql`:
`mv_ohlcv_1d_to_1M`, `REFRESH EVERY 1 DAY APPEND`, 400-day window aligned to the
month. A freshly created refreshable MV runs its first refresh immediately at
creation (measured on the 26.3.10.60 pin, 0142 §3), so no manual
`SYSTEM REFRESH VIEW` is needed.

## 6. Rollback

Real, and it is why section 3 exists. Per affected coarse table and partition:

```sql
-- 1. Remove what the new definition wrote.
ALTER TABLE prices.price_ohlcv_15m DROP PARTITION 202609;

-- 2. Put the frozen copy back.
ALTER TABLE prices.price_ohlcv_15m ATTACH PARTITION 202609
  FROM '/var/lib/clickhouse/shadow/rollout_0286_prices_price_ohlcv_15m_202609/';
```

Then re-CREATE the pre-0286 MVs — the bodies are in git, not in your memory:

```bash
git show 43215eb:packages/prices-clickhouse/schema/rollups.sql
```

That commit is the last one carrying the pre-0286 rollup chain, including the
week-fed `mv_ohlcv_1w_to_1M`. Roll the ingest back at the same time, or the old
MVs will start turning dust-only minutes into zero coarse lows.

Release the snapshots only once you are sure you will not need them:

```sql
SYSTEM UNFREEZE WITH NAME 'rollout_0286_prices_price_ohlcv_15m_202609';
```

The FREEZE/ATTACH idiom, including the grant that `prices_writer` does NOT hold,
is documented in [`repair-coarse-usd-values.md`](repair-coarse-usd-values.md).

## 7. Expected alarms

- **`prices-production-mv-drift` WILL fire** for the whole of sections 4 and 5.
  That is the check working: the file and the live objects genuinely disagree
  from the moment the branch merges until the last MV is re-created. Expect it,
  and expect it to clear.
- **`MvDriftCritical` must stay 0.** A non-zero value means an MV is live in
  REPLACE mode — it is deleting pre-rolled history on every refresh (task 0090).
  **STOP the rollout** and fix that MV before doing anything else.
- **The 0137 rollup freshness alarm may fire** for whichever tier is momentarily
  dropped. Do not silence it. Let it prove it works and confirm it returns to OK
  in section 8.

## 8. Verification

**8a. No drift.**

```bash
./prices-clickhouse-drift   # on the host, per 0142 — exits 0
```

**8b. OHLC ordering, per tier.** There is no clamp anywhere in the rollup, so
this holds only because the four price aggregates read the same gated subset of
children. Over freshly rolled buckets:

```sql
SELECT count() AS violations
FROM prices.price_ohlcv_15m FINAL
WHERE timestamp >= now() - INTERVAL 2 HOUR
  AND NOT (low <= open AND low <= close AND open <= high AND close <= high);
```

Zero, on each of `15m`, `1h`, `4h`, `1d`, `1w`, `1M` (widen the window to the
tier's own).

**8c. The pf columns are WRITTEN, not defaulted.** `pf_trade_count DEFAULT
trade_count` makes the failure invisible, so prove the ingest is populating them:

```sql
SELECT countIf(pf_trade_count != trade_count) AS differs, count() AS rows
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 1 HOUR;
```

`differs = 0` over an hour of live SDEX traffic means the column is taking its
DEFAULT and every minute is claiming to be fully price-forming.

**8d. Dust is visible, and priced once.** The population this task exists for:

```sql
SELECT count() AS dust_only_minutes
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 1 DAY
  AND pf_trade_count = 0 AND trade_count > 0;
```

Each such minute must end up with `volume_quote_usd > 0` (its volume is real)
and `close_usd = 0` (it has no price), and must NOT reappear in the enrichment
backlog on the following pass. Watch `EnrichmentRowsEnriched` — a pass reporting
0 enriched against a non-empty backlog is the latched state the candidate
predicate was widened to end.

**8e. Freshness recovered.** Per-MV: `last_success_time` recent and no
`exception`.

```sql
SELECT view, status, last_success_time, next_refresh_time, exception
FROM system.view_refreshes WHERE database = 'prices' ORDER BY view;
```

And the 0137 alarm back to OK.

## 9. History is NOT re-rolled in phase 1

Deliberate (Adam, 2026-09-16). Rows written before this rollout keep the
pre-0286 meaning through the column DEFAULTs — `pf_trade_count = trade_count`,
i.e. "every fill formed price" — until phase 3 re-ingests the chain and every
candle is rebuilt from the raw fills. A candle from before the rollout and one
from after it therefore mean slightly different things, and that is the accepted
state for the duration of phase 1. The only history this rollout touches is
`price_ohlcv_1M`, and only because the month changed SOURCE (section 5).

⚠️ **`preroll-incremental.sql` and `preroll-amm-reprice.sql` are HISTORICAL and
must not be run.** They carry the pre-0286 candle definition, they roll the
month from the week, and their positional `INSERT … SELECT`s now fail with
Code 20 against the eighteen-column tables — by design. The two MAINTAINED
pre-rolls are `preroll.sql` and `preroll-live-gap.sql`, both generated from
`src/rollup_sql.rs`.

**Known gap, by design:** a DUST-ONLY candle (`close = 0`) is selectable by
the enrichment for its `volume_quote_usd` exactly ONCE — whichever tier reaches
it first prices the volume, and the widened candidate term
(`close_usd = 0 AND (close > 0 OR volume_quote_usd = 0)`) then excludes it
everywhere. A later, better-evidence tier — an external `usd_rate` series
backfilled over a bucket the peg already priced at a flat $1.00 — does not
overwrite it. That is the write-once `volume_quote_usd` rule this rollout
relies on to terminate (section 7), and it applies to EVERY row, not only dust
ones: the peg-before-external ordering predates task 0286. The way back for a
mispriced row is `reset_sql` (`repair-coarse-usd-values.md`), which zeroes both
USD columns and re-admits the row.

**Follow-up, deliberately not done here:** the 1M freshness slack. Both
`packages/rollup-freshness-probe/src/lib.rs` and `infra/src/lib/types.ts` carry
6 days of alignment slack for `price_ohlcv_1M`, justified by a week having to
START inside the month. The month now rolls from the day, so its bucket exists
as soon as a day starts in it and the slack is wider than it needs to be. The
number is kept: a wider bound can only delay an alarm, never fire a false one,
and a rollout is not the place to tighten a threshold.

## Related

- [`0142-rollup-mv-reapply.md`](0142-rollup-mv-reapply.md) — the per-MV DROP +
  re-CREATE procedure this runbook orders.
- [`repair-coarse-usd-values.md`](repair-coarse-usd-values.md) — the
  FREEZE/ATTACH idiom and the grants it needs.
- [`0136-coarse-rollup-merge-recovery.md`](0136-coarse-rollup-merge-recovery.md)
  — what a coarse tier looks like when it silently stops rolling up.
- `packages/prices-clickhouse/schema/rollups.sql`, `schema/preroll.sql`,
  `schema/init.sql` — the generated statements and the deploy-order comment.
- [`0286-reingest-history.md`](0286-reingest-history.md) — phase 3: rebuilding
  the history this rollout deliberately leaves alone (section 9).
- ADR 0287 §2–§6; lore task 0286 phase 1.
