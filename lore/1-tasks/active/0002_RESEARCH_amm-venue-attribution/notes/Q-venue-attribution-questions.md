---
title: "Open questions for AMM venue attribution"
type: question
status: seed
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, venue-attribution]
links:
  - "../../../archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
history:
  - date: 2026-05-08
    status: seed
    who: claude
    note: "Captured open questions from 0001 'Open questions for venue attribution' section."
---

# Questions

## Q1 — `Symbol("swap")` emitters (44 distinct in wider sample)

Which venue does each of these belong to? Top 5 by event volume:

| Events | Contract |
|---:|---|
| 11,947 | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` |
| 4,128 | `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX` |
| 2,706 | `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` |
| 2,480 | `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` |
| 440  | `CBCZGGNOEUZG4CAAE7TGTQQHETZMKUT4OIPFHHPKEUX46U4KXBBZ3GLH` |

Source: `R-swap-topic-shapes.md` §"`Symbol("swap")` revisited".

Hypothesis: top emitters are routers/aggregators (high per-contract
volume); long tail is more varied. Phoenix could be hidden inside this
set per `R-swap-topic-shapes.md`.

## Q2 — `Symbol("trade")` emitters (29 distinct)

Are all 29 Aquarius pools? Or a mix of venues? Sample emitter from
3.5-day window: `CA6PUJLBYK...` (full ID needs to be re-extracted from
evidence/trade_event_sample.json if needed).

Hypothesis: Aquarius pools — the `update_reserves` co-emission pattern
(29/29 overlap) is consistent with a single protocol family.

## Q3 — `String("SoroswapPair")` emitters (79 distinct in wider sample)

These are confirmed Soroswap pool contracts by the topic name, but can
we derive them from a Soroswap factory? What is the factory address?

## Q4 — Phoenix presence

Why does Phoenix not appear in the wider sample under any obvious
PascalCase topic? Possibilities (per `R-swap-topic-shapes.md`):

1. Phoenix is hidden inside the 44 `Symbol("swap")` emitters (its open
   source code emits `Symbol("swap")` from pool contracts).
2. Phoenix activity in this 4-day window was below the noise floor.
3. Phoenix uses a topic name we still haven't seen.

Resolution: locate Phoenix factory address; match against the 44 `swap`
emitters.
