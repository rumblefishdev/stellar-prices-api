---
id: "0232"
title: "The RFP types Current Price as a float and we ship a string — record the answer before a reviewer asks"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0128", "0120", "0123", "0217"]
tags: [layer-docs, priority-medium, effort-small, milestone-M2, scf, submission, evidence, api]
milestone: 2
links:
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

- [ ] The position is recorded with its reasoning and its evidence citations
      (0123's 7e-8 measurement, 0120's round-trip assertions)
- [ ] [[0128]]'s package carries a "deviations from the RFP" entry naming
      `Current Price (float USD)` and quoting the RFP line
- [ ] `price_usd`'s published OpenAPI description states the string form and
      the precision reason
- [ ] The entry states that the answer covers every Decimal-valued field, not
      just this one

## Notes

- Deliberately **not** an API change. If the decision goes the other way the
  API work is a separate task; this one owns the recorded answer.
- Related but distinct: [[0217]] asks how `price_usd` is *computed* (the RFP's
  weighted-average reading). This task is only about how it is *typed on the
  wire*.
