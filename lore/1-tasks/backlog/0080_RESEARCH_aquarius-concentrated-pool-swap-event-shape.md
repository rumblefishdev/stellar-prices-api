---
id: "0080"
title: "Verify Aquarius concentrated-liquidity pool swap-event shape against AquariusPoolExtractor"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0079", "0018", "0087", "0053"]
tags: [layer-research, priority-medium, effort-small, amm, aquarius, pool-registry, extractor, decode, router]
links:
  - "../active/0079_FEATURE_pool-registry-seed-from-soroswap-api.md"
  - "../active/0087_BUG_unresolved-guard-fatal-on-aquarius-router-swap.md"
history:
  - date: 2026-07-03
    status: backlog
    who: okarcz
    note: >
      Spawned from 0079: the live Soroswap API reports 20 Aquarius pools with
      poolType 'concentrated' (alongside 269 xyk + 41 stable). The 0079 seeder
      deliberately holds these back because AquariusPoolExtractor is documented
      only for constant-product + stableswap (inline-token `trade` event); a
      concentrated-liquidity pool may emit a different shape → mis-decoded prices.
  - date: 2026-07-07
    status: backlog
    who: okarcz
    note: >
      Absorbed a second, distinct coverage gap found in 0087: at the EARLY Soroban
      epoch the underlying AMM pools emit NO events — the router `swap` summary is
      the sole trade record — so those trades are silently dropped. Deferred here
      by decision (guard fix already unblocks 0053; lost volume is small + early +
      idempotently re-fillable). Full design + quantification + open decisions in
      the new "Related gap" section below. May warrant splitting into its own
      FEATURE task when picked up.
---

# Verify Aquarius concentrated-pool swap-event shape

## Summary

Determine whether the 20 mainnet Aquarius `concentrated` pools emit the same
inline-token `trade` event that `AquariusPoolExtractor` already decodes. If they
do, they can be added to the `pool_registry` seed (0079); if not, document the
divergence and scope an extractor variant before seeding them.

## Context

- 0079's API seeder classifies venue-aware: Aquarius `xyk`+`stable` are seeded
  (extractor doc confirms both use the inline-token `trade` shape), but
  `concentrated` (20 pools) is skipped to avoid seeding a pool whose swaps might
  mis-decode into wrong OHLCV. Unseeded, their live swaps go to
  `unresolved_pools` (no data) rather than producing bad data.
- **The seeder's hold is only a seeder-side filter — the concentrated pools reach
  the registry anyway via the on-chain factory-event path.** `learn_factory`
  registers every Aquarius `add_pool` event as `venue=Aquarius` regardless of
  sub-type (`soroban.rs`), so the SDEX/AMM backfill (0053) and the live processor
  both classify concentrated pools as Aquarius and dispatch their swaps to
  `AquariusPoolExtractor`. So this mis-decode risk is a **pre-existing property of
  the AMM discovery path, not something the seeder introduced** — it must be
  resolved before AMM prices for Aquarius concentrated pools can be trusted,
  independent of whether the API seeder is used. If the shape diverges, the fix
  likely needs to gate the extractor by sub-type (or add a variant), not just
  filter the seed.
- The extractor lives at `packages/aquarius-extractor/src/lib.rs`; its header
  documents constant-product / stableswap only. Reference decode methodology:
  task 0018 (per-AMM swap event shapes).

## Implementation

1. Pick a real `concentrated` Aquarius pool (from the API list) and dump a real
   swap tx's events (same method as 0018 — `dump-swap-events --contract … --tx …
   --show-xdr --pretty` against a Galexie range containing the tx).
2. Compare the event shape (topics + data) to the `trade` event
   `AquariusPoolExtractor::extract_one` expects (inline `sold_token` /
   `bought_token` addresses + amounts).
3. If identical → widen the 0079 seeder to include `concentrated` (`classify_pool_type`
   for `aquarius` adds `"concentrated"`), add a regression sample, re-seed.
4. If it diverges → document the shape, keep it excluded, and spawn a task to add
   a concentrated-pool extractor variant.

## Related gap (absorbed from 0087, 2026-07-07) — early-epoch router-only swaps

Distinct from the concentrated-pool shape question above, but the same
"AMM-coverage" family. 0087's targeted archive fetch established:

**At the early Soroban epoch the underlying pools emit NO events — the router
`swap` summary is the sole machine-readable trade.** Verified on the two endpoint
ledgers of the 0053 tranche (`50639018`, `50686276`): across 2071 events the only
signatures were `transfer/fee/set_authorized/approve/swap/burn`; the pools
referenced in the router `swap` `data[0]` (`CC7LUVAF…`, `CCKWA3RE…`) emitted
nothing. The router `swap` shape is self-contained:

```
topics = [ Symbol("swap"), Vec([tokenA, tokenB]), Address(trader) ]
data   = [ Address(pool), Address(token_in), Address(token_out),
           u128(amount_in), u128(amount_out) ]     # u128 → TaggedValue::I128
```

Contrast: in the LATER epoch (62.0M sample) the pools DO emit `trade` and the
router `swap` is redundant (14/14 router-referenced pools also emit `trade`;
32/32 router swaps share the pool `trade`'s transaction). So the router summary
is the sole signal ONLY early; ignoring it there drops real (if small) volume.

### Proposed fix — price the router swap with a SAME-TX dedup (no cutover ledger)

Rule: **price a router `swap` iff its `data[0]` pool did not already produce a
tick in the same transaction.** `classify_amm_groups` runs per-tx and holds the
whole tx's groups, so it can enforce this. Self-correcting across epochs:
early → pool silent → price the router; later-registered → pool `trade` prices →
skip router; later-unregistered → pool `trade` dropped → router prices as fallback
(also recovers unregistered-Aquarius drops). Validated safe by the 32/32 same-tx
colocation — no hardcoded cutover ledger needed.

Implementation sketch:
1. `router_swap_to_trade(row) -> Option<TradeRow>` in `aquarius-extractor`
   (self-contained; reuse `is_aquarius_router_swap` shape guard). Unit-testable.
2. Restructure `classify_amm_groups` into two passes: (1) registered pools →
   `dispatch` → ticks, recording a `priced_pools: HashSet<contract_id>`; set
   router swaps aside; keep the genuine-unknown `swap` → `unresolved` guard.
   (2) each router swap → `router_swap_to_trade`; if `data[0]` pool ∈
   `priced_pools` skip (dedup) else `amm_trade_to_tick` → push. Multi-hop paths
   work naturally (one `swap` event per hop).
3. Tests: router-alone → 1 tick; router + sibling pool `trade` → 1 tick (not 2);
   genuine unknown-pool `swap` → still `unresolved`.

### Two decisions this needs (why it was deferred, not built)
1. **`source` tag** for recovered router trades (`"aquarius"` vs a dedicated
   `"amm_router"`). Price is `amount_out/amount_in` regardless; only the tag is
   affected.
2. **Protocol identity** of the early routers (`CBVSLUYH…/CANMWW5D…/CDVTDAUA…`).
   The extractor names `CBQDHNB…` as *the Aquarius router*, but the early ones are
   unconfirmed — the shape also resembles a Soroswap-style path router. Also
   confirm **bounded-early vs ongoing** (do these routers/their silent pools
   persist past the cutover, or are they superseded by the `trade`-emitting era?).
   A bounded probe (~10 ledgers across `[51M..61M]`) settles it.

### Volume estimate (why deferring is safe)
Early-epoch AMM is provably sparse: 18 router swaps in the first ~2 weeks
(`swap_count 16/1/1`), `amm_ticks: 0`; extrapolated → order hundreds–low-thousands
of lost swaps chain-wide, clustered at the 2024 launch. The 0053 backfill is
idempotent + minute-aligned per source, so the early window can be re-run with the
router extractor later without redoing the chain. Skip-now was the accepted call.

## Acceptance Criteria

- [ ] One real Aquarius concentrated-pool swap decoded + archived as evidence.
- [ ] Decision recorded: same shape (→ include in seed) or divergent (→ exclude +
      extractor-variant task).
- [ ] If include: 0079 seeder updated; the 20 pools land in `pool_registry`.
- [ ] Early-router gap: bounded-vs-ongoing settled (probe), `source` tag + protocol
      identity decided; then implement `router_swap_to_trade` + same-tx dedup (or
      split into its own FEATURE task) and re-run the early backfill window.
