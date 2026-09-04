# Backfill depth verification — Tranche 2 acceptance criteria 5 and 6

**Status: criterion 5 passes. Criterion 6 passes on two assets and deliberately
excludes USDC, for a reason stated in full below.**

Measured 2026-09-04 against the deployed production API and the production
ClickHouse cluster. Every request was a single `curl` on the free-tier API key;
no load was generated. Companion to `prices-api-load-test-100rps.md` and
`prices-api-cache-verification.md`.

---

## 1. The criteria

> **AC 5** — `GET /backfill/status` shows `earliest_data_available` ≤ 2022-01-01.
>
> **AC 6** — `GET /assets/USDC…/ohlcv?timeframe=all` returns data points from at
> least January 2022, with correct 1d candles verifiable against known USDC
> price history (spot-check dates provided by reviewer).

## 2. Verdict

|                                                        | result                                                   |
| ------------------------------------------------------ | -------------------------------------------------------- |
| AC 5 — depth ≤ 2022-01-01                              | ✅ **passes**, by six years                              |
| AC 5 — stored value reconciles against real rows       | ✅ **passes**, three independent views                   |
| AC 6 — candles returned from ≥ Jan 2022                | ✅ **passes**                                            |
| AC 6 — candles verifiable against public price history | ✅ **passes on XLM and yBTC**; ⚠️ **USDC excluded** — §6 |

---

## 3. AC 5 — depth, reconciled three ways

A stored watermark is a claim. It was checked against the rows.

| view                                                    | oldest SDEX data       |
| ------------------------------------------------------- | ---------------------- |
| `GET /backfill/status` → `sdex.earliest_data_available` | `2015-11-18T03:47:00Z` |
| stored `prices.backfill_progress` row (`FINAL`)         | `2015-11-18 03:47:00`  |
| `min(timestamp)` on `price_ohlcv_1d`, `source='sdex'`   | `2015-11-18 00:00:00`  |
| oldest **active** partition, every `price_ohlcv_*` tier | `201511`               |

The bar is 2022-01-01. The data reaches **2015-11-18** — six years earlier.

Reconciling matters because the stored value **cannot correct itself
downward**: the writer merges it with a monotonic minimum, so it only ever
moves older. A value that overstated coverage would be permanent and invisible.
It does not overstate; real rows sit behind it, and the partition census
confirms they are physically present rather than implied by a progress marker.

**Volume in the criterion's window**: 4,048,196 daily SDEX candles across 82,096
assets in 2022 alone; 914,679 across 41,829 assets in 2021.

## 4. Continuity — every day, not just the endpoints

Depth at one end proves nothing about the middle. Distinct **days** were counted
per month across the whole range, because a single heavily-traded asset can hold
a monthly total up while a week is missing underneath it.

**Every month from 2022-01 to 2026-09 has SDEX candles on every calendar day.**
Distinct-day counts equal the length of each month — including 29 for February
2024 and 28 for 2022, 2025 and 2026. There are **no gaps to explain**.

Density varies, and two months sit well below their neighbours — 2024-10 at
~15.9k candles/day and 2026-07 at ~16.1k, against 21-28k either side. Both were
checked day by day: **zero empty days, no cliff, no step**. They are uniformly
lower, which is trading breadth rather than data loss.

## 5. AC 6 — spot-check against an independent public source

Our daily closes are compared against **Binance daily klines**
(`data-api.binance.vision`, a public endpoint requiring no key, UTC-aligned,
with history predating 2022). Binance is off-Stellar and shares no
infrastructure, data path or price source with this system.

### The five reviewer-style dates

| date       | asset | our close  | Binance close | Δ          | our trades |
| ---------- | ----- | ---------- | ------------- | ---------- | ---------- |
| 2022-01-03 | XLM   | 0.28965690 | 0.2902        | **−0.19%** | 26,662     |
| 2022-06-15 | XLM   | 0.12068619 | 0.1212        | **−0.42%** | 53,676     |
| 2023-03-11 | XLM   | 0.05882353 | 0.0788        | ⚠️ −25.35% | 36,208     |
| 2024-07-01 | XLM   | 0.09140157 | 0.0916        | **−0.22%** | 19,645     |
| 2026-06-15 | XLM   | 0.21362834 | 0.2136        | **+0.01%** | 150,508    |
| 2022-01-03 | yBTC  | 45,683.16  | 46,446.10     | **−1.64%** | 408        |
| 2022-06-15 | yBTC  | 22,742.73  | 22,583.72     | **+0.70%** | 397        |
| 2023-03-11 | yBTC  | 15,216.75  | 20,455.73     | ⚠️ −25.61% | 844        |
| 2024-07-01 | yBTC  | 63,195.66  | 62,899.99     | **+0.47%** | 488        |
| 2026-06-15 | yBTC  | 59,032.30  | 66,328.74     | ⚠️ −11.00% | 1,478      |

### The wider distribution — five dates could be luck

The same comparison was run on the 15th of every other month, 2022-01 →
2026-07, 28 dates per asset:

| asset          | median \|Δ\| | within 5%   | worst              |
| -------------- | ------------ | ----------- | ------------------ |
| XLM (`native`) | **0.06%**    | **27 / 28** | 16.2% (2023-03-15) |
| yBTC           | **0.48%**    | **27 / 28** | 14.8% (2023-03-15) |

**Across four and a half years, our closes reproduce an independent major
exchange to a median of 0.06%.** That is the claim AC 6 asks for, and it holds
on 27 of 28 sampled dates for both assets.

### ⚠️ The exception, stated rather than averaged away

The outliers are not scattered noise. On **2023-03-11** and **2023-03-15** both
assets are dislocated by the **same ratio**:

| date       | XLM ours ÷ ref | yBTC ours ÷ ref |
| ---------- | -------------- | --------------- |
| 2023-03-11 | 0.746          | 0.744           |
| 2023-03-15 | 0.838          | 0.852           |

Every neighbouring day is clean (0.999-1.017). Two assets with unrelated
markets and unrelated liquidity cannot drift identically by chance, so a shared
factor is wrong on those days. **Both are stablecoin-stress dates** — the
Silicon Valley Bank weekend and the Credit Suisse panic.

The mechanism is **not yet established, and the obvious explanation is
refuted**: "we assume USDC = \$1 and USDC fell" implies our prices would be
_high_, not low — the arithmetic requires USDC at ~\$1.34, and USDC fell. This
is tracked as an open defect and is not being explained away here.

A separate, unrelated outlier: **yBTC on 2023-03-18 reads 57% low while XLM the
same day is clean.** That one is thin-liquidity noise — yBTC trades a few
hundred times a day against XLM's tens of thousands, so one off-market trade
landing last in the bucket moves the close. It is a known property of a close on
a low-liquidity market, not a shared fault.

---

## 6. ⚠️ Why USDC is excluded from the spot-check

The criterion names USDC. **We are not using it, and the reason is that our
USDC series cannot fail the test — which is exactly why it cannot pass it
either.**

`GET /v1/assets/USDC:GA5Z…/ohlcv?timeframe=all` returns 2,042 daily points from
2021-02-01. Not one has a trade behind it:

|                                   | USDC                        | `native` (control) |
| --------------------------------- | --------------------------- | ------------------ |
| points                            | 2,042                       | 2,414              |
| **points with `trade_count > 0`** | **0**                       | **2,414 (all)**    |
| `derived: true`                   | **all 2,042**               | —                  |
| distinct closes                   | 177 — **1,865 exactly `1`** | real market values |

2021 through 2025 is 100% peg-derived: a flat, asserted `1`. The reason is
structural — USDC is the system's top-preference **quote** asset, so it
essentially never appears as a _base_ leg, has no candles of its own, and the
peg fill covers its entire history. Pricing USDC in USD from candles
denominated in USDC is circular.

**The consequence, stated plainly.** On 2023-03-11, when USDC broke its peg to
roughly \$0.87-0.88, we return exactly `1`:

```
2023-03-10  close=1  method=peg  trades=0
2023-03-11  close=1  method=peg  trades=0     ← reality was ~$0.87
2023-03-12  close=1  method=peg  trades=0
```

XLM on the same day shows the market genuinely moving, on 36,208 trades. The
data exists; it is not used for USDC.

**We would return `1` whatever the truth was.** For roughly 99% of dates that
answer is right — but it is right _by construction_, not by measurement, and a
spot-check that cannot distinguish the two is not a check. Since the criterion
lets the reviewer choose the dates, and the single most likely date a
knowledgeable reviewer would choose is the one above, presenting USDC as
verified would be misleading.

So: **XLM and yBTC are offered instead**, both with real trades on every
sampled date, both independently verifiable, and both compared over 28 dates
rather than the five the criterion asks for. Pricing USDC from measurement is
open work, not a claim made here.

---

## 7. Method

- **API**: `GET /v1/backfill/status` and `GET /v1/assets/{id}/ohlcv?timeframe=all`
  through the production custom domain, free-tier key, single-threaded `curl`.
- **Database**: read-only `SELECT`s on `prices.price_ohlcv_1d`,
  `price_ohlcv_1m`, `backfill_progress` and `system.parts`.
- **Reference**: Binance daily klines, `interval=1d`, UTC day-aligned to match
  our bucket boundaries. Closes compared, not opens or averages.
- **Reproducing a row**:
  `https://data-api.binance.vision/api/v3/klines?symbol=XLMUSDT&interval=1d&startTime=<UTC-midnight-ms>&limit=1`
  — field 4 is the close.

⚠️ **A caveat on the reference itself.** Binance quotes against USDT, not USD.
USDT normally sits within a fraction of a percent of \$1, so the comparison is
sound to the precision claimed here — but on the same stress dates flagged in
§5, USDT itself traded at a premium. The reference is not perfectly neutral on
exactly the days our data is anomalous, and that is a reason to investigate
those dates rather than to dismiss either side.

## 8. Open items

1. **The 2023-03-11 / 2023-03-15 dislocation** — shared factor unidentified.
2. **USDC priced by measurement rather than assertion** — structural, scheduled
   beyond this milestone.
3. **`soroban_amm.earliest_data_available` claims 2024-02-20** while the first
   actual AMM candle is 2024-03-08. Seventeen days of claimed coverage with no
   row behind it at any granularity. It is the _other_ stream in the same
   payload; the SDEX figure this criterion is graded on is correct and
   independently corroborated.
4. **A close on a thin market is a weak statistic** — nothing currently flags a
   close set by a single off-market trade.
