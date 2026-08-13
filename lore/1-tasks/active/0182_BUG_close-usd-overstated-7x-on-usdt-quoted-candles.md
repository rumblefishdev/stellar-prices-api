---
id: "0182"
title: "44,657 stored candles across 495 assets carry a close_usd ~7.4x too high — the USDT peg fix stops new ones but does not correct history"
type: BUG
status: active
related_adr: []
related_tasks: ["0172", "0196", "0165", "0145", "0111", "0114"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "enrichment", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Spawned from 0172. That task fixed the WRITER (USDT moved from the peg
      tier to the pivot tier, so new candles are priced at the measured rate),
      but every close_usd already written through the old peg path is still on
      disk and still ~7.4x too high. Filed separately because correcting it is a
      re-enrichment run with its own risk profile, not a code change.
  - date: 2026-08-13
    status: active
    who: okarcz
    note: >
      Activated. BE answered the two open consumer questions (see "BE answers"
      below): they filter close_usd > 0 and render "--" on a miss, so a zero is
      read as absent rather than as a zero valuation; and historical correctness
      DOES matter to them, for the 30D/1Y pool charts. They have not deployed
      yet, so no consumer has seen the inflated TVL. Advised them not to hold the
      deployment — the two are independent.
---

# `close_usd` is ~7.4× too high on every USDT-quoted candle ever written

## Measured on prod (2026-08-12)

```
price_ohlcv_1d WHERE quote_asset_id = 111 (canonical USDT):
  candles              44,657
  distinct base assets    495
  span                 2018-05-15 -> 2026-08-12   (i.e. still being written)
  priced (close_usd>0) 44,653
  implied USDT rate     0.999999   <- every one valued at par
```

The correct rate is USDT's measured market value — ~$0.13 in 2026-08, and
varying over time (see [[0172]] for the full monthly series). So each affected
`close_usd` is overstated by roughly `1 / 0.13 ≈ 7.4×`, with the exact factor
depending on the bucket's date.

## Why this is not fixed by 0172

0172 changed how candles are enriched **going forward**: `stable_ids()` no longer
contains USDT, and a second pivot pass prices USDT-quoted candles from the
measured USDT/USDC market. But enrichment only writes rows where
`close_usd = 0` — already-enriched rows are deliberately skipped (that filter is
what makes the pass idempotent and restartable). So the 44,657 wrong values are
inert: nothing will revisit them.

## 🛑 TWO ORDERING CONSTRAINTS — violating either makes this task destructive

Both surfaced in the 2026-08-13 review of 0172's PR #205. Neither is optional,
and neither is visible from inside this task's own plan.

### 1. `oracle_prices` must be purged FIRST ([[0196]])

The enrichment **oracle tier runs before the peg-pivot tier and wins where it
applies** (`ch_enrich.rs:19-22`). [[0196]] measured **46,378 mis-attributed
Reflector rows** on the USDT identity in `prices.oracle_prices`, covering
2026-03 → present and current to the hour.

So zeroing the rows and re-enriching *while those exist* re-applies
`close_usd = close × ~$1.00` to every 2026-03 → 2026-08 USDT-quoted candle and
labels it `method = 'oracle'` — a consumer reads that as **more** authoritative
than the peg placeholder it replaced. Same failure mode already recorded for
[[0168]].

### 2. Pre-2021 candles have NO pivot reference — do not zero them

The USDT/USDC market begins **2021-02-07** (2,011 daily candles). USDT-quoted
candles begin **2018-05-15**. The pivot's `ASOF LEFT JOIN` + `AND r.usd IS NOT
NULL` drops any candle with no reference at or before its bucket, so
**2018-05-15 → 2021-02-07 has nothing to pivot on**.

Those rows currently hold `close × $1` from the old peg path. Zero them and they
stay at `close_usd = 0` permanently — the ambiguous zero read unguarded by ~130
`argMax(close_usd, …)` sites ([[0145]]), which 0172's own rationale argues is
*worse* than a wrong-but-visible number.

⚠️ **And the old `$1` is CORRECT for that window.** 0172 measured USDT at par
from 2021-02 until the June 2022 break, and the depeg is what makes the peg
wrong *after* it — not before. So this is not "wrong data we cannot fix", it is
**right data this task would destroy**.

Options: bound the re-enrichment to ≥ 2021-02-07; or give the pivot a dated peg
epoch (par before the break, measured after). Decide before writing the driver.

⚠️ Also unresolved: `volume_quote_usd` is preserved write-once
(`if(p.volume_quote_usd > 0, …)`), so a row whose `close_usd` this task corrects
keeps a `volume_quote_usd` computed at $1 — the same row then carries two USD
columns that disagree by 7.4×. This task's scope is `close_usd` only; either
widen it or spawn a follow-up.

## ✅ BE answers 2026-08-13 — two open questions closed, one constraint relaxed

Asked BE how they consume `close_usd` and whether history matters. Their reply
settles decisions this task had been holding open.

**1. Nothing on prod has seen the inflated values yet.** BE have *not* deployed
the consuming changes. So this is not an incident with live blast radius — it is
a correctness debt with a window to fix it in. Priority is unchanged but the
urgency framing in [[0172]] ("102 priceable pools silently 7.4× high") describes
what *would* ship, not what has.

**2. A zero reads as ABSENT on their side, not as a zero valuation.** Verbatim:
*"for TVL we take the last `close_usd > 0` value from the last 48h, and if there
was no price for 48h we show an empty `--` TVL."*

⚠️ This **partially relaxes** the pre-2021 constraint above, but does not remove
it. The "ambiguous zero" argument ([[0145]], ~130 unguarded `argMax(close_usd,…)`
sites) is about **our own views**, not BE's client. BE is guarded; we are not.
So zeroing a row is safe *for BE* and still unsafe *for us*. Do not cite this
answer as clearance to zero the pre-2021 window.

**3. Missing hourly rows are harmless.** *"So missing hourly rows don't break
anything on our side."* Removes the pressure that shaped [[0165]]'s peg-fallback
rationale, for this consumer at least.

**4. History matters — for the charts, not the list view.** *"historical
correctness matters for the 30D/1Y charts, so this re-enrichment would be useful
so those USDT pool charts are correct and also show that drop."* The pool-list
TVL takes the last value only, so it has no historical dependency.

→ **This selects the scope.** The deepest surface BE reads is **1 year**. A 1Y
chart never reaches 2021-02-07, so **constraint 2's pre-2021 window is out of
scope for every consumer we know of.** Bound the run at `>= 2021-02-07` and
document the epoch boundary; the "dated peg epoch" option is unnecessary
complexity for a window nobody reads.

**5. They offered to hold their deployment. We declined.** Advised BE to ship:
the writer fix is live as of 2026-08-13, so their pool-list TVL (last
`close_usd > 0` within 48h) is correct the moment they deploy. Only the charts
read history, and that history is equally wrong whether they ship now or later.
The repair is additive and non-destructive, so it lands underneath a deployed
consumer with no coordination.

⚠️ **Open question sent back to BE, not yet answered:** do they read
`volume_quote_usd`? It is preserved write-once, so a row this task corrects keeps
a `volume_quote_usd` computed at $1 and the row then carries two USD columns
disagreeing by 7.4×. Their answer decides whether scope widens or the mismatch is
documented.

## Estimate given to BE 2026-08-13 — ~2-4 h of run, 2-4 working days end to end

Extrapolated from [[0114]]'s **measured** figures, not from first principles.

| | 0114 (measured) | 0182 (this task) |
|---|---|---|
| candidates / month, `_1h` | 2,788,693 | ~10k (44,657 in `_1d` over ~99 mo, ×24) |
| batches / month | ~101 | ~2-6 |
| wall clock / month | **6m56s** | ~30-60 s (est.) |

Scan cost is **per batch, not per row** (0114: `--batch-size` is the lever), so a
100× smaller candidate set does not run 100× faster — it runs however long a
handful of partition `FINAL` scans take.

**Scope: five tables, not six.** `price_ohlcv_1m` (7 day) and `_15m` (30 day) are
retention-bounded (`cleanup-worker/src/lib.rs:31-32`), so they hold no history to
correct. The forever tables are `1h/4h/1d/1w/1M`.

⚠️ Cleanup is currently **disabled** on prod, so `_1m`/`_15m` do physically hold
old rows today. Do **not** repair them — they are expired-by-design and will be
dropped when cleanup re-enables. Repairing them spends hours on rows with a
scheduled death.

**Span:** 2021-02-07 → today ≈ 67 months (per BE answer 4 above).
**Run:** 67 months × 5 tables at ~30-60 s ⇒ **~2-4 h**.

**Why the calendar figure is days, not hours** — the run is not the long pole:

1. **The [[0114]] driver will not pick these rows up.** `repair.rs:193` selects
   `(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`. Our rows are
   **wrong-but-nonzero**, so the driver's own enumeration reports them as clean.
   Needs a targeted zero/version-bump step first — small, but it needs a test and
   a dry run before it touches prod.
2. **Snapshots need a CH admin.** `prices_writer` cannot `FREEZE` and **cannot be
   granted it** — the user is XML-defined and that storage is readonly
   (`ACCESS_STORAGE_READONLY`, hit and documented in 0114). The operator takes all
   snapshots up front, then the tool runs `--skip-snapshot`. That is a
   coordination step, not a code step.
3. **`--skip-snapshot` prints a warning that is false under this path.** Verify
   `ls /var/lib/clickhouse/shadow/ | grep repair_` plus a non-trivial `du` before
   trusting it (0114).

## What needs deciding

- [x] **Scope — DECIDED 2026-08-13: the five forever tables, direct.** Repair
  `1h/4h/1d/1w/1M` each in place; skip `_1m`/`_15m` (retention-bounded, no history
  to correct — see the estimate section). **Not** "`_1m` + re-roll": the rollup MVs
  are `sum(version)`-based ([[0095]], [[0136]]) so a re-insert into `_1m` does not
  propagate, and `_1m` no longer holds the source rows anyway. This is the same
  shape 0114 already solved — repair each coarse table directly.
- [x] **Span — DECIDED 2026-08-13: `>= 2021-02-07`.** Per BE answer 4, the
  deepest consumer surface is a 1Y chart. Leave the pre-2021 window at its
  existing `close × $1` (which 0172 argues is *correct* for that era) and
  document the epoch boundary. Dated-peg-epoch option dropped as unnecessary.
- [ ] **Mechanism.** The [[0114]] `CoarseRepairDriver` is the right shape and
  should be reused, but see the gap below — it cannot see these rows as-is.
- [ ] **Zeroing first.** Enrichment skips `close_usd > 0`, so the rows must be
  reset to 0 (or written with a higher `version`) before the corrected pivot can
  fill them. On a `ReplacingMergeTree(version)` the version route is safer than a
  mutation; confirm against the [[0097]] pre-roll notes (RMT ties need
  DELETE-first).

### ⚠️ The 0114 driver gap — it reports these rows as CLEAN

`repair.rs:193` enumerates months with
`(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`. Every row this
task targets has `close_usd > 0` — wrong, but nonzero. So a `--dry-run` today
returns **"no months with enrichable zeros"** and looks like a green all-clear.

That is the same failure mode 0114 pre-registered for itself ("`enriched ≈ 0`,
`zeros_after` unmoved → the silent no-op"). Anyone reaching for the existing tool
without reading this will conclude the data is already fine.

The driver needs a **selection predicate** it does not currently have: rows whose
quote leg is the USDT identity, in the affected span. Zeroing those is what makes
them visible to the existing, tested enumeration — so the change is a targeted
pre-step, not a rewrite of the driver.

## Blast radius — read this before prioritising

⚠️ **Volume weight is NOT the impact measure.** Measured over 2026-05-01+, the
assets that actually *depend* on the USDT leg for their USD price are tiny:
`SPCXLM` ($12 total volume, 100% via USDT), `SCOP`/`RCC` ($0, 100%), `BAT`
(90.9%, $652), `LINK` (86.1%, $2,487), `BTC`-anchor (59.6%, $3,090), `MXN`
(36.8%, $4,022), `GYEN` (17.2%, $9,520). Everything with real volume — XLM
($470M), AQUA ($14.7M), SHX ($12.5M), XRP ($12.3M), yXLM ($10.3M) — draws ~0%
through the USDT leg, so **published prices for the major assets are unaffected**
(XLM measured at 0.0078% of weight).

**But BE values holdings, not flow.** A pool can hold a large position in an
asset that barely trades. 0172's opening recorded **106 pools with a USDT leg,
102 priceable** — every one of those positions is valued ~7.4× high regardless of
how thin the trading is. Prioritise on that, not on the volume table above.

## Acceptance Criteria

- [x] Decision recorded: **correct history from 2021-02-07 on**, leave the
      pre-2021 window at `close × $1` and document the epoch boundary. BE
      confirmed 2026-08-13 that history matters (30D/1Y charts) and that their
      deepest surface is 1Y, which never reaches the boundary.
- [ ] If correcting: the **five forever granularities** (`1h/4h/1d/1w/1M`)
      consistent afterwards, verified by re-running the `implied rate` probe
      (should move from ~1.0 to the measured per-bucket USDT rate).
      ⚠️ Not "all six" — `_1m`/`_15m` are retention-bounded and deliberately
      excluded.
- [ ] The `--dry-run` preview is **non-vacuous** — i.e. it reports months, not
      the "no enrichable zeros" all-clear the unmodified 0114 driver returns for
      these rows (see the driver gap above). Prove the tool can see the defect
      before trusting that it fixed it.
- [ ] Guard against re-introduction: the [[0172]] regression tests already pin
      the writer; add a data-level check that no USDT-quoted candle carries
      `close_usd / close ≈ 1.0`
- [ ] BE notified of the corrected values and the window affected
- [ ] `volume_quote_usd` resolved — widen scope, or document the two-column
      mismatch. Blocked on BE's answer to the question sent 2026-08-13.
