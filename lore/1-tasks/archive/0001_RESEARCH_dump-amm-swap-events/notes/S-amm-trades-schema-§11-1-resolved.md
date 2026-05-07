---
title: "Resolved: amm-trades-schema §11.1 — swap event topic symbols differ across AMMs"
type: synthesis
status: developing
spawned_from: R-swap-topic-shapes.md
spawns: []
tags: [soroban, amm, schema-validation, decision]
links:
  - "../../../../../docs/database-schema/amm-trades-schema.md#11-open-questions-for-the-be-team"
history:
  - date: 2026-05-07
    status: seed
    who: okarcz
    note: "Synthesis drafted from R-swap-topic-shapes.md"
---

# Decision: AMM swap topic symbols differ across venues — filter must accept multiple

## Conclusion

The open question §11.1 in `docs/database-schema/amm-trades-schema.md`
("Filter symbol per AMM. Does each of Soroswap / Aquarius / Phoenix emit
swap events under the topic symbol `\"swap\"`, or does any of them use a
different symbol?") is answered: **at least two distinct topic symbols are
in use**, and the per-venue filter mapping in the BE Ledger Processor is
**load-bearing**, not optional.

Concretely, in the sampled window (mainnet ledgers 62016000–62079999,
~3.5 days):

- `Symbol("swap")` is emitted by a single contract that also emits
  `add_pool` / `config_rewards` — clearly an AMM **factory/router**, not a
  pool.
- `Symbol("trade")` is emitted by 29 distinct contracts, all of which also
  emit `update_reserves` in the same window — clearly **pool-level**
  events from one or more AMM venues whose pool contract emits `trade` per
  swap.

See `R-swap-topic-shapes.md` for raw counts, full topic/data shapes, and
sample event JSON.

## What this changes in `prices_amm_trades`

Nothing in the **schema** — the SQL DDL in §4 of
`docs/database-schema/amm-trades-schema.md` is correct as written. The
`venue VARCHAR(10)` column already permits per-venue tagging, and the
typed `(token_in, token_out, amount_in, amount_out)` columns are agnostic
to which topic symbol carried the values.

What it changes in the **filter spec** (§7 of the same doc):

- §7 step 3 currently reads:
  > The event topic identifies a **swap** (e.g. `topics[0] = Symbol("swap")`).
  > The exact symbol per AMM is documented in the BE indexer; if any AMM
  > uses a different symbol for swaps the indexer's per-venue mapping
  > handles it.

  Empirically confirmed. Recommend updating the doc to say "the indexer
  MUST hold a per-venue `(contract_id → topic_symbol → decoder)` mapping
  because no single symbol covers all AMMs". This is no longer a
  hypothetical "if".

- §7 step 4 (decoded payload yields `(token_in, token_out, amount_in,
  amount_out)`) is satisfied by both observed shapes, but the **decoder
  is per-symbol**:

  | Symbol | Where amounts live | Type | Where token pair lives |
  |---|---|---|---|
  | `swap` | `data.vec[3]`, `data.vec[4]` | `U128` | `topics[1]` (Vec of 2 Addresses) AND `data.vec[1..3]` |
  | `trade` | `data.vec[0]`, `data.vec[1]` | `I128` | `topics[1]`, `topics[2]` |

  These are not interchangeable; the indexer must dispatch by
  `topics[0]`.

- A **third decoder** must reject `Symbol("SwappedFromVUsd")` and
  similar non-AMM swap-named events. The §7 filter is a `(contract_id ∈
  known_amm_set) AND (topic[0] ∈ {known_amm_symbols})` AND-conjunction —
  the contract-set guard already prevents this misclassification, but
  worth restating in the doc.

## Reasoning

1. **Two empirically distinct shapes** rule out the simplifying
   assumption that all three target AMMs use the literal string `"swap"`.
   Even if `Symbol("swap")` is one of them, at least one other (the
   `trade` emitters) is not — and the `trade` payload shape is different
   enough (3-element Vec, I128 not U128, no per-pool address inside the
   payload) that a single decoder cannot handle both.

2. **The schema is robust to this.** `prices_amm_trades` was deliberately
   designed to lift only the four normalised fields (`token_in`,
   `token_out`, `amount_in`, `amount_out`) and the `venue` tag, with no
   `topics` or `data` JSONB columns (§3 "What is deliberately omitted").
   That decision pre-paid for exactly this case: per-venue decoder
   variation does not leak into the table.

3. **Phoenix's absence from the sample is informative but not
   conclusive.** A 3.5-day window can plausibly contain zero Phoenix
   trades if Phoenix volume is below ~30 trades/day, which is realistic
   for a third-tier Stellar AMM in May 2026. The schema must still budget
   for a Phoenix decoder; the BE indexer's per-venue mapping is the right
   place for that.

## Alternatives considered

- **Drop `venue` and treat all swap events identically.** Rejected. The
  Prices API VWAP attribution explicitly needs per-venue volume
  (overview §5.5), and the §3.3 `current_prices.sources` JSONB key is
  per-venue. Without `venue`, that contract breaks.

- **Capture raw `topics` and `data` JSONB and decode at read time.**
  Rejected on cost grounds (overview §5.6 + this doc §3 "What is
  deliberately omitted") — the BE writes ~tens of thousands of rows/day
  and storing dead JSONB is ~250 B/row of waste. Per-venue decoders at
  write time stay aligned with the existing design.

## Future work (spawned)

These are tracked as separate backlog tasks rather than left here as
prose:

1. **Venue attribution for the observed contracts** — requires either
   public Soroswap/Aquarius/Phoenix contract registries or a query to
   the Stellar Expert API. Out of scope for the lore-0001 task because
   the §11.1 question is *about topic symbols, not addresses*; venue
   tagging is the BE's responsibility per §11.2.

2. **DOCS update to `amm-trades-schema.md` §7 and §11.1** — replace the
   hypothetical "if any AMM uses a different symbol" wording with the
   empirical finding, and link to this synthesis as evidence.

3. **Wider sample for Phoenix detection** — re-run the tool against a
   longer ledger range (e.g. one full month) before locking the
   filter set. Optional; can be done as part of (1).
