---
id: "0228"
title: "XLM's measured USD rate is fetched every 5 minutes and thrown away, while 11 M candles are priced by deriving it indirectly through USDC"
type: BUG
status: backlog
related_adr: ["0011"]
related_tasks: ["0167", "0170", "0172", "0182", "0061", "0227", "0173"]
tags: ["priority-medium", "effort-medium", "oracle", "enrichment", "data-correctness", "usd", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-08-26
    status: backlog
    who: okarcz
    note: >
      Found while checking `usd_rate` coverage for [[0170]]. Measured on prod:
      `oracle_prices` holds **51,235 Reflector readings for XLM** and
      `prices.usd_rate` holds **zero** rows for it, against 48,049 for USDC.
      `peg_identities()` is exactly canonical USDC by design and pinned by a
      test, so XLM is never snapshotted — and `oracle_prices` is pruned at
      INTERVAL 13 MONTH, so those readings age out permanently.
      Meanwhile the enrichment pivot tier prices ~11 M XLM-quoted candles using
      "XLM's volume-weighted close against USDC", i.e. deriving XLM's dollar
      value through our own candle data, which is dollars only because USDC is
      assumed to be $1. We measure the thing directly, discard the measurement,
      and then infer it.
      ⚠️ Filed as a design question, not a wrong-number claim. Whether the
      derived value materially differs from the measured one is UNMEASURED and
      is the first acceptance criterion.
---

# We measure XLM's dollar price, throw it away, then derive it from USDC

## Summary

Reflector reports XLM's USD price every 5 minutes and we keep **none of it**. The
readings sit in `prices.oracle_prices` until the 13-month retention prunes them,
and are never snapshotted into the forever-retained `prices.usd_rate`.

At the same time, the enrichment **pivot tier** prices every XLM-quoted candle by
deriving XLM's dollar value from XLM/USDC candles — an indirect route that is
dollars only because USDC is assumed to be $1.

So the direct measurement is discarded and an inferred substitute is used in its
place, on the reference asset that the largest part of the store depends on.

## Measured on prod — 2026-08-26

```
code   oracle_readings   oldest                newest                usd_rate_rows
XLM    51,235            1970-01-21 15:41:56   2026-08-26 10:00:00   0
USDC   51,235            1970-01-21 15:41:56   2026-08-26 10:00:00   48,049
```

⚠️ The `1970` oldest is [[0227]]'s timestamp defect, unrelated to this task.
The `48,049` vs `51,235` gap is the same defect — it is not evidence of a
snapshotting problem for USDC.

🔑 **The number that matters is XLM's `0`.** It is not a shortfall; nothing is
written for XLM at all.

## Why it is zero — by design, and the design is narrow

`oracle-worker/src/lib.rs` snapshots only `peg_identities()`, which is exactly
canonical USDC and nothing else. There is a test pinning that:

> *"the peg set must be exactly canonical USDC. Adding a member is a claim that
> its oracle feed names that ISSUER, not just that code — write the evidence in
> the doc comment above before changing this."*

That test is **correct and should not be weakened**. It exists because of
[[0173]]: Reflector prices the *ticker* USDT — Tether's own token, genuinely at
par — and filing that under the Stellar issuer's address asserts ~$1.00 for an
asset worth $0.13. The lesson is that a symbol is not an identity.

⚠️ So this task is **not** "add XLM to `peg_identities()`". XLM is not a peg, and
the set is named for what it is. The question is whether a *second*, differently
named path should snapshot measured non-peg rates — and what the evidence is that
Reflector's `XLM` symbol names our native asset rather than something else.

## What the pivot does instead

From `ch_enrich.rs`:

> **pivot:** a candle quoted in a *measured* reference asset gets
> `close_usd = close × ref_usd`, where `ref_usd` is that asset's
> **volume-weighted close against USDC** at or before the bucket, forward-filled
> by an `ASOF LEFT JOIN`.

Follow it through: XLM's dollar value comes from XLM/USDC candles, and those are
dollars only via the $1 peg. So ~11 M XLM-quoted candles rest on the peg
assumption one hop away — while an independent measurement of the same quantity
exists and is discarded.

⚠️ **This is a structural observation, not a measured error.** The derived value
may track the measured one closely. Nobody has checked. That check is AC 1 and it
decides whether this task is worth anything at all.

## Why it could matter

- **[[0172]]/[[0182]] is the precedent.** Assuming a stablecoin sits at par cost
  567,760 corrected candles when USDT turned out to trade at ~$0.13. The USDC peg
  is a much better assumption than the USDT one was — but "much better" is what
  was believed about USDT too, and the answer came from measuring.
- **[[ADR-0011]] leans on the rate being measured.** Its load-bearing argument is
  that the USD-per-USDC rate wobbles 0.9976-1.0008 and so a flat $1 is wrong. The
  same reasoning applies to a pivot resting on that peg.
- **Retention makes it irreversible.** Every day of unsnapshotted XLM readings
  ages out for good. Whatever is decided, the readings currently in
  `oracle_prices` are recoverable only until the 13-month window passes them.

## Implementation

- **First, measure the disagreement.** Compare, over the window where both exist,
  the pivot's derived `ref_usd` for XLM against Reflector's measured XLM reading
  at the same bucket. Report the distribution, not a single summary number.
  ⚠️ If they agree to within noise, close this task with that evidence — it is a
  legitimate outcome and cheaper than the alternative.
- If they diverge, options to cost:
  1. **Snapshot measured non-peg rates** into `usd_rate` under a correctly-named
     path, with `method` reflecting that they are measured readings. Requires the
     symbol-to-identity evidence [[0173]] demands.
  2. **Have the pivot prefer a measured rate** where one exists, falling back to
     the derived one otherwise — which reintroduces a provenance question the
     response must carry.
  3. **Backfill `usd_rate` from `oracle_prices`** for XLM before the retention
     window passes, independently of which source the pivot uses. Cheap, and it
     stops the bleeding while the rest is decided.
- ⚠️ Whatever ships must not weaken the `peg_identities()` test. If a new set is
  needed, give it its own name and its own evidence.

## Acceptance Criteria

- [ ] The derived `ref_usd` and the measured Reflector reading for XLM are
      compared over the overlapping window, and the distribution of the
      difference is recorded. **This gates the rest of the task** — agreement to
      within noise is a valid close.
- [ ] If they diverge, the affected candle population is counted before any fix
      is chosen.
- [ ] The evidence that Reflector's `XLM` symbol names our native asset — not
      merely a matching ticker — is written down, to [[0173]]'s standard.
- [ ] `peg_identities()`'s test is intact, or its replacement carries the same
      "a symbol is not an identity" guarantee with the same explicitness.
- [ ] Whether the readings currently in `oracle_prices` are preserved before the
      13-month window passes them is decided explicitly, not by default.

## Out of scope

- [[0227]]'s timestamp unit bug, though it affects the same table and its
  ~30% loss must be accounted for when measuring the overlap above.
- [[0173]]'s USDT mis-attribution.
- Re-enriching existing candles — that follows the measurement, and only if it
  shows a material difference.

## Notes

- Found by a query aimed at `usd_rate` coverage for [[0170]]. The hypothesis
  under test was that a *window* of readings had gone unsnapshotted; that was
  **falsified** — USDC's coverage is complete apart from [[0227]]'s rows. The
  real gap was a whole asset, not a date range.
- ⚠️ Nothing here is a claim that current USD prices are wrong. It is a claim
  that we prefer an inference over an available measurement, and that nobody has
  checked what that costs.
