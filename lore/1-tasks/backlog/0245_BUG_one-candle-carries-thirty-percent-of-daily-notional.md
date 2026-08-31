---
id: "0245"
title: "A single 1m candle carries 30% of the network's 24h notional — $13.4M of $44.2M in one row"
type: BUG
status: backlog
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

- [ ] The row is identified and classified: real trade, or defect.
- [ ] If a defect, the root cause is named and a fix or repair task spawned.
- [ ] If real, this file records why it is credible so the next person checking
      network volume does not re-open it.
