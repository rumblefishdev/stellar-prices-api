---
id: "0199"
title: "oracle_prices holds rows timestamped in 1970 — a millisecond reading divided by 10^6 instead of 10^3"
type: BUG
status: superseded
related_adr: []
related_tasks: ["0196", "0167", "0154", "0227"]
tags:
  ["priority-medium", "effort-small", "oracle", "data-correctness", "clickhouse", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Found while measuring the 0196 purge set. Both canonical USDC and canonical
      USDT reported first_seen = 1970-01-21 15:41:56 in prices.oracle_prices —
      identical to the second across two independent assets, so systematic rather
      than one corrupt row. USDT's copy was deleted with the 0196 purge; USDC's is
      untouched and still investigable.
  - date: 2026-08-26
    status: superseded
    who: okarcz
    by: ["0227"]
    note: >
      Folded into [[0227]], which has the same defect with the code site
      identified (`oracle-worker/src/lib.rs:298`, an unconditional `/1000`
      applied to a value already in seconds) and a proven x1000 reconstruction
      of the corrupted timestamps. This task's contributions carried over: the
      independent /10^6-vs-/10^3 arithmetic that reached the same mechanism from
      the number alone; the `min(timestamp)` coverage trap that already gave
      [[0167]] a wrong start date; `raw_data` as the upstream-vs-ours
      discriminator; and the 13-month retention question.
      ⚠️ Two of this task's positions were RETIRED in the fold, both recorded in
      0227 rather than dropped. (1) "It is inert" — the claim that a 1970 row can
      never win the enrichment ASOF join is reasoning from the staleness bound,
      not a measurement; 0227 carries it as an OPEN question that gates severity.
      (2) The AC asking for the bad rows to be DELETED as unrecoverable is
      superseded — the x1000 mapping is now proven, so they are repairable.
---

> **Superseded by [[0227]]** (2026-08-26). One defect, three filings — see also
> [[0086]] (2026-07-06), archived the same day. Findings consolidated into
> `lore/1-tasks/active/0227_BUG_oracle-timestamp-divided-by-1000-twice-when-reflector-sends-seconds.md`.
> ⚠️ Two claims below were retired in the fold: "it is inert" (unmeasured — 0227
> treats it as open) and the delete-the-rows AC (they are recoverable). Kept here
> for history; read 0227 for the current position.

# `oracle_prices` has rows timestamped in 1970

## What was seen

Measuring the [[0196]] purge set on prod 2026-08-13:

```
asset_code  issuer     first_seen            last_seen
USDC        GA5ZSEJY   1970-01-21 15:41:56   2026-08-13 11:05:00
USDT        GCQTGZQQ   1970-01-21 15:41:56   2026-08-13 11:00:00
```

**Identical to the second across two assets** that are polled independently, so
this is a systematic conversion fault, not a one-off corrupt row.

## The arithmetic points at a divisor

`1970-01-21 15:41:56` ≈ **1,784,516 epoch seconds**. A mid-2026 instant is
≈ 1,786,000,000 seconds ≈ 1.786 × 10¹² milliseconds. Dividing the millisecond
value by **10⁶** instead of **10³** lands almost exactly on the observed value.

So the likely shape is a Reflector reading whose timestamp is in milliseconds
being converted with the wrong scale somewhere between the SEP-40 decode and the
`DateTime` column — plausibly a double division, since one correct `/1000` plus
a second one gives the same result.

⚠️ Unverified — this is inference from one number, not a read of the code path.
Confirm against `oracle-worker` and the `update`-event decode in `soroban.rs`
before changing anything.

## Why it is worth fixing despite being harmless today

The enrichment oracle tier joins `ASOF … o.timestamp <= p.timestamp` with a
staleness bound, so a 1970 row never matches a real candle and cannot poison a
price. It is inert.

But:

- **It is a live conversion bug**, and the same code path produces the timestamps
  that *do* get used. A divisor that is wrong sometimes is worth understanding
  before it is wrong when it matters.
- **It defeats `min(timestamp)` as a coverage measure.** [[0167]]'s whole
  argument turned on when oracle coverage starts; a 1970 row makes the naive
  query answer "1970" and hid the real start date once already.
- **It survives retention.** `oracle_prices` prunes at 13 months, which should
  have removed a 1970 partition long ago. That it is still present suggests the
  retention job's window predicate does not reach it — worth checking, because
  the same gap would strand any other out-of-range partition.

## Where to look

- How many rows, and are they confined to one partition?
  ```sql
  SELECT toYYYYMM(timestamp) AS part, count(), uniqExact(asset_id), min(timestamp), max(timestamp)
  FROM prices.oracle_prices
  WHERE timestamp < '2020-01-01'
  GROUP BY part ORDER BY part;
  ```
- Do the same rows exist in `prices.usd_rate`? **Measured: no** — `usd_rate`'s
  min for USDT was a sane `2026-03-11 14:00`. The snapshot copy either filters
  them or never saw them; knowing which narrows the source.
- Is the bad value present in `raw_data`, or introduced during decode? The column
  keeps the original payload, which should settle whether the fault is upstream
  or ours.

## Acceptance Criteria

- [ ] Row count and partition spread established, and whether any asset other
      than USDC/USDT is affected
- [ ] The conversion site identified — poll path, event-decode path, or both
- [ ] Fixed, with a test pinning a known millisecond reading to its expected
      `DateTime`
- [ ] Existing bad rows removed (they are unusable at any scale — the true
      instant is not recoverable from them alone unless `raw_data` carries it)
- [ ] Whether the 13-month retention job should have dropped them, answered
