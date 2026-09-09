---
id: "0237"
title: "The RFP types Current Price as a float and we ship a string — record the answer before a reviewer asks"
type: DOCS
status: completed
related_adr: ["0011"]
related_tasks: ["0262", "0127", "0248", "0128", "0120", "0123", "0217"]
tags: [layer-docs, priority-medium, effort-small, milestone-M2, scf, submission, evidence, api]
milestone: 2
links:
  - "../../../docs/scf/milestone-2-rfp-deviations.md"
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from the 2026-08-28 read of the original SCF RFP (`RFP 1: Prices
      API`), which types the field as "Current Price (float USD)" while §3.3
      serialises every Decimal as a JSON string. Noted in [[0217]] as
      deliberately out of that task's scope (it is serialization, not outlier
      protection) and left without an owner until now.
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Renumbered 0232 -> 0237 after syncing with `develop`, which had
      already taken 0232 (0193's renumbering of its own spawns). Same
      collision 0193 hit twice: the shared id sequence is allocated from a
      local view of the tree, so two branches in flight pick the same next
      free number. Nothing references the old id outside this branch.
  - date: 2026-09-04
    status: active
    who: okarcz
    note: >
      Activated as the third instance of one pattern, now handled twice in a
      day: a promise we cannot meet literally, for a good reason nobody wrote
      down. The first two were Tranche 2 AC 3's `X-Cache` wording ([[0262]],
      ADR 0012) and USDC's exclusion from the backfill spot-check ([[0127]],
      `docs/prices-api-backfill-depth-verification.md` §6). All three land in
      the same place — [[0128]]'s deviations section — so this is done before
      0128 rather than discovered mid-write.
      ⚠️ The evidence for the answer already exists and is only unconnected:
      the 7e-8 price measured on prod during [[0123]], and [[0120]]'s
      conformance assertions that every numeric string parses and that
      `Decimal(38,14)` survives the JSON round-trip. This task is writing, not
      measuring — resist re-deriving what is already recorded.
  - date: 2026-09-04
    status: completed
    who: okarcz
    note: >
      Closed — 4 of 4. Deliverable is
      `docs/scf/milestone-2-rfp-deviations.md` (PR #287, `9ca2729`), which
      records the float-vs-decimal-string answer and gathers the other two M2
      deviations already decided, so [[0128]] folds in one file rather than
      three. 🔑 The argument leads with a **measured production defect**, not
      with theory: ADR 0011's BTC 1h finding, where deriving through
      `toFloat64` returned a `close` below its own `low` by 1.343e-11 — 0.92 of
      one float64 ulp on a value carrying 19 significant digits against
      float64's 15-16. One internal conversion did that; publishing floats
      would impose it on every consumer, field and request. Supporting
      citations: 7e-8 on RON ([[0123]]), a `close` of 5e-14 five ticks above the
      `Decimal(38, 14)` floor, and [[0120]]'s verified *"Decimal(38,14) strings
      parse everywhere"*. Two things nothing had recorded: the rule covers every
      Decimal-valued field rather than the one the RFP names, and counts and
      ledger sequences deliberately stay plain integers because they are exact
      in a float — making it a precision decision, not a stylistic one. The
      OpenAPI now states the reason on `price_usd` itself rather than only in
      the API-level blurb. No API behaviour changed, as the task required.
---

# "Current Price (float USD)" vs our Decimal strings

## Summary

The RFP's Asset Metadata Required list types the field as **`Current Price
(float USD)`**. Every numeric field we publish — `price_usd`, `vwap_24h`,
`volume_24h_usd`, the `sources` values, OHLCV candles — is a JSON **string**,
by the deliberate §3.3 design point ("numeric values serialised as strings to
preserve `Decimal(38,14)` precision").

This is a literal deviation from the RFP on a field the RFP names explicitly.
The deviation is almost certainly right; what is missing is a **recorded
answer**, so it is defended from evidence at review time rather than
improvised.

## Context

The justification is already measured, it is just not written down anywhere a
reviewer reads:

- A JSON number is an IEEE-754 double in every mainstream parser. Prices on
  this store reach **7e-8** (RON, measured on prod during [[0123]]'s asset
  selection), and `Decimal(38,14)` carries fourteen fractional digits by
  design — a float round-trip silently destroys the low-order digits of
  exactly the long-tail assets the RFP asks us to cover.
- [[0120]]'s conformance suite already asserts the other half: every numeric
  string parses, and `Decimal(38,14)` precision survives the JSON round-trip.
  That is evidence the string form works, sitting in a report nobody has
  connected to this question.
- The M1 package set the precedent of answering deviations in the evidence
  doc rather than in the code comments; this is the M2 instance of the same
  pattern.

## Implementation

- Decide and record the position. The expected answer is **keep strings**,
  with the precision argument and the 7e-8 measurement as backing; if the
  team instead wants a numeric field, that is an API change and belongs in
  its own task, not here.
- Write it into the [[0128]] package as a short "deviations from the RFP"
  entry — field named, RFP wording quoted, reason given, evidence cited
  (0123's measurement + 0120's round-trip assertions).
- Add the same one-liner to the OpenAPI description of `price_usd`, so a
  consumer reading the spec learns it is a decimal string and why, without
  the evidence doc.
- Check the scope while there: the RFP types **only** this one field, but the
  answer covers every Decimal-as-string field we publish. Say so once rather
  than per field.

## Acceptance Criteria

- [x] The position is recorded with its reasoning and its evidence citations —
      **done 2026-09-04**, `docs/scf/milestone-2-rfp-deviations.md` §1 (PR #287).
      🔑 Carries a **stronger citation than this task knew about**: ADR 0011
      records a *measured production defect* from exactly this mechanism —
      deriving through `toFloat64` returned a `close` **below its own `low`** by
      **1.343e-11** at BTC 1h, 0.92 of one float64 ulp, on a value carrying 19
      significant digits against float64's 15-16. That was one internal
      conversion; publishing floats imposes it on every consumer, every field,
      every request. Plus the two citations this task listed: 7e-8 on RON
      ([[0123]]) and [[0120]]'s verified *"`Decimal(38,14)` strings parse
      everywhere"*. A third turned up alongside — a `close` of `5e-14`, five
      ticks above the `Decimal(38, 14)` floor.
- [x] A "deviations from the RFP" entry names `Current Price (float USD)` and
      quotes the RFP line — **done 2026-09-04**. ⚠️ Written **standalone** at
      `docs/scf/milestone-2-rfp-deviations.md` rather than into [[0128]], which
      has not started: the same reasoning [[0248]] and [[0122]] settled — an
      evidence artefact that stands on its own should not be blocked behind an
      unstarted package. 🔑 It also gathers the **other two** M2 deviations
      already decided (the `X-Cache` wording, USDC's exclusion), so 0128 folds
      in one file rather than three.
- [x] `price_usd`'s published OpenAPI description states the string form and
      the precision reason — **done 2026-09-04** (PR #287). It was already
      stated at the **API level** (`openapi/mod.rs`) and on `PriceResponse`'s
      schema summary, but **not on the field itself**, which is what a reader
      following a `price_usd` reference actually lands on. Now carries the
      `Decimal(38, 14)` vs IEEE-754 digit counts, the 7e-8 floor, and the
      guidance to parse with a decimal type. `AssetListItem.price_usd` points at
      it rather than repeating it.
- [x] The entry states that the answer covers every Decimal-valued field, not
      just this one — **done 2026-09-04**. Named explicitly: `vwap_24h`,
      `volume_24h_usd`, the per-venue `sources` values and every OHLCV
      `open`/`high`/`low`/`close`. 🔑 It also states the converse, which nothing
      had recorded: **counts and ledger sequences stay plain JSON integers**,
      because they are exact in a float and nothing is lost — so the rule is a
      precision decision, not a blanket stylistic one.

## Design Decisions

### Emerged

1. **The deviations document is standalone, not written into [[0128]].**
   [[0128]] has not started, and the same reasoning [[0248]] and [[0122]]
   settled applies: an evidence artefact that stands on its own should not be
   blocked behind an unstarted package. 0128 cites it, as it cites the load
   test, cache and backfill-depth reports.

2. **It hosts all three M2 deviations, not only this one.** This task owns the
   float-vs-string answer; the `X-Cache` wording ([[0262]], ADR 0012) and USDC's
   exclusion ([[0127]]) were already decided and written elsewhere. Gathering
   them into one document with §2 and §3 as summaries-plus-pointers means 0128
   folds in **one** file, and a reviewer reading about deviations finds all of
   them in one place. The source documents are not duplicated or rewritten.

3. **The argument leads with the measured defect, not with the theory.** The
   obvious way to write this is "floats have 53-bit mantissas, therefore…".
   ADR 0011's BTC 1h finding is stronger: it is *our own system*, on
   *production*, already broken once by exactly this mechanism, with the ulp
   arithmetic done. A reviewer can dismiss a general principle; a measurement
   from the codebase under review is harder to wave away.

4. **The "strictly more capable" framing is deliberate.** `parseFloat` on our
   string yields exactly what the RFP's literal reading would have delivered,
   while the reverse is unrecoverable. That reframes the deviation from *"we
   did not do what was asked"* to *"we delivered a superset"*, which is both
   true and the right thing for a reviewer to hear first.

5. **A leftover from PR #283 was fixed in passing.** The `SdexStream` schema
   description still read *"walks the ledger history in order"* — the same
   forward-orientation error #283 corrected in the route summary and the field
   descriptions, missed because it lives in the schema-summary table rather than
   the field table. Unrelated to this task's subject; too small and too adjacent
   to leave.


## Future Work

None spawned. The task was a recorded answer, and it is recorded.

⚠️ **One thing for [[0128]] to carry, not a task**: the deviations document is
written to be **folded in, not linked past**. Its §2 and §3 are deliberately
summaries that point at the fuller reports
(`prices-api-cache-verification.md`, `prices-api-backfill-depth-verification.md`);
if 0128 reproduces those in full it will have three copies of each argument
drifting apart. Cite the document, keep the pointers.


## Notes

- Deliberately **not** an API change. If the decision goes the other way the
  API work is a separate task; this one owns the recorded answer.
- Related but distinct: [[0217]] asks how `price_usd` is *computed* (the RFP's
  weighted-average reading). This task is only about how it is *typed on the
  wire*.
