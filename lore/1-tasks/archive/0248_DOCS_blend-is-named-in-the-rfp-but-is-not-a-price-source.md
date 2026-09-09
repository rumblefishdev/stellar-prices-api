---
id: "0248"
title: "The RFP names Blend as a market to aggregate and we do not ingest it — record why that is not a shortfall"
type: DOCS
status: completed
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
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      WRITTEN. The operator chose a destination neither option offered: not
      0128's package and not this task file, but the GENERAL design doc -
      docs/prices-api-general-overview.md, new §5.7 "Venue coverage - the
      markets we ingest, and why Blend is not one", plus a Revision History
      row. 🔑 The reasoning behind that call is worth keeping: "Blend produces
      no trades" is a permanent fact about the protocol, true whether or not
      SCF ever asks, so filing it as a submission artifact would have
      mis-shelved it AND blocked it behind a task that has not started. §5.7
      carries a five-row venue table with `Ingested` and `Named in the RFP`
      columns so the three-of-four count reads off the page, the
      price-is-a-property-of-a-trade argument, the decisive oracle-consumer
      point with the circularity named, the backstop 80/20 BLND:USDC AMM
      flagged as UNMEASURED, and a "what would change the answer" paragraph so
      the section cannot be read as a permanent refusal. Five of six ACs met;
      the sixth is [[0128]] quoting or linking §5.7 when the package is built,
      recorded in 0128's own Notes as inherited debt rather than left to
      memory. Nothing outside docs/ was touched - no extractor, no Venue arm,
      no registry seeding.
  - date: 2026-09-02
    status: completed
    who: okarcz
    note: >
      CLOSED. 5 of 6 acceptance criteria met; the sixth - 0128 quoting or
      linking §5.7 - is DEFERRED to [[0128]] and left unticked, because this
      task cannot perform it: the M2 package does not exist and that task has
      not started. Delivered in one commit (2c10339): a 66-line §5.7 in
      docs/prices-api-general-overview.md plus a Revision History row, with the
      RFP quote, a five-row venue table carrying `Ingested` and `Named in the
      RFP` columns, the price-is-a-property-of-a-trade mechanism, the decisive
      oracle-consumer point with the circularity named, the backstop 80/20
      BLND:USDC AMM flagged UNMEASURED, and a "what would change the answer"
      paragraph pricing the reversal. Closes the exposure the task was filed
      for: Blend previously appeared EXACTLY ONCE in the whole repository, in
      [[0217]]'s quotation of the RFP, with no position to point a reviewer at.
      ⚠️ Two things carried forward rather than fixed: 0128 owes the citation
      (recorded in its Notes), and the overview has a pre-existing broken link
      to a deleted 0017 task file, noticed and left unswept. ⚠️ Also recorded
      under Issues: the ACs as filed all targeted a document that does not
      exist, which is what small answers queuing behind an unstarted package
      looks like - [[0237]] is in the same position today.
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

⚠️ **DESTINATION CHANGED on activation, 2026-09-02, by the operator.** These
ACs were written against "[[0128]]'s package", and 0128 is still in backlog —
there is no `milestone-2-evidence.md`, and the sibling [[0237]] has not created
the deviations slot either. The operator's call: **this is a permanent project
fact, not a submission artifact**, so it lands in the general design doc and the
M2 package quotes it later. ACs re-pointed accordingly; the substance is
unchanged.

- [x] The position is recorded in a **general, reviewer-reachable document**
      quoting the Price Aggregation line and naming Blend —
      `docs/prices-api-general-overview.md` **§5.7**, plus a Revision History
      row. Chosen over 0128's package so the fact does not wait on a task that
      has not started
- [x] The entry says what we DO ingest (SDEX, Soroswap, Aquarius + Phoenix),
      not only what we don't — Table 5.7 gives all five venues with an
      `Ingested` and a `Named in the RFP` column, so the three-of-four count is
      readable without reconstructing it
- [x] The reason given is "produces no trades, and is itself an oracle
      consumer" — not "out of scope" without a mechanism. §5.7 leads on **a
      price is a property of a trade**, and gives the oracle-consumer point as
      the decisive one, with the circularity named
- [x] The backstop 80/20 BLND:USDC AMM is acknowledged, with its status
      (unmeasured) stated rather than implied — §5.7 says the volume is
      unmeasured and that no claim is made either way
- [x] No extractor, no `Venue` enum arm, no registry seeding — nothing outside
      `docs/` was touched. §5.7's "What would change the answer" names what a
      reversal would cost and marks it a feature of its own
- [ ] **0128 quotes or links §5.7** when the M2 package is built —
      **(deferred to [[0128]])**. Not this task's to perform: the package does
      not exist and 0128 has not started. Recorded in 0128's own Notes as
      inherited debt rather than left to memory, and left unticked here so
      nobody reads a tick as evidence the package cites §5.7

## Implementation Notes

One file of substance: `docs/prices-api-general-overview.md`, new **§5.7 "Venue
coverage — the markets we ingest, and why Blend is not one"** (66 lines), placed
at the end of §5 where the ingestion venue set is already described, plus a
Revision History row per that document's own convention for substantive changes.

Structure, and why each part is there:

| Part | Why |
|---|---|
| The RFP quote | So the reader sees the exact line being answered, not a paraphrase of it |
| Five-row venue table, `Ingested` + `Named in the RFP` columns | The three-of-four count reads off the page. Phoenix is volunteered in the same table, which is what makes it scope reporting rather than an excuse |
| "A price is a property of a trade" | Leads on the mechanism, so the answer is obviously right rather than merely defensible |
| Oracle-consumer point, circularity named | The decisive fact — Blend sits *downstream* of a service like this one |
| Backstop 80/20 BLND:USDC AMM | Stated as **unmeasured**, so the section cannot be read as "we did not know Blend has any market at all" |
| "What would change the answer" | Stops the section reading as a permanent refusal, and prices the reversal (a `Venue` arm, extractor, registry seeding, backfill) |

Two lore files changed alongside: this one, and [[0128]]'s Notes carrying the
inherited AC. **Nothing outside `docs/` and `lore/`** — no extractor, no `Venue`
enum arm, no registry seeding, as the task required.

## Issues Encountered

- **Every AC pointed at a document that does not exist.** Found on activation,
  not mid-write. The ACs targeted "[[0128]]'s package"; 0128 is still backlog,
  there is no `milestone-2-evidence.md`, and `docs/scf/` holds only the M1 set.
  The sibling precedent [[0237]] is *also* still backlog, so it had not created
  the "deviations from the RFP" slot either and could not be copied from. Not a
  defect in the task — a consequence of small answers queuing behind a package
  that has not started. Resolved by the operator choosing a different
  destination; see Design Decisions.
- **A guessed link path.** The first draft of §5.7 cited
  `0217_RESEARCH_price-usd-aggregation-method.md`, a filename invented from the
  task's subject rather than read from disk. The real file is
  `0217_FEATURE_decide-whether-price-usd-is-outlier-protected.md`. Caught by
  checking both new links resolve before committing — worth keeping as the
  reason to check rather than recall.
- **Pre-existing broken link noticed, deliberately not fixed.** The overview
  still links `../lore/1-tasks/backlog/0017_FEATURE_local-clickhouse-for-prices-backfill.md`,
  which no longer exists. Out of scope for this task; unswept.

## Design Decisions

### From Plan

1. **Record the position, do not build an extractor.** The task existed to close
   a documentation exposure, not to add a venue. Held throughout.
2. **Volunteer Phoenix.** Naming a venue the RFP does not ask for, in the same
   table as the one we are missing, is what makes the entry read as scope
   reporting rather than as an excuse for a gap.

### Emerged

3. **Destination changed from the M2 package to the general design doc** —
   the operator's call, taken after being shown three options. The reasoning is
   the part worth keeping: *"Blend produces no trades" is a permanent fact about
   the protocol*, true whether or not SCF ever asks. Filing it as a submission
   artifact would have mis-shelved it **and** blocked it behind a task that has
   not started. The ACs were re-pointed and the change recorded in place rather
   than quietly rewritten.
4. **Written as a numbered section with a table, not prose.** The task asked for
   "one paragraph". A table was chosen because the AC wanted the ingested set
   stated as plainly as the missing one, and a prose sentence listing five
   venues with two attributes each is harder to check than a grid.
5. **Added a "what would change the answer" paragraph**, which the plan did not
   call for. Without it the section states a boundary with no exit, and a
   reviewer cannot tell a reasoned scope decision from a closed door.
6. **Added a Revision History row.** That document's own header says to append
   one when a change touches the architecture or scope framing; a new section
   recording a venue-scope boundary qualifies.
7. **Recorded 0128's inherited AC in 0128's Notes rather than spawning a backlog
   task.** It is one sentence of debt inside a task that already exists and is
   already scoped to write this package — a separate task would be overhead, and
   a shared-sequence ID spent on a cross-reference.

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
