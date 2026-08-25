---
id: "0170"
title: "GET /assets/{USDC}/ohlcv returns an empty series in every mode — the endpoint asks for a USDC/USDC self-pair, and blocks 0127's M2 acceptance criterion"
type: BUG
status: active
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
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Activated. Picked up as the highest-leverage task left in M2: it gates
      [[0127]] AC 3/AC 4 directly and is one of [[0120]]'s three blockers, so
      it stands between the milestone and two acceptance criteria at once.
      Scope taken as the WIDER reading from the 2026-08-19 measurement above —
      every XLM-only-quoted asset, not the USDC self-pair alone; the title
      still describes the narrow case and should be re-read against that note.
      Ordered ahead of [[0178]] deliberately: same root cause and both need the
      same return-semantics decision, but this one is a handler change with
      [[0120]]'s suite already standing as its regression gate, where 0178 is a
      refreshable-MV DROP+CREATE — the [[0095]] shape — and wants an
      uninterrupted window plus a written rollback. Settling the semantics here
      gives 0178 a precedent to inherit.
      ⚠️ Open before implementation: the empty-200 vs 503 vs synthesized-from-
      peg question is a public API contract and is shared with 0178 and with
      what 0120's suite asserts — to be agreed with the 0120 owner rather than
      decided unilaterally, likely as an ADR.
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Blast radius MEASURED on prod, not estimated: **20,481 assets** return an
      empty 200 on the default `base_currency=USD` request — every asset with an
      XLM-quoted candle and no USDC leg in the last 30 days. The 2026-08-19 note
      said "five of 0120's twenty majors"; five was the sample, not the
      population. Also settled the fix path: `close_usd` is already on the candle
      row at **99.9%** coverage (76,757 of 76,803 rows) and **uniform across
      granularities** (1m 98.3%, 15m 98.1%, 1h 98.1%), so the dominant case needs
      no rate join and no dependency on [[0167]]'s coverage window — the
      per-bucket rate derives in-table as `close_usd / close`. `close = 0` count
      is 0 across the sample. Two things got WORSE on measurement, both recorded
      in full: rows quoted against a leg that is neither USDC nor XLM carry 0%
      USD coverage and are NOT in the 20,481 (the population query filters
      `xlm_rows > 0`), so the honest limit is 26 dark assets plus an uncounted
      exotic-quoted population; and serving the 20,481 requires reinterpreting
      `base_currency` from a PAIR FILTER to a DENOMINATION, which is a public API
      contract change shared with [[0178]] and [[0120]]'s suite — flagged for an
      ADR rather than decided here. Sections added: "Measured on prod", the
      `base_currency` meaning decision, sketch steps 6-9, six new ACs.
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

> ⚠️ **Measured 2026-08-25 — this section understates it by four orders of
> magnitude.** The consumer impact is **20,481 assets**, not USDC alone. See
> "Measured on prod" below; the entries here remain accurate as the *milestone*
> impact.

- **[[0127]] AC 3** — *"`GET /assets/{USDC}/ohlcv?timeframe=all` returns 1d
  candles from 2022-01 or earlier"* — cannot pass at any backfill depth. 0088
  finishing changes nothing here.
- **[[0127]] AC 4** — the spot-check table (*"≥5 dates × ≥2 assets, our close vs
  an independent public source"*) names USDC as the reviewer's example asset, so
  it is exposed too.
- **[[0128]]** — the M2 submission package evidences both of the above.
- Any consumer charting USDC. It is the asset a reviewer is most likely to try
  first, precisely because its expected answer (~$1) is the easiest to verify.

## Measured on prod — 2026-08-25

The 2026-08-19 note above put the blast radius at *"five of 0120's twenty
majors"*. Measured directly, it is **20,481 assets** — and the fix is cheaper
than this task assumed, because the data needed to serve them is already in the
candle row.

### The population

`price_ohlcv_1d`, last 30 days. Assets with ≥1 XLM-quoted row and **zero**
USDC-quoted rows — i.e. every asset the default `base_currency=USD` request
returns an empty `200` for:

| metric | value |
|---|---|
| assets XLM-quoted-only | **20,481** |
| rows | 76,803 |
| rows carrying `close_usd > 0` | 76,757 — **99.9%** |
| assets fully covered | 20,449 |
| assets fully dark (`close_usd` never set) | 26 |

This is a different order of magnitude from the self-pair the title names. The
self-pair is one asset; this is twenty thousand.

### The five named majors — 100%, and no division landmine

Same window, the assets 0120 flagged (`quote_leg` derived from `quote_asset_id`;
USDC = `asset_id` 3, XLM = 4):

| code | issuer/contract | quote leg | buckets | rows | `close_usd` | `close = 0` |
|---|---|---|---|---|---|---|
| — | `CBIJBDNZNF` | XLM | 30 | 30 | 30 (100%) | 0 |
| RON | `GDE6EMCCVP` | XLM | 13 | 13 | 13 (100%) | 0 |
| EQL | `GBKIUHEKEC` | XLM | 8 | 8 | 8 (100%) | 0 |
| BOL | `GDOV2XVGNQ` | XLM | 10 | 10 | 10 (100%) | 0 |
| AUD | `GBBWRCJSZR` | XLM | 23 | 23 | 23 (100%) | 0 |

**Not one `USDC` quote-leg row appears in the whole result set** — the 2026-08-19
finding reproduces exactly, from an independent query.

`close = 0` is **0** on every row, so deriving a per-bucket rate as
`close_usd / close` has no division landmine in this sample. That must still be
guarded in code — the sample is 30 days, not all history.

⚠️ **`AUD` resolves to 15 distinct issuers** with XLM-quoted candles in the
window. "AUD" is not one asset; any test or evidence row must pin the issuer, or
it is asserting on whichever issuer happened to sort first.

### Coverage is uniform across granularities

XLM-quoted rows, last 2 days (2 days not 30 — `_1m` is 7-day retention):

| grain | with `close_usd` | total | pct |
|---|---|---|---|
| `1m` | 152,633 | 155,272 | **98.3%** |
| `15m` | 59,342 | 60,468 | **98.1%** |
| `1h` | 32,660 | 33,290 | **98.1%** |

So there is **no separate design for fine granularities** — the same derivation
serves all seven. This was the main open risk before measuring: had `1m` been
dark, the fix would have worked for `1d`/`1w`/`1M` and returned nothing for
`1m`, which is worse than today's uniform emptiness.

⚠️ This does **not** contradict the 2026-08-21 reading that *"every exotic leg is
exactly 100% unpriced"*. That population is legs which are **neither** USDC nor
XLM — see below. XLM-quoted rows were never the dark ones. Two different slices,
both true.

The residual ~1.8% is enrichment lag on the most recent buckets, which is a
design input, not a defect — see the open question below.

### 🔴 The exotic-quoted population is dark, and the 20,481 does not include it

Rows quoted against something that is neither USDC nor XLM carry **zero** USD
coverage — 0% on every single one:

| code | issuer | buckets | rows | `close_usd` |
|---|---|---|---|---|
| AUD | `GDNUSUAPQ6` | 28 | 177 | **0** |
| AUD | `GADSZSZVMK` | 22 | 34 | **0** |
| AUD | `GBBWRCJSZR` | 23 | 30 | **0** |
| BOL | `GCD6T6GKYM` | 3 | 4 | **0** |

This is [[0114]]'s no-reference floor surfacing on the read path — the same floor
[[0218]]'s record measured at ~10.1 M rows.

⚠️ **The 20,481 figure excludes them.** The population query filters
`usdc_rows = 0 AND xlm_rows > 0`, so an asset quoted *only* against an exotic leg
has `xlm_rows = 0` and was never counted. The honest limit for [[0128]] is
therefore **26 fully-dark assets plus an uncounted exotic-quoted population** —
that second number must be measured before any evidence is written, not left as
"26".

### What this settles

1. **Case 2 needs no rate join.** `close_usd` is already on the candle row at
   99.9% coverage, uniform across grains. No `ASOF` join, no dependency on
   [[0167]]'s coverage window, for the twenty thousand.
2. **[[0167]]/[[0168]]'s rate path is still required** — but only for the USDC
   self-pair and inverse cases, which are one asset.
3. **The task is mis-titled and under-prioritised.** It reads as a USDC edge
   case; it is the default response for 20,481 assets.

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

### 🔑 The larger decision the measurement exposes — what does `base_currency` MEAN?

Today `base_currency=USD` is a **pair filter**: it selects `quote_asset_id =
USDC` and returns OHLC denominated in USDC. That is why 20,481 assets get an
empty `200` — they have no USDC leg to select.

Serving them requires reinterpreting the parameter as a **denomination**: return
this asset's candles expressed in USD, whatever leg they were traded against.
Those are not the same contract, and the choice cannot be made per-asset without
reintroducing the exact defect the warning above names — a response whose meaning
depends on data availability.

Two coherent positions, and this task must pick one explicitly:

- **`base_currency` = denomination.** `USD` means "priced in USD", sourced from
  `close_usd` (case 2) or the rate path (the self-pair). Consistent across every
  asset; matches what a caller asking for "USD" almost certainly wants; matches
  what [[0127]] AC 4's spot-check assumes. Cost: the returned OHLC is no longer
  literally the stored quote-asset candle, so
  [[ohlcv-returns-quote-asset-not-usd]] needs restating — and O/H/L are *derived*
  rather than measured (see the sketch).
- **`base_currency` = pair filter** (today's meaning). Then the honest answer for
  20,481 assets is **not** an empty `200` — it is a `503`/`404` saying that leg
  does not exist, and USD-denominated history moves to a different parameter or
  endpoint. Truthful, but it fails [[0127]] AC 3/AC 4 by design and leaves the
  reviewer's default request erroring on most of the store.

⚠️ **This decision is shared with [[0178]] and with what [[0120]]'s suite
asserts.** It is a public API contract on 20 k assets — it wants an ADR and the
0120 owner's agreement, not a unilateral call inside this task.

### Open: what to emit for a bucket whose `close_usd` is not yet filled

The ~1.8% residual above is enrichment lag on the most recent buckets, so a
window ending at `now` will have its right-hand edge unpriced. Three options —
**drop the bucket**, **emit the USD fields as null**, or **emit with a provenance
marker**. Dropping silently puts a hole at the right edge of every chart, which
is the worst of the three and is also the [[0144]] one-value-many-meanings shape.
Folds into the same provenance decision; not a separate thread.

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

**The dominant case, added 2026-08-25 — XLM-quoted assets in USD mode (20,481).**
Not covered by steps 1-5 above, which address the self-pair only.

6. `queries_ch::ohlcv` (`queries_ch.rs:557`) gains a second strategy. The
   blocking condition is `queries_ch.rs:560` —
   `vec!["asset_id = ?", "quote_asset_id = ?"]` — a hard `AND` on both legs with
   no fallback, orientation flip or conversion.
7. Derive the per-bucket rate **in-table** from the row itself:

   ```
   rate     = close_usd / close          -- implied USD per quote unit
   open_usd = open × rate                -- and high, low likewise
   ```

   `close_usd` is a column on `price_ohlcv_1m` and every rolled copy
   (`init.sql:115`); the current SELECT (`queries_ch.rs:583-596`) simply never
   reads it. ⚠️ Guard `close = 0` even though the 30-day sample shows none.
8. Column semantics under re-denomination — **not uniform**, do not map blindly:

   | column | under conversion |
   |---|---|
   | `open`/`high`/`low`/`close` | × rate; ordering preserved, so `high` stays the max |
   | `volume_base` | unchanged — base units do not move |
   | `volume_quote_usd` | **already USD**, whatever the quote leg |
   | `vwap` | × rate |
   | `trade_count` | unchanged |

   ⚠️ `volume_quote_usd` being already-USD means today's response *already* mixes
   a USD volume with quote-denominated OHLC. That inconsistency predates this
   task; decide it here rather than inheriting it.
9. ⚠️ **O/H/L are derived, not measured.** Scaling by a single per-bucket rate
   assumes the rate is constant within the bucket; the true USD high may fall at
   a different instant than the quote-denominated high. Defensible at `1d`,
   weaker at `1m`. State it in the response provenance and in [[0128]] — do not
   ship it as if it were a measured extreme.

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

Added 2026-08-25, for the population the measurement found:

- [ ] `base_currency`'s meaning (denomination vs pair filter) decided and
      recorded as an ADR, agreed with the [[0120]] owner — not chosen inside the
      implementation.
- [ ] A non-empty USD series for the five 0120 majors (`CBIJ…`, RON, EQL, BOL,
      AUD **with its issuer pinned** — 15 AUD issuers exist), through the
      deployed API.
- [ ] O/H/L derivation documented as *derived, not measured*, and carried in the
      response provenance rather than only in this file.
- [ ] `close = 0` guarded, with a test — the clean 30-day sample is not proof
      over all history.
- [ ] Behaviour defined and tested for an unpriced recent bucket (the ~1.8%
      enrichment-lag residual), and it is not a silent drop.
- [ ] The exotic-quoted dark population is **counted**, and stated as a known
      limit — the 26 figure covers XLM-quoted assets only.

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
