---
id: "0184"
title: "We still steer pairs into a depegged quote leg — is USDT's rank-1 quote preference still justified?"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0172", "0139", "0165"]
tags:
  ["priority-medium", "effort-medium", "canonicalisation", "adr-input", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/canonical.rs"
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Spawned from 0172, which deliberately left this alone. Changing quote
      preference re-orients pairs and is not a bug fix — it needs its own
      analysis of what it does to historical comparability.
---

# Should a depegged asset still be a preferred quote?

## The situation

`canonical.rs::is_preferred_quote` ranks quote legs:

```rust
USDC (canonical issuer) => Some(0)
USDT (canonical issuer) => Some(1)
Native XLM              => Some(2)
```

Rank 1 means canonicalisation **actively orients pairs so this USDT is the
quote** whenever it is one of the two legs and USDC is not. That is how 495 base
assets came to have USDT-quoted candles at all ([[0182]]).

The ranking made sense when USDT was a dollar. It depegged in June 2022
([[0172]]) and now trades at ~$0.13, so we are steering price discovery into a
leg whose own value has to be measured and pivoted through before any USD figure
can be derived from it.

## Why 0172 did not change it

Quote preference decides pair **orientation**, not value. Changing it:

- alters which asset is `asset_id` and which is `quote_asset_id` for new candles,
  so a pair's history would split across two orientations at the cutover;
- does not fix any wrong number by itself (0172 already fixed the values via the
  pivot);
- interacts with [[0139]] (`asset_id` collisions) and with every consumer that
  joins on `(asset_id, quote_asset_id)`.

It is a data-model decision, not a defect.

## Questions to answer

- **Does demoting USDT below XLM improve anything measurable?** XLM is liquid and
  already the pivot reference. If a pair would price better through XLM, that is
  an argument. If the pairs in question do not trade against XLM at all,
  demotion just makes them unpriceable.
- **What happens to the existing 44,657 candles?** Left as-is they are a
  historical orientation that no longer matches new writes. Is that acceptable,
  or does it need a migration ([[0182]] may be the natural place)?
- **Is "preferred quote" the right concept at all,** or should preference be
  derived from measured liquidity rather than a hardcoded list? A hardcoded list
  is exactly what failed here — it encoded a belief ("USDT is a dollar") that
  silently stopped being true.
- **Does any peg/preference list elsewhere carry the same stale assumption?**

## Acceptance Criteria

- [ ] Recommendation with measured support: keep rank 1, demote, or derive
      preference from liquidity
- [ ] If changing: migration plan for the orientation split, and BE told what
      changes in the pair keys they join on
- [ ] ADR if the answer is "derive from liquidity" — that is an architectural
      change, not a config tweak
