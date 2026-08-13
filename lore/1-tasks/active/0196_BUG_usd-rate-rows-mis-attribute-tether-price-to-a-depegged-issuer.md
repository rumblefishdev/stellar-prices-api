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

## ✅ PURGE EXECUTED ON PROD 2026-08-13

### Preconditions, in the order they had to happen

1. **Both writers fixed** — `reflector_key_to_identity` (the single seam the
   oracle-worker poll loop and the ledger-processor event-decode path share).
2. **Both writers deployed.** This is the step that nearly went wrong: the two
   paths ship in **different stacks**. `Prices-production-EventBridge` at 10:55
   UTC covers oracle-worker; the ledger-processor is in
   `Prices-production-Compute`, deployed 11:50 UTC. Deploying only the first
   would have left a live writer and the purge would have regrown.
3. **Writer death confirmed by measurement**, not by deploy success — USDT's
   `oracle_prices` `last_seen` frozen at 11:00 while USDC kept ticking at 2 min
   staleness. ~10 poll cycles.

### Measured immediately before deletion (`FINAL`)

| table | rows | oracles/methods | first | last | avg |
|---|---|---|---|---|---|
| `oracle_prices` | **46,423** | 1 | 1970-01-21 15:41:56 | 2026-08-13 11:00 | 0.999565 |
| `usd_rate` | **44,318** | 1 | 2026-03-11 14:00 | 2026-08-13 10:50 | 0.999582 |

⚠️ **The 46,378 figure recorded earlier in this file was measured without
`FINAL`** and undercounted. `count()` on a `ReplacingMergeTree` moves with
background merges — it drifted 46,378 → 46,425 → 46,423 across three readings
of unchanged data. Use `FINAL` or don't quote the number.

### Decision: DELETE, not re-key

Recorded per this task's own open question. There is no correct identity to
re-key **to** — no asset in `prices.assets` *is* real Tether — so a re-keyed row
would be data preserved for a consumer that does not exist, next to a footgun
that does.

⚠️ **Correction to an earlier argument in this task:** "Reflector still
publishes the feed so nothing is lost" is true only going *forward*. The SEP-40
contract serves current prices, not a five-month backfill, so **the deleted
series is not re-derivable.** Both sets were dumped to TSV before deletion
(46,424 and 44,319 lines incl. header) on the operator's local machine.

### Execution

Async `ALTER … DELETE` mutations, not `DROP PARTITION` — both tables partition
by month and those partitions hold every other asset's rows.

```sql
ALTER TABLE prices.oracle_prices DELETE WHERE asset_id = 111;
ALTER TABLE prices.usd_rate      DELETE WHERE asset_code = 'USDT'
                                   AND issuer_address = 'GCQTGZQQ…TG6V';
```

Both `is_done = 1`, `parts_to_do = 0`, no `latest_fail_reason`. Post-delete count
**0** on both. `asset_id` is `oracle_prices`'s leading sort-key column, so the
mutation pruned cleanly.

### The 1970 timestamp went with them — deliberately, and a twin survives

`oracle_prices` held a USDT row at `1970-01-21 15:41:56` ≈ 1,784,516 epoch
seconds — consistent with a millisecond timestamp divided by 10⁶ instead of 10³,
i.e. a mid-2026 reading landing in 1970. **Canonical USDC showed the identical
`first_seen`**, so the defect is systematic and its USDC instance is untouched
and still investigable. Not filed as a task yet; needs one.

## ✅ Mapping audit — the two survivors, with evidence

The general question is [[0173]]. This is the narrow version: for each identity
still on the oracle surface, is there a reason to believe the feed's **ticker**
names that **issuer**?

| symbol | identity | basis | verdict |
|---|---|---|---|
| `XLM` / `native` | `AssetIdentity::Native` | Not an issued asset. There is exactly one native asset on the network, so the ticker cannot be filed against the wrong issuer — the failure mode requires an `issuer_address` to get wrong, and there isn't one. | ✅ structurally safe |
| `USDC` | Circle `GA5ZSEJY…KZVN` | `usd_rate` for this identity measured **1.000086 – 1.000639** monthly over 2026-03 → 2026-08, never outside ±0.0015. That is what real Circle USDC does. Had the feed been naming a different `USDC` issuer (there are ~220), a copycat would have to hold par to 15 bp for six months to fake this. | ✅ measured |

⚠️ **This is weaker evidence than it looks for USDC, and worth saying plainly.**
Agreement at par is consistent with the feed naming Circle's USDC — but it is
*also* consistent with any issuer that happens to hold par. What makes USDT's
case decisive is the **disagreement**: a 7.4× gap cannot be explained away. So
this audit can falsify a mapping but cannot fully confirm one. Treat USDC as
"no evidence of mis-attribution" rather than "confirmed correct", and leave the
positive proof to [[0173]].

⚠️ It also only covers what we poll **today**. The real protection is that the
sets are now pinned by test, so a future addition cannot be silent:
`peg_identities_is_exactly_canonical_usdc` (`oracle-worker`) and
`reflector_resolves_exactly_xlm_and_usdc_and_nothing_else`
(`prices-ingest-core`).

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

- [x] **`prices.oracle_prices`** rows for asset_id 111 removed — 46,423 deleted
      2026-08-13, post-count 0. This was the blocking one: 0172's fix did not
      take effect until they were gone.
- [x] `prices.usd_rate` rows removed or re-keyed, decision recorded — 44,318
      deleted; **decision = delete**, reasoning above. Dumped to TSV first,
      because the series is NOT re-derivable from Reflector.
- [x] Both purges run only after the fixed worker is deployed — confirmed on
      *both* stacks (EventBridge 10:55, Compute 11:50) and by measured writer
      death, not by deploy exit status.
- [x] **Regrowth re-check** — re-run after the purge with both writers live:
      `oracle_prices` for asset 111 still **0**. Nothing is rewriting the
      identity, so the deletion is durable rather than a momentary state.
- [x] Every other `TRACKED_SYMBOLS` → identity mapping audited the same way —
      only XLM and USDC survive; evidence and its limits recorded above. XLM is
      structurally safe; USDC is "no evidence of mis-attribution" rather than
      positively confirmed, which is left to [[0173]].
- [x] 0168's hold note cleared — **lifted 2026-08-13**, and converted into a
      standing condition on ADDING peg members rather than a blanket block.
- [x] A test that fails if a peg/oracle identity is added without a documented
      basis — `peg_identities_is_exactly_canonical_usdc` (`oracle-worker`) and
      `reflector_resolves_exactly_xlm_and_usdc_and_nothing_else`
      (`prices-ingest-core`). Neither can prove a justification was written, but
      both make the addition impossible to make silently.
- [ ] **Spawned:** [[0199]] — the 1970 timestamps found while measuring the purge
- [x] Writer stopped — `reflector_key_to_identity` no longer resolves `USDT`
      (done on 0172's branch, PR #205; guarded by
      `reflector_drops_usdt_because_the_ticker_is_not_this_issuer`)
