---
id: "0196"
title: "usd_rate AND oracle_prices assert ~$1.00 for a Stellar IOU worth $0.13 — Reflector's ticker feed was filed under an issuer identity"
type: BUG
status: active
related_adr: []
related_tasks: ["0172", "0173", "0167", "0168"]
tags:
  ["priority-high", "effort-small", "oracle", "data-correctness", "clickhouse", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Spawned from 0172. That task removed USDT from peg_identities() so no NEW
      mis-attributed rows are written, but every row already snapshotted under
      that identity is still in prices.usd_rate and still asserts par for an
      asset trading at ~$0.13.
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Scope widened after a prod measurement. The mis-attribution was NOT
      confined to the inert usd_rate table — prices.oracle_prices carried it
      too (46,378 rows, avg 0.99957, current to the hour), and that table feeds
      the enrichment oracle tier, which runs BEFORE the peg-pivot tier and wins
      where it applies. So 0172's peg removal was being bypassed on every new
      USDT-quoted candle. The writer half is fixed on 0172's branch (the USDT
      arm is gone from reflector_key_to_identity, the single seam both oracle
      writers share); the row purge for BOTH tables stays here.
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Renumbered 0183 -> 0196. The onboarding epic was re-cut into 0183-0195
      four minutes after PR #205 merged these spawned tasks, so both sides
      claimed the same ids with no chance to see the other. Ours moved because
      the other 0183 was already active and its thirteen slices are a contiguous
      block; these three were backlog with no work in flight. Referring sites
      updated: views.sql, views_it.rs, 0172, 0182, 0168.
  - date: 2026-08-13
    status: active
    who: okarcz
    note: >
      Activated. Both writers are now fixed AND deployed to prod: the
      oracle-worker poll loop (Prices-production-EventBridge, 10:55 UTC) and the
      ledger-processor event-decode path (Prices-production-Compute, 11:50 UTC).
      Confirmed on prod at 11:52 - USDT's oracle_prices last_seen frozen at
      11:00 (52 min stale across ~10 poll cycles) while USDC keeps ticking at 2
      min. The rows can now be purged without regrowing, which was the whole
      precondition. This unblocks 0172 and 0182.
---

# `usd_rate` files Tether's price under a different token's identity

## The confusion

Two different things are called "USDT":

1. **Tether's own token** (Ethereum, Tron, …) — genuinely ~$1.00.
2. **A Stellar IOU issued by `GCQTGZQQ…TG6V`** — depegged June 2022, ~$0.13
   since (see [[0172]] for the evidence).

Reflector publishes a feed named for the **ticker**, so it is quoting #1. We
snapshot that reading keyed on `(asset_code='USDT', issuer_address='GCQTGZQQ…')`
— i.e. #2's identity. The oracle is not wrong; **the identity we file it under
is.**

An asset code is not an identity on Stellar. `prices.assets` holds **~220
distinct issuers using the code `USDC`** and **~220 using `USDT`**.

## Measured on prod (2026-08-12)

| Month | `usd_rate` (oracle) | our candles (`close`) | ratio |
|---|---|---|---|
| 2026-07 | 0.999267 | 0.132027 | **7.57×** |
| 2026-08 | 0.999232 | 0.134087 | **7.45×** |

Coverage runs 2026-03 → present, `method = 'oracle'`, ~8,500 observations/month.

## 🔴 WIDER THAN FILED (2026-08-13) — `oracle_prices` had it too, and that one is live

This task was filed against `prices.usd_rate`, which is **inert**: nothing reads
it until [[0168]] ships, and 0168 is on hold. That framing was too narrow.

`prices.oracle_prices` carries the same mis-attribution, and it is **not** inert
— it is the source of the enrichment **oracle tier**. Measured on prod:

```
asset_code  issuer     asset_id   obs    last_seen             avg_price_usd
USDC        GA5ZSEJY          3  46378   2026-08-13 08:05:00   1.00029
USDT        GCQTGZQQ        111  46378   2026-08-13 08:05:00   0.99957
```

Why that bypasses 0172's fix: `ch_enrich.rs:19-22` documents the oracle tier as
running **first** and winning where it applies; the peg-pivot tier only fills
what it left at `close_usd = 0`. The join is
`o.asset_id = p.quote_asset_id` (`ch_enrich.rs:472`), so a USDT-quoted candle
inside the staleness window took `close_usd = close × ~$1.00` — the exact ~7.4×
overstatement 0172 removed from the peg tier, re-entering through the tier 0172
never touched.

0172's new IT passed while this was live because its fixture inserts **no**
`oracle_prices` rows at all (`ch_enrich_it.rs:1127-1152`), leaving the oracle
tier a no-op and the pivot to handle everything. Green test, open path.

### Writer fixed on 0172's branch; rows are still this task's

The `USDT` arm is removed from `reflector_key_to_identity`
(`prices-ingest-core/src/soroban.rs`) — deliberately there rather than in
`TRACKED_SYMBOLS`, because that function is the single seam **both** oracle
writers share (the poll loop and the Soroban `update`-event decode path), so
the poll list alone would have fixed only one of them. `TRACKED_SYMBOLS` drops
to `["XLM", "USDC"]` to satisfy the pre-existing
`every_tracked_symbol_resolves_to_an_identity` invariant.

⚠️ **The code fix alone does not stop the bad pricing.** It stops new rows; the
enrichment tier keeps reading the 46,378 rows already stored. **The purge is
what makes 0172 take effect**, so it is on 0172's critical path, not merely a
tidy-up after it.

⚠️ Sequencing: purge only *after* the fixed worker is deployed, or the next run
re-writes the rows. Same constraint for both tables.

### Recommendation on delete-vs-re-key (below): delete

Reflector still publishes the feed, so a deleted row is re-derivable and nothing
is permanently lost. There is no correct identity to re-key **to** — no asset in
`prices.assets` *is* real Tether — so a re-keyed row would be data preserved for
a consumer that does not exist, sitting next to a footgun that does.

### Side observation, needs one query (not part of this task)

Both rows above report `first_seen = 1970-01-21 15:41:56` ≈ 1,784,516 epoch
seconds — close to a millisecond timestamp divided by 10⁶ instead of 10³, i.e. a
mid-2026 reading landing in 1970. Identical across both assets, so systematic
rather than one corrupt row. Harmless here (the `ASOF` join never matches it).
Worth a `WHERE timestamp < '2020-01-01'` count when someone is next in the table.

## Why this is urgent beyond the wrong rows

⚠️ **[[0168]] must not ship for this identity while these rows exist.** 0168's
whole design is "replace the hardcoded `1` with the measured rate from
`usd_rate`". Pointed at this identity it would import the mis-attribution into
`price_usd_series` and stamp it `method = 'oracle'` — which a consumer reads as
*more* authoritative than the `peg` placeholder it replaced. It would relabel the
same 7.4× error as measured truth. A hold note is on 0168's task file.

## What needs deciding

- **Delete, or keep and re-key?** The rows are factually a record of Tether's
  price; they are only wrong because of what they are keyed to. If some consumer
  wants real Tether's price, re-keying is better than deleting.
- **The general rule.** This is a specific instance of [[0173]] — the mapping
  from oracle feed symbol to Stellar issuer identity is undocumented and
  unverified for *every* feed, not just USDT. Fixing one row set without fixing
  the mapping invites the same bug on the next asset. Consider doing 0173 first.
- **Does any other tracked symbol have the same problem?** `TRACKED_SYMBOLS` maps
  several feeds to identities; each mapping needs the same "is this feed actually
  about this issuer?" check.

## Acceptance Criteria

- [ ] **`prices.oracle_prices`** rows for asset_id 111 removed — the blocking one,
      because 0172's fix does not take effect until they are gone
- [ ] `prices.usd_rate` rows removed or re-keyed, decision recorded
- [ ] Both purges run only after the fixed oracle worker is deployed, confirmed
      by a re-count showing no regrowth on the next run
- [ ] Every other `TRACKED_SYMBOLS` → identity mapping audited the same way
- [ ] 0168's hold note cleared or made permanent, explicitly
- [ ] A test that fails if a peg/oracle identity is added without a documented
      basis for the symbol→issuer mapping
- [x] Writer stopped — `reflector_key_to_identity` no longer resolves `USDT`
      (done on 0172's branch, PR #205; guarded by
      `reflector_drops_usdt_because_the_ticker_is_not_this_issuer`)
