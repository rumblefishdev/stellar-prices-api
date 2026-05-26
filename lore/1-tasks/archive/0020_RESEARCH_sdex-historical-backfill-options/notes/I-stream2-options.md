---
title: 'Stream 2 (SDEX) backfill — four options'
type: idea
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, backfill, options, clickhouse, archive-reads]
links:
  - './R-sdex-operation-xdr-shape.md'
  - './G-sdex-trade-extraction-design.md'
  - '../../../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/I-integration-options.md'
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: 'Four options expanded from the README sketches; trade-offs surfaced for synthesis.'
---

# Stream 2 (SDEX) backfill options

Four shapes. Same axes as the 0015 Stream 1 I-note but the answer
turns out different because **the price-relevant payload
(`ClaimAtom`s) is NOT in BE's CH today** — only operation
appearances are. CH offers a pre-filter for SDEX, not a payload
replacement.

## Option A — Status quo: prices-api owns the archive-reader

**Sketch:** prices-api Fargate Rust task reads `LedgerCloseMeta`
objects from public archives end-to-end, walks the structure
documented in [R-note](./R-sdex-operation-xdr-shape.md), extracts
`ClaimAtom`s with the algorithm in [G-note](./G-sdex-trade-extraction-design.md),
writes OHLCV to prices-api PG. Today's §5.6 plan, refined by the
G-note's extraction shape.

**Pros:**

- Single moving part: one Fargate task, one decoder crate.
- Zero coupling to BE's CH plans or its backfill runner.
- §10 cost estimates already reflect this shape.
- The extractor is small (~150 lines of Rust + decode) and
  protocol-stable (no per-app conventions to track).

**Cons:**

- ~16 days of pure compute for the full 57M-ledger walk (§5.6's
  estimate). All ledgers read, including the (likely large)
  fraction with no trade-shaped ops.

**Verdict:** Baseline. Always available; no external dependency.

## Option B — Run BE's `backfill-runner --target=clickhouse`, then pre-filter

**Sketch:** Since task 0017 stands up a local CH instance for
Stream 1 anyway, **reuse the same local CH** for SDEX pre-filtering.
BE's runner populates `operations_appearances` as a side effect of
its Soroban pass — the SDEX-relevant op types (2, 3, 4, 12, 13)
land there with the rest. Query CH for "ledgers with at least one
trade-shaped op", produce a sorted Int64 ledger list, and feed it
to the prices-api Fargate task. The task then reads archive only
for the filtered ledger set, decoding the same way Option A does.

**Pros:**

- Zero marginal infra cost — the CH instance from task 0017 is
  already there.
- Modest speedup proportional to the trim ratio. If 50% of
  ledgers carry no SDEX-shaped op, you halve the bytes-read.
- Decoder logic is unchanged from Option A — only the _which
  ledgers_ changes.

**Cons:**

- Adds a pre-filter pipeline step (one CH query + ledger-list
  shipping to the Fargate task).
- Locked to the same backfill window as Stream 1. If Stream 1
  finishes Tranche 1 and the CH instance is torn down before
  Stream 2 runs full-history, you'd need to keep CH alive or
  pre-export the filter list.
- The trim ratio is unmeasured today.
  [Task 0021](../../../backlog/0021_RESEARCH_measure-sdex-op-density.md)
  is the discrete measurement spike.

**Verdict:** Cheapest optimisation over baseline. Build only if the
measured trim ratio justifies the plumbing.

## Option C — Push BE to add an `sdex_trades` table to CH

**Sketch:** Ask BE to add a CH table analogous to `soroban_events`
but for SDEX trades — one row per `ClaimAtom`, with
`(asset_sold, amount_sold, asset_bought, amount_bought,
ledger_sequence, transaction_id, op_index, claim_index)` and
LowCardinality codecs. Prices-api then queries CH for the trade
data directly, no archive walk.

**Pros:**

- Would mirror the Stream 1 shape, giving prices-api a uniform
  read pattern for both streams.
- Could power BE's own /trades endpoint over CH; mutual benefit.

**Cons:**

- **Out of prices-api's unilateral control.** Requires BE-side
  scope expansion, ADR, implementation, and testing on the BE
  side. The Stellar protocol has 10+ years of history; populating
  this table is itself an archive walk — someone has to do it.
  If BE does it, BE pays; the prices-api timeline still depends
  on when BE finishes.
- No signal from BE that they want this. ADR 0044 §Decision §4a's
  full-content unfold was specifically _for Soroban events_
  because BE wanted the analytical query path. SDEX trades already
  have a working answer (archive + Horizon `/trades`) — BE may
  not see the same urgency.
- Even if BE built it, prices-api would still need a fallback
  archive-walker for ledgers outside CH's population window.

**Verdict:** Strongest long-run answer; weakest short-run
deliverability. Cross-team ask, not a prices-api decision.

## Option D — CH pre-filter only (Option B without the Stream-1 dependency)

**Sketch:** Stand up a CH instance for the sole purpose of
populating `operations_appearances` and pre-filtering SDEX
ledgers. Independent of Stream 1's CH instance.

**Pros:**

- Decouples Stream 2 from Stream 1's CH lifecycle.
- Same pre-filter benefit as Option B.

**Cons:**

- **Net negative vs Option B.** You pay for the same CH instance
  separately when Stream 1 was getting it for free.
- Population time (running BE's backfill runner for the full 57M
  ledgers) is the same archive walk Option A does anyway, so the
  pre-filter advantage is washed out unless Stream 2 runs long
  enough that the saved bytes-read time exceeds the population
  time.

**Verdict:** Strictly dominated by B in the common case. Only
makes sense if Stream 1 and Stream 2 are temporally disjoint AND
the Stream-2 window is long enough to amortise the CH population.

## Comparison

| Dimension                       | A                | B                            | C                         | D                |
| ------------------------------- | ---------------- | ---------------------------- | ------------------------- | ---------------- |
| Marginal infra over baseline    | None             | Reuses 0017 CH               | None (BE owns)            | New CH instance  |
| Speedup vs A                    | 1×               | 1.5–2× (est.)                | Potentially 5–10×         | 1.5–2×           |
| Depends on cross-team work      | No               | No (BE runner ships today)   | **Yes (BE-side feature)** | No               |
| Risk if dependency slips        | n/a              | low (graceful fallback to A) | high (gate)               | low              |
| Stellar protocol changes affect | only A's decoder | only A's decoder             | BE's writer               | only A's decoder |
| §10 cost estimate aligned       | Yes              | Yes (no marginal cost)       | Unknown                   | Adds CH op cost  |

## Discriminating measurement

The single quantity that picks between A and B is the **trim
ratio** — fraction of historical ledgers with at least one op
of type ∈ {2, 3, 4, 12, 13}. Trim ratio ≥ 50% justifies B's
pre-filter plumbing; trim ratio < 30% argues to stay with A and
not bother. Measurement deferred to task 0021.

The single quantity that re-opens C is BE's appetite — answered by
an inbox message to fmazur asking whether SDEX `ClaimAtom`s warrant
a CH table analogous to `soroban_events`. Cost is one message;
do it.
