---
id: "0002"
title: "Stream 2 SDEX historical backfill is fully independent of Block Explorer (archive reads + imported BE XDR parser crate)"
status: superseded
deciders: [okarcz]
related_tasks: ["0012", "0013", "0020", "0021", "0022"]
related_adrs: ["0001", "0005"]
tags: [architecture, backfill, sdex, archive-reads, fargate, stream-2, parser-crate, block-explorer, superseded]
links:
  - "./0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "./0005_stream2-sdex-local-workstation-backfill.md"
  - "../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/README.md"
  - "../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md"
  - "../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/notes/I-stream2-options.md"
  - "../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-13
    status: proposed
    who: okarcz
    note: "Drafted to pin Stream 2 architecture after the user directive: SDEX backfill must be entirely independent of BE."
  - date: 2026-05-13
    status: accepted
    who: okarcz
    note: >
      Accepted on the same day. Complements ADR 0001 (Stream 1) by
      committing Stream 2 to a fully prices-api-owned archive-read
      Fargate task — the BE-authored stellar-xdr parser is consumed
      as a library crate only, no BE runtime or DB coupling.
  - date: 2026-05-14
    status: superseded
    who: okarcz
    by: "0005"
    note: >
      Superseded by ADR 0005. The "BE-independent archive-read"
      architectural commitment is preserved verbatim; only the
      deployment shape changes — from a prices-api-owned ECS Fargate
      task to a local workstation Rust CLI mirroring BE's
      backfill-bench/backfill-runner pattern (BE ADR 0010). Cloud
      push of finalised prices tables to RDS is a separate
      post-backfill step. See ADR 0005 §Context for the rationale.
---

# ADR 0002: Stream 2 SDEX historical backfill is fully independent of Block Explorer

**Related:**

- [ADR 0001: Stream 1 Soroban AMM historical backfill is sourced from BE's ClickHouse `soroban_events` (local instance)](./0001_stream1-clickhouse-sourced-amm-backfill.md) — the complementary Stream 1 decision
- [Task 0012: Design SDEX + AMM backfill on Prices-owned Fargate cluster](../1-tasks/backlog/0012_FEATURE_design-prices-owned-backfill-fargate.md) — operational landing for this ADR
- [Task 0020 (archived): Research SDEX historical backfill options](../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/README.md) — research that surfaced Options A/B/C/D
- [Task 0021 (canceled by this ADR): Measure SDEX trade-shaped op density in CH `operations_appearances`](../1-tasks/archive/0021_RESEARCH_measure-sdex-op-density.md) — measurement that would have discriminated Option B vs A; premise removed by this ADR
- [Task 0022: SDEX filter predicates and extraction spec for the dedicated archive-read backfill](../1-tasks/backlog/0022_RESEARCH_sdex-filter-and-extraction-spec.md) — produces the consumer-ready filter + decode spec

---

## Context

ADR 0001 fixed Stream 1 (Soroban AMM, ~8.5M ledgers, Nov 2023→) onto a one-shot
backfill against a locally-run ClickHouse instance populated by BE's
`backfill-runner --target=clickhouse`. That ADR explicitly carved out Stream 2
(SDEX, all-time, ~57M ledgers) for follow-up.

Task 0020 enumerated four Stream 2 options:

- **A** — prices-api-owned Fargate archive-reader, baseline §5.6 design.
- **B** — Hybrid: query BE's CH `operations_appearances` as a trade-bearing
  ledger pre-filter, then archive-read only those ledgers to fetch the
  `offersClaimed[]` payload that CH does not unfold.
- **C** — Push BE to add an `sdex_trades` full-content table to CH; prices-api
  consumes it analogously to Stream 1.
- **D** — Strict subset of B (CH pre-filter only, no extra writes).

Task 0020 recommended Option A as the baseline with Option B gated on task
0021's still-unmeasured trim-ratio. That left a small but real residual
runtime coupling to BE on the Stream 2 path.

**User directive (2026-05-13):** the SDEX backfill is to be entirely
independent of the Block Explorer team — no BE database, no BE CH, no BE
runtime. The only BE artifact prices-api consumes is the BE-authored
`stellar-xdr` parser crate, imported as a Cargo workspace dependency
(matching §8's existing "shared workspace crate" tech-stack note).

This ADR records the resulting architectural decision.

---

## Decision

Prices-api's SDEX historical backfill (Stream 2 of §5.6) is a
**prices-api-owned ECS Fargate task that reads `LedgerCloseMeta` directly
from Stellar's public history archives, from ledger 1 to current tip, with
zero runtime or data coupling to Block Explorer infrastructure.**

Concretely:

1. **Source:** Stellar public history archives only (S3-compatible). No BE
   PostgreSQL, no BE ClickHouse, no BE-deployed services are read at any
   point in the SDEX pipeline.

2. **Coverage:** ledger 1 (Nov 2015, network genesis) → current realtime
   tip (~57M ledgers).

3. **Code dependency on BE — limited to source-level reuse.** The
   `stellar-xdr` Rust parser crate authored and maintained as part of the
   BE / prices-api shared workspace is imported by the SDEX backfill task
   as a Cargo dependency. This is a **library dependency**, not a runtime
   one — the backfill task's deployed binary embeds the parser; it does
   not call out to any BE-hosted service.

4. **Filter logic and trade extraction live inside the prices-api SDEX
   backfill binary.** Predicates select the five trade-shaped operation
   types (`MANAGE_SELL_OFFER`=3, `MANAGE_BUY_OFFER`=12,
   `CREATE_PASSIVE_SELL_OFFER`=4, `PATH_PAYMENT_STRICT_RECEIVE`=2,
   `PATH_PAYMENT_STRICT_SEND`=13) from each ledger's `TransactionResultMeta`
   and emit one `TradeTick` per `ClaimAtom` (V0 / ORDER_BOOK / LIQUIDITY_POOL).
   The detailed filter + decode spec lives in task 0022, building on
   archived 0020's R-note (XDR shape) and G-note (extractor design).

5. **Tranche 1 stance.** The Stream 2 backfill task is fully provisioned,
   deployed, and running by end of Tranche 1. Full historical completion
   (ledger 1 reached) is expected to extend well past Tranche 3 — this is
   acceptable and aligns with §9's existing Tranche 1 acceptance criteria
   ("Historical backfill ECS Fargate task started; covers approximately
   6 months of recent history by end of Tranche 1"). The `GET /backfill/status`
   endpoint surfaces ongoing progress.

6. **Live ingestion is unchanged.** §5.2's Prices Ledger Processor Lambda
   already extracts SDEX trades from new `LedgerCloseMeta` files via the
   Galexie → S3 pipeline. This ADR governs only the historical backfill
   stream.

---

## Rationale

### Why full independence, even at the cost of skipping the CH trim factor

Task 0020's quantitative discriminator between Option A and Option B was
the **trim ratio** — fraction of historical ledgers carrying at least one
trade-shaped op. Even at the optimistic end (~50% trim), Option B's gain
is bounded:

- The `offersClaimed[]` payload is *not* unfolded into CH
  `operations_appearances`; the SDEX payload still requires an archive
  read for every trade-bearing ledger.
- Option B's only saving is skipping the *read* of the non-trade-bearing
  ledgers, not the *parse* of the trade-bearing ones.

Against that bounded gain, Option B re-introduces BE coupling on
prices-api's longest-running historical stream — exactly the coupling
shape that §11.4 sought to bound to Stream 1's time-boxed window. The
user's directive explicitly removes that residual coupling. The cost is
~16 days of pure compute, already budgeted in §10.

### Why ledger 1, not "from current tip backwards" or "from Soroban activation"

The §5.6 plan currently describes a tip-backward task. This ADR commits
to a ledger-1 → tip range as the **target coverage** of the stream. The
**direction** of processing (oldest-first, newest-first, or chunked) is
an implementation choice owned by task 0022 / task 0012 — picked for
operational concerns (resumability, heartbeat cadence, candle-overwrite
semantics under ON CONFLICT). The acceptance criterion is the range, not
the order.

### Why import the BE parser crate but no BE service

§8 already establishes the `stellar-xdr` parser crate as a shared Rust
workspace member compiled into both the BE Ledger Processor and the
prices-api Ledger Processor. Extending this to the SDEX backfill task is
the same pattern. A library crate dependency:

- has versioned, reproducible semantics (Cargo lockfile);
- does not require any BE process to be running at backfill time;
- keeps XDR truth single-sourced (no parallel implementation drift);
- is fully under prices-api's control via pinning.

This satisfies "entirely independent of the BE team" — the BE *team* is
not on prices-api's runtime critical path, only their published crate is
a build-time input.

### Why this is not a reversal of ADR 0001

ADR 0001 makes a different trade for a different stream. Stream 1's full
data shape (decoded ScVal topics + data) is already materialized in the
BE CH copy of `soroban_events`, so the CH path saved both *reads* and
*parses* — a categorical win, not a marginal trim. Stream 2 has no
equivalent fully-materialized CH table (Option C would create one,
rejected), so the architecture diverges. The two ADRs are
complementary, not contradictory.

---

## Alternatives Considered

### Alternative 1: Option B from task 0020 — CH `operations_appearances` as a trade-bearing ledger pre-filter

**Description:** Run BE's `backfill-runner --target=clickhouse` to populate
`operations_appearances` for the full archive range; query CH for the
ledger sequences containing trade-shaped ops; then archive-read only
those ledgers for the `offersClaimed[]` payload.

**Pros:**

- Reduces archive reads by the trim factor (unmeasured; plausibly 30–70%
  era-dependent).
- Reuses BE tooling (`backfill-runner`).

**Cons:**

- Re-introduces BE coupling on the longest-running prices-api stream,
  violating the user directive.
- The CH path saves reads, not parses — the gain is bounded.
- Requires task 0021 (trim-ratio measurement) as a gating dependency.
- Adds a CH-population step to the critical path of Tranche 1 readiness.

**Decision:** REJECTED — coupling cost exceeds the bounded perf gain
under the new independence constraint.

### Alternative 2: Option C from task 0020 — Push BE to add an `sdex_trades` full-content table to CH

**Description:** Negotiate with BE to extend their CH schema with a
`sdex_trades` (or `claim_atoms`) table that unfolds `offersClaimed[]` per
event, analogous to how `soroban_events` unfolds CAP-67 event content.
Prices-api consumes it as in Stream 1.

**Pros:**

- Would restore the Stream 1 "hours, not weeks" architecture for SDEX
  too.

**Cons:**

- Out of prices-api's unilateral control — depends on BE accepting scope.
- Expands BE's CH schema commitment; BE has not signalled appetite.
- Still violates the independence directive in any case (BE-managed CH
  becomes the source of truth for SDEX history).

**Decision:** REJECTED — out of scope and inconsistent with the
independence directive.

### Alternative 3: Option D from task 0020 — CH-only pre-filter, no CH writes from prices-api

**Description:** A strict subset of Option B. Use BE's already-populated
CH (if any) to compute the trade-bearing ledger set; archive-read only
those ledgers; do not populate CH from prices-api.

**Pros:**

- Same trim benefit as B without prices-api CH writes.

**Cons:**

- Same coupling cost as B (read dependency on BE CH).

**Decision:** REJECTED — same reason as Option B.

---

## Consequences

### Positive

- **Stream 2 has zero BE runtime / data coupling.** Aligns with the §11
  "BE database not shared" guarantee on the longest-running stream.
- **Tranche 1 readiness is gated only on prices-api artifacts.** No need
  to wait on BE CH population, BE deployment, or BE scope decisions.
- **Single source of XDR truth preserved.** The BE-authored
  `stellar-xdr` parser crate is consumed as a library, matching §8's
  pre-existing tech-stack pattern.
- **`GET /backfill/status` semantics unchanged.** The endpoint already
  models Stream 2 as a long-running task with progress + heartbeat.
- **Task 0021's dependency on local CH being populated is removed.**

### Negative

- **No trim factor.** All ~57M ledgers must be archive-read by the
  prices-api Fargate task. The bounded compute cost (~16 days) was
  already in §10's estimate, so this is a "no upside" rather than a
  "downside" — but it is worth recording that the optimistic Option B
  win (whatever it would have measured to) is forgone.
- **Backfill completion extends past Tranche 3.** Already acknowledged
  in §5.6 / §9; this ADR explicitly accepts the consequence.
- **Coupling to the parser crate's quality is unchanged but
  acknowledged.** If the BE-authored `stellar-xdr` crate changes API or
  introduces decode bugs, prices-api inherits them. Mitigation: pin a
  Cargo version; treat crate upgrades as design-doc-touching events.
- **Task 0021 is canceled** as obsolete (its premise — pick between
  Option A and B based on trim ratio — no longer applies).

---

## References

- [ADR 0001: Stream 1 Soroban AMM historical backfill is sourced from BE's ClickHouse `soroban_events`](./0001_stream1-clickhouse-sourced-amm-backfill.md)
- [Task 0020 (archived) — Stream 2 options research](../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/README.md)
- [Task 0020 — R-note: SDEX operation XDR shape](../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/notes/R-sdex-operation-xdr-shape.md)
- [Task 0020 — G-note: SDEX trade extraction design](../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md)
- [Task 0020 — I-note: Stream 2 options A/B/C/D](../1-tasks/archive/0020_RESEARCH_sdex-historical-backfill-options/notes/I-stream2-options.md)
- [Prices API design — §5.6 Historical Backfill, §8 Tech Stack, §10 Cost](../../docs/prices-api-general-overview.md)
