# R — Historical USD-quoted price series (`price_usd(asset, t)`): feasibility & design

> **Status:** Research / design proposal. Emerged from the Block Explorer LP-analytics
> cross-team questions (point 3: per-asset USD-quoted historical series). Scopes a
> new feature on top of the task-0060 schema + the task-0026 enrichment worker.
>
> **TL;DR:** The USD conversion is **cheap** — ~90% of the machinery already exists
> (the 0026 `volume_quote_usd` enrichment ASOF join). The real work is one code fix
> (oracle ↔ asset id reconciliation) and accepting one hard boundary (on-chain USD
> reference only exists from Reflector genesis, ~2024). End-to-end ≈ **1 week**.

## 1. The requirement

Block Explorer computes LP analytics (volume, fee_revenue, TVL) in USD **at read
time**. They need a single primitive from the Prices API:

```
price_usd(asset, t)  -- historical USD price of any asset at a given ledger's close
```

Their formulas all reduce to this lookup:

```
volume_usd       = gross_volume_a × price_usd(asset_A, t)
fee_revenue_usd  = volume_usd × fee_bps / 10000
tvl_usd          = reserve_a × price_usd(asset_A, t) + reserve_b × price_usd(asset_B, t)
```

They read `prices.*` directly via named views in the same ClickHouse cluster (no
HTTP), so a prices-owned view is the ideal delivery surface.

## 2. What exists today (and the gap)

| Capability | Status |
|---|---|
| `price_ohlcv_*.close` | Present — but denominated in the **quote asset** (`quote_asset_id`), not USD |
| `current_prices.price_usd` | Present — **"now" only**, not a historical series |
| `volume_quote_usd` | USD **volume** (oracle × volume), filled by 0026 enrichment — not a price |
| `oracle_prices.price_usd` | Historical USD **for reference assets only** (Reflector feeds) |
| **per-asset USD close per (asset, t)** | ❌ **does not exist** — this is the gap |

There is no `close_usd` column on the OHLCV tables. USD must be *derived*.

## 3. Key insight — the conversion is a one-line change

A candle's `close` is the **base-asset price expressed in the quote asset**. Quotes
are canonicalized to `USDC > USDT > XLM` (`packages/sdex-backfill/src/canonical.rs:59-78`).
Therefore:

```
close_usd(asset, t) = close × usd_price(quote_asset, t)
```

is the **USD price of the base asset** — exactly `price_usd(asset, t)`. Because the
quote is almost always USDC/USDT/XLM and Reflector publishes USD for those, it is a
**single hop**. Reference assets themselves (XLM, USDC) get `price_usd` straight from
`oracle_prices`.

The 0026 enrichment worker already performs this exact operation for *volume*
(`packages/enrichment-worker/src/ch_enrich.rs:120-156`):

```sql
CAST(o.price_usd * p.volume_quote AS Decimal(38,14)) AS volume_quote_usd
FROM price_ohlcv_1m AS p FINAL
ASOF LEFT JOIN oracle_prices AS o
  ON o.asset_id = p.quote_asset_id AND o.oracle_name = ? AND o.timestamp <= p.timestamp
```

A USD close is the **identical join**, multiplying `close` instead of `volume_quote`.

## 4. The boundary to communicate

USD history reaches back only as far as an **on-chain USD reference existed**.
Reflector is a Soroban contract → live only from ~2024 (Soroban mainnet).

- **2024 → now (~ledger 50.4M+):** ✅ full coverage with a full-history backfill (every
  Reflector `update` event captured).
- **Pre-Soroban classic history (2015–2024):** ❌ no on-chain USD oracle. SDEX candles
  stay quote-denominated; USD would require an *off-chain* XLM/USD backfill (out of
  scope). The explorer's stated target is 2024, so this boundary is acceptable.

## 5. The one real code fix — oracle ↔ asset id reconciliation

Today the backfill oracle extractor mints oracle assets a **separate synthetic id
space `≥ 1_000_000`**, keyed by symbol/contract, not written to `prices.assets`
(`packages/sdex-backfill/src/soroban.rs:42-56`). The enrichment join keys
`o.asset_id = p.quote_asset_id` — so with backfilled data **it currently matches
nothing**. This silently affects `volume_quote_usd` too.

**Fix:** resolve Reflector asset keys through the *same* `AssetRegistry` used for
trades, so oracle rows carry the canonical `asset_id` (the one used as
`quote_asset_id`), and UPSERT those reference assets into `prices.assets`:

```rust
// soroban.rs — replace synthetic oracle_id() with canonical resolution
fn reflector_key_to_identity(key: &str) -> Option<AssetIdentity> {
    match key {
        "XLM" | "native" => Some(AssetIdentity::Native),
        "USDC" => Some(AssetIdentity::Credit { code: "USDC".into(), issuer: USDC_ISSUER.into() }),
        "USDT" => Some(AssetIdentity::Credit { code: "USDT".into(), issuer: USDT_ISSUER.into() }),
        k if is_contract_address(k) => Some(resolve_sac_or_contract(k)),
        _ => None,
    }
}
// in decode_reflector:
let asset_id = match reflector_key_to_identity(&key) {
    Some(id) => reg.assets.intern(id),   // same id space as quote_asset_id ✅
    None => continue,
};
```

This is the load-bearing change. Everything else is mechanical.

## 6. Schema changes (small)

1. **Add a USD close column to every grain** (the `AS`-copies don't inherit post-hoc
   ALTERs, so apply per table):

   ```sql
   ALTER TABLE prices.price_ohlcv_1m  ADD COLUMN close_usd Decimal(38,14) DEFAULT 0;
   -- repeat: _15m, _1h, _4h, _1d, _1w, _1M
   ```

   One column if only `close` is needed; add `open_usd/high_usd/low_usd` only for
   full USD OHLC charting (same mechanism, 4× width).

2. **`oracle_prices`** — no shape change; the fix is the canonical `asset_id` (§5).

3. **Canonical view `prices.price_usd_series`** (§8) — no new physical table required;
   promote to a materialized `price_usd_1d` only if read latency demands it.

## 7. Recommended flow (index → calculate → save → serve)

```
S3 ledger XDR ──(download · decompress · parse ONCE)──> parsed LedgerCloseMeta
   │                                                          │
   ├─ SDEX trades + AMM swaps ─> candles (close in QUOTE)     │
   └─ Reflector update events ─> oracle USD per ref asset ────┘
                                   │  resolve key → canonical AssetRegistry id
                                   │  + UPSERT prices.assets
                                   ▼
                          oracle_prices (asset_id == quote_asset_id space)
                                   │
   price_ohlcv_1m (close, quote_asset_id, source, close_usd=0)
                                   │
        ENRICHMENT PASS — ASOF join on quote_asset_id:
           close_usd        = oracle_usd × close
           volume_quote_usd = oracle_usd × volume_quote
           re-INSERT with version+1  (ReplacingMergeTree collapses)
                                   │
        ROLLUP / PREROLL — argMax(close_usd, timestamp) per bucket
                                   ▼
        _1h / _1d / _1w / _1M  (forever-retained → deep history lives here)
                                   ▼
        view prices.price_usd_series  ──>  API
```

Concrete changes per stage:

- **Index:** `soroban.rs` oracle-id canonicalization (§5).
- **Calculate:** `enrichment-worker/src/ch_enrich.rs` — add
  `CAST(o.price_usd * p.close AS Decimal(38,14)) AS close_usd` to the SELECT, and
  change the candidate filter to include `close_usd = 0`. Idempotency / `version+1` /
  `FINAL` unchanged.
- **Save / rollup:** `schema/rollups.sql` + `schema/preroll.sql` — add
  `argMax(close_usd, timestamp) AS close_usd` to each grain's aggregation. USD close
  aggregates like `close` (last value), **not** like a sum.
- **(Optional) peg fallback:** COALESCE USDC/USDT reference to `1.0` when an oracle row
  is momentarily missing.

## 8. The view + API

The view collapses per-source/per-quote rows to one USD close per `(asset, bucket)`,
keeping source/quote/grain policy on the prices side:

```sql
CREATE VIEW prices.price_usd_series AS
-- reference assets: USD direct from oracle
SELECT asset_id, timestamp AS bucket, argMax(price_usd, timestamp) AS close_usd
FROM prices.oracle_prices
GROUP BY asset_id, timestamp
UNION ALL
-- everything else: volume-weighted USD close across quotes/sources in the bucket
SELECT asset_id, timestamp AS bucket,
       sum(close_usd * volume_base) / nullIf(sum(volume_base), 0) AS close_usd
FROM prices.price_ohlcv_1d FINAL          -- grain picked per retention
WHERE close_usd > 0
GROUP BY asset_id, timestamp;
```

**API — buildable directly on this:**

- **Point lookup (the primitive):** `GET /assets/{id}/price/at?ledger=N` → map
  `ledger → closed_at → bucket`, select `close_usd` from the finest grain still
  retained for that time (1m ≤7d, 15m ≤30d, else 1h/1d). Returns `price_usd(asset, t)`.
- **Series:** `GET /assets/{id}/price/history?from=&to=&interval=1d` →
  `SELECT bucket, close_usd FROM prices.price_usd_series WHERE asset_id=? AND bucket BETWEEN ? AND ? ORDER BY bucket`.

For the explorer's read-time LP joins this is the cheap, unambiguous lookup they
asked for — one column, canonical USD, no multi-source/multi-quote pick per query.

## 9. Coverage contract (what we can guarantee)

- USD close for any asset that traded against a USDC/USDT/XLM quote,
- at any time **from Reflector genesis (~2024) onward**,
- at whatever grain retention keeps (1d retained forever → deep history covered).
- Exotic-quote-only assets (no stable/XLM leg, no oracle) and pre-Soroban classic
  history → **`NULL`**. The API returns `null` rather than guessing; the explorer
  decides NULL-vs-partial TVL on their side.

## 10. Effort estimate

| Work item | Effort |
|---|---|
| `close_usd` ALTERs + writer Row struct | ~0.5 day |
| **Oracle ↔ asset id reconciliation** (`soroban.rs`; shared fix with 0026) | 1–2 days |
| Enrichment `close_usd` expression + candidate filter | ~0.5 day |
| Rollup / preroll propagation | ~0.5 day |
| `price_usd_series` view + API endpoints | ~1 day |
| Tests / fixtures | ~1 day |
| **End-to-end (recent history, ≤ Reflector genesis)** | **~1 week** |
| Off-chain XLM/USD backfill for pre-2024 classic history | Out of scope; decision-gated |

## 11. Open questions / next steps

1. **Reflector asset-key format** — confirm symbol vs contract-address keys against
   captured oracle samples so `reflector_key_to_identity` is exact. (See
   [[soroban-events-samples]] / [[soroban-events-gotchas]].)
2. **Cross-source collapse policy** — volume-weighted (proposed) vs canonical-source
   priority for the single per-asset USD close.
3. **Confirm production Oracle Fetcher** (task 0039) assigns `prices.assets` ids, so
   the §5 reconciliation is consistent between backfill and live paths.
4. ~~Spawn a dedicated task for this feature~~ — **done: task 0061** (this note's
   home), branched off `develop` for clean separation from 0060.

## 12. Refinements from BE follow-up (2026-06-12)

Five clarifying questions from the Block Explorer team locked the following
decisions. They supersede / sharpen §4, §5, §8, §9 where they overlap.

### 12.1 Deep-history USD reference = XLM/USDC pivot with USDC≡$1 (not `oracle_prices`)

`oracle_prices` is retention-capped (13 months), so it **cannot** be the
read-time USD source for deep history. Instead:

- We **bake `close_usd` into the candle at enrichment time** — a stored column,
  retained forever on the rolled grains (`_1h/_1d/…`). The read-time view reads a
  column, it does **not** join `oracle_prices`. Historical USD survives because
  it is already multiplied-in.
- **Reference tiering** for the multiplier:
  - recent window (oracle row present) → real oracle USDC/XLM price (captures depeg);
  - beyond / pre-Reflector → **USDC≡$1 (and USDT≡$1) peg × the XLM/USDC candle**.
    This is the deep-history backbone and the *primary* pre-2024 mechanism (not a
    mere fallback — supersedes the "(optional) peg fallback" note in §7).
- How far back this reaches is **backfill-determined, not retention**: with the
  full-history backfill, to XLM/USDC SDEX genesis (≈ USDC-on-Stellar launch) —
  **earlier than 2024-02-20**. Confirm the concrete first XLM/USDC ledger once the
  production backfill range is locked.
- **Caveat to surface:** USDC≡$1 is a peg approximation; during a depeg, peg-based
  deep-history USD is slightly off vs a true oracle. Acceptable for LP analytics.

### 12.2 Public lookup key = natural Stellar identity, never `asset_id`

`asset_id` is an internal app-assigned `UInt32` surrogate (`canonical.rs:69`) —
not portable. The public key is:

- native XLM → `native`;
- classic → `(asset_code, issuer_address)`;
- SAC / Soroban token → `contract_address`.

The view/API resolves identity → `asset_id` internally via `prices.assets`.
**Reconciliation:** the writer currently stores native XLM as `asset_type='classic'`
with empty issuer (`sink.rs:125`); align the *public* key to expose XLM as `native`
and map internally.

### 12.3 NULL semantics + per-asset vs systemic discriminator

`close_usd` is **NULL — never an error, never drops the row** (LEFT-JOIN-friendly).
A discriminator distinguishes the two failure modes BE must tell apart:

| status | meaning | TVL impact |
|---|---|---|
| `ok` | priced | — |
| `no_asset_price` | this asset has no candle/illiquid leg at T, **but the USD reference IS available** | partial TVL from the other leg is valid |
| `no_reference` | the USD reference itself is missing at T (systemic — **all** XLM-pivot assets NULL at that T) | partial does **not** save you |

Computable: `no_reference ⟺ no XLM/USDC pivot (and no oracle) for bucket T`; else a
missing own-candle ⟹ `no_asset_price`. Also expose a companion
`prices.usd_reference(bucket)` (reference value / boolean per bucket) BE can
`LEFT JOIN` to detect systemic blackouts independently of any single asset.

### 12.4 SAC + classic underlying = ONE row, ONE price

Collapse a SAC to its underlying classic/native identity → SDEX-classic and
AMM-via-SAC share one `asset_id` and one USD price. **Required** by the
cross-source merge (ADR 0004: per-source rows under the same `asset_id`); two rows
would split liquidity and break the merge. A pure Soroban-native token (no classic
underlying) gets its own row keyed by `contract_address`.

**Not implemented yet:** `AssetIdentity` models only `Native` / `Credit{code,issuer}`
(`canonical.rs:6-9`) and the SDEX writer always writes `contract_address=''`
(`sink.rs:135`). The SAC-address → underlying-classic resolver is part of this
task's §5 reconciliation work.

### 12.5 `price_usd_at` is a single-asset primitive

It returns the USD price of **one** asset at `t`. `volume_usd =
gross_volume_a × price_usd_at(A,t)` is a single call; TVL is just two independent
calls (one per leg). Single-asset is the base case — confirmed.

### 12.6 Grain-selection ownership: views = caller-passes, 0040 API = view-picks

`close_usd` exists at several grains with different retention (`_1m` 7d, `_15m`
30d, `_1h`/`_4h`/`_1d`/… forever), so the *finest grain still retained for a time
`T`* varies with how old `T` is. Who picks the grain for a lookup?

**Decision (2026-06-15):**

- **Views = caller-passes.** The in-cluster views are exposed *per grain*
  (`price_usd_series` / `_1h`, `usd_reference` / `_1h`); the consumer JOINs the
  grain its query needs. Rationale: a chart wants **one consistent grain per
  query** (chosen by its zoom/window) — mixing grains across points would create
  resolution discontinuities — and it keeps the views a dumb, fast,
  retention-agnostic data surface (no coupling of grain/retention policy into the
  view, and none of the cross-grain `UNION` a single "smart" view would need).
  This matches BE's own `timeframe`/`start+end` API mental model.
- **0040 HTTP API = view-picks.** The turnkey point-lookup primitive
  `price_usd_at(id, ts)` (task 0040) maps `ledger → ts → finest-retained grain`
  and returns one `close_usd`. A point lookup wants *finest-available*, and the
  API layer is the natural home for that retention-aware routing — keeping the
  policy out of both the views and the consumer.

Net: **views = caller-passes; the 0040 API primitive = view-picks.** No new view
design needed (already shipped per-grain). Open only: BE confirms they own grain
choice at the JOIN layer; 0040 implements the `ts → grain` routing when built.
