---
id: "0139"
title: "current_price_usd returns duplicate rows — assets is keyed on natural identity, not asset_id"
type: BUG
status: active
related_adr: []
related_tasks: ["0072", "0061", "0067", "0144", "0150", "0129"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "milestone-M2"]
milestone: 2
links: []
history:
  - date: "2026-10-01"
    status: active
    who: akot
    note: >
      Activated for implementation via GSD (plan 261001-fwh), one PR on
      fix/0139, worked in .claude/worktrees/0139.
  - date: "2026-10-01"
    status: backlog
    who: akot
    note: >
      Fix chosen: ClickHouse derives asset_id = xxh3(code:issuer:contract) as
      UInt64 (MATERIALIZED on assets, EPHEMERAL identity + DEFAULT on the fact
      tables); migration by copy + EXCHANGE TABLES; one PR; variant A (0286
      phase 3 paused after 2021-12, Oskar decides). GA1, GA2 and OP1 decided.
      See "Decision, 2026-10-01".
  - date: "2026-09-30"
    status: backlog
    who: akot
    note: >
      Research spike (no fix chosen, nothing promoted). Re-measured as
      `dev_read`: 3,315 ids serve 6,636 identities. Classified ALL of them,
      not a sample: 100% are unrelated assets, 0 superseded identities — AC 1
      is answered. O2 is answered: XLM (id 4, not 9), USDC (3) and USDT (111)
      are clean and `usd_reference` is not contaminated. New: the candles
      under a colliding id are BLENDS of both assets (checked against
      Horizon), 164.8M `price_ohlcv_1m` rows (23%) sit under colliding ids,
      prod has no Keeper, and a 32-bit hash id would already collide 5-8
      times. Options for allocation, repair and view joins are compared in
      the spike summary; see "Re-measured on prod, 2026-09-30".
  - date: 2026-09-17
    status: backlog
    who: okarcz
    note: >
      Re-measured on prod while preparing [[0282]]'s SDEX loss measurement,
      where the collisions fan out an asset-id join: 3,312 asset_ids now serve
      6,630 identities (3,300 / 6,606 on 09-02), still at most 3 per id (6 such
      ids), soroban slice unchanged at 10 ids / 20 identities. Still growing.
      The hourly full re-seed stopped on 2026-09-16 ([[0256]]), so a plain
      count() is now stable — but every row carries that last re-seed's
      updated_at, so the table cannot date when a collision appeared.
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Re-measured on prod while deploying [[0210]]: 3,300 asset_ids now serve
      6,606 identities (was 3,275 ids on 2026-08-03), and one id carries three
      identities rather than two. 10 of those ids touch soroban contracts,
      covering 20 identities — the measurement that made 0210 key its symbol
      table on contract_address instead of asset_id, so that work sits beside
      this defect rather than adding to it.
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Found during [[0072]] step 5 on ch-prod-01. `current_price_usd` returned
      **4,442 rows for 4,068 `current_prices` rows** — 374 duplicates. Cause:
      `prices.assets` is `ReplacingMergeTree(updated_at) ORDER BY (asset_code,
      issuer_address, contract_address)`, so `FINAL` dedups on natural identity
  - date: 2026-08-06
    status: backlog
    who: okarcz
    note: >
      **The "deeper question" is answered, and it is option 1 — genuine
      `asset_id` collisions between unrelated assets, not superseded
      identities.** BE's samples (4194 = STW/ARBRIDGE, plus 4628, 4195, 4287)
      are all different issuers; the code path matches (in-memory `next_id`
      counter in `canonical.rs:169-225`, no shared sequence, ≥3 components
      assigning independently). Blast radius is **every table keyed on
      `asset_id`**, not this view. Also: first consumer sizing — 5.5% of every
      pool BE displays a TVL for is tainted. See
      `0144/notes/S-be-0199-response-received.md`.
      and **not** on `asset_id`; **3,275 asset_ids are mapped to two or more
      natural identities**, and the view's `INNER JOIN … ON a.asset_id =
      c.asset_id` multiplies them out. Believed **pre-existing** (the v1
      six-column view carried the same join) — 0072 only made it measurable.
      BE reads this view in-cluster (0199 contract) and has just been pointed at
      its new columns, so they are consuming the duplicates too.
---

# `current_price_usd` fans out on duplicate `asset_id`

## Summary

`prices.current_price_usd` joins `current_prices` to `assets` on `asset_id`:

```sql
FROM prices.current_prices AS c FINAL
INNER JOIN prices.assets  AS a FINAL ON a.asset_id = c.asset_id
```

`prices.assets` (`init.sql:48-66`) is:

```sql
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address)
```

`FINAL` collapses on **natural identity**, so a given `asset_id` survives on as
many rows as it has distinct `(asset_code, issuer_address, contract_address)`
tuples. Joining on `asset_id` therefore fans out.

## Measured on ch-prod-01, 2026-08-03

```
current_prices FINAL rows                       4,068
current_price_usd rows                          4,442   (+374 duplicates)
asset_ids with >1 row in assets FINAL           3,275
```

## Re-measured on prod, 2026-09-02 — still live, and slightly worse

Taken while deploying [[0210]], a month after the numbers above:

```
distinct natural identities                     207,754
distinct asset_ids                              204,448
excess identities (identities − ids)              3,306
asset_ids serving >1 identity                     3,300
identities living under those ids                 6,606
worst single asset_id                                 3   identities
```

The 3,275 figure of 2026-08-03 is now **3,300 affected ids**. It grows with the
registry, which is what "the intern assigns ids that do not track identity"
predicts — this is not drift in the measurement, it is the defect accreting.

**One id carries three identities**, not merely two. Any fix that assumes pairs
will leave that case behind.

### The soroban slice, and why [[0210]] did not inherit it

Of the 3,300, **10 ids touch a Soroban contract**, covering 20 identities. That
measurement is why 0210's `prices.asset_symbol` is keyed on `contract_address`
rather than `asset_id`: an `asset_id`-keyed symbol would have been
unattributable for 19% of the 52-contract population, and would have added to
this defect rather than sitting beside it.

So the symbol table is unaffected — but the `assets` side of every join it
participates in still carries the ambiguity. 0210 dissolved its own exposure,
not this one.

### A measurement hazard worth knowing before re-counting

`prices.assets` is rewritten in full every hour ([[0140]]), so a plain `count()`
reads 1×, 2× or 3× depending on where the merge cycle is — 623,154 / 207,741 /
415,495 were observed on three consecutive hours. **Every number in this section
is `uniqExact`.** A raw count taken mid-cycle led, during 0210's deploy, to a
false conclusion that the registry had doubled.

## Re-measured on prod, 2026-09-17 — still growing

Taken as `dev_read` while preparing [[0282]]'s SDEX loss measurement:

```
asset_ids serving >1 identity                     3,312   (3,300 on 09-02)
identities living under those ids                 6,630   (6,606 on 09-02)
worst single asset_id                                 3   identities (6 such ids)
ids touching a soroban contract                      10   (20 identities — unchanged)
prices.assets FINAL rows                        209,291
```

+12 ids in 15 days. What changed around the measurement:

- **The hourly full re-seed is gone** — [[0256]] (PR #319, deployed
  2026-09-16) made `ensure_seed` write only assets above the watermark. The
  "measurement hazard" below no longer applies to new counts: `count()` now
  reads 1x.
- ⚠️ **`updated_at` cannot date a collision.** The last full re-seed rewrote
  every row at 2026-09-16 12:17 UTC (209,208 rows in that hour), so all 6,630
  colliding rows carry that timestamp. Whether a given collision is old or new
  has to come from somewhere else.
- **It bit a measurement directly:** joining `price_ohlcv_1m` to `assets FINAL`
  on `asset_id` for one 4-day SDEX window returned 1,491,309 rows for 1,459,439
  candles — +31,870 fanned-out rows. [[0282]]'s SDEX measurement therefore
  compares per-minute totals (no identity needed) and excludes pairs that touch
  a colliding id.

## Re-measured on prod, 2026-09-30 — research spike

Taken as `dev_read` (`readonly=1`) at 11:54 UTC, `uniqExact` / `FINAL` throughout.
This was research only: no fix is chosen, and the option comparison (allocation,
repair of collided ids, view joins) lives in the spike summary
`.planning/spikes/SUMMARY-0139-asset-id-collision.md`, which is not committed.

```
asset_ids serving >1 identity                     3,315   (3,312 on 09-17)
identities living under those ids                 6,636   (6,630 on 09-17)
ids with 2 identities / with 3                    3,309 / 6
prices.assets FINAL rows = distinct identities  210,519
distinct asset_ids                              207,198   (max 207,929; 731 unused below max)
identities carrying more than one id                  0   (raw rows, not FINAL)
```

- ⚠️ **`count()` reads 3x again** (631,509 raw rows). Two full 210k-row re-emits
  landed today (08:30 and 10:36 UTC), so the 09-17 note that `count()` is stable
  no longer holds, and every colliding row now carries a 2026-09-30 `updated_at`.
- **XLM is `asset_id 4`**, not 9 as written below. USDC is 3, USDT is 111.

### Collision or superseded identity — the whole population, not a sample

| class | ids | share |
|---|---|---|
| unrelated: different code and different issuer | 3,262 | 98.40% |
| unrelated: same issuer, different code | 36 | 1.09% |
| unrelated: classic asset vs soroban contract | 10 | 0.30% |
| unrelated: 3 identities, partial issuer overlap | 5 | 0.15% |
| unrelated: same code, different issuer | 2 | 0.06% |
| superseded (same code+issuer gaining a contract, or a contract equal to a sibling's `sac_address`) | **0** | **0%** |

**Option 2 does not occur.** Every colliding id is a genuine collision. The
inverse defect ([[0242]]) exists separately: 13 soroban rows are the SAC of a
classic asset held under a different id (11 when 0242 was filed).

### O2 — answered: the reference ids are clean

XLM (4), canonical USDC (3) and USDT (111) each serve exactly one identity.
`usd_reference` admits 4,640 daily candles through its two `assets` joins and
exactly 4,640 sit on the `(4, 3)` key, so no foreign candle is admitted.

### Where the collisions sit

Contiguous id runs: 4188–5044 (856 ids), 25292–25417 (126), 56827–56878 (52),
71123–71144 (22), 122540–124754 (2,213, including all six triples), then 46 ids
scattered from 200539 to 207539. The scattered ones are the live era: first
candles from 2026-07-28 to 2026-09-26, about one new collision every 1.3 days,
36 of the 46 pairing two assets of the same issuer.

Inference, not measured: the live ledger processor loads its registry once per
cold start and never reloads, and `reservedConcurrency: 1` does not limit the
number of warm containers, so the live path can collide with itself. Overlapping
components are not the only mechanism.

### The candles under a colliding id are blends

Checked against public Horizon for four live-era ids (202950 IDR/IRR, 203911
AMERWATER/XRPBONDS, 204205 META/MICROSOFT, 202539 AMEX/MCKESSON): both identities
trade, and on the days both traded our single daily candle has `trade_count 2`.
A row carries nothing that says which identity it belongs to, so the stored
candles cannot be split after the fact. Four ids is a small sample; Horizon
keeps about a year, so the 2016–2022 runs cannot be checked this way.

### Rows under colliding ids, per table keyed on `asset_id` (FINAL)

| table | base id colliding | quote id colliding | rows touching | partitions |
|---|---|---|---|---|
| `price_ohlcv_1m` | 164,797,083 | 2,179,927 | 166,312,541 of 716,759,854 (23%) | 92 |
| `price_ohlcv_15m` | 21,339,031 | 786,141 | 21,982,454 | 93 |
| `price_ohlcv_1h` | 9,810,491 | 776,835 | 10,516,098 | 120 |
| `price_ohlcv_4h` | 4,389,612 | 498,211 | 4,849,485 | 120 |
| `price_ohlcv_1d` | 1,427,390 | 238,829 | 1,647,655 of 25,614,171 (6.4%) | 120 |
| `price_ohlcv_1w` | 359,838 | 77,992 | 430,368 of 6,818,322 | 120 |
| `price_ohlcv_1M` | 109,205 | 28,519 | 134,346 of 2,496,478 | 120 |
| `asset_supply` | 3,315 | — | 3,315 of 207,159 | 1 |
| `current_prices` | 326 (276 priced) | — | 326 of 4,096 | 1 |
| `oracle_prices`, `asset_metadata` | 0 | — | 0 | — |

Not long tail in row terms: 826 ids carry 99% of the 1m rows and the largest has
869,102. Another 17 backup tables (about 785M raw rows) are keyed the same way.
Already orphaned: 12 base ids in `price_ohlcv_1d` (274 rows) and 12
`asset_supply` ids have no `assets` row.

### Fan-out per view

| view | returned | expected | excess |
|---|---|---|---|
| `current_price_usd` | 4,418 | 4,092 | +326 (8.0%); `volume_24h_usd` over-reported by $3,634.53 |
| `price_usd_series`, 2026-09-01..29 | 103,731 | 97,496 | +6,235 (6.4%) |
| `price_usd_series_coverage`, same window | 117,522 | 110,398 | +7,124 (6.5%) |
| `price_usd_series_1h`, week 09-22..28 | 135,183 | 131,435 | +3,748 (2.9%) |
| `price_usd_series_coverage_1h`, same week | 167,179 | 162,410 | +4,769 (2.9%) |
| series base arm over all history, from `price_ohlcv_1d` | 12,606,655 | 11,749,183 | +857,472 (7.3%) |
| same, from `price_ohlcv_1h` | 83,368,531 | 75,140,427 | +8,228,104 (10.95%) |
| `usd_reference`, `_1h`, `identity_by_contract` | — | — | 0 |

The series views cannot be read in full as `dev_read` (3.73 GiB cap), hence the
windows; the all-history rows are derived from the candle tables (one published
row per identity on the id, per id-bucket) and are an upper bound on rows.

### Two constraints on any allocator fix

- **Prod has no Keeper.** `system.zookeeper` does not exist and nothing is
  replicated, so `generateSerialID` and `KeeperMap` are unusable as deployed.
- **A 32-bit hash of the identity is not viable.** Hashing the 210,520 real
  identities gives 5–8 shared ids with four 32-bit functions (expected 5.2) and
  0 with four 64-bit ones. A hash-derived id means `UInt64` key columns.

### How it was measured

47 read-only queries over `prices.assets`, the seven `price_ohlcv_*` tiers, the
small id-keyed tables, the eight views and `system.*`. An id is "colliding" when
`assets FINAL` holds more than one row for it. View fan-out is returned rows
against rows expected with one identity per id. Attribution used Horizon's
public `/trades` endpoint per identity against XLM. Queries and outputs are kept
with the spike, outside the repo.

## Decision, 2026-10-01 — the database derives `asset_id` from the identity

Decided by Adam after spikes 005–008 (`.planning/spikes/`, not committed). The
plan is `.planning/quick/261001-fwh-*/261001-fwh-PLAN.md`.

- **Formula:** `asset_id = xxh3(concat(asset_code, ':', issuer_address, ':', contract_address))`,
  `UInt64`, no counter. Case preserved, empty fields stay empty, native XLM is
  `XLM::`, 0 forbidden (the REDSTONE no-asset sentinel). XLM becomes
  `5209214538714742248`; 4194's STW and ARBRIDGE get two different ids.
- **ClickHouse computes it, the backend never does.** `assets`: `MATERIALIZED`
  (a writer sending `asset_id` is rejected). Candle tiers and `oracle_prices`:
  identity in six `EPHEMERAL` columns, ids as `DEFAULT xxh3(…)` — not
  `MATERIALIZED`, because the rollup MVs and enrichment `INSERT … SELECT` the
  id. clickhouse crate 0.13.3 inserts through EPHEMERAL (spike 008); ≥ 0.14
  would break every writer (schema validation + qualified table names).
- **UInt64, not UInt32.** A 32-bit hash already collides 5–8 times today. Cost
  measured on prod: id columns are 0.17% of `price_ohlcv_1m` compressed
  (37 MiB of 20.9 GiB, ratio 156:1); the 1m primary index is 1.17 MiB.
- **Rejected:** `generateSerialID`, `generateUUIDv4`, snowflake (a new value on
  every re-emit; two writers give one token two ids; serial needs a Keeper prod
  does not have), the explorer's CityHash128, a 32-bit hash, any claim store.
- **Migration keeps table names:** `assets` in place (`MODIFY` then
  `MATERIALIZE COLUMN`); the other 11 id-keyed tables copied to `UInt64`
  `__new` tables through an old→new map (incl. `0 → 0`) and swapped with
  `EXCHANGE TABLES`. Rows under colliding ids are not copied; their months are
  re-ingested. Writers stop for the window; coarse tiers get a gap backfill
  after catch-up (the 15m MV only looks back 2 h).
- **One PR** for all of 0139, merged on window day.
- **Variant A:** 0286 phase 3 pauses after 2021-12 until the window, so
  2022–2023 (95% of the colliding 1m rows) is ingested once. Oskar decides the
  pause. Estimate with a 2026-10-20 window: done 2026-10-31…11-05.
- **GA1:** drop `rollout_0286_bak_*` and `price_ohlcv_*_bak` once their owners
  confirm; rename `reingest_0286_bak_*` to `*_pre0139`, keep the map as decoder.
- **GA2:** colliding and orphan rows are not copied; the swapped-out
  `X__pre0139` tables are the quarantine until the second pass verifies + 7 days.
- **OP1:** the MVs are recreated from prod's captured DDL, not from develop's
  generator (which would also ship 0143/0203).
- [[0242]] stays separate; the migration map already supports many old ids → one.

Colliding 1m rows per year (prod, 2026-10-01): 2016–2021 ~2.0M over 63 months,
2022 33.3M, 2023 136.2M, 2024 6.5M (2 months), 2026 0.3M (4 months). Prod disk:
871 GiB free against 48 GiB for the copy.

## The deeper question this exposes

374 duplicate rows is the symptom. **3,275 asset_ids mapped to more than one
natural identity is the disease** — `asset_id` is supposed to be the surrogate
key for a natural identity, and at that scale it is not unique. Before patching
the view, establish which is true:

1. **ID assignment genuinely collides** — two different assets were handed the
   same `asset_id`. Then every table keyed on `asset_id` is suspect, not just
   this view, and the blast radius is far wider than a read surface.
2. **Historical rows with superseded natural identities persist** — e.g. an
   asset whose `contract_address` was filled in later (the §12.4 SAC collapse,
   [[0061]]) creates a *new* natural-identity row while the old one remains,
   both carrying the same `asset_id`. Then `assets` is behaving as designed and
   only the view's join is wrong.

Option 2 is the more likely reading given the §12.4 write-time collapse and
`sac_address` being a later addition — but it must be **measured, not assumed**.
The discriminator: for a sample of duplicated `asset_id`s, inspect the differing
tuples and their `updated_at`. Superseded identities will look like the same
asset gaining a `contract_address`/`sac_address`; genuine collisions will look
like unrelated assets.

### ⚠️ ANSWERED 2026-08-06 — it is **option 1**, the worse one

BE ran the discriminator for us (`0144/notes/S-be-0199-response-received.md`).
Every sampled pair is **unrelated assets with different issuers**, not one asset
gaining a contract address:

| `asset_id` | identity A | identity B |
|---|---|---|
| 4194 | `STW` (`GA2LHOPXZF…`) | `ARBRIDGE` (`GBACKRJVX7…`) |
| 4628 | `GESARA` | `GL1` |
| 4195 | `SPACEWALK` | `GIFT` |
| 4287 | `INSILVERMINE` | `NUTT` |

4194's two identities carry **862 series rows each, the same last bucket, and
prices identical to 14 decimals** — one asset's candles served under both names.

**The code path matches.** `asset_id` is "an app-assigned UInt32 surrogate"
(`init.sql:45`) handed out by an **in-memory counter**: `AssetRegistry::
from_existing` seeds `next_id = max(existing id) + 1` and `get_or_assign`
increments it locally (`canonical.rs:169-225`). There is no shared sequence and
no uniqueness constraint. At least three components construct their own registry
— the live ledger processor, asset-discovery, and events-backfill — so two
running concurrently load the same watermark and hand the same number to two
*different* newly-discovered assets. Nothing errors, because `assets` sorts on
natural identity and those are legitimately two distinct rows.

That explains the long-tail concentration: collisions only occur on assets being
discovered for the *first* time while two writers run. Established assets are
already in everyone's snapshot.

**Consequences, and they widen this task:**

- The blast radius is **every table keyed on `asset_id`**, not this view.
  `price_ohlcv_*` is `ORDER BY (asset_id, quote_asset_id, source, timestamp)` —
  two assets' candles are interleaved in one key space and cannot be separated
  after the fact by the id alone.
- Patching the join fixes the *symptom*. The fix must also make id assignment
  authoritative (a shared sequence, or derive the id from the identity by hash)
  and decide what to do with already-collided ids — renumber, or accept and
  disambiguate at read time.
- Option 2 is **not disproven**; four samples show option 1 exists. Both may be
  present. Sample more widely before choosing a repair strategy.

## Measured consumer impact (BE, 2026-08-06)

First real sizing, from the only external consumer:

```
identities in assets FINAL      204,381
asset_ids in assets FINAL       201,146     <- more identities than ids
duplicated asset_ids              3,279     (6,564 rows; our 08-05 count: 3,278)
BE pools touching a tainted identity   3,128
BE pools tainted AND priced            1,286   = 5.5% of every pool BE shows a TVL for
```

All long tail, no flagship (STW/Farsight, SGB/STW, NUTT/yXLM, …). BE are
deliberately **not** working around it — "we can't tell a tainted row from a
clean one without your authority data, and replicating that judgement would fork
the single source of truth" — so this fix is the only path for them.

## Does the fan-out inflate `volume_base`? (BE's question, answered from the SQL)

Read from `views.sql`; **not yet measured on prod**. Three answers:

1. **`price_usd_series` / `_1h` — no.** The `GROUP BY` includes the identity
   columns (`views.sql:190, 228`), so duplicates land in **different groups**.
   Each candle appears once per group; `sum(volume_base)` is real traded volume
   and the weighted average is arithmetically correct. The defect is **pure
   misattribution** — `ARBRIDGE` publishes `STW`'s real price from `STW`'s real
   weights.
2. **Cross-identity aggregation — yes, double-counted.** The volume is real
   once but appears under two identities. Any market-wide total sums it twice.
3. **`current_price_usd` — yes, genuinely inflated.** `views.sql:289-308` joins
   `assets FINAL ON asset_id` with **no `GROUP BY` at all**; a duplicated id
   emits two complete rows including `volume_24h_usd` (`views.sql:303`).

### ⚠️ Open — check `usd_reference` before closing this task

`usd_reference` / `_1h` (`views.sql:155-166, 201-212`) join `assets` **twice**
(base *and* quote), `GROUP BY p.timestamp` **only**, and filter on the *joined*
identity. If the XLM-native or the USDC `asset_id` is among the 3,279, **foreign
candles are admitted as XLM/USDC** — contaminating the reference series
consumers use to distinguish "no reference existed" from "prices-api bug", and
feeding the pivot tier's `xlm_usd`. Uniform duplication leaves the weighted
value unchanged, so it would be **invisible in the number**.

Low prior — collisions cluster on newly-discovered assets and XLM is `asset_id
9` — but it is one query and must not be assumed.

## Implementation (once the above is settled)

If option 2 — pick one row per `asset_id` deterministically, e.g. `argMax` over
`updated_at` in a subquery before the join, or key the join on natural identity
rather than `asset_id`. Prefer the latter if `current_prices` can carry it: it
removes the surrogate-key dependency instead of papering over it.

If option 1 — this becomes an ingestion-side task and the view fix is only a
stopgap. Spawn accordingly.

- Audit the **other** read surfaces in `views.sql` for the same join
  (`price_usd_series`, `identity_by_contract`, …) — if they join on `asset_id`
  against `assets`, they fan out identically and this is not a one-view bug.
- Add a test that fails on fan-out: seed two `assets` rows sharing an `asset_id`
  with different natural identities, assert the view returns one row per
  `current_prices` row.
- Tell BE once a direction is chosen — they read this view in-cluster and were
  pointed at it on 2026-08-03.

## Acceptance Criteria

- [x] Determined whether the 3,275 duplicated `asset_id`s are ID collisions or
      superseded natural-identity rows, with the measurement recorded.
      **2026-09-30: all 3,315 are collisions, 0 superseded.**
- [ ] `current_price_usd` returns exactly one row per `current_prices` row.
- [ ] Every other view in `views.sql` audited for the same join defect.
- [x] **O2 — the XLM-native and USDC `asset_id`s checked against the 3,279
      duplicates** (one query; see the open section above). **2026-09-30: XLM
      (4), USDC (3) and USDT (111) are clean; no foreign candle is admitted.**
      If either collides,
      `usd_reference` / `_1h` admit foreign candles as XLM/USDC and contaminate
      the pivot tier's `xlm_usd` — invisibly, because uniform duplication leaves
      the weighted value unchanged. Low prior, but it must not be assumed.
      Raised by [[0144]] while answering BE's `volume_base` question.
- [ ] A test fails if the fan-out reappears.
- [ ] BE informed of the resolution.

Carried from [[0129]] when it closed into this task (2026-09-24) — they check
the allocator, not the view, so a view-only dedupe does not satisfy them:

- [ ] `SELECT count(), countDistinct(asset_id) FROM prices.assets FINAL` returns
      equal values in production.
- [ ] 0129's two-query cross-check (its §Evidence) agrees to the row.
- [ ] An invariant test or probe guards `asset_id` uniqueness going forward.
- [ ] `GET /assets` verified to emit no duplicate asset across a full cursor
      walk (extends 0074's pagination test).
