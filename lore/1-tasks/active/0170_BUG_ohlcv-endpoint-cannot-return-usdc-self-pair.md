---
id: "0170"
title: "GET /assets/{id}/ohlcv returns an empty 200 for 20,481 assets — base_currency filters the quote leg instead of denominating, and blocks 0127's M2 acceptance criterion"
type: BUG
status: active
related_adr: ["0011"]
related_tasks: ["0165", "0127", "0167", "0168", "0139", "0061", "0040", "0120", "0225", "0178"]
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
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Peg question settled by measurement, and it decides the `base_currency`
      fork. `close_usd / close` on USDC-quoted candles — the implied USD-per-USDC
      rate — **wobbles** (0.9976-1.0008 on ordinary days) with `exactly 1.0` a
      small minority of rows, so the rate is MEASURED and this task does not
      inherit [[0212]]'s hardcoded-peg defect. That also kills the "pair filter"
      half of the fork: keeping `base_currency=USD` as a USDC-leg filter means
      labelling USDC prices as USD, which is wrong by however far the peg has
      drifted — not merely unhelpful. Separately, exactly one row in 30 days
      falls outside ±1%: a non-canonical USDC (`GC4F4IX6DV`) at `close = 5e-14`,
      `close_usd = 4e-14`. That is 5 ticks over 4 ticks of the
      `Decimal(38, 14)` floor — quantisation, **not** [[0116]]'s absurd-large
      defect. 🔴 Consequence for the guard: a band check on the derived rate is
      insufficient, because a row quantising to ~1.02 would pass it and still be
      meaningless. The precondition must be on the INPUTS' precision. Two ACs
      added for it.
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Reviewed with the [[0120]] owner; denomination path AGREED, ADR to follow.
      Two evidence gaps they caught, both now closed by measurement. (1) The
      99.9% was a 30-DAY figure and `timeframe=all` reads all history — reconciled
      against [[0201]]'s 12,981,344 floor: both correct, different legs. 0201's
      no-reference floor IS the exotic-quoted population (13,102,543 dark rows
      today, within 1% a week later). Honest all-history coverage for the
      population this fix serves is **99.90% live / 98.08% pre-Soroban**, and the
      ~2% shortfall lands on [[0127]] AC 3. (2) The USDC peg check said nothing
      about USDT; checked both tiers, `exactly 1.0` is **0 rows** on every month
      of `_1d` and on `_1m`, implied rate ~0.13 — the real depegged value from
      [[0172]]/[[0182]]. So no dependency on [[0212]], though that is scoped to
      USDT-as-QUOTE and is not a verdict on 0212 itself. Also accepted: reuse
      0165's `traded`/`peg`/`oracle` vocabulary rather than coining a fourth
      word; fold [[0211]]'s boundary semantics into the same ADR; state the
      exotic 13.1 M as an explicit non-goal. 🔴 Sequencing REVERSED from the
      activation note: [[0178]] is blocked by PR #241 (same `current.sql`, same
      CTEs, same [[0095]] DROP+recreate hazard) and cannot start first.
  - date: 2026-08-26
    status: active
    who: okarcz
    note: >
      🔑 TITLE CORRECTED to match the scope. It described only the USDC self-pair
      while the task has covered the whole 20,481-asset population since
      2026-08-19. That mismatch has now caused the same defect to be spawned as a
      NEW task twice from the [[0120]] conformance runs — once on 08-19 (retired
      same day) and again on 08-25 as [[0225]]. A task list that shows the narrow
      case invites the general one to be filed as new; fixing the title is the
      actual remedy, not vigilance.
      [[0225]] reassigned here and activated as this fix's verification vehicle —
      it gets no separate implementation.
      [[ADR-0011]] §4 settled the last open decision: derived O/H/L rides as a
      separate flag, `method` keeps 0165's traded/peg/oracle. Implementation is
      no longer gated on a design question.
      ACs split below into the M2 gate and follow-on, so this can close on the
      evidence that matters rather than on all 19 at once.
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

⚠️ **The 99.9% above is a 30-DAY figure and must not be quoted as an all-history
coverage claim** — `timeframe=all` reads the whole table, which is [[0127]] AC 3.
Corrected all-history figures are in "Reconciled against [[0201]]" below. Caught
in review by the [[0120]] owner before it reached the ADR.

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

## Agreed in review — 2026-08-25

Reviewed with the [[0120]] owner, who owns the conformance suite this fix
changes. Path agreed: `base_currency` becomes a **denomination**. Their input,
and what was accepted:

1. **The peg argument is the load-bearing one, not the 20,481.** A measured
   0.9976-1.0008 rate under a field labelled `USD` is wrong for every asset that
   *does* get an answer today — so this is a defect, not a preference between two
   readings. Recorded because the first framing led with the population count and
   that framing is weaker.
2. **Reconcile against [[0201]] INSIDE the ADR, not after it.** Done above.
3. **Name the residue precisely.** The earlier caveat called it "low-value rows
   where the arithmetic isn't meaningful" — that is the precision-floor row, a
   different thing from the 13.1 M no-reference rows. A value-based guard cannot
   catch the latter, because value is not what is wrong with them. Two guards,
   two populations, stated separately.
4. **Reuse [[0165]]'s provenance vocabulary** — `traded` / `peg` / `oracle` — do
   not coin a fourth word for the same concept on a third endpoint.
5. **Fold [[0211]] in.** Window-boundary semantics (both ends inclusive) are
   measured but documented nowhere. If the ADR defines what a candle means, it
   settles that for free.
6. **[[0212]] checked, not assumed** — see the USDT arm above.

### Sequencing — corrected

- **0170 is not blocked.** No overlap with any in-flight work.
- 🔴 **[[0178]] IS blocked by PR #241** — same file (`current.sql`), same CTEs
  (`@31`, `@123`, `@144`, `@203` — `unfiltered` and `per_source`, which 0178
  restructures), same refreshable-MV DROP + recreate ([[0095]]'s hazard). 0178's
  apply cannot go before #241 lands. This reverses the order assumed when 0170
  was activated.

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

### Reconciled against [[0201]] — all history, by leg and era

Raised in review: [[0201]]'s recovery pass reports **12,981,344** `_1d` rows left
at the no-reference floor, which sits badly next to a 99.9% coverage claim. Both
figures are correct; they measure different legs. `price_ohlcv_1d` FINAL, all
history:

| leg | era | rows | with `close_usd` | pct |
|---|---|---|---|---|
| USDC | live | 542,484 | 542,285 | 99.96% |
| USDC | pre-soroban | 235,083 | 235,065 | 99.99% |
| **XLM** | **live** | **4,842,528** | **4,837,891** | **99.90%** |
| **XLM** | **pre-soroban** | **6,320,362** | **6,199,191** | **98.08%** |
| other | live | 9,491,899 | 17,027 | **0.18%** |
| other | pre-soroban | 3,655,518 | 27,847 | **0.76%** |

Dark rows total **13,228,568**, of which **13,102,543 are exotic-quoted** —
within 1% of 0201's 12,981,344, measured a week later and growing in the right
direction.

🔑 **0201's no-reference floor IS the exotic-quoted population.** Not a
contradiction with the coverage figure; a different leg. The ADR carries both,
labelled.

**The claim this task may make:** for the population 0170 serves — XLM-quoted
assets with no USDC leg — coverage is **99.90% live and 98.08% pre-Soroban**. The
~2% historical shortfall lands on `timeframe=all`, i.e. [[0127]] AC 3, and must
be stated there rather than rounded away.

⚠️ Aside, worth not tripping over: the 121,199 gap between 0201's floor and
today's exotic-dark is near-identical to XLM pre-soroban's 121,171 dark rows.
Almost certainly coincidence — do not build on it.

### The peg is MEASURED, not hardcoded — option A is depeg-safe

The conversion in step 7 is only honest if `close_usd` is real USD rather than
"USDC units assumed to be $1". Checked directly: `close_usd / close` on a
USDC-quoted candle **is** the implied USD-per-USDC rate.

`price_ohlcv_1d`, `quote_asset_id = 3`, last 30 days, by day:

| day | rows | min rate | max rate | exactly 1.0 |
|---|---|---|---|---|
| 2026-08-25 | 663 | 1.000000 | 1.000264 | 9 |
| 2026-08-24 | 716 | 0.997608 | 1.000263 | 39 |
| 2026-08-23 | 733 | **0.800000** | 1.000123 | 13 |
| 2026-08-22 | 817 | 0.996711 | 1.000187 | 18 |
| 2026-08-21 | 807 | 0.999982 | 1.000434 | 26 |

The rate **wobbles** — 0.9976 to 1.0008 on ordinary days — and `exactly 1.0` is a
small minority of rows, not the population. A hardcoded peg would show
`min = max = 1` with `exactly_one = rows` on every line. It does not.

**USDT-quoted rows — checked separately, after review flagged [[0212]].** The
USDC check filters `quote_asset_id = 3` and says nothing about the other peg leg.

| tier | window | rows | min rate | max rate | exactly 1.0 |
|---|---|---|---|---|---|
| `_1d` | 12 months | ~500-650/mo | 0.127 | 0.349 | **0 every month** |
| `_1m` | 5 days | 2,449 | 0.151 | 0.174 | **0** |

The implied USD-per-USDT sits around **0.13** — the real depegged value
established by [[0172]] and repaired by [[0182]] — not $1. `_1m` was checked
separately because [[0212]]'s title names that tier specifically and the tiers
have diverged before ([[0114]], [[0218]]).

🔑 So this task does **not** inherit [[0212]]'s hardcoded-peg defect on either
peg leg, and the conversion stays correct if USDC drifts. That was the live risk
in choosing option A and it is now closed by measurement.

⚠️ **Scope of that claim:** measured where USDT/USDC is the **quote** leg, which
is the population this fix converts through. It is **not** a verdict on 0212,
which may concern USDT-as-base or a span not covered here. Do not close 0212 off
this evidence.

### ⚠️ The guard must be on the INPUTS, not on the output rate

Exactly **one** row in 30 days falls outside ±1%. It is not a dust-price defect:

| code | issuer | close | close_usd | implied | volume_base | trades |
|---|---|---|---|---|---|---|
| USDC | `GC4F4IX6DV` | `0.00000000000005` | `0.00000000000004` | 0.8 | 1,460,498,063,318 | 4 |

`Decimal(38, 14)` makes `1e-14` the smallest representable increment, so these
are **5 ticks and 4 ticks of the last digit**. The 0.8 is `4/5` — pure
quantisation, not a price movement. The asset is a non-canonical "USDC"
(`GC4F4IX6DV`, not the canonical issuer) trading against real USDC at 5e-14: a
worthless lookalike.

**This is NOT [[0116]].** 0116 is absurdly *large* `close_usd` (up to $29.6 M);
this is the opposite end, where the values are too small for the arithmetic to
mean anything. Both corrupt a derived rate; they need different guards.

🔴 **A band check on the derived rate is insufficient.** It catches this row only
because `0.8` looks wrong. A row whose quantisation happened to land at `1.02`
would pass the band and produce a meaningless multiplier that looks plausible.
The guard must refuse to derive a rate when `close` (or `close_usd`) is within a
few ticks of the `Decimal(38, 14)` floor — a *precision* precondition, not an
*outlier* filter.

⚠️ And note what the fix changes about severity. Today a bad `close_usd` corrupts
one column that `/ohlcv` does not even return. Under option A it becomes the
multiplier for **open, high, low, close and vwap together**. The fix amplifies
existing bad rows rather than creating them, but the visible blast radius of one
bad row grows from a field to a whole candle.

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

Split 2026-08-26. **Gate** is what [[0127]]'s M2 criterion and [[0120]]'s re-run
actually require — this task closes when the gate is met. **Follow-on** is real
work that does not block the milestone; it moves to a spawned task at close
rather than holding this one open.

### Gate — blocks [[0127]] AC 3/AC 4 and [[0120]]

- [x] `GET /assets/{USDC}/ohlcv?timeframe=all` returns a non-empty 1d series
      spanning the backfilled range, verified **through the deployed API**, not
      only in a test.
      → **2,034 buckets, 2021-02-01 → 2026-08-27**, `HTTP 200`, through the
      production API on 2026-08-27 (see "Verified on prod" below). Took two
      deploys: the first shipped a query production refused.
- [x] The returned USDC/USD closes sit within a stated tolerance of ~$1, and the
      tolerance is justified (not asserted as exactly `1.0`).
      → **±0.5%**, measured. Over the **169** `oracle` buckets the extremes are
      `0.99947311646448` and `1.00110725843914` — a largest deviation of
      **0.1107%**, so the bound carries ~4.5× headroom. ⚠️ Stated over the
      MEASURED buckets only: the 1,865 `peg` buckets are exactly `1` *by
      construction* and cannot fail any tolerance, so including them would make
      the criterion vacuous.
- [x] `?base_currency=XLM` returns a non-empty, correctly-oriented series, with
      `volume_*` and `vwap` verified rather than assumed to invert.
      → **Derived, not inverted** (§6). `USDC_usd / XLM_usd` per bucket, so the
      inversion pitfalls the ADR warns about never arise: nothing is flipped, so
      there is no high↔low swap, no volume re-basing and no vwap re-weighting.
      Volumes are `0` for the same reason as the USD peg series — USDC is not
      traded as a base. Pinned by
      `ohlcv_usdc_in_xlm_is_derived_from_two_usd_rates`, seeded so the derived
      answer (3.9972) differs from what a naive inversion would give (4.0).
- [x] Fallback **semantics** tested, not the constant: no rate available → peg;
      rate available → that rate. A test asserting exactly `1.0` must not be the
      only coverage ([[0168]] would have to rewrite it).
      → `ohlcv_usdc_self_pair_is_synthesized_from_the_measured_rate` seeds a
      **moving** rate (0.9993 → 1.0007) and asserts the two buckets differ, so a
      hardcoded peg fails it. `ohlcv_usdc_before_any_observation_falls_back_to_a_labelled_peg`
      covers the other arm and asserts `method = "peg"`.
- [x] USDT behaves correctly too — it trades genuinely as a base in 102 pools, so
      confirm the synthetic path does **not** override real market data (the same
      trap [[0165]] documents).
      → The peg path keys on canonical USDC's identity alone, so USDT never
      enters it. `ohlcv_usdt_as_a_base_keeps_its_real_market_data` asserts a
      depegged 0.13 survives and is not synthesized at par.
- [x] ~~A non-peg asset's response is byte-identical to today's~~ — **RESTATED,
      not ticked as written.** See "AC restated — byte-identical" below.
      → Replaced by: **every numeric field is identical**, and the response
      differs only by the two additive provenance fields. Verified by
      `ohlcv_merges_sources_and_notes_backfill`, whose expected values are
      unchanged from the pre-0170 fixture.
- [ ] [[0127]] AC 3 + AC 4 re-run and passing.

- [x] Response carries provenance distinguishing a measured rate from a
      placeholder.
      → `method` is `oracle` for a measured `usd_rate` observation and `peg`
      where none exists at or before the bucket. Both pinned by tests.
- [x] `base_currency`'s meaning (denomination vs pair filter) decided and
      recorded as an ADR, agreed with the [[0120]] owner — not chosen inside the
      implementation.
      → **[[ADR-0011]] accepted 2026-08-25**, deciders okarcz + stkrolikiewicz.
      §4's open item (derived O/H/L provenance) settled 2026-08-26: a separate
      flag, `method` unchanged. No design question remains open.
- [x] A non-empty USD series for the five 0120 majors (`CBIJ…`, RON, EQL, BOL,
      AUD **with its issuer pinned** — 15 AUD issuers exist), through the
      deployed API.
      → All five non-empty on a 30-day window, every priced bucket labelled
      `traded`: `CBIJ…` 30, AUD (`GBBWRCJSZR…`) 23, RON 14, EQL 11, BOL 12.
      EQL carries **1 unpriced bucket** (2026-08-25, `volume_base = 1`,
      `trade_count = 2`) — the guard behaving as designed, a dust bucket
      returned present-but-price-less rather than dropped.
- [x] O/H/L derivation documented as *derived, not measured*, and carried in the
      response provenance rather than only in this file.
      → `Candle::derived` ships in every response, and its doc comment
      publishes to the OpenAPI document (verified out of
      `extract_openapi`: *"Whether `open`/`high`/`low`/`vwap` were **derived**
      rather than measured (ADR 0011 §3)"*). A separate axis from `method`, per
      §4.
- [x] `close = 0` guarded, with a test — the clean 30-day sample is not proof
      over all history.
      → `ohlcv_guards_a_zero_close`. Covered separately from `close_usd = 0`
      because they are different populations.
- [x] Rate derivation refuses to run when `close` or `close_usd` sits within a
      few ticks of the `Decimal(38, 14)` floor — a **precision precondition**,
      tested with the `GC4F4IX6DV` row's shape. A band check on the derived rate
      alone does not satisfy this.
      → `PRECISION_FLOOR` = 1e-12 (100 ticks) on **both inputs**, applied in
      `valid`. `ohlcv_refuses_to_derive_a_rate_from_values_at_the_decimal_floor`
      uses the measured shape — `close = 5e-14`, `close_usd = 4e-14`, implied
      rate **1.25**, an ordinary-looking number no band check could reject.
      ⚠️ The exact threshold is a judgement; the measurement establishes that a
      floor is needed, not that 100 ticks is the uniquely right line.
- [ ] [[0225]]'s acceptance criteria pass — it is this fix's consumer-facing
      verification and gets no separate implementation.
- [x] A test proves the conversion tracks a **moving** USDC rate, not a constant
      — the measurement below shows the rate genuinely wobbles, so asserting
      `1.0` would be asserting the wrong thing.
      → `ohlcv_usdc_self_pair_is_synthesized_from_the_measured_rate` seeds
      0.9993 → 1.0007 and asserts the two buckets **differ**, so a hardcoded peg
      fails rather than passes.
- [x] Provenance uses [[0165]]'s existing `traded` / `peg` / `oracle` values, not
      a new vocabulary.
      → Both paths map onto the trio. `usd_rate`'s `pivot`/`pivot2` fold into
      `traded` — a pivot is priced through a reference asset's own traded
      candles — so no fourth word is coined.

### Follow-on — real, but does not gate M2

⚠️ These move to a spawned task when the gate closes. Listing them here is not a
commitment to hold this task open for them; [[0222]] AC 5 is the precedent for
restating rather than silently carrying.

- [ ] Behaviour defined and tested for an unpriced recent bucket (the ~1.8%
      enrichment-lag residual), and it is not a silent drop.
- [ ] The exotic-quoted dark population is **counted**, and stated as a known
      limit — the 26 figure covers XLM-quoted assets only.
- [ ] The ADR states the all-history coverage per leg (99.90% live / 98.08%
      pre-Soroban for XLM-quoted) and names the exotic-quoted 13.1 M as an
      explicit non-goal — not a rounded-away caveat.
- [ ] [[0211]]'s window-boundary semantics settled in the same ADR.

## AC restated — "byte-identical" became unachievable, and had to

As written the criterion asked that a non-peg asset's response be **byte-identical**
to today's. That is now impossible by construction: [[ADR-0011]] §4 puts `method`
and `derived` on **every** candle, so every response gains two fields.

🔑 **The criterion was not wrong — it was overtaken by a decision made after it
was written.** Its intent, "prove no regression on the normal path", is intact
and met. What changed is that the ADR settled a provenance contract the AC
predates, and a response cannot both carry provenance and be byte-identical to
one that does not.

**Restated as:** a non-peg asset's response carries **identical values in every
field that existed before**, differing only by the additive `method` and
`derived`. That is verified by `ohlcv_merges_sources_and_notes_backfill` — its
expected numbers are unchanged from the pre-0170 fixture, which is exactly the
regression check the original wording was reaching for.

⚠️ The change is **additive**, so a consumer reading only the old fields is
unaffected. That is what makes the restatement honest rather than convenient: had
the shape changed in a way that broke existing readers, "no regression on the
normal path" would have been **false**, and the right move would have been to
fail the criterion rather than reword it.

Same pattern as [[0222]] AC 5 and [[0218]] AC 4: restate when a criterion turns
out to demand something impossible, and say so — quietly ticking it would not be
the same thing. Mirrored in [[0225]], which carries the same wording.

## ⚠️ Semantic change from PR #253's second review — the peg series is anchored on XLM/USDC

The synthesized USDC series originally took its buckets from **any** USDC-quoted
candle. It now takes them from the **XLM/USDC market specifically**, in both
denominations.

🔑 **Why: it was a full-table scan.** `price_ohlcv_*` is
`ORDER BY (asset_id, quote_asset_id, source, timestamp)`, so filtering on the
quote leg alone is **not a key prefix** — no granule pruning applies and the
query degenerated into a `FINAL` scan of every asset's candles in the covered
partitions (~24.9 M rows in `price_ohlcv_1d` alone, far more at finer grains) on
an endpoint with a p95 < 200 ms target ([[0121]]). `views.sql:370` already flags
this exact shape. Adding `asset_id` restores the prefix.

### ✅ The narrowing was MEASURED, and it costs 2 buckets in total

Anchoring on XLM/USDC means buckets exist only where that pair traded, not
wherever *any* asset traded against USDC. That was recorded as an unverified
judgement; it has now been measured on prod (2026-08-26), per year on
`price_ohlcv_1d`:

| year | any USDC quote | XLM/USDC | lost | covered |
|---|---|---|---|---|
| 2021 | 336 | 334 | **2** | 99.40% |
| 2022 | 365 | 365 | 0 | 100% |
| 2023 | 365 | 365 | 0 | 100% |
| 2024 | 366 | 366 | 0 | 100% |
| 2025 | 365 | 365 | 0 | 100% |
| 2026 | 238 | 238 | 0 | 100% |

**2 of 2,035 buckets — 99.90%.** Both are `2021-01-25` and `2021-01-26`,
consecutive days at the very start of the dataset, before XLM/USDC was
continuously liquid. From 2022 onward the two sets are **identical**.

🔑 The judgement holds: XLM/USDC is not merely the reference market by
reputation, it is a complete proxy for USDC-quoted activity across every year
that matters. The scan it replaces was ~24.9 M rows on a p95-bounded endpoint.

### ⚠️ KNOWN LIMIT — the USDC series starts 2021-02-01, not 2021-01-25

They *were* the earliest, and the calendar gap is wider than the bucket count
suggests:

```
first bucket, any USDC quote   2021-01-25
first bucket, XLM/USDC         2021-02-01
```

So between 01-25 and 02-01 only two days carried any USDC-quoted candle at all
(01-25 and 01-26), and XLM/USDC does not begin until 02-01. The synthesized USDC
series therefore starts **seven calendar days later** than the earliest
USDC-quoted market activity, losing **two** real buckets.

**Accepted, and stated rather than discovered.** The alternative is the ~24.9 M
row `FINAL` scan the anchoring exists to remove, on an endpoint with a p95
< 200 ms target — two buckets at the dawn of the dataset is not worth that.

🔑 **Carry this into AC 1's verification.** That criterion says the series must
span *"the backfilled range"*; on prod it will start 2021-02-01. That is the
correct answer under this design, not a shortfall to be explained away at
verification time — and [[ADR-0011]]'s coverage section should state it alongside
the per-leg figures it already carries.

## §6 peg-asset path — implemented 2026-08-26

The **original narrow defect**, and the one the main denomination change did not
touch: canonical USDC is never stored as a base leg, so `GET /assets/{USDC}/ohlcv`
matched zero rows. Dropping the quote filter does not help — there is nothing to
filter. The series is synthesized instead (`ohlcv_peg_series`).

- **Buckets** come from candles where USDC is the *quote*, so the series spans
  the whole backfilled range and every bucket is a period the market was open.
- **Value** is the newest `prices.usd_rate` observation **at or before** the
  bucket — [[0167]]'s stated rule for that table (observations + `ASOF`
  at-or-before, never an average), and the same shape enrichment's oracle tier
  uses, so read and write cannot drift.
- **Fallback** is $1 labelled `method = 'peg'` for buckets with no observation.
  `usd_rate` starts **2026-03-11** while `timeframe=all` reads back to 2021, so
  this is the majority of the real series, not an edge case.

⚠️ **This is the one place a literal `1.0` is correct.** §6 forbids a hardcoded
peg *where a measurement exists*; where none exists the peg IS the fallback and
`method` is what keeps the two apart. A response rendering both as the same
number would be [[0212]]'s defect in a new place.

`volume_base` and `trade_count` are `0` — USDC is not traded as a base, and
reporting its volume as a quote would answer a different question.

### ⚠️ Two ClickHouse traps hit while building it, both silent

1. **`join_use_nulls` is load-bearing.** By default an unmatched `LEFT JOIN` row
   yields the column's **DEFAULT, not NULL**, so `r.rate` came back `0` and
   `ifNull(..., 1)` never fired — rendering USDC at **$0.00** for every
   pre-observation bucket. That is the whole pre-2026-03 series. Caught only
   because the fallback test asserted the value rather than just the label.
2. **`ASOF JOIN` needs an equality alongside the inequality**, and there is no
   natural key here; both sides carry a constant `1 AS k`.

## 🔴 The first prod verification FAILED — `SETTINGS` is refused for the API's user

**2026-08-27.** Trap 1's fix — `SETTINGS join_use_nulls = 1` — is itself
inadmissible in production. The batch deploy of 08-27 shipped this endpoint, and
the first call through the deployed API answered **`500 db_error`**:

```
GET /v1/assets/USDC:GA5Z…/ohlcv?timeframe=all&granularity=1d  → 500
{"code":"db_error","message":"ohlcv peg series failed"}
```

`system.query_log` names it exactly:

| field | value |
|---|---|
| user | `prices_reader` |
| type | `ExceptionBeforeStart` |
| exception_code | **164** |
| exception | `Cannot modify 'join_use_nulls' setting in readonly mode. (READONLY)` |

`prices_reader` runs **read-only**, and a read-only user may not modify a
setting — so ClickHouse refused the query **before executing a row**. The Lambda
`REPORT` line corroborates: 277 ms total of which 234 ms was cold start, i.e. a
~40 ms round trip. It is the only query in the whole service that carried a
`SETTINGS` clause.

**Two diagnostic notes worth keeping.**

- The error surfaced as `bad response: ` with an **empty reason**. The client
  asks for compressed responses, so a plain-text error body decompresses to
  nothing and `clickhouse-rs` reports the status with no text
  (`response.rs:106-111`). An empty error message here means *"read the
  `query_log`"*, not *"the proxy answered"* — the first reading cost a wrong
  hypothesis (a repeat of [[0215]]'s Caddy timeout), killed by the 277 ms.
- **Every local test passed throughout**, because the local user is not
  read-only. This is the permissions twin of the version-pinning rule: matching
  the engine version is not enough if the *privileges* differ.

**The fix — a sentinel, not a setting.** `usd_rate.method` is
`LowCardinality(String)` (`init.sql:299`), so an unmatched ASOF row defaults it
to `''`, and no real row can carry one (every writer sets it; it is in the
table's ORDER BY key). `ifNull(r.meth, '') = ''` is therefore the no-match test,
and it holds under `join_use_nulls` **either way** — so the answer no longer
depends on a server default in either direction, which the `SETTINGS` version
could not claim. The clause is gone.

Rejected: asking BE for `readonly = 2` on `prices_reader`. It loosens a
read-only user for one query's convenience, it is an XML-managed user so it
needs their change and a restart, and it would leave us depending on a
permission we do not control.

**Guarded.** `ohlcv_peg_series_answers_for_a_readonly_user` runs the endpoint as
a `readonly = 1` user. Verified non-vacuous: restoring the `SETTINGS` clause
makes it fail with the exact prod symptom (`500`, `ohlcv peg series failed`)
while the other 17 tests in the file still pass — which is precisely why this
reached production.

## ✅ Verified on prod — 2026-08-27

Shipped in the 08-27 batch deploy (the API Lambda had not been deployed since
08-14), then re-deployed with the read-only fix above. `CodeSha256`
`bvrPfpYRehco5lL04rEm4xvYVIPvDtuLgYKOszIai3o=`, 15:02 local.

**The USDC self-pair — the defect this task is named for.**

| | |
|---|---|
| buckets | **2,034** (1d) |
| span | **2021-02-01 → 2026-08-27** |
| `oracle` / `peg` | **169 / 1,865** |
| measured range | `0.99947311646448` … `1.00110725843914` |
| largest deviation | **0.1107%** |
| `derived` | `true` on every bucket |

🔑 **The 169/1,865 split is the load-bearing number, not the 2,034.** `usd_rate`
begins 2026-03-11, and 03-11 → 08-27 is ~170 days: the boundary falls exactly
where the observations start. A fallback that silently won everywhere, or one
that never fired, would both still have produced a non-empty series — this is
what distinguishes "the ASOF works" from "the endpoint returns numbers".

**The five 0120 majors**, 30-day window, USD mode — the population the wider
reading of this task serves (20,481 assets), all previously an empty `200`:

| asset | buckets | methods |
|---|---|---|
| `CBIJBDNZNF…` (soroban) | 30 | 30 `traded` |
| AUD `GBBWRCJSZR…` | 23 | 23 `traded` |
| RON `GDE6EMCCVP…` | 14 | 14 `traded` |
| BOL `GDOV2XVGNQ…` | 12 | 12 `traded` |
| EQL `GBKIUHEKEC…` | 11 | 10 `traded` + **1 unpriced** |

`traded` throughout is the expected label: these are XLM-quoted assets
denominated from `close_usd`, so they never touch the peg path.

⚠️ **Two measurement traps worth keeping**, both of which produced a wrong
number before a right one:

1. A tolerance computed over the whole series is **vacuous** — 92% of the
   buckets are `peg`, exactly `1` by construction. Filter to `method = 'oracle'`
   first.
2. `jq`'s `//` yields *all truthy outputs of the left side* and only falls
   through when there are none, so `[.data[].method // "unpriced"]` silently
   **drops** null methods instead of relabelling them. EQL read as 10 methods
   over 11 buckets; the missing one was the unpriced bucket.

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
