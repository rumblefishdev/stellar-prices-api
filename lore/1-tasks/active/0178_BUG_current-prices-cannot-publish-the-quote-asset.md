---
id: "0178"
title: "current_prices / current_price_usd cannot publish USDC either — the MV groups on the base leg, so /price has the same structural hole 0165 fixed in the series views"
type: BUG
status: active
related_adr: []
related_tasks: ["0165", "0072", "0095", "0139", "0150", "0170", "0061"]
tags:
  [
    "priority-high",
    "effort-medium",
    "clickhouse",
    "data-correctness",
    "read-surface",
    "be-interop",
    "refreshable-mv",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0165]]'s read-surface audit, which was deliberately scoped
      to the series views. The audit found this defect at code level on 08-10 and
      the confirming prod measurement was taken 08-11: USDC at the canonical
      issuer returns 0 rows from current_price_usd, while USDC at 10 OTHER
      issuers is present - the same issuer-split control that made 0165's
      diagnosis conclusive, reproduced on a second surface. Kept out of 0165 on
      purpose: that fix was an atomic plain-view replace, whereas this one is a
      refreshable-MV DROP + recreate - the operation that wiped the coarse tables
      in 0095 - so it needs its own rollback plan and its own task.
  - date: 2026-08-31
    status: active
    who: okarcz
    note: >
      Activated. Picked as the next piece of work because it is the sole
      remaining blocker on [[0120]], an M2 conformance criterion, and it is the
      second surface of the defect [[0165]] closed in the series views. Starting
      with the "Design question to settle first" section - provenance of a
      measured 1.0000 versus a peg placeholder, and whether the peg identities
      keep their market price - before any schema change, since the fix is a
      refreshable-MV DROP + recreate and needs its rollback plan written first
      per [[0095]].
---

# `/price` cannot return USDC — same defect, harder fix

## Summary

[[0165]] fixed `price_usd_series*`, which emitted one row per **base** asset. The
same base-only assumption sits in `current_prices`, so the **live spot** surface
has the identical hole: **our top-preference quote asset has no current price.**

`current.sql` derives every row from `price_ohlcv_1m` grouped by `asset_id`
(`current.sql:125,142`); **`quote_asset_id` appears nowhere in the MV**.
`current_price_usd` then joins `current_prices` to `assets` on `asset_id` alone
(`views.sql:491-509`), so it can only ever surface assets that appear as a base.

## Measured on prod — 2026-08-11

```
current_price_usd:
  USDC @ canonical issuer (GA5ZSEJY…KZVN)  ->      0
  USDC @ any OTHER issuer                  ->     10
  USDT @ canonical issuer (control)        ->      1
  native XLM              (control)        ->      1
  total rows                               ->  3,428
```

The two controls prove the predicate and the hand-copied issuer literals are
sound, so the zero is a real absence rather than a broken filter.

**The 10-vs-0 issuer split is the evidence, not the raw zero.** Asset code held
fixed, quote-preference the sole variable — the same control that made 0165
conclusive, now reproduced on an independent surface. If the cause were anything
about stablecoins, peg handling or enrichment reach, both halves would behave
alike.

⚠️ **"Absent = it didn't trade in 24 h" does not explain this.** `current_prices`
does only hold assets with a `price_ohlcv_1m` row in the last 24 h, but 0165
established USDC has **zero candles as a base at any time**, so the absence is
structural rather than a quiet day.

## Why this is NOT a copy of 0165's fix

The defect is the same shape; the deployment risk is not.

| | [[0165]] | this task |
|---|---|---|
| object | plain `VIEW` | **refreshable MV + `TO` table** |
| change mechanism | `CREATE OR REPLACE` — **atomic**, no read-side exposure | **`DROP VIEW` + re-`CREATE`** (a refreshable MV's definition is fixed at create time; `ALTER` does not take — `current.sql:18-23`) |
| worst case | none — replacement is atomic | ⚠️ **this is the operation that wiped the coarse tables in [[0095]]** |
| rollback | re-apply previous definition | needs a plan *before* the drop |

`current_prices` keeps serving its last-written rows during the gap, so the
exposure is a staleness window (~1 refresh) rather than an outage — **provided
the recreate succeeds**. The 0095 lesson is that this is exactly where an
apply-time mistake destroys data rather than merely delaying it.

## Design question to settle first

0165's answer was a **zero-weight peg-fill arm unioned before the aggregation**,
so precedence falls out of the weighted average. The same shape probably
transplants, but `current_prices` is materially different:

- It is a **tip** surface (one row per asset), not a per-bucket series, so there
  is no bucket key to union on — precedence has to be decided per asset.
- It carries derived columns the series views do not: `price_xlm`,
  `change_24h_pct`, `change_7d_pct`, `market_cap_usd`, `vwap_24h`, `sources`.
  **What should a peg-filled USDC row report for those?** A `$1` price with a
  fabricated `change_24h_pct` would be a new instance of the
  [[0144]] "one value meaning several things" defect.
- [[0167]]'s `prices.usd_rate` is live, so unlike 0165 this task can reach a
  **real** depeg-aware rate rather than a placeholder — and probably should,
  since [[0168]] is a one-expression change. Consider going straight to the
  measured rate here and skipping the `$1` step entirely.

⚠️ Whatever it emits must carry a **provenance value** distinguishable from a
traded reading, per 0165's requirement 2. `current_prices` has no `method`
column today, so adding one is part of this task.

# 📕 DEPLOY RUNBOOK — the DROP + recreate

Written 2026-08-31, BEFORE any schema work, per AC 6.

## First: the 0095 comparison in this task's header OVERSTATES the danger

The header says this is "the operation that wiped the coarse tables in [[0095]]".
The statement form is the same; **the failure mechanism is not**, and the
difference decides how much ceremony this needs.

0095's rollup MVs wrote into tables holding YEARS of history, in replace mode,
from a SELECT windowed to 2 hours — so a refresh replaced the whole table with
2 hours and the history was gone. `mv_current_prices` is also replace-mode, but
`current_prices` **holds no history at all**: it is one row per asset, fully
recomputed from `price_ohlcv_1m` every minute. There is nothing in it that
cannot be rebuilt by the next successful refresh. `current.sql:20-24` says so
outright — "the MV fully recomputes every row on each refresh".

**So the realistic worst case is not data loss. It is a SILENT FREEZE.**

## 🔴 The actual risk: a failed recreate is invisible

Measured 2026-08-31: **no alarm covers `current_prices` freshness.**
`rollup-freshness-probe` watches the `price_ohlcv_*` tiers only, and no
Observability construct references `current_prices`.

So if the `DROP` succeeds and the `CREATE` fails, the table keeps its last rows
and serves them **indefinitely**, and nothing pages. `/price` returns HTTP 200
with a plausible price that silently stops moving. This is worse than an outage
because every consumer-side health check passes.

⚠️ Second failure mode, ranked ABOVE the first for damage: the recreate
SUCCEEDS with a `TO (...)` / SELECT ordering mismatch. The MV inserts
POSITIONALLY (`current.sql:26-30`), so a mismatch writes every column into the
wrong slot — publishing confident garbage rather than stale truth.

## Sequence

Run as the DDL-capable user, not `prices_reader` (it is READ-ONLY — a
`SETTINGS` clause alone is refused, code 164). CH password from AWS Secrets
Manager, never typed. SQL goes through the operator's `CHQ <<'SQL'` heredoc.

**Step 0 — capture, before touching anything.** All three, to files:

```sql
SHOW CREATE VIEW prices.mv_current_prices;   -- verbatim, this IS the rollback
SHOW CREATE TABLE prices.current_prices;
SELECT count() AS rows, max(updated_at) AS tip,
       countIf(price_usd > 0) AS priced
FROM prices.current_prices FINAL;
```

Record `rows` / `tip` / `priced` in this task file. They are the baseline every
later check compares against.

**Step 1 — `ALTER TABLE prices.current_prices ADD COLUMN method ...`.**
Additive and independent of the MV. Safe to land on its own and safe to LEAVE
in place on rollback — an unwritten column with a default harms nothing. Doing
it first keeps the risky window as short as possible.

**Step 2 — DROP + CREATE the MV, as ONE pasted block, not two statements.**
Minimises the window in which the table has no writer.

**Step 3 — verify within 2 refresh cycles (~2 min).** Not one: a single cycle
can be mid-write.

```sql
SELECT count() AS rows, max(updated_at) AS tip,
       countIf(price_usd > 0) AS priced,
       countIf(method = 'oracle') AS oracle_rows
FROM prices.current_prices FINAL;
```

Pass conditions, ALL of them:
- `rows` within a percent or two of baseline — a large drop means the new SELECT
  lost assets;
- `tip` **advancing** between two runs a minute apart — this is the freeze check
  and the one that matters most;
- `priced` not below baseline;
- `oracle_rows` = exactly 1 (USDC and nothing else) — the allowlist assertion;
- spot-check three identities by hand: **USDC** (non-zero, `method='oracle'`),
  **USDT @ `GCQTGZQQ…TG6V`** (still ~$0.13, NOT `'oracle'`), **XLM**
  (`volume_24h_usd` risen, per Design Decision 1 — an unchanged XLM volume means
  the both-legs change did not take).

**Step 4 — rollback, if any check fails.** Re-apply the Step 0 definition
verbatim. One statement. Leave the `method` column in place; it is inert
without the MV writing it. Then re-verify `tip` advances.

## Deploy-order note

Nothing here needs the API deployed first — `method` is additive, and the
existing handlers pin explicit column lists rather than `SELECT *`
(`queries_ch.rs:195,252,340`). Confirm that before deploying, not after.

## Owed follow-up

The absence of a `current_prices` freshness alarm is a real gap this task
UNCOVERED but should not absorb. Spawn a backlog task for it — the probe
already has the shape (`rollup-freshness-probe`), and the check is
`max(updated_at)` against wall clock. Cf. [[task-0137-rollup-freshness-alarm-live]].

## Acceptance Criteria

- [ ] `GET /price` (and `current_price_usd`) returns a row for USDC at the
      canonical issuer, with a plausible USD value.
- [ ] Provenance is expressible — a consumer can tell a measured `1.0000` from a
      filled one. Requires adding the column; follow `views.sql:273` (append
      last) and 0165's `'traded'`/`'peg'`/`'oracle'` vocabulary.
- [ ] The derived columns are decided explicitly, not left to fall out of the
      arithmetic — each is either populated meaningfully or lands on its
      documented "unavailable" sentinel. **No fabricated `change_*` values.**
- [ ] USDT and the other peg identities are not flattened from their market
      value — 0165's regression, re-tested here.
      ✅ **Sequencing cleared 2026-08-31.** [[0172]] closed + archived
      2026-08-18, and its finding INVERTS this note: USDT at `GCQTGZQQ…` really
      did depeg, the candles were right, and the ~$0.14 was never our bug. So
      the control is readable now — and its pass condition is that USDT KEEPS
      its real market value. A peg arm that drags USDT to $1.00 is the
      regression this AC catches.
- [ ] `volume_24h_usd` counts both legs for every asset, and USDC reports a
      non-zero figure matching its real quote-side volume. Per-venue
      `src_volume` unchanged (base-only) — assert both in one test so a future
      edit cannot quietly re-base the weighting too.
- [ ] The `GET /assets` default-sort change is asserted, not discovered:
      a test pins that USDC appears in the first page of the default
      (`SortCol::Volume24h`) listing where it previously could not.
- [ ] BE re-confirmed as a non-consumer of `volume_24h_usd` — one line, before
      the deploy, per the shelf-life warning above.
- [ ] The oracle allowlist is USDC-only and PROVEN so: a test asserts USDT at
      `GCQTGZQQ…TG6V` still reports its market value and is NOT tagged
      `'oracle'`. This is AC 4's other half.
- [ ] `method` is present and correct on all three layers — table, MV
      (`TO (...)` list AND SELECT), and `current_price_usd` — with a
      positional-decode note in the PR body.
- [ ] A rollback plan written **before** the DROP, given [[0095]]. At minimum:
      the current definition captured verbatim, and the `TO` table's row count
      recorded immediately before and after.
- [ ] Regression test on the 26.3.10.60 pin.
- [ ] BE told — `/price` is a surface they consume, and "USDC pricing is fixed"
      is currently only true of the series views.

## Design Decisions

### Emerged

Settled with the operator on 2026-08-31, before any schema work.

1. **`volume_24h_usd` counts BOTH legs, for every asset** — not base-only, and
   not a second column beside the old one.

   *How it works today* (`current.sql:424-432`): `sum(volume_quote_usd)
   GROUP BY asset_id` over the 24 h window. It groups on the **base** leg and
   never mentions `quote_asset_id`. Canonical USDC has essentially no rows where
   it is the base, so the sum runs over an empty set and returns `0` — the same
   base-only assumption that hides the price, showing up in a second column.

   *Why not leave USDC at `0`* — rejected outright by the operator: USDC has
   large real volume and `0` is simply a wrong number, not a missing one.

   *Why not a new column beside it* (the third option) — a new
   `volume_24h_total_usd` leaves `volume_24h_usd` at `0` for USDC, which is the
   field `/price` serves. It publishes the fix next to the bug and only helps
   consumers who migrate. Since `0` for USDC is the thing being rejected, this
   option is rejected with it.

   *Cost, accepted knowingly*: XLM and USDT change value, because they trade
   heavily on both sides. The resulting column means "total 24 h USD volume this
   asset participated in", which is the ordinary reading of per-asset volume on
   a price API — and crucially it is ONE rule for every asset, which is what
   keeps this out of the [[0144]] "one value meaning several things" class.

2. **The change is confined to the `unfiltered` CTE.** `per_source.src_volume`
   (`current.sql:221`) stays base-only. That figure drives the §5.5
   `min_volume_usd` threshold and the VWAP weighting — both are per-venue
   *weighting* judgments, where the base leg is the right unit. Only
   `volume_24h_usd`, which reads from `unfiltered`, changes.

3. **The `GET /assets` default sort order will change, and that is intended.**
   `handlers.rs:241` — `sort = p.sort.unwrap_or(SortCol::Volume24h)` — makes
   volume the DEFAULT sort of the listing. Re-basing the column reorders the
   first page of `/assets`; USDC rises from unsortable-at-zero to near the top.
   Correct, but visible. Recorded here so [[0120]]'s conformance run reads it as
   an intended change rather than a regression.

4. **The price comes from `prices.usd_rate`, not a `$1` placeholder.** [[0167]]
   made a real depeg-aware rate available, and this is a TIP surface, so
   `usd_rate`'s coverage starting 2026-03-11 is sufficient — it only ever needs
   "now". Read with `ASOF` at-or-before, never averaging (0167's rule). This
   skips the placeholder step [[0165]] had to take, so nothing on this surface
   ever emits `method = 'peg'`.

5. **`method` is EXTENDED to this surface, not invented.** The column and its
   vocabulary already exist on `price_usd_series*` from [[0165]]
   (`views.sql:169-183`): `'traded'` / `'peg'` / `'oracle'`. Existing rows get
   `'traded'`; USDC gets `'oracle'`, which is the value that vocabulary RESERVED
   for a measured depeg-aware rate. `'peg'` would be a false label here — it
   means "no measured rate was available" and one is.

   Consequence: **0178 is the first surface to emit `'oracle'`, arriving ahead
   of [[0168]]**, which reserved it. 0168 stays open and still owes the series
   surface; it inherits the `usd_rate` read pattern from here rather than
   defining it.

6. 🔴 **The oracle read is allowlisted to USDC BY NAME — never "the peg set",
   never "any asset with a `usd_rate` row".**

   Two different tokens are both called USDT. Tether's own is genuinely at par.
   The canonical Stellar USDT at `GCQTGZQQ…TG6V` is a different asset that
   depegged in June 2022 and has traded around **$0.13** ever since — real, not
   a defect ([[0172]]). Reflector prices the TICKER, so `prices.usd_rate` files
   **~$1.00 under this issuer's address**: the oracle is confidently wrong about
   that identity by ~7.4x.

   Nothing reads the oracle for USDT today, so the error is inert. A rule
   phrased as "use the oracle for stablecoins" would activate it and publish
   $1.00 tagged `'oracle'` — a label that reads as MORE authoritative than the
   guess it replaced. Strictly worse than the bug being fixed here, and it would
   fail AC 4 from the other side.

   Already fenced in-tree at `views.sql:102-107`. Widening the allowlist is
   gated on **[[0173]]** (the symbol→issuer mapping); the code comment must say
   so.

7. **Three places must change together**, or the column is broken:
   `prices.current_prices` (`ALTER TABLE … ADD COLUMN`, non-nullable → carries a
   sentinel, never NULL); `mv_current_prices` (**both** the `TO (...)` list and
   the SELECT, in matching order — `current.sql:26-30`, the MV inserts
   POSITIONALLY and a mismatch silently writes every column into the wrong
   slot); and `prices.current_price_usd` (`CREATE OR REPLACE VIEW`, appended
   last per `views.sql:181`). Arity changes, order does not — anything decoding
   positionally off `SELECT *` gets an extra column.

## Consumer exposure — measured 2026-08-31

`volume_24h_usd` is a PUBLIC field on three endpoints: `GET /assets/{id}/price`,
`GET /assets`, and `POST /batch` (`prices-api/src/assets/dto.rs:39,210`;
`batch/handlers.rs:84`).

**BE is not a consumer.** Confirmed by BE against their merged code on
2026-08-19: they read `price_usd_series` / `price_usd_series_1h` and only the
identity triple, the bucket, and `close_usd` — never `current_prices`, never a
volume column. See [[be-reads-close-usd-only-not-volume-columns]]. So the
affected consumers are our own published API: the portal, and API-token holders.

⚠️ **Re-confirm before relying on it.** We stated a BE exposure claim once that
was true when written and false by the time they deployed. One line to BE, not a
re-derivation from their repo.

## Notes

- ⚠️ **Do not report "the USDC pricing bug is fixed" while this is open** — the
  same warning [[0165]] carries about [[0170]]. Three surfaces had the USDC hole:
  the series views (fixed), `/assets/{USDC}/ohlcv` ([[0170]], different root
  cause), and this one.
- [[0150]] (materialise `price_usd_series` as a table) overlaps: if that lands
  first it may supply a cleaner source for the tip than re-deriving from `_1m`.
- ⚠️ [[0139]] is open, so any diagnostic here that resolves an identity to an
  `asset_id` and counts on it must check that id for collisions first.
