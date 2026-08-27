---
id: "0230"
title: "The 0120 conformance suite fails by design on ADR 0011 §5 unpriced buckets — and flakes, because enrichment lag decides which assets have one"
type: TEST
status: backlog
related_adr: ["0011"]
related_tasks: ["0120", "0170", "0225", "0128", "0145"]
tags: ["priority-medium", "effort-small", "testing", "api", "read-surface", "ohlcv", "milestone-M2"]
milestone: 2
links:
  - "../../../tools/scripts/conformance-0120.mjs"
history:
  - date: 2026-08-27
    status: backlog
    who: okarcz
    note: >
      Found by the [[0120]] conformance re-run that verified [[0170]] on prod:
      17 failures of `all OHLCV values are decimal strings`, none of them a
      defect in the API. The suite predates [[ADR-0011]] §5, which made a
      present-but-unpriced bucket a documented response shape. ⚠️ Three of the
      17 (USDCAllow, VELO, SHX) **passed on a re-check twelve minutes later**,
      because the unpriced bucket had been enriched in the meantime — so the
      check is not merely wrong, it is non-deterministic, which is exactly the
      flakiness [[0128]] exists to keep out of this suite.
---

# The suite asserts a shape the contract no longer promises

## Summary

`conformance-0120.mjs:450` asserts that `open`, `high`, `low`, `close`,
`volume_base`, `volume_quote_usd` and `vwap` are **all** decimal strings on
**every** bucket:

```js
if (!(typeof c[f] === 'string' && NUM_RE.test(c[f]))) numeric = false;
```

ADR 0011 §5 made that false on purpose. A bucket that traded but cannot yet be
priced is returned **present with null price fields**, keeping `volume_base` and
`trade_count` real — that is what distinguishes *"traded, not yet priceable"*
from *"never traded"*, which is the distinction [[0170]] and [[0225]] were
fought over.

Measured example (EQL, `2026-08-25T11:00:00Z`):

```json
{"open":null,"high":null,"low":null,"close":null,
 "volume_base":"1","volume_quote_usd":"0","vwap":null,
 "trade_count":2,"method":null,"derived":null}
```

## 🔑 Worse than wrong — non-deterministic

Recent buckets are unpriced until enrichment catches up. So *which* assets carry
a null bucket depends on when the suite runs: **USDCAllow, VELO and SHX failed
at 13:26 and passed at 13:38** on 2026-08-27, unchanged code on both sides.

A conformance suite whose result moves with a background worker cannot be cited
as evidence, which is the entire purpose [[0128]] gives it. This is the same
hazard the [[0120]] task file already records for volume-sensitive assertions,
arriving on a new axis.

## Implementation

- Treat a bucket as **unpriced** when the price fields are null, and assert the
  null-ness is **all-or-nothing**: `open`, `high`, `low`, `close`, `vwap`,
  `method` and `derived` are either all null or all present. A *partially*
  null bucket is a genuine defect and must still fail.
- Keep asserting decimal strings on `volume_base`, `volume_quote_usd` and
  `trade_count` for every bucket — those never depend on the USD rate, which is
  the property that makes an unpriced bucket still worth returning.
- Apply the same treatment to `low <= open,close <= high`, which currently
  coerces `null` to `0` via `Number(null)` and so compares against a price of
  zero. ⚠️ Do not merely skip null buckets there — [[0229]] is a real ordering
  defect found by that same check, and it must keep failing.
- Report unpriced buckets as a **counted, non-failing observation** in the JSON
  report, so a run still surfaces how much of the window is awaiting enrichment
  rather than hiding it.

## Acceptance Criteria

- [ ] The suite passes against the deployed API with unpriced buckets present,
      and the report states how many were seen, per asset and granularity.
- [ ] A partially-null bucket still FAILS — asserted against a fixture, not
      only reasoned about.
- [ ] Two consecutive runs minutes apart produce the same pass/fail verdict for
      every asset, with only the unpriced counts allowed to differ.
- [ ] [[0229]]'s ordering failures still fail — this task must not mask them.
