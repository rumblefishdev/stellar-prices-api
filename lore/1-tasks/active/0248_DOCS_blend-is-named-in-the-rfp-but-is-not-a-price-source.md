---
id: "0248"
title: "The RFP names Blend as a market to aggregate and we do not ingest it — record why that is not a shortfall"
type: DOCS
status: active
related_adr: []
related_tasks: ["0128", "0237", "0217", "0173"]
tags: [layer-docs, priority-medium, effort-small, milestone-M2, scf, submission, evidence, ingestion]
milestone: 2
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../packages/prices-ingest-core/src/registry_io.rs"
history:
  - date: 2026-09-01
    status: backlog
    who: okarcz
    note: >
      Raised from outside the task queue — someone asked whether Blend, the
      Stellar lending protocol, should be added to this project. The technical
      answer is no and is not close. But checking the question turned up that
      the SCF RFP names Blend as one of four "major markets" for price
      aggregation and we ingest three of them, which is a submission-facing gap
      with no written answer. Same shape as [[0237]]: an RFP line we do not
      satisfy literally, needing a recorded position before a reviewer asks.
      Blend appears exactly ONCE in all of lore/ today ([[0217]]'s quotation of
      the RFP) — there is no prior decision to point at.
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      Activated as a short DOCS task. ⚠️ SEQUENCING FOUND ON ACTIVATION, not
      yet resolved: every AC here writes into "[[0128]]'s package", and 0128 is
      still in BACKLOG — there is no milestone-2-evidence.md; docs/scf/ holds
      only the M1 set. The sibling precedent [[0237]] is ALSO still backlog, so
      it has not created the "deviations from the RFP" slot either and cannot
      be copied from. So this task can be worked in one of two shapes and the
      choice is the operator's: (a) write the position into this task file now
      and let 0128 lift it into the package verbatim when it is built, which
      unblocks it today and leaves 0128 holding the debt; or (b) hold it until
      0128 creates the deviations section, which is where the ACs currently
      point. Nothing about the Blend ANSWER is in doubt - it produces no trades
      and is itself an oracle consumer - only where the text lands.
---

# Blend is named in the RFP's aggregation list and we do not ingest it

## Summary

The SCF RFP's Core Requirements state:

> *"Price Aggregation: Weighted average across major markets (Soroswap,
> Aquarius, SDEX, **Blend**)"*

We ingest **SDEX, Soroswap and Aquarius**, plus **Phoenix**, which the RFP does
not name. **Blend is the one listed market we do not ingest**, and we have never
written down why. This task records the position; it does **not** build an
extractor.

## Context — why Blend cannot be a price source

**Blend is a lending protocol, not an exchange.** Users deposit assets to earn
interest or borrow against collateral; there is no swap. A price is a property of
a *trade*, so a lending pool has none to give. Concretely, from the protocol's
own description ([case study](https://stellar.org/case-studies/meru-wallet-uses-blend-defi-protocol-for-yield),
read 2026-09-01):

- **Isolated lending pools.** A pool creator sets supported assets, collateral
  requirements, interest rates and utilization caps. None of these is a traded
  price.
- **Pool creators specify which ORACLE prices the collateral.** This is the
  decisive fact: Blend is a **consumer** of price data, positioned downstream of
  a service like ours — not a producer of it.
- **The backstop module is an 80/20 BLND:USDC AMM.** See below; this is the only
  part of Blend that trades.

So the RFP's bullet groups a lending protocol with three DEXes under "major
markets". ⚠️ **Read as an inference, not a quotation** — the RFP does not say
Blend is a DEX, and a reviewer is entitled to ask the question regardless of what
we conclude the bullet meant.

## The one technically coherent version of "add Blend"

Not a reason to do it, but it must be stated so the answer is not overclaimed.

Blend's **backstop module** is an 80/20 BLND:USDC AMM — a genuinely swappable
market, and plausibly the main venue where **BLND itself** is priced. Ingesting
it would give **one new asset a price**; it would contribute nothing to any
existing asset's price, which is what "aggregate across markets" asks for.

⚠️ **Unmeasured.** Nobody has checked the backstop pool's trade volume, or
whether our ledger-processor would see its events at all. If the answer is "thin
enough to fail the [[0118]] volume threshold", the coherent version is coherent
and still not worth building. **Measure before arguing either way.**

## Implementation

- Record the position: **Blend is out of scope as a price source because it
  produces no trades.** One paragraph, with the downstream-consumer point, since
  that is the part that makes the answer obviously right rather than merely
  defensible.
- Write it into the [[0128]] package as a **"deviations from the RFP"** entry —
  the RFP line quoted, the four named venues listed against what we ingest, the
  reason given. Same slot [[0237]] uses for the `float` vs decimal-string answer.
- State the venue count plainly in the same entry: **three of the four named
  markets, plus Phoenix which the RFP does not name.** Volunteering Phoenix is
  what makes the entry read as scope reporting rather than an excuse.
- Note the backstop-AMM possibility in one sentence so the entry cannot be
  read as "we did not know Blend has any market at all".
- ⚠️ Do **not** generalise this into a rule about lending protocols. The
  reasoning is "no trades, therefore no prices", which happens to cover lending
  today; it is not a claim about a protocol category.

## Acceptance Criteria

- [ ] [[0128]]'s package carries a "deviations from the RFP" entry quoting the
      Price Aggregation line and naming Blend
- [ ] The entry says what we DO ingest (SDEX, Soroswap, Aquarius + Phoenix),
      not only what we don't
- [ ] The reason given is "produces no trades, and is itself an oracle
      consumer" — not "out of scope" without a mechanism
- [ ] The backstop 80/20 BLND:USDC AMM is acknowledged, with its status
      (unmeasured) stated rather than implied
- [ ] No extractor, no `Venue` enum arm, no registry seeding — if the decision
      ever reverses, that is a FEATURE task of its own

## Out of scope

- **Building a Blend extractor.** Adding a venue here is a `Venue` arm, an
  extractor crate, pool-registry seeding and a historical backfill; the
  registry-seeding step is where venues reliably bite — Phoenix and Soroswap
  both miss on historical backfills until the factory registry is seeded. Not
  worth spending on a protocol with no swaps.
- **Pricing BLND.** If someone wants it, that starts with measuring the backstop
  pool's volume and is a FEATURE task, not this one.
- **Blend as a CUSTOMER.** Pool creators choose an oracle, which makes Blend a
  candidate consumer of this API. That is a commercial conversation and belongs
  with whoever owns it — worth raising, not worth filing here.

## Notes

- Blend appears **exactly once** in the whole `lore/` tree today: [[0217]]'s
  quotation of the RFP at line 153. There is no earlier decision, no ADR, and no
  note — which is precisely the exposure this task closes.
- The related question of whether the headline `price_usd` should even BE the
  weighted aggregate the same RFP bullet describes is **[[0217]]**, and is
  separate. This task is about the venue list; that one is about the method.
- Sibling precedent: [[0237]] handles the `Current Price (float USD)` deviation
  the same way. Milestone 1's package was accepted partly because its Section 6
  listed gaps honestly with a destination for each; this is an M2 instance of the
  same discipline.
