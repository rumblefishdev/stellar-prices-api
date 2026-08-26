---
id: "0227"
title: "~30% of oracle readings land at 1970-01-21 — the Reflector timestamp is divided by 1000 unconditionally, and it is not always milliseconds"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0167", "0173", "0170", "0061", "0141"]
tags: ["priority-high", "effort-small", "oracle", "data-correctness", "enrichment", "ops", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-08-26
    status: backlog
    who: okarcz
    note: >
      Found while checking an unrelated `usd_rate` coverage question for
      [[0170]]. A monthly histogram of `oracle_prices` returned a `1970-01-01`
      bucket holding 3,186 rows per asset — with entirely plausible PRICES
      (XLM 0.1526-0.2199, USDC 0.9999-1.0013) and only the timestamp wrong.
      Root cause is ours: `oracle-worker/src/lib.rs:298` divides Reflector's
      timestamp by 1000 unconditionally. Reflector began returning some readings
      already in SECONDS around 2026-07-20; dividing those again yields
      1.787e6 = 1970-01-21. Nothing in our code changed at that date — the git
      log for that window is 0095/0097/0088/0096, all rollups and backfill — so
      the payload changed upstream and we had no tolerance for either shape.
      ⚠️ The same unconditional divide exists a second time at
      `prices-ingest-core/src/soroban.rs:667`, which ships in a DIFFERENT stack.
      Fixing one and deploying is the silent half-fix recorded in
      [[oracle-writers-span-two-stacks]].
      Ongoing at the time of filing: today's partial count is 74 rows.
---

# Reflector timestamps are divided by 1000 whether or not they are milliseconds

## Summary

`prices.oracle_prices` holds **6,372 rows** (3,186 per asset, XLM and USDC) whose
timestamp reads **1970-01-21**. The prices on those rows are fine. Only the
timestamp is destroyed, by our own unit conversion.

Since roughly **2026-07-20** about **30% of every day's oracle readings** land
this way. It is still happening.

## The defect

`packages/oracle-worker/src/lib.rs:298`:

```rust
// Reflector reports millisecond timestamps; oracle_prices.timestamp
// is DateTime (epoch seconds), so divide by 1000 to match the
// event-decoded path (prices-ingest-core soroban.rs). The clamp is
// a backstop for the 2106 u32 ceiling, not the unit conversion.
timestamp: (pd.timestamp / 1000).min(u32::MAX as u64) as u32,
```

The comment states the assumption and the code never checks it. A value already
in seconds (~1.787 × 10⁹) becomes ~1.787 × 10⁶, which is 1970-01-21.

🔑 **This is not a regression we shipped.** No commit touched `oracle-worker`
around 2026-07-20. Reflector changed what it sends; the divide had no tolerance
for it. A magnitude check would have absorbed the change silently.

## 🔴 The same divide exists twice, in two different stacks

| site | stack | shape |
|---|---|---|
| `oracle-worker/src/lib.rs:298` | **EventBridge** | `(pd.timestamp / 1000)` |
| `prices-ingest-core/src/soroban.rs:667` | **Compute** (ledger-processor) | `(ts_ms / 1000) as u32` |

⚠️ Deploying one is a **silent half-fix** — see [[oracle-writers-span-two-stacks]].
The event-decode path was not measured for this defect; whether it is affected
depends on whether the event payload carries the same shape as the RPC response,
and that is an open question, not an assumption to carry.

## Evidence — measured on prod 2026-08-26

Reconstructing the corrupted timestamps by multiplying by 1000 gives a clean,
consecutive daily series from **2026-07-20 to 2026-08-26**, at a metronomic
172-174 rows/day across both assets:

```
2026-07-20   152      <- partial first day
2026-07-21   174
   …          …
2026-08-13   148      <- Hetzner disk-full incident
2026-08-14   120      <- 11.5 h ingest stall
   …          …
2026-08-26    74      <- partial, still accruing
```

🔑 **The reconstruction validates itself.** Rows for 08-13 and 08-14 dip to 148
and 120 against an otherwise flat 172-174 — that is the known Hetzner disk-full
stall showing through. A wrong ×1000 mapping would not reproduce a real outage on
the correct dates, so the mapping is established rather than merely plausible.

⚠️ **The rate figure was corrected once already.** 3,186 of 51,235 readings per
asset is 6.2% *across all history*, but the corruption spans only five weeks —
within that window it is ~86 of ~288 readings per asset per day, i.e. **~30%**.
Quote the window figure; the all-time one understates the live problem.

## Impact — contained, but not harmless

✅ **Nothing corrupt reached `prices.usd_rate`.** It holds 48,049 rows for USDC
against 51,235 oracle readings, and `51,235 − 3,186 = 48,049` exactly. The
snapshotter rejects every affected row.

❌ **~30% of the rate readings for the last five weeks are being discarded**,
thinning the ASOF join's coverage during precisely the period the depeg-aware
oracle tier exists to cover. A candle enriched in that window had fewer rate
points to match against than it should have.

⚠️ **Not yet established:** whether a 1970 row can ever WIN the enrichment
`ASOF LEFT JOIN`. It is at-or-before every candle ever recorded, so it is a
candidate for every row — the staleness window should reject it, but that has not
been verified. If the window is applied loosely this stops being a data-loss bug
and becomes a wrong-value bug. **Check this before anything else**; it changes the
severity.

## Implementation

- Magnitude check at both sites rather than a unit assumption: treat a value
  ≥ 10¹¹ as milliseconds and divide, otherwise take it as seconds. Bounds chosen
  so both shapes are unambiguous for any date this system can serve.
- ⚠️ Do **not** fix only `oracle-worker`. Establish the event-decode path's
  actual payload shape first, then fix and deploy both stacks or state explicitly
  why one is exempt.
- Repair the 6,372 existing rows. The ×1000 mapping is proven, so they are
  recoverable rather than lost — but `oracle_prices` is an input to enrichment,
  so a repair means deciding whether affected candles get re-enriched.
- Add a guard so this cannot recur silently: a reading whose timestamp is
  implausible for this system (before the oracle window, or in the future)
  should be **rejected loudly**, not written.

## Acceptance Criteria

- [ ] Whether a 1970 row can win the enrichment ASOF join is **established by
      measurement**, and the answer is recorded. This gates the severity.
- [ ] Both conversion sites handle either unit, with a test per site covering a
      seconds input, a millis input, and the boundary between them.
- [ ] The event-decode path's real payload shape is stated as evidence, not
      assumed from the oracle-worker comment.
- [ ] Whatever ships is deployed to **both** stacks, or the exemption is written
      down with its reasoning.
- [ ] New readings stop landing before the oracle window — verified on prod after
      deploy, not from the code.
- [ ] The 6,372 existing rows are repaired or explicitly written off, with the
      decision recorded.
- [ ] A malformed timestamp is rejected loudly rather than written, with a test.

## Out of scope

- [[0173]]'s USDT mis-attribution — a different oracle defect on the same table.
- [[0228]] — XLM's readings never being snapshotted. Found in the same session and
  on the same table, but a design question rather than a bug.
- Re-enriching candles, unless the ASOF finding above shows wrong values were
  written.

## Notes

- Found by a query aimed at something else entirely. The monthly histogram was
  checking `usd_rate` coverage for [[0170]]; the `1970-01-01` bucket was not what
  it was looking for.
- ⚠️ **Nothing value-based would ever have caught this.** The prices on the
  affected rows are entirely plausible — XLM at 0.15-0.22, USDC at ~1.00. Only
  the timestamp is wrong, and no alarm or guard reads timestamps for plausibility.
  Same class as [[0215]]'s invisible failure, where every data signal read normal.
