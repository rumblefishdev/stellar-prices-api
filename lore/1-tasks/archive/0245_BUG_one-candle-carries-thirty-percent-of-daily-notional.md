---
id: "0245"
title: "A single 1m candle carries 30% of the network's 24h notional — $13.4M of $44.2M in one row"
type: BUG
status: completed
related_adr: []
related_tasks: ["0178", "0242", "0123", "0114"]
tags:
  [
    "priority-medium",
    "effort-medium",
    "data-correctness",
    "clickhouse",
    "ingest",
    "enrichment",
  ]
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Found while sanity-checking [[0178]]'s both-legs volume change against
      prod. Kept out of that task deliberately: 0178 only re-attributed rows
      that were already in the table, so this predates it and is unaffected by
      it either way.
  - date: "2026-09-28"
    status: active
    who: akot
    note: >
      Activated. Time-sensitive: 0286 phase 3 will drop and re-ingest the
      202608 1m partition, so the row must be identified (read-only prod
      SELECTs) before that happens. Not absorbed by 0286 — it changes
      prices, not volume.
  - date: "2026-09-28"
    status: completed
    who: akot
    note: >
      Not a data defect. The row is a real SDEX fill: USDCAllow → USDC at
      exactly 1:1 against Circle's USDC issuer's standing offer, matching
      Horizon to the unit. The pattern repeats every weekday (93% of network
      1m USD volume 2026-08-20 → 09-27). Kept as is by Adam's decision —
      StellarExpert, Horizon and SDF's Hubble all count it the same way. No
      follow-up task.
---

# One candle, 30% of the day

## Measured on prod — 2026-08-31

```
total notional, trailing 24h   44,217,399.69
largest single 1m candle       13,414,181.36   ← 30.3% of the day
```

One minute-candle carries nearly a third of the entire network's daily traded
value. Either it is real — a single very large swap — or `volume_quote_usd` is
wrong on that row.

## Why it matters now

[[0178]] made `volume_24h_usd` count both legs, which is correct, but it also
means any inflated row is now attributed to **two** assets instead of one.
Canonical USDC's headline figure (~$44M, 99.6% of network notional) leans
heavily on this single candle. A wrong row is now twice as visible.

## Where to start

- Identify the row: `asset_id`, `quote_asset_id`, `source`, `timestamp`,
  `volume_base`, `volume_quote`, `volume_quote_usd`, `trade_count`.
- `volume_quote_usd` is computed at enrichment time as the quote leg's amount ×
  that asset's USD rate. Check both factors: a plausible `volume_quote` with an
  implausible rate is a different defect from an implausible `volume_quote`.
- Compare against the same pool/pair's neighbouring minutes. A genuine whale
  swap is isolated; a units or decimals error usually repeats.
- ⚠️ Check whether the identity is one of [[0242]]'s SAC duplicates before
  concluding anything about which asset the volume belongs to.

## Acceptance Criteria

- [x] The row is identified and classified: real trade, or defect. — real
      trade, see Findings.
- [x] If a defect, the root cause is named and a fix or repair task spawned. —
      n/a, not a defect.
- [x] If real, this file records why it is credible so the next person checking
      network volume does not re-open it. — see Findings and Decision.

## Findings — 2026-09-28

Read-only queries on production ClickHouse (`dev_read`) and Horizon.

**The row.** `prices.price_ohlcv_1m`, `2026-08-31 14:29:00`, `asset_id 741`
(USDCAllow) / `quote_asset_id 3` (USDC), `sdex`, `trade_count 1`,
`volume_base = volume_quote = 13,411,498.52`, `open = close = 1`,
`close_usd = 1.0002` (the day's USDC rate), `volume_quote_usd = 13,414,181.36`.
Both factors of `volume_quote_usd` are right — no rate or decimals error.

**What it is.**

- **USDCAllow** — issuer `GDIEKKIQWMIZ4LD3RP3ABPN7X5KEAEWYMR634BRHB7EULIMEVREWLF3G`,
  `home_domain: circle.com`, `auth_required`; only 3 authorized holders.
  Circle's permissioned form of USDC. A separate classic asset, **not** one of
  [[0242]]'s SAC duplicates.
- **Maker** on every fill is Circle's USDC issuer
  (`GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN`) through a standing
  1:1 offer; **taker** is one account,
  `GDEWOLMOPAVRTGNJVWOE6U6LHZVAWIJZVWM6PDLCFTUTJJEKSU32TO5W` (60.6 M USDCAllow).
  In effect a conversion between two forms of Circle's dollar.
- Amounts match Horizon's `/trades` for the pair to the unit (e.g. 2026-09-28
  06:46 UTC, 8,727,864.44).

**It is not one candle.** The pair trades every weekday, a few dozen fills of
0.3–25 M each, almost nothing at weekends. First fill 2021-01-25; 9,078 candles,
≈ 5.47 B USD to date. From 2026-08-20 to 09-27 it was 1.61 B of 1.73 B network
1m USD volume (93.1%); on 2026-08-31, 88.2 M of 88.9 M (99.3%). Since 09-21 the
share is lower (77–92%) because the rest of the network grew; not investigated.
The 44.2 M "trailing 24h" figure above was a different window and does not
change the classification.

**How others count it.** StellarExpert (asset and market `volume7d`), Horizon
`trade_aggregations` and SDF's Hubble dbt models all include it. DefiLlama's
methodology is the only one found that would exclude it ("a protocol's OWN
actions (mint/redeem …) are never volume").

## Decision

**Not a data defect; kept as is** (Adam, 2026-09-28). Our volume matches the
Stellar ecosystem's own tools. The cost, knowingly accepted: USDC's and
USDCAllow's `volume_24h_usd` and network totals are dominated on weekdays by
Circle's conversions. If that is ever to change, the simplest rule is
DefiLlama's — a fill whose offer owner is the issuer of the asset it sells or
buys is not trading volume (price still forms). No task was spawned for it.
