---
id: "0170"
title: "GET /assets/{USDC}/ohlcv returns an empty series in every mode — the endpoint asks for a USDC/USDC self-pair, and blocks 0127's M2 acceptance criterion"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0165", "0127", "0167", "0168", "0139", "0061", "0040", "0120"]
tags:
  ["priority-high", "effort-medium", "api", "data-correctness", "read-surface", "scf", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
history:
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Spawned from 0165 while checking whether that fix also unblocks 0127's
      backfill-depth ACs. It does not — this is a THIRD surface with the same
      root cause (USDC is always the quote leg, never the base) but a different
      code path. 0165 rewrites the price_usd_series view; /ohlcv never reads
      that view, it queries price_ohlcv_1d directly with its own base+quote
      filter. Shipping 0165 leaves this exactly as broken. Confirmed from code
      plus 0165's existing prod measurement; no new query needed.
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: >
      Blast radius measured wider than the self-pair by the [[0120]]
      conformance run: the default `base_currency=USD` mode returns an empty
      200 for ANY asset that never traded against canonical USDC, not just
      for USDC itself. Five of 0120's twenty majors hit it — AUD, RON, BOL,
      EQL (top-20 by volume) and the top soroban asset `CBIJ…` all return
      0 buckets in USD mode over a 30-day window while returning 2–31 real
      1d buckets with `base_currency=XLM` over the same window. The
      "empty 200 is the wrong answer" argument below therefore applies to
      every XLM-only-quoted asset in the store; a spawned-then-retired
      duplicate spawn from that run (retired the same day; its id was since
      reused by an unrelated task) folds into this task, and the 0120
      suite's empty-window checks are its regression gate.
---

# `/assets/{USDC}/ohlcv` can never return candles

## Summary

`GET /assets/{USDC}/ohlcv` returns **`200 OK` with `data: []`** in every mode,
at every timeframe, regardless of backfill depth. It is structurally
unsatisfiable, not a coverage gap.

The endpoint resolves the path asset as the **base** leg and a separate **quote**
leg from `base_currency`, then filters on both. `base_currency` defaults to
`USD`, which maps to USDC — so the default request asks ClickHouse for a
**USDC/USDC self-pair**.

This blocks [[0127]] AC 3, and therefore [[0128]] and Milestone 2.

## Mechanism

`handlers.rs:315` resolves the base asset from the path. `handlers.rs:322-330`
resolves the quote leg from `base_currency`:

```rust
let quote_ident = match base_currency {
    BaseCurrency::Usd => AssetIdentifier::Classic {
        code: "USDC".to_string(),
        issuer: prices_clickhouse::USDC_ISSUER.to_string(),
    },
    BaseCurrency::Xlm => AssetIdentifier::Native,
};
```

`queries_ch.rs:545` then filters on **both** legs:

```rust
let mut conds = vec!["asset_id = ?".to_string(), "quote_asset_id = ?".to_string()];
```

So for `/assets/USDC:GA5ZSEJY…KZVN/ohlcv` the query is:

| `base_currency` | resulting predicate | rows |
|---|---|---|
| `USD` (default) | `asset_id = <USDC> AND quote_asset_id = <USDC>` | **0** — a self-pair; no asset trades against itself |
| `XLM` | `asset_id = <USDC> AND quote_asset_id = <XLM>` | **0** — see below |

The `XLM` mode fails for the [[0165]] reason rather than the self-pair reason:
canonicalisation makes USDC the **preferred quote**, so the XLM/USDC pair is
stored base=XLM / quote=USDC (exactly as `usd_reference` reads it,
`views.sql:160-166`) and the inverse orientation is never written.

**No new measurement is needed.** 0165 already established on prod, twice
(2026-08-06, re-verified 2026-08-07):

```
price_ohlcv_1d WHERE asset_id = <USDC>   ->  0 candles
```

That is **unconditional on quote** — USDC is the base of nothing, so both rows
of the table above are settled by a measurement already in the repo.

The asset itself resolves fine (`asset_id = 3`, collision-guard verified in
0165), so there is no 404 — the request succeeds and returns nothing.

## Why [[0165]] does not fix this

Different code path, same root cause. **This is the load-bearing point of the
task.**

| | reads | fix vehicle |
|---|---|---|
| [[0165]] | `prices.price_usd_series` (a view) | `CREATE OR REPLACE VIEW` |
| **this** | `price_ohlcv_1d` directly, via `queries_ch::ohlcv` | Rust handler/query change + deploy |

`/ohlcv` never touches `price_usd_series`. Ship 0165 and this endpoint is
unchanged. Anyone reasoning "we fixed the USDC pricing bug" after 0165 merges
will be wrong about this surface — hence a separate task rather than a footnote.

## Why an empty 200 is the wrong answer here

The handler already contains precedent for exactly this judgement. When the
*quote* leg is untracked it deliberately returns 503 rather than an empty 200,
with this comment (`handlers.rs:333-336`):

> its absence is a server-side data gap, not "no candles". Surface it as a 503
> instead of masking it as an empty 200 (which looks like a healthy asset with
> no history).

An empty 200 for USDC has the identical defect: it is indistinguishable from "a
tracked asset that simply never traded". USDC is the most-traded stable asset on
the network. The response is not merely unhelpful — it is **wrong**, and it is
wrong silently.

## Blast radius

- **[[0127]] AC 3** — *"`GET /assets/{USDC}/ohlcv?timeframe=all` returns 1d
  candles from 2022-01 or earlier"* — cannot pass at any backfill depth. 0088
  finishing changes nothing here.
- **[[0127]] AC 4** — the spot-check table (*"≥5 dates × ≥2 assets, our close vs
  an independent public source"*) names USDC as the reviewer's example asset, so
  it is exposed too.
- **[[0128]]** — the M2 submission package evidences both of the above.
- Any consumer charting USDC. It is the asset a reviewer is most likely to try
  first, precisely because its expected answer (~$1) is the easiest to verify.

## Design question to settle before implementing

The AC's own wording points at the answer: *"verifiable against known USDC price
history."* USDC's USD price history is a flat ~$1. So:

**`base_currency=USD` on a peg asset** — synthesize the series from the peg rate
rather than from candles. Same semantics as [[0165]]'s peg-fill arm, applied one
layer up. Buckets should follow the requested granularity over the requested
window.

**`base_currency=XLM`** — the data exists in the opposite orientation, so either
invert the stored XLM/USDC pair (`1/close`, and OHLC inverts with high↔low
swapping) or derive from `usd_reference`. **Inversion is the subtler of the two**
— `volume_base`/`volume_quote_usd` do not invert by taking a reciprocal, and
`vwap` must be recomputed rather than flipped.

⚠️ **Whatever ships must carry [[0167]]/[[0168]]'s real-rate path**, not a
hardcoded `1.0` — same three requirements 0165 took on: test the *fallback
semantics* (no rate → peg, rate → that rate), expose **provenance** so a consumer
can tell a real `1.0000` from a placeholder `1.0000`, and reference 0168 by ID in
the code comment. A flat `$1` contradicts our own candles, which already price a
`TF/USDC` candle at `close × 0.9993` (`ch_enrich.rs:20`).

⚠️ **Do not "fix" this by making the endpoint fall back to the inverse pair
generally.** `/ohlcv` returns OHLC **in the quote asset**
([[ohlcv-returns-quote-asset-not-usd]]); silently inverting for some assets and
not others makes the response's meaning depend on data availability, which is the
`close_usd = 0` mistake in a new place.

## Implementation sketch

1. Detect the degenerate case in `get_ohlcv`: resolved `asset_id ==
   quote_asset_id`. Today it falls through to a query that cannot match.
2. For a **peg** asset (USDC/USDT at canonical issuer) with `base_currency=USD`,
   serve the peg/oracle rate as a synthetic series at the requested granularity.
3. For `base_currency=XLM`, decide invert-vs-`usd_reference` per above and
   implement one, with the volume/vwap caveats handled explicitly.
4. For a **non-peg** self-pair (any asset requested against itself), a flat 1.0
   is defensible and cheap — decide explicitly rather than letting it fall
   through to an empty 200.
5. Provenance field on the response, consistent with whatever 0165 names its
   `method` values.

## Acceptance Criteria

- [ ] `GET /assets/{USDC}/ohlcv?timeframe=all` returns a non-empty 1d series
      spanning the backfilled range, verified **through the deployed API**, not
      only in a test.
- [ ] The returned USDC/USD closes sit within a stated tolerance of ~$1, and the
      tolerance is justified (not asserted as exactly `1.0`).
- [ ] `?base_currency=XLM` returns a non-empty, correctly-oriented series, with
      `volume_*` and `vwap` verified rather than assumed to invert.
- [ ] Fallback **semantics** tested, not the constant: no rate available → peg;
      rate available → that rate. A test asserting exactly `1.0` must not be the
      only coverage ([[0168]] would have to rewrite it).
- [ ] Response carries provenance distinguishing a measured rate from a
      placeholder.
- [ ] USDT behaves correctly too — it trades genuinely as a base in 102 pools, so
      confirm the synthetic path does **not** override real market data (the same
      trap [[0165]] documents).
- [ ] A non-peg asset's response is byte-identical to today's, proving no
      regression on the normal path.
- [ ] [[0127]] AC 3 + AC 4 re-run and passing.

## Out of scope

- **`price_usd_series`** — that is [[0165]], already active.
- **`current_prices` / `current_price_usd`** — suspected to carry the same
  base-only assumption, but it is a refreshable-MV rebuild (the operation that
  wiped the coarse tables in 0095) and gets its own task. 0165's audit AC covers
  confirming it.
- **[[0139]]** — `current_price_usd` duplicate rows. Different defect, same
  neighbourhood.
- Changing canonicalisation so USDC is sometimes a base. That would be a
  write-side change invalidating existing candles; the whole point of the
  quote-preference design is that it is stable.

## Notes

- The existing test `ohlcv_xlm_quote_has_no_candles` (`ohlcv_it.rs:158`) asserts
  an empty series for an unseeded pair. It is correct as written — a genuinely
  untraded pair *should* be empty — but it means "empty series" is currently a
  **blessed** outcome in the suite. New coverage must distinguish
  *untraded* from *structurally unrepresentable*.
- Same lesson as [[0166]]/[[0169]]: the endpoint reports success while returning
  nothing, so no signal fires. Found only by asking what the query actually
  resolves to.
