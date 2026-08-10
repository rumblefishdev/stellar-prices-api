---
id: "0167"
title: "Create prices.usd_rate and populate the peg-asset rates from oracle_prices — extracted from 0154 so it is not blocked behind 0111"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0154", "0151", "0165", "0168", "0139", "0111"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "schema", "enrichment", "data-correctness", "milestone-M2"]
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/cleanup-worker/src/lib.rs"
  - "../../../lore/1-tasks/archive/0144_BUG_be-0199-usd-read-surface-defects/notes/I-usd-rate-table.md"
history:
  - date: 2026-08-07
    status: backlog
    who: okarcz
    note: >
      Extracted from [[0154]]. The rate table's shape was decided in [[0151]]
      (2026-08-06) and assigned to 0154 as its implementation — but 0154 is hard
      blocked behind [[0111]] on cost grounds, and that blocker applies to the
      *second pivot tier*, not to the table. Meanwhile the peg rates this table
      would hold are on a 13-month expiry clock in `oracle_prices`. Extracting
      the table unblocks [[0165]]'s depeg follow-up ([[0168]]) and lands 0154's
      riskiest piece early and separately. Confirmed against 0151 that this does
      not re-open the schema-wide refactor it rejected — see §Relationship to
      0151.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      ACTIVATED, and pulled ahead of the rest of the queue on ONE argument: it
      is the only open task whose INPUT DATA EXPIRES. oracle_prices is pruned by
      monthly partition drop at INTERVAL 13 MONTH (cleanup-worker/src/lib.rs:24)
      and coverage starts ~2025-09, so 202509 ages out ~2026-10/11. Every month
      this waits, a month of depeg-aware history is lost permanently. Everything
      else on the board (0170, 0171, 0172, 0142) is static - the defects will be
      just as fixable next month.
      Now unblocked downstream as well: 0165 shipped to prod 2026-08-10 and is
      publishing 891 daily USDC buckets at a hardcoded $1 with method='peg'. Its
      three forward-compat requirements all landed, so 0168 is a one-expression
      change against this table rather than a rewrite.
      Originating reasoning is the I-usd-rate-table note under archived task
      0144 - now linked from `links` and from that note's `spawns`, because it
      was previously reachable only by knowing it existed in an archived task's
      notes directory.
      ⚠️ Scope guard restated: the SCHEMA-WIDE refactor in that note stays
      REJECTED (0151). close_usd is not touched, nothing published becomes NULL,
      and the key is NATURAL IDENTITY not quote_asset_id - 0139 is confirmed
      genuine collisions, measured 2026-08-10 at 3,281 asset_ids across 6,568
      identities, which a quote_asset_id-keyed table would have inherited.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      IMPLEMENTED (schema + population). prices.usd_rate added to init.sql with
      0154's exact shape; populate_usd_rate_from_oracle on OhlcvWriter copies
      peg observations from oracle_prices, incremental by per-identity
      watermark; oracle-worker calls it after write_oracle.
      Design decision worth flagging - the snapshot is NON-FATAL in the worker.
      oracle_prices is the source of truth and is already written by that point;
      the copy is derived and watermarked, so a failed pass self-heals on the
      next run. Failing the worker would stop oracle POLLING itself, trading a
      durable gap for a live outage.
      The 0139 guard is the load-bearing piece: oracle_prices is keyed on
      asset_id and usd_rate on natural identity, so this copy is the ONE place
      the two key spaces meet. With 3,281 ids serving 6,568 identities on prod,
      an unchecked translation would file one asset's readings under another's
      identity in a table meant to be trusted forever. The write is refused in
      both directions (identity -> exactly one id; that id -> exactly one
      identity) and the IT asserts the refusal writes NOTHING.
      XLM is polled but deliberately NOT snapshotted - 0154 owns the pivot
      methods. Recorded as an explicit scope boundary on peg_identities(),
      WITH the counter-argument: the 13-month expiry applies to XLM's history
      identically, so if 0154 has not started before 202509 ages out this should
      be reconsidered rather than deferred by default.
      Tests: 2 new CH ITs on the 26.3.10.60 pin (copy + watermark + no-duplicate
      re-run; and the 0139 refusal), 1 shape IT, 1 retention tripwire in
      cleanup-worker. Whole workspace lib suite green; no new clippy warnings.
      Two self-inflicted bugs found and fixed while building, both recorded
      because they were invisible in review: watermark_before was Default 0 with
      a .min() that pinned it there forever (now Option<u32>), and the two ITs
      shared the real `prices` database and truncated each other's fixtures
      under cargo's parallel runner - both failed in ways that looked like
      product bugs until serialised.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      REVIEW FIXES (PR #191). Eight findings, all accepted after verifying each
      against the code rather than on assertion. Two were serious.
      HIGH 1 - the 0139 guard ran INSIDE the write loop, so a failure on a later
      identity left earlier ones already written. That is a partial write, the
      exact failure mode the guard exists to prevent, and my test could not
      catch it because it used a single identity. Guards now run as a pre-pass
      over every identity before any write, and the error names every offender.
      New test asserts a collision on USDC writes nothing for the clean USDT.
      HIGH 2 - the resume watermark was max(timestamp) with a strict > filter,
      which silently skips any reading that lands BELOW the frontier. Not
      hypothetical: write_oracle is also called from sdex-backfill/ingest.rs and
      prices-ledger-processor/reconcile.rs, which decode oracle readings from
      HISTORICAL ledgers. Once the 5-minute worker advanced the watermark, any
      backdated reading would never be snapshotted and would then expire from
      oracle_prices at 13 months - precisely the permanent loss this table
      exists to prevent. Replaced with a gap-filling LEFT ANTI JOIN on
      (timestamp, value); both tables are small so the cost is nil. Anti-joining
      on the value also makes an upstream correction re-copy and win on version,
      which the strict > had made unreachable despite the doc claiming it.
      MEDIUM - `method` added to the sorting key. Without it a 'pivot' row from
      0154 at the same (identity, timestamp) as a measured 'oracle' reading
      would silently REPLACE it under RMT, with the later write winning rather
      than the better evidence. Fixed while the table is still empty; changing a
      sorting key later means a rebuild.
      MEDIUM - OracleStats.rates_snapshotted was computed and then dropped by
      the Lambda entrypoint. Combined with the deliberate non-fatal error path,
      a permanently broken snapshot would report success on ~288 invocations a
      day with no counter moving. Now logged and returned, per-identity.
      LOW - stats counted identities ATTEMPTED not rows written (now
      rows_inserted); watermark_after was a max() across identities that would
      hide one stalled peg (now per-identity newest); and oracle_name is a
      PARAMETER rather than a hardcoded 'reflector', because the enrichment tier
      reads it from config and the rate we snapshot must be the rate that priced
      the candles or 0154 constraint 5 compares two different things.
      Full workspace lib suite + all CH ITs green on the 26.3.10.60 pin.
  - date: 2026-08-10
    status: active
    who: okarcz
    note: >
      0086 GUARD ADDED, found by measuring prod rather than by review. Sizing the
      table meant querying oracle_prices, and min(timestamp) came back
      1970-01-21 - which is [[0086]], an open confirmed bug where the oracle
      worker intermittently writes the real epoch divided by ~1000, with a
      CORRECT price and a junk timestamp.
      My population copied o.timestamp verbatim, filtered only on price_usd > 0,
      so those rows would have been snapshotted. That is strictly worse here
      than upstream: oracle_prices sheds them at 13 months, usd_rate is retained
      FOREVER, so a known defect would have become permanent history in a table
      whose entire selling point is that it is trustworthy.
      Added ORACLE_EPOCH_FLOOR (2020-01-01) to the copy predicate, with a test.
      The floor cannot exclude real data - no oracle we poll existed before
      Soroban - and it does NOT fix 0086, which still pollutes oracle_prices and
      every other reader.
      Prod shape while measuring: oracle_prices holds 452,596 rows of which
      90,722 are peg-asset rows. The gap is expected - write_oracle is also fed
      by the event-decoded path, which carries the whole Reflector symbol set
      (BTC/ETH/XRP/...), not just the three we poll. 90,722 is what the first
      snapshot will copy, minus the 0086 rows.
---

# `prices.usd_rate` + peg-asset rate population

## Summary

Build the USD rate table exactly as [[0151]] decided and [[0154]] specified, and
populate it with the one rate class that needs no new enrichment machinery: the
**peg assets** (USDC, USDT), whose real, depeg-aware USD price we already poll
and store but never publish.

This is a strict extraction. **No shape changes to the table**, no change to
`close_usd`, no new key. 0154 keeps the second pivot tier and consumes a table
that already exists and has been validated against today's data.

## Context

`oracle_worker` already polls Reflector for all three reference assets:

```rust
// packages/oracle-worker/src/lib.rs:33
pub const TRACKED_SYMBOLS: &[&str] = &["XLM", "USDC", "USDT"];
```

So a real USDC/USD rate — 0.999x, not a hardcoded `1` — is written to
`prices.oracle_prices` today. Two things stop it reaching a consumer:

1. **Nothing publishes it.** [[0165]] can only fill USDC at a flat `$1`, which
   is a ~0.1% systematic error on every row (small depegs are routine, not a
   crisis event), and which **contradicts our own candles**: the oracle tier is
   depeg-aware and already prices a `TF/USDC` candle at `close × 0.9993`
   (`ch_enrich.rs:20`, *"the depeg-aware tier and it wins where it applies"*).
   Publishing `USDC = 1.0000` beside those candles is internally inconsistent.
2. ⏳ **`oracle_prices` expires.** Retention is **13 months**
   (`cleanup-worker/src/lib.rs:24`), by monthly partition drop. Coverage starts
   **2026-03-11** (measured on prod 2026-08-10 — see the correction below), so
   the earliest partition `202603` ages out around **2027-04**. Every month this waits,
   a month of depeg-aware history is lost for good — the same
   "we had the data and let it expire" shape as the 2026-07-20 cleanup incident.

The view layer cannot solve this by joining `oracle_prices` directly: that makes
the published series **mutate** as rows age out (a bucket reading `0.9993` today
silently reverts to `1.0000` later), which is exactly why `views.sql:47` forbids
it. The rate must be **snapshotted into a forever-retained table**, the same way
every other USD number is baked rather than joined.

## Relationship to 0151 — why this is not the rejected refactor

[[0151]] (`:106`) decided: *adopt the rate table **narrowly inside 0154**; reject
the schema-wide refactor.* What it rejected was demoting `close_usd` to a derived
cache — Nullable, dissolving the [[0145]]/[[0146]]/[[0147]] bug class,
restructuring the fact table. Its three grounds, checked against this task:

| 0151's reason | Applies here? |
|---|---|
| Headline benefit lost its consumer — NULL renders as a dash for BE | **No.** `close_usd` untouched, non-nullable. Nothing published is NULL; absence of a rate row falls back to the documented peg value. |
| The bugs it dissolves are cheap one-liners already queued | **No.** This claims none of them. 0145 already shipped. |
| [[0139]] grew underneath it — a `quote_asset_id` key would inherit collisions | **No** — it uses 0154's **natural-identity** key precisely because of 0139 (`asset_id 4194` is both `STW` and `ARBRIDGE`). Reuse here is compliance, not evasion. |

The revisit trigger at `0151:142` (*"revisit if 0139's repair needs a fact-table
migration anyway"*) is **not tripped** — nothing here needs one.

⚠️ **One gating unknown IS pulled forward, deliberately.** `0151:138` notes that
scoping to 0154 shrank two unknowns. This task hits one of them and not the other:

- **Projection cost — not hit.** That unknown is about `quote_asset_id` being the
  *second* sort-key column on `price_ohlcv_1m` (`init.sql:122`), so "every candle
  quoted in X" has no clean index path. This task never asks that — it reads
  `oracle_prices`, ordered `(asset_id, oracle_name, timestamp)`.
- **Time-resolution — hit, and settled below.** Peg assets are the **cheapest
  possible place to settle it**: USDC trades in a ~0.1% band, so any reasonable
  rule differs by <0.05%, while the vocabulary it establishes is what 0154
  inherits for volatile pivot assets where the choice actually matters and
  getting it wrong *"invents a subtler restatement of finding 3"* (`0151:174`).
  Settling it here is a favour to 0154, not a raid on it.

## Decision — the time-resolution rule

**`usd_rate` stores observations, not bucket aggregates. Every consumer resolves
a rate by `ASOF` at-or-before the timestamp it cares about, bounded by a
staleness window. There is no averaging, no vwap, and no per-grain rule.**

Concretely:

1. **Write one row per oracle observation**, at the source's own cadence. No
   downsampling — three tracked assets at any plausible poll interval is
   ~10⁶ rows/year, which is nothing, and downsampling would destroy information
   a later consumer cannot recover.
2. **A consumer needing a rate at time `T`** takes the newest observation with
   `timestamp <= T`. For a bucket-grained consumer such as `price_usd_series`,
   `T` is the **bucket's end** — i.e. the bucket's *closing* rate.
3. **Forward-fill is bounded by a staleness window.** Past it, there is no rate
   and the consumer's own fallback applies. Unbounded forward-fill would silently
   present a three-day-old reading as current.

**Why this and not average/vwap:**

- **It is the rule the codebase already uses.** The oracle tier is
  `ASOF LEFT JOIN … ON o.timestamp <= p.timestamp` with a post-join staleness
  floor (`ch_enrich.rs:434-440`), and the XLM pivot forward-fills the same way
  (`:773-781`). One rule everywhere beats a second vocabulary.
- **It composes across all six granularities for free.** A daily close is the
  ASOF at day-end, which *is* the last hourly close. Averages do not compose —
  the mean of hourly means is not the daily mean unless the counts match — so an
  averaging rule would need six separate definitions and a consistency proof
  between them. This is the property that makes 0154's job smaller.
- **It matches what the consuming view means.** `price_usd_series` is *"one USD
  **close** per (natural identity, day bucket)"* (`views.sql:169`), and
  `usd_reference` is likewise a close. A bucket average would be a different
  statistic wearing the same column name.
- **vwap is not available** — oracle observations carry no volume.

**Known cost, accepted:** a close is more exposed to a single outlier reading at
the bucket boundary than an average is. For peg assets the band is ~0.1% so the
exposure is negligible; for 0154's volatile assets the ASOF is at the *candle's*
timestamp rather than a bucket end, so the boundary case does not arise there
either. Record it rather than mitigate it.

## Implementation

1. **Table** — `prices.usd_rate` exactly per [[0154]]'s spec (natural identity
   key, `method`, `reference_asset`, `hops`, `version`, RMT). ⚠️ Deliberately
   **not** added to `cleanup-worker`'s `RETENTION` allowlist — that list is
   opt-in, so a new table is retained forever by default. Add a comment saying so,
   or a future reader will "helpfully" add it.
2. **Peg population** — aggregate `oracle_prices` → `usd_rate` rows for the peg
   assets with `method = 'oracle'`, `hops = 0`, `reference_asset = ''`.
   `oracle_worker` is the natural home: it already owns this asset list and this
   concept. Must run well inside the 13-month window; incremental by watermark.
3. **Deep-history rows** — pre-oracle buckets (before **2026-03-11**) get **no row**.
   Absence is the signal; the consumer's peg fallback covers it. Do *not* write
   synthetic `method = 'peg'` rows at `$1` — that would make a fallback
   indistinguishable from a measurement, which is the [[0165]] / `close_usd = 0`
   mistake again.
4. **Verification before trust** — [[0154]]'s constraint 5: check the table
   reproduces today's `close_usd` for the tiers that already work, before
   anything reads it for pricing. A mismatch is a bug found either way.

## Acceptance Criteria

- [x] `prices.usd_rate` exists with 0154's exact shape, keyed on natural identity.
      Asserted in `usd_rate_it.rs` on the exact column list **and order**, the
      `ReplacingMergeTree(version)` engine, and — the load-bearing one — that the
      sorting key does **not** contain `asset_id`.
- [x] Absent from `cleanup-worker`'s `RETENTION` list, with a comment stating that
      the omission is deliberate. Plus a **tripwire test** in `cleanup-worker`
      itself: the protection is an *absence*, which is exactly the invariant a
      later reader breaks with one tidy-looking line.
- [x] Peg-asset observations populated from `oracle_prices`, incremental, with
      `method`/`hops` set; re-runnable without duplication (RMT `version`).
      `populate_usd_rate_from_oracle`, watermarked per identity; the IT copies,
      re-runs (no duplication), then appends only a newly-arrived reading.
- [~] The ASOF + staleness resolution rule implemented and documented in the
      table's schema comment, not only in this task. **Documented in full at the
      table** (`init.sql`), including why it is not an average and the
      composes-across-grains argument. *Implementation* is necessarily a
      **consumer** concern — nothing reads `usd_rate` yet, and [[0168]] is its
      first reader. Not claimed as done here.
- [~] Pre-oracle buckets have **no** row — verified, not assumed. True **by
      construction**: the population only ever copies rows that exist in
      `oracle_prices`, so a bucket with no oracle reading gets no row, and there
      is no synthetic-`peg`-row path to write one. Not yet asserted against real
      pre-2026-03-11 data — DISCHARGED on prod 2026-08-10: `min(timestamp)` in
      `usd_rate` is `2026-03-11 14:00:00`, matching `oracle_prices` exactly, and
      `count() WHERE timestamp < '2020-01-01'` is 0.
- [ ] Reproduces today's `close_usd` for the oracle and peg tiers on a sample
      window (0154 constraint 5), on CH **26.3.10.60**. **Requires prod data —
      operator-run, see §Prod backfill.** This is the gate before anything
      *reads* the table for pricing.
- [x] `0154` and `0151` updated to record that the table moved here. **Already
      done in PR #182** when 0167 was authored — re-checked 2026-08-10, both
      reference 0167; no further edit needed.

## Prod backfill — operator-run, not automatic

The worker only snapshots **forward** from its watermark, so on first run against
prod it copies the whole surviving `oracle_prices` window in one pass — which is
the point, given the 13-month clock. Nothing is deployed by merging this; the
population runs when `oracle-worker` next executes against prod, or on demand.

⏳ ~~Do this before ~2026-10, or `202509` is gone.~~ **CORRECTED — the real
deadline is ~2027-04.** See §Coverage correction. Done anyway on 2026-08-10.

**Gate before any consumer trusts it** (0154 constraint 5 — reproduce today's
`close_usd` from the stored rate):

```sql
-- Every USDC-quoted candle should satisfy close_usd ~= close * rate-at-or-before.
SELECT round(100 * countIf(abs(toFloat64(p.close_usd) - toFloat64(p.close) * toFloat64(r.usd_rate)) > 1e-6)
             / count(), 4) AS pct_mismatch,
       count() AS sampled
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
ASOF LEFT JOIN prices.usd_rate AS r
  ON r.asset_code = q.asset_code AND r.issuer_address = q.issuer_address
  AND r.timestamp <= p.timestamp
WHERE q.asset_code = 'USDC' AND p.close_usd > 0
  AND p.timestamp >= now() - INTERVAL 30 DAY;
```

⚠️ Expect a **small non-zero** mismatch, not exactly 0: enrichment baked
`close_usd` at the rate current *then*, and the peg tier used a flat `$1` where
the oracle tier did not. A large mismatch means the rate table disagrees with
the tier that produced the candle — a bug either way, which is the point of the
check.

⚠️ **Do not gate on USDT until [[0172]] is understood.** Its candles close at
~0.14 against USDC on prod, so any reconciliation through USDT will fail for a
reason that has nothing to do with this table.

## Out of scope

- **The second pivot tier** — stays in [[0154]], still blocked behind [[0111]].
  This task must not add any join to the enrichment hot path.
- **Any change to `close_usd`** — non-nullable, `DEFAULT 0`, written in place.
  That is [[0151]]'s rejected refactor.
- **Publishing the rate** — that is [[0168]].
- **Populating pivot/pivot2 rates** — 0154 owns those methods.

## Notes

- Do not key anything on `asset_id` while [[0139]] is open — confirmed genuine
  collisions between unrelated assets.
- The 13-month clock is soft but real: build this before ~2026-10 or the earliest
  depeg-aware history is gone. It is not a reason to rush [[0165]], which ships
  its documented `$1` approximation independently.
