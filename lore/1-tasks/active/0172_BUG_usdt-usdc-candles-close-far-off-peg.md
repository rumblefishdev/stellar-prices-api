---
id: "0172"
title: "USDT/USDC candles close at ~0.14 instead of ~1.00 — 891 days of real, high-volume trades at an impossible stablecoin price"
type: BUG
status: active
related_adr: []
related_tasks: ["0165", "0139", "0116", "0144", "0026"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "enrichment", "sdex", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Found while verifying the 0165 deploy on prod. USDT was 0165's control
      asset - the peg arm must NOT flatten it - and that check passed, but the
      values it returned are impossible. Not caused by 0165 and not changed by
      it: arm B contributes 0/0 wherever sum(w) > 0, so every one of these
      values is arithmetically identical to what the old view published. This is
      pre-existing and was simply never looked at.
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      CONFIRMED BY BE INDEPENDENTLY, and WIDER than this task was filed on. They
      hit it while re-measuring 0165: canonical USDT (GCQTGZQQ...TG6V) publishes
      method='traded' daily closes of 0.129-0.143 for 08-04 -> 08-10 in
      price_usd_series. So the defect is in USDT's OWN published identity series,
      not just the USDT/USDC pair - the 106-pool blast radius understates it, and
      any consumer reading USDT's USD price reads a wrong number.
      Their consumer-visible symptom is USDT FLAPPING between $0.14 and $1.00:
      traded buckets carry this defect while the newest bucket, where USDT does
      not trade as a base, takes 0165's peg fallback of $1. 0165 does NOT cause
      this and does not worsen it (arm B contributes 0/0 wherever sum(w) > 0, so
      every traded value is arithmetically identical to the old view) - it made a
      uniformly wrong column visibly discontinuous, which is a diagnostic
      improvement. Describe the symptom as flapping, but do not imply 0165
      introduced it.
      BE asked to bump priority; the tag is already priority-high and 0172 is
      already first in the queue, so what changed is the justification, not the
      rank.
  - date: 2026-08-12
    status: active
    who: okarcz
    note: >
      Activated. Picked up straight after 0137/0136 closed. Starting from the
      two open leads recorded below - writer-side asset_id assignment/
      renumbering, and whether the raw trades agree with the candle
      (volume_quote / volume_base vs close). The two falsified hypotheses
      (reader-side asset_id collision; XLM misattribution) are NOT to be re-run.
---

# USDT/USDC closes at ~0.14, not ~1.00

## Measured on prod (2026-08-10, after the 0165 apply)

```
price_usd_series, USDT @ canonical issuer:
  method=traded  rows=891  avg_close=0.1923

price_ohlcv_1d, asset_id 111 (= USDT), grouped by quote leg:
  quote=USDC  candles=891  avg_close_usd=0.1923  avg_close_native=0.192333
              min_usd=0.0912  max_usd=0.4915

by source:
  sdex  candles=891  trades=132,685  total_volume_base=491,984.77
```

Two dollar-pegged assets cannot trade at 0.09–0.49 against each other. A
stablecoin pair should sit at 1.0000 ± a fraction of a percent.

`close_usd ≈ close_native` is **correct** and not the bug — a USDC-quoted candle
takes the stablecoin-direct tier, so `close_usd = close × $1`. The defect is in
`close` itself.

## Four hypotheses tested and FALSIFIED — do not re-run them

1. **[[0139]] `asset_id` collision.** ❌ `asset_id = 111` resolves to exactly one
   natural identity (USDT @ `GCQTGZQQ…TG6V`). No sharing.
   *(Separately confirmed while checking: 0139 is real and wide — **3,281
   `asset_id`s serve 6,568 identities**. That number appears nowhere in 0139 and
   should be carried there.)*
2. **The rows are XLM's prices misattributed.** ❌ `usdt_close` 0.142572 vs
   `xlm_close` 0.163999 on 2026-08-10 — close in magnitude, never equal.
   ⚠️ When re-testing this, note `price_ohlcv_1d` holds **one row per source**,
   so a naive join to XLM/USDC fans out ~4× per day. Aggregate or filter source.
   ⚠️ **Re-tested 2026-08-12 across the full series, not one day**: 2,011 joined
   days give `corr = -0.3157`, `avg_ratio = 1.4233`, `ratio_sd = 1.5704`. USDT's
   series does not track XLM's in level *or* direction. This is now closed for
   good — a leg swap against XLM cannot produce these numbers.
3. **Writer-side `asset_id` renumbering.** ❌ Every USDT/USDC/XLM natural key in
   `prices.assets` shows `uniqExact(asset_id) = 1` across every version ever
   written; `asset_id = 111` has only ever meant USDT @ `GCQTGZQQ…TG6V`. This
   mattered because the RMT sorts on `(asset_code, issuer_address,
   contract_address)` with `asset_id` **outside the key**, so a rewrite was
   mechanically possible — it just never happened.
   ⚠️ **`created_at` cannot date an identity.** The hourly re-emit rewrites the
   whole row with `DEFAULT now()`, so every asset — XLM included — reports
   `created_at` = today. Do not read it as first-seen (see [[0132]]).
4. **A copycat USDC as the quote leg.** ❌ `prices.assets` holds ~220 distinct
   `USDC` issuers and ~220 distinct `USDT` issuers, so "quoted in USDC" was
   ambiguous. Measured: the quote leg is `asset_id = 3` = canonical Circle USDC
   (`GA5ZSEJY…KZVN`) on every candle in the window. Both legs are canonical.

## The candle is a faithful aggregate — the inputs are already ~0.13

Measured 2026-08-04 → 08-10 on `price_ohlcv_1m FINAL`, per day, one source
(`sdex`) and one quote leg (`3` = Circle USDC):

```
d           candles trades vol_base vol_quote implied_vwap day_close  stored_vwap
2026-08-04     29     36    20.3347   2.6305     0.129359   0.129551    0.129417
2026-08-06     60     65    60.9380   7.8935     0.129533   0.129313    0.129741
2026-08-08     40     44    47.0042   6.4182     0.136546   0.142668    0.136189
2026-08-10     41     48    35.4849   5.0411     0.142064   0.140961    0.142117
```

`volume_quote / volume_base` agrees with `close` and with the stored `vwap` to
~3 decimal places on every day. **The aggregation is not at fault** — OHLC,
VWAP and the volume columns are mutually consistent. Whatever produced these
candles was already seeing ~0.13, so the defect (if any) is upstream of the
candle, or is not a defect at all.

Also note the price is not pinned: it drifts 0.1294 → 0.1420 (+9.8%) over seven
days, with intraday `lo`/`hi` spreads of ~1%. That is the shape of a real thin
market, not a constant scaling factor. `1/0.129 = 7.75` is not a power of ten,
so the 7-decimal fixed-point scaling theory does not fit either.

## ⚠️ CORRECTION: it IS thin. The "actively traded" premise was wrong.

This task originally argued 891 gapless candles / 132,685 trades / ~492k base
volume proved a real market and excluded [[0116]]. Those are **891-day totals**,
which work out to ~149 trades and ~552 USDT per day. The measured week is
thinner still: **15–61 USDT of base volume per day across 36–65 trades**, i.e.
roughly 0.5–1.5 USDT per trade.

A market clearing ~$20–60/day in sub-$2 lots is dust by any definition, so
[[0116]] is back on the table and the original section here is retracted.

## The hypothesis this now points at

If both legs are canonical, the aggregation is faithful, and the market is
genuinely thin, then the remaining reading is that **~$0.13 is what this token
actually trades at** — a thinly-traded wrapped/bridged "USDT" that is not
redeemable at par. Under that reading the traded value is *correct* and the
defect is the **peg assumption**: [[0165]]'s fallback publishes `$1` for the
newest bucket because the asset does not trade as a base there, so a consumer
sees $0.14 (measured) next to $1.00 (assumed) and reports flapping.

That would make 0172 a **pricing-policy** bug, not an ingestion bug, and would
mean the fix is in how we decide something is a dollar peg — not in the candles.

## ✅ ROOT CAUSE CONFIRMED 2026-08-12 — the candles are correct

`price_ohlcv_1d` for `asset_id = 111`, monthly, full history:

| Period | avg_close | Character |
|---|---|---|
| 2021-02 → 2022-04 (15 mo) | **0.975 – 1.014** | at par; monthly `lo`/`hi` inside ~1% |
| 2022-05 | 0.975 (lo 0.832) | peg slipping |
| 2022-06 | **0.684** (lo 0.265) | collapse |
| 2022-07 → 2026-08 | 0.29 → 0.13 | never returns to par |

**The same pipeline, the same `asset_id`s, the same code path printed ~1.0000
for fifteen months.** No ingestion defect — scaling, leg swap, orientation, or
mis-assigned id — can be correct for 2021 and 2022Q1 and then wrong from June
2022 while continuing to track a coherent, drifting price. The par period is a
self-controlled experiment and it exonerates the candles.

The break is May–June 2022 = Terra/UST + Celsius/3AC. A bridged or wrapped USDT
losing backing in that window is an ordinary event.

**Root cause: `USDT` @ `GCQTGZQQ…TG6V` genuinely depegged in June 2022 and has
traded at a deep discount ever since. `close ≈ 0.13` is the correct market
value. The defect is that we classify this identity as a dollar peg.**

### Corrected headline figures (the ones in this task's opening were wrong)

Not 891 candles / 132,685 trades / ~492k volume. Measured on the full series:
**2,011 gapless daily candles from 2021-02-07, 1,019,898 trades, 14,071,740
base volume**, one source (`sdex`), and USDC (id 3) as the *only* quote leg in
all of history — there is no reverse USDC/USDT pair and no XLM-legged USDT
candle, so **no in-house cross-check for this asset exists**.

### Blast radius: exactly one identity

`views.sql:339-340` pins the peg set by code **AND** issuer, so the ~219
copycat `USDT` issuers and ~219 copycat `USDC` issuers are *not* pegged:

```sql
AND ((q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJY…KZVN')
  OR (q.asset_code = 'USDT' AND q.issuer_address = 'GCQTGZQQ…TG6V'))
```

Only USDT @ `GCQTGZQQ…TG6V` is affected. (Canonical USDC's own par status is
still unverified — worth one query before closing.)

### ⚠️ TRAP: [[0168]] will NOT fix this

`views.sql:76-83` says the `1` is a placeholder that 0168 replaces with the
Reflector rate from `prices.usd_rate` ([[0167]], live). But **Reflector's USDT
feed prices real Tether, which is at par** — 0167 measured `avg ≈ 1.000271 /
0.99959`. Swapping the constant `1` for a measured `1.0003` changes nothing
here: the wrapper trades at 0.13, not 1.00.

So 0168 would close the "flat $1 is a ~0.1% systematic error" complaint and
leave 0172's 7× error fully intact — while *looking* like the fix, because the
`method` column would read `oracle` instead of `peg`. **0168 must not be
recorded as resolving 0172.** Which feed refers to which issuer is [[0173]].

### CONFIRMED 2026-08-12: the oracle is mis-attributed, and 0168 is dangerous

`prices.usd_rate` (Reflector, via [[0167]]) vs our own candles, same months:

| Month | `usd_rate` (oracle) | `close` (our market) | Ratio |
|---|---|---|---|
| 2026-07 | 0.999267 | 0.132027 | **7.57×** |
| 2026-08 | 0.999232 | 0.134087 | **7.45×** |

Reflector quotes the **ticker** "USDT" — real Tether, at par. It has no notion
of this particular Stellar IOU. But we store that rate keyed on
`(asset_code='USDT', issuer_address='GCQTGZQQ…TG6V')`, so **`usd_rate` already
asserts a wrong price for this identity.** That is a defect in 0167's output,
not only in the view.

⚠️ Consequence: [[0168]] would import the mis-attributed rate into
`price_usd_series` and label it `method = 'oracle'` — which a consumer reads as
*more* authoritative than the `peg` placeholder it replaces. **0168 must not
ship for this identity while 0172 is open.** Concrete instance of [[0173]]:
the feed is keyed to a ticker, not to an issuer.

### USDC verified at par — the peg set has exactly one bad member

`usd_rate` for canonical USDC, 2026-03 → 2026-08: monthly avg 1.000086 –
1.000639, never outside ±0.0015. The placeholder is correct for USDC.

⚠️ **USDC's par status cannot be checked from our candles** — it is our
top-preference quote, so it never appears as a base and
`price_ohlcv_1d WHERE asset_id = 3` returns **zero** rows by construction
(views.sql:62-64, the 0165 finding). Use `usd_rate`, not the candles. A query
returning empty here is the correct answer, not a missing-data problem.

### Why the fix is asymmetric (the actual argument for the one-liner)

|  | Trades as a base? | What the `$1` placeholder does |
|---|---|---|
| **USDC** (id 3) | Never — 0 candles, by design | The *only* way USDC gets a price. Removing it re-breaks [[0165]]. |
| **USDT** (id 111) | Yes — 2,011 candles, effectively gapless since 2021-02-07 | Adds nothing; injects a wrong $1.00 on non-trading days. |

The mechanism is load-bearing for one member and purely harmful for the other.
Because USDT has a traded value on essentially every day of its history, the
usual cost of dropping a fallback — buckets that publish nothing — is close to
zero here.

### CHALLENGED AND RE-CONFIRMED 2026-08-12 — three independent controls

The first pass argued "same pipeline printed 1.0000 for 15 months". That was
**under-verified**: this table was filled by several backfill passes (0088,
0090, 0097), so a pass boundary near June 2022 would have made the "self
control" no control at all. Challenged on exactly that. Three tests:

**T1 — sibling stablecoins (decisive).** Every asset quoted in USDC that sat
within ±5% of $1 before 2022-05, compared before vs after:

| Asset | before | after | ratio |
|---|---|---|---|
| `yUSDC` (32) | 0.998606 | 0.999466 | **1.0009** |
| `USDCAllow` (741) | 1.000000 | 1.000000 | **1.0000** |
| `USD` (673) | 0.981653 | 0.987319 | **1.0058** |
| `USDS` (179) | 0.992363 | 1.000313 | **1.0080** |
| `USD` (454) | 0.974358 | 0.911359 | 0.9353 |
| `SLVR` (36) | 1.004522 | 0.716161 | 0.7129 (silver token, real move) |
| **`USDT` (111)** | **1.000813** | **0.257686** | **0.2575** |

Four pegs held par to within 1% through the same window, same quote leg, same
code path. A `close`-computation defect cannot spare four and hit one.

**T2 — no backfill seam.** `version` = ledger_seq × 1000 + order, and it runs
monotonic and continuous through the break: 40.69B (Apr) → 41.12B (May) →
41.56B (Jun) → 42.01B (Jul). No discontinuity. The par period and the broken
period were written by the same pass. The hole in the original argument is
closed.

**T2b — the volume columns move too.** `sum(volume_quote)/sum(volume_base)`
is computed from columns independent of OHLC and collapses in step: 0.980
(May) → 0.924 (Jun) → 0.349 (Jul) → 0.221 (2023-03). Had only `close` been
miscomputed, this would have stayed at 1.0.

**T2c — liquidity left and never returned.** `trade_count`: 140,945 (2022-03)
→ 62,934 (05) → 36,338 (06) → **5,087 (07)** → **805 (2022-10)**. A
calculation bug does not empty an order book. This is the signature of an
issuer failing.

**T3 — XLM test, re-run correctly.** The original `corr = −0.3157` over all
2,011 days was **contaminated** by the 15-month par period and should not have
been cited as proof. Re-run on the post-break window only (1,473 days):
`corr = −0.5622`, `avg_ratio 0.654614`, `sd 0.516475`. Still not XLM — now
properly established rather than accidentally right.

**Why the oracle disagrees, and why it is not evidence against this.**
Reflector prices the **ticker** "USDT" = Tether the company's token, genuinely
at par. It has no knowledge of a Stellar IOU that reuses those three letters.
We then store that rate under this issuer's address. The oracle is correct
about Tether; **our symbol→issuer mapping is the error** ([[0173]]).

### Correction plan for history

**`price_ohlcv_*.close` — leave as-is.** The 2,011 USDT candles are correct.

⚠️ **`close_usd` — NOT correct, and this reverses an earlier conclusion in this
file.** An earlier pass here recorded "leave history alone" after looking only at
asset 111's own `close`. But `close_usd` is *stored*, written by the enrichment
peg tier, and wrong for **44,657 candles across 495 base assets** (2018-05-15 →
today) that are quoted in USDT — all valued at par (`implied rate = 0.999999`).
Those need re-enrichment; tracked separately.

## Implementation (2026-08-12)

Branch `fix/0172_usdt-depeg-remove-from-peg-set`. Four sites held the $1
assumption, not one:

1. **`views.sql`** ×2 (`price_usd_series`, `price_usd_series_1h`) — USDT removed
   from the arm-B peg predicate. USDC stays: it is the only member that cannot be
   priced as a base.
2. **`ch_enrich.rs`** — `ReferenceIds::stable_ids()` is now USDC-only; new
   `pivot_ids()` returns `[xlm, usdt]` and `enrich_peg_pivot_step` runs one pivot
   pass per reference asset. `pivot_sql`'s `xlm_id` param renamed `ref_id` — the
   function was **already generic**, only its name said XLM, so this reuses
   tested machinery rather than adding a third tier.
3. **`oracle-worker::peg_identities()`** — USDT removed, stopping new
   mis-attributed `usd_rate` rows. Existing rows still wrong (see below).
4. **`canonical.rs::is_preferred_quote`** — **deliberately NOT changed.** USDT is
   still quote-preference rank 1, so pairs keep being canonicalised into it.
   Changing that alters orientation for every historical pair; its own decision.

### Why pivot instead of simply deleting USDT from the peg set

Deleting it alone would leave those 44,657 candles at `close_usd = 0`, which in
this schema is ambiguous (missing / genuinely zero / not-yet-enriched) and is
read unguarded by ~130 `argMax(close_usd, …)` sites — trading a wrong-but-visible
number for a silent one. The pivot prices them correctly instead.

### Tests

- `ch_enrich.rs` unit: `peg_sql_never_pegs_usdt`,
  `pivot_sql_prices_usdt_quoted_candles_from_its_usdc_market`,
  `reference_ids_helpers` (rewritten).
- `ch_enrich_it.rs`: `usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg`
  — end-to-end on the 26.3.10.60 pin. FOO/USDT at 10.0 with USDT/USDC at 0.13
  must yield **1.3**, not 10.0. **Verified non-vacuous**: restoring the bug makes
  it fail with `got 10`. Also asserts `> 0` so "delete the peg, add no pivot"
  cannot pass.
- `views_it.rs`: `usdt_quote_only_gets_no_peg_fallback_but_usdc_still_does` —
  pins both halves; asserting only "USDT gets nothing" would also pass if someone
  deleted arm B entirely and re-broke [[0165]].

All green: workspace unit tests, 11 enrichment ITs, 7 view ITs, clippy clean.

### ⚠️ Pre-existing hazard found while fixing the tests (needs its own task)

`peg_asset_with_only_zero_volume_candles_falls_back_instead_of_publishing_garbage`
used USDT as its peg subject, so it broke here. Porting it to USDC exposed that
its own doc comment is **wrong**: it claims fixture A (an asset that is only a
zero-volume base, with no placeholder) "publishes Decimal128::MIN". On the prod
pin it does not — `nullIf(sum(w), 0)` yields NULL, the `CAST` to non-Nullable
`Decimal(38,14)` fails, and the query **raises `CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN`
(code 349)**. That does not corrupt one row: it fails `price_usd_series` for
every row in the result.

Arm A filters only `p.close_usd > 0`, never `volume_base > 0`, so any
priced-but-zero-volume bucket can reach it. Pre-existing and not caused by this
change — but removing USDT from the peg set removes USDT's accidental protection,
so it is worth confirming on prod that USDT has no zero-volume-only buckets.

## Where to look next

- **Is `asset_id 111` really the asset the writer meant?** The reader-side
  collision is ruled out; the writer side is not. Check whether the USDT
  identity has ever been assigned more than one `asset_id`, and whether ids were
  renumbered at any point (historical candles would then point at the wrong
  asset without any duplicate appearing in `assets` today):
  ```sql
  SELECT asset_id, asset_code, issuer_address, contract_address, asset_type
  FROM prices.assets FINAL WHERE asset_code = 'USDT' ORDER BY asset_id;
  ```
- **Does the raw trade agree with the candle?** Pull one day's underlying trades
  and check `volume_quote / volume_base` against `close`. If they agree, the
  candle is a faithful aggregate of trades that are themselves wrong (an
  ingestion/parse issue — amount scaling, or a leg swap at extract time). If
  they disagree, the aggregation is at fault.
- **Is the pair orientation right?** `1 / 0.1426 = 7.01`, which is not 1 either,
  so a simple base/quote inversion does **not** explain it — but a scaling error
  (Stellar amounts are 7-decimal fixed point) is worth checking against a known
  trade.
- **Check other USDT pairs.** The prod query showed USDC as the *only* quote leg
  for id 111. If USDT trades against XLM anywhere, its implied USD price via
  that leg is an independent cross-check.

## 🔴 CONFIRMED BY BE 2026-08-11 — and it is WIDER than this task was filed on

BE hit this independently while re-measuring [[0165]], and their reading
**escalates the defect on two axes**:

1. **It is not the `USDT/USDC` pair — it is USDT's own published identity
   series.** They observe canonical USDT (`GCQTGZQQ…TG6V`) publishing
   `method = 'traded'` daily closes of **0.129–0.143 for 08-04 → 08-10** in
   `price_usd_series` — the surface they consume directly. This task was filed
   from the pair; the blast radius is the **asset**. In their words: *"0172 is
   not 'distortion on those pairs' but a wrong published price for USDT itself;
   you may want to bump its priority accordingly."*
2. **A consumer now sees USDT flapping between $0.14 and $1.00.** The traded
   buckets carry this defect's value while the newest bucket — where USDT does
   not trade as a base — takes [[0165]]'s peg fallback of `$1`.

⚠️ **0165 does not cause the flapping and does not make the data worse.** Arm B
contributes `0/0` wherever `sum(w) > 0`, so every traded value is arithmetically
identical to what the old view published. What 0165 changed is **visibility**: a
correct `$1` now sits next to a wrong `$0.14` in one series, so a uniformly
wrong column became a visibly discontinuous one. Describe the symptom as
*flapping* rather than as a quiet 7× understatement — that is what a consumer
now reports — but do not let the framing imply 0165 introduced it.

**On BE's "bump its priority":** the tag is *already* `priority-high` and 0172 is
*already* first in the recorded queue (`0172 → 0170 → 0168 → 0127 → 0128`), so
there is no re-tagging to do — what their message changes is the **justification
and the framing**, not the rank. Worth saying plainly when replying, so they
know it is next rather than merely re-labelled.

## Blast radius

`close_usd` is what BE multiplies into TVL. If USDT positions are valued at
~$0.14 instead of ~$1.00, every pool with a USDT leg is understated ~7×. BE's
CSV counts **106 pools with a USDT leg, 102 of them priceable** — so these are
pools that look healthy and are silently wrong, which is worse than the
never-priced pools [[0165]] fixed.

⚠️ **Update 2026-08-11:** the 106-pool figure now *understates* it, because the
defect is in the asset's own series (above), not only in pools whose two legs
are the affected pair. Any consumer reading USDT's USD price — pool-valuing or
not — reads a wrong number.

⚠️ It also means **USDT is not a trustworthy control** for peg-related work until
this is resolved — 0165 used it as one.

## Spawned tasks

- **[[0182]]** — re-enrich the 44,657 stored `close_usd` values (the writer is
  fixed; history is not)
- **[[0183]]** — `prices.usd_rate` rows that file Tether's price under this
  issuer's identity
- **[[0184]]** — should a depegged asset still be quote-preference rank 1?
  (`canonical.rs`, deliberately untouched here)
- **[[0185]]** — `price_usd_series` *raises* on a zero-weight group; the existing
  code comment describing that case is wrong about the failure mode
- **[[0168]]** — hold note added: it must not be recorded as resolving this task

## Acceptance Criteria

- [ ] Root cause identified and stated (writer-side id, ingestion scaling, leg
      swap, or something else), with the falsified hypotheses above left on the
      record so they are not re-run.
- [ ] Whether the defect is USDT-specific or a class affecting other assets —
      a sweep for assets whose `close_usd` is implausible for their type.
- [ ] Correction plan for the 891 existing daily candles (and their 1h/coarse
      counterparts), or an explicit decision to leave history as-is.
- [ ] Regression test on the 26.3.10.60 pin.
- [ ] BE notified if any published TVL was affected.
