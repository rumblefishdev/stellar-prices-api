---
title: "Soroswap produced no candles for five days (2026-07-06 -> 07-11) - measured on production"
type: research
status: mature
tags: ["soroswap", "amm", "ingestion", "data-correctness", "measurement", "prod"]
links:
  - "../README.md"
  - "../../../../../packages/soroswap-extractor/src"
  - "../../../../../packages/prices-ledger-processor/src"
history:
  - date: "2026-09-08"
    status: mature
    who: okarcz
    note: >
      Measured while scoping 0176/0264 for the Milestone 2 package. Found by
      locating candles in ledger space via intDiv(version, 1000) rather than by
      timestamp, which is what exposed the alignment to the backfill handoff
      floor. Disclosed in milestone-2-evidence.md section 8 with "cause under
      investigation" and a destination of before Tranche 3.
  - date: "2026-09-11"
    status: mature
    who: okarcz
    note: >
      Carried here verbatim when task 0271 was folded into 0101 and archived as
      superseded. Kept as written: it was filed without knowing 0101 already
      carries a diagnosed cause for the same darkness, and the 07-11 vs 07-15
      contradiction between the two is the first thing 0101 has to settle. The
      implementation and acceptance-criteria sections below are superseded by
      0101's; the evidence is not.
---

> 📌 Folded into [[0101]] on 2026-09-11. Filed as task 0271, now archived as
> superseded. Body preserved verbatim below.

# Soroswap goes dark for five days, starting at the backfill handoff

## Summary

`soroswap` wrote **no candles at all** between 2026-07-06 and 2026-07-11 while
`phoenix` and `aquarius` produced normally throughout the same window. The gap is
in the ledger range **live ingestion owns**, not the backfill's, and it begins
essentially at the boundary where the two hand over.

Durable, not a `_1m` retention artefact — it is present in `price_ohlcv_1d`.

## Evidence

Measured on production 2026-09-08 as `dev_read`:

| boundary                                   | ledger         |
| ------------------------------------------ | -------------- |
| Soroswap's last candle before the gap       | **63,352,574** |
| documented backfill handoff floor           | **63,352,611** |
| Soroswap's first candle after the gap       | **63,433,850** |

**37 ledgers.** Soroswap stops immediately below the floor the combined backfill
was told to stop at (`--end` = SDEX live floor − 1, per the 0088 runbook), and
does not resume for ~81,239 ledgers.

Daily counts over the window, from `price_ohlcv_1d` — the forever table:

```
day          soroswap  phoenix  aquarius
2026-07-06          6        6       113
2026-07-07          0        7       128
2026-07-08          0        5       130
2026-07-09          0        5       107
2026-07-10          0        4       116
2026-07-11         10        4       110
```

⚠️ **The alignment is the finding, not the zero.** Soroswap is low-volume enough
that a five-day trading lull is not absurd on its own. What is not plausible is a
lull that begins within 37 ledgers of a handoff boundary nobody chose for market
reasons. That is the thing to falsify first.

## Context

The backfill delivered Soroswap right up to its floor. Live ingestion then
produced Phoenix and Aquarius but no Soroswap for five days. So this is a
live-path defect, and the backfill is not implicated.

Nearest known mechanism: [[amm-live-pool-registry-preload-gap]] — the live
processor constructs `Registries::new()` empty, and swaps for unregistered pools
are **dropped silently rather than erroring**. If the Soroswap pool registry was
unseeded across a restart, that is exactly this shape: one venue dark, the others
unaffected, no error anywhere.

## Implementation

- Falsify the trading-lull explanation first: check Soroswap swap **events** in
  the raw ledger range, not candles. If swaps exist and candles do not, ingestion
  dropped them; if no swaps exist, the venue was genuinely quiet and this task
  closes as a non-defect.
- If ingestion dropped them, establish why the other two venues were unaffected.
  The pool-registry preload gap is the first hypothesis, not the conclusion.
- Decide whether to backfill the range. ⚠️ **It is live-owned**, so a repair is
  the same-source overlap hazard from [[backfill-live-no-code-coordination]]:
  minutes straddling the range edges land as partial rows and silently
  undercount. The CLI has **no per-venue filter** — `--mode` is `combined` or
  `sdex-only` — so a run that fills Soroswap necessarily rewrites Phoenix and
  Aquarius rows that live already wrote correctly. Either add the filter or find
  another repair path.
- Fix the cause before refilling. A refill without it recurs at the next restart.
- Sweep for other instances: this was found by accident in one 5-week window. The
  same query shape over the full AMM range would say whether it has happened
  before.

## Acceptance Criteria

- [ ] Trading-lull vs dropped-ingestion is settled from raw swap events, said
      plainly either way.
- [ ] If ingestion: the mechanism is identified and fixed, and it is stated why
      Phoenix and Aquarius were unaffected.
- [ ] A decision is recorded on repairing the range, with the overlap hazard
      addressed rather than ignored.
- [ ] A sweep over the full AMM range reports whether other instances exist.
- [ ] `milestone-2-evidence.md` §8's row is updated to the outcome — it currently
      reads "cause under investigation" and promises resolution before Tranche 3.

## Notes

- 🔑 **How this was found, worth reusing:** candles carry no ledger column, but
  `version = ledger_seq * 1000 + op_index`, so `intDiv(version, 1000)` locates any
  candle in ledger space. Comparing that against the backfill's documented floor
  is what turned a curiosity into a 37-ledger alignment.
- ⚠️ Do not investigate this in `price_ohlcv_1m` — see
  [[amm-history-is-not-in-price-ohlcv-1m]]. AMM history lives in the coarse
  tables; `_1m` holds non-SDEX rows only from 2026-07-01.
