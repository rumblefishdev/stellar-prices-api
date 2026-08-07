---
id: "0167"
title: "Create prices.usd_rate and populate the peg-asset rates from oracle_prices — extracted from 0154 so it is not blocked behind 0111"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0154", "0151", "0165", "0168", "0139", "0111"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "schema", "enrichment", "data-correctness", "milestone-M2"]
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/cleanup-worker/src/lib.rs"
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
   ~2025-09, so `202509` ages out around **2026-10/11**. Every month this waits,
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
3. **Deep-history rows** — pre-oracle buckets (before ~2025-09) get **no row**.
   Absence is the signal; the consumer's peg fallback covers it. Do *not* write
   synthetic `method = 'peg'` rows at `$1` — that would make a fallback
   indistinguishable from a measurement, which is the [[0165]] / `close_usd = 0`
   mistake again.
4. **Verification before trust** — [[0154]]'s constraint 5: check the table
   reproduces today's `close_usd` for the tiers that already work, before
   anything reads it for pricing. A mismatch is a bug found either way.

## Acceptance Criteria

- [ ] `prices.usd_rate` exists with 0154's exact shape, keyed on natural identity.
- [ ] Absent from `cleanup-worker`'s `RETENTION` list, with a comment stating that
      the omission is deliberate.
- [ ] Peg-asset observations populated from `oracle_prices`, incremental, with
      `method`/`hops` set; re-runnable without duplication (RMT `version`).
- [ ] The ASOF + staleness resolution rule implemented and documented in the
      table's schema comment, not only in this task.
- [ ] Pre-oracle buckets have **no** row — verified, not assumed.
- [ ] Reproduces today's `close_usd` for the oracle and peg tiers on a sample
      window (0154 constraint 5), on CH **26.3.10.60**.
- [ ] `0154` and `0151` updated to record that the table moved here.

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
