---
id: "0242"
title: "A SAC is minted as a second identity for an asset we already hold — same token under two asset_ids, with volume split across both"
type: BUG
status: active
assignee: akot
related_adr: []
related_tasks: ["0210", "0139", "0120", "0286", "0252"]
tags: [layer-backend, layer-database, priority-high, effort-medium, milestone-M2, ingest, identity, defect]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/canonical.rs"
history:
  - date: "2026-10-05"
    status: active
    who: akot
    note: >
      Activated for implementation via GSD (quick 261005-htu, research done),
      one PR with ordered commit slices, worked in .claude/worktrees/0242.
      Re-measured: 36 SAC contracts held as a Contract identity (13 split
      pairs, 23 whose classic we never held) plus 6 latent. Decisions D1–D7
      recorded below. The heal rides 0286 phase 3: a 29-row classic seed
      goes in between stage B and C, and stage C runs to 202609.
  - date: "2026-09-02"
    status: backlog
    who: stkrolikiewicz
    note: >
      Re-measured after [[0210]] shipped. All 11 SACs resolve `symbol()` to the
      exact code of the classic asset they wrap, so the split is now two
      well-formed answers under two addresses rather than one named row and one
      blank. Not a regression — nothing was written to `prices.assets` (52
      contracts, 52 identities, unchanged) — but the defect stopped being
      invisible, which argues for raising priority. Also separated these from
      the impersonation set: `USD`/`EUR`/`USDP` are SACs and belong here, while
      [[0252]]'s five are non-SAC contracts.
  - date: "2026-08-28"
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0210]]. Looking for soroban assets whose symbol could be
      derived without an RPC call, 11 of the 52 turned out to be SACs of classic
      assets already in the registry — which means they are not nameless, they
      are duplicated: the same token exists twice, under two `asset_id`s, and
      three of the pairs have candles on both sides. That is a bigger defect
      than the missing name 0210 is about, so it is split out rather than folded
      in.
---

# A SAC becomes a second identity for an asset we already have

## Summary

`AssetRegistry::resolve_sac` is a **lookup table**, not a derivation: it maps a
contract address to a classic identity only for classic assets the registry has
already interned. A SAC whose underlying asset had not been seen at mint time
therefore falls through and is interned as a fresh `AssetIdentity::Contract`,
with its own `asset_id`.

The asset is then in `prices.assets` twice — once as `('CODE','G…','')`, once
as `('','','C…')` — and every surface keyed on `asset_id` treats them as two
different assets.

## Measured on prod, 2026-08-28

11 of the 52 soroban rows have a `contract_address` equal to the `sac_address`
already stored on a classic row, i.e. they are provably the same asset:

| symbol | classic id | candles | SAC id | candles |
|---|---|---|---|---|
| XCR | 87262 | 2,127 | 1153 | **76** |
| USDM1 | 109012 | 37 | 4142 | **2** |
| POINTS | 2146 | 6,614 | 1738 | **1** |
| USDP | 3374 | 2,067 | 1739 | 0 |
| EUR | 90371 | 55 | 70913 | 0 |
| USD | 90372 | 3 | 70914 | 0 |
| ESP | 93848 | 3 | 93843 | 0 |
| WHLAQUA, FrogST, VEUR, VCHF | — | 0 | — | 0 |

**Three pairs trade on both sides.** XCR's 2,127 classic candles and 76 SAC
candles are the same token, so neither row's `volume_24h_usd` is that token's
volume and neither row's price is formed from all of its trades. The listing
shows it twice: once named with the SDEX history, once nameless with a
fraction of the activity.

The derivation needed to detect this is **already in the codebase and already
persisted** — `assets.sac_address` is populated on 207,471 classic rows by
`sac_address_of()` at intern time. Only `resolve_sac`'s direction is missing.

## Re-measured 2026-09-02: the split now has a name on it

[[0210]] shipped, so every soroban contract's `symbol()` is resolved and
published as `asset_code`. Asking all 11 SACs what they call themselves gives
the same answer every time — **the code of the classic asset they wrap**:

```
ESP → ESP    EUR → EUR    USD → USD    USDP → USDP    XCR → XCR
VEUR → VEUR  VCHF → VCHF  POINTS → POINTS  USDM1 → USDM1
FrogST → FrogST           WHLAQUA → WHLAQUA
```

Eleven for eleven. This is the expected behaviour of a Stellar Asset Contract —
it faithfully reports its underlying asset — and it is precisely what makes the
split legible now in a way it was not before.

**What a consumer sees today.** `GET /v1/assets/{sac_address}` returns 200 with
`code: "USDP"` and essentially no price history, while `GET /v1/assets/USDP:GDTEQ…`
returns the same code with 2,067 candles. Same asset, two addresses, two answers,
both well-formed. Before 0210 the SAC row was at least visibly nameless; it now
looks like a legitimate but inexplicably empty listing of the same token.

**This is not a regression introduced by 0210** — the two rows, two `asset_id`s
and split candles all predate it, and 0210 writes nothing to `prices.assets`
(verified: 52 contracts, 52 identities, unchanged across the deploy). What
changed is that the defect stopped being invisible. If anything that raises this
task's priority, because the failure mode is now "two plausible answers"
rather than "one answer and one blank".

**Still hidden from the listing**, which `INNER JOIN`s `current_prices`: only
the SACs with candles surface, and the four zero-volume ones do not. Detail
reads `FROM assets` with `LEFT JOIN`s and has no such floor, so every one of the
11 is addressable right now.

### These are not the impersonation cases

Worth stating because the symbols invite the confusion: `USD`, `EUR` and `USDP`
resolve off **SACs**, and faithfully name their classic counterpart. They are
this task, not [[0252]]. The impersonation set is disjoint and is five
**non-SAC** contracts — `USDC`, `USDT` (×2), `BTC`, `XRP` — verified against
`assets.sac_address` on 2026-09-02. A first pass at recording the sweep folded
the two together; they are separate defects with separate fixes.

## Why this is not [[0139]]

0139 is the same `asset_id` appearing on several rows because `assets` is
sorted on natural identity — one id, many rows. This is the inverse: one
economic asset holding **two different ids**. They compound (a joined query can
fan out across 0139's duplicates of either identity) but the fixes are
unrelated, and neither blocks the other.

## Implementation sketch

- Make `resolve_sac` derive rather than look up, or seed the index from
  `assets.sac_address` at registry load, so a SAC mints onto its classic
  identity and no new split is created. That stops the bleeding; it does not
  heal the 11.
- Decide what happens to the existing pairs. Merging two `asset_id`s touches
  `price_ohlcv_*`, `current_prices` and every side table — closer to a data
  migration than a metadata fix, and it needs its own plan and rollback.
  Cross-referencing them instead (a pointer column, no re-keying) is the
  cheaper option and may be enough for the read surface.
- ⚠️ Worth looking at how BE models this before choosing: their
  `default.asset_sac` (455,116 rows) carries `asset_code`, `issuer_id`,
  `contract_id`, `sac_contract_id`, `sac_deployed` — an explicit *link* between
  a classic asset and its SAC rather than a second identity. If their shape
  avoids the defect by construction, that is the design to copy.

## Re-measured on prod, 2026-10-05

Measured as `dev_read` (read-only). The queries and outputs are kept with GSD quick `261005-htu`, outside the repo.

- `prices.assets` holds 59 contract rows:
  - **13 split pairs:** the 11 above plus SODA and USDT0.
  - **23 SACs whose classic asset is not in `assets` at all**, e.g. nBTC, nETH, BURNMKR. BE `soroban_contracts.is_sac` identifies them, and sha256 of the underlying reproduces all 36 addresses.
  - 23 genuine Soroban tokens.

  Another **6** pool-leg SACs (SUSHI, HYPE, STELLA, TESTTTT, TEST77, TEST12) have no identity yet and would mint one on their first priced trade. The newest mint is BURNMKR, on 2026-09-30.
- **The split pairs are mostly double counts, not splits.** 145 of 151 SAC-keyed 1m rows are byte-identical to a classic-keyed row at the same key. Only 4 trades exist solely under a split SAC id (USDT0 ×3, SODA ×1, all 202609). The 23 have no classic row, so their candles are the only copy and must never be deleted.
- **In-band proof is reliable.** Every sampled SAC swap leg had a SAC `transfer`/`mint`/`burn`/`clawback` event in the same transaction, with the SEP-11 asset as the last topic: 0 misses in 338,601 legs on Aquarius, Soroswap, Phoenix, Comet and Sushiswap.
- **`events-backfill` reads only pool events, so it never sees that proof.** One start-of-run query (one proof event per SAC that `prices.assets` cannot resolve) returns 1,137 contracts in 2.6 s, and all 1,137 verify.
- **Two causes.**
  - A race: the SAC traded before anything interned its classic. USDT0's SAC traded at 16:30 and its classic first appeared on SDEX in the 17:00 hour.
  - The classic was never seen at all.
- After 0139 the id is `xxh3(identity)`, so emitting the classic identity is enough. No id needs coordinating.

Prior art: soroban-block-explorer ADR 0051 (a SAC is a facet of the classic asset).

- Reverse detection uses the same crypto gate (`crates/xdr-parser/src/sac.rs`).
- Explorer 0374 fixed the same defect for pool legs with a DB-backed map.
- Explorer 0571 shows that map drifting: 138 deployed SACs are flagged `sac_deployed = 0` and silently fall back to a contract id.

## Decisions (2026-10-05, Adam)

- **D1 Resolver.** `AssetRegistry::learn_sac(contract, sep11)` accepts a mapping only when `sac_address(sep11) == contract`.
  - Live path: fed from the SAC events of the same transaction, before the swap events are classified.
  - `events-backfill`: one proof preload at run start.
  - Rejected:
    - BE `asset_sac`: needs a new grant and carries the 138 drifted flags.
    - RPC `name()`: a network call in the write path.
    - Our own table: it would duplicate `assets.sac_address`.
- **D2 Unprovable SAC.** When BE says `is_sac` but there is no proof, the tick is skipped and counted, and alarms. It never becomes a `Contract` identity. The phase-3 orchestrator STOPs on the counter. The live path loads the `is_sac` set once per cold start.
- **D3 Heal.** Through the 0286 phase-3 re-ingest, which drops and rebuilds every month. No SQL rekey: that would mean 4 kinds of row operation, base/quote flips for Z/Q and XLD/SODA, merges and re-enrichment. Afterwards, DELETE the residual SAC-keyed rows, then 36 `assets` rows, then 36 `asset_symbol` rows.
- **D4 Sequencing.**
  - Phase 3 is not held for the code.
  - A 29-row classic seed (the 23 + the 6, each `sac_address` hash-verified) goes into `prices.assets` after stage B and before stage C. With it, the deployed binary resolves every pool-leg SAC.
  - Stage C runs `--to-month 202609`, not 202608. 202609 holds the only copies of the USDT0/SODA trades and part of 0139's hole.
  - The single Z/Q row of 202404 joins the residual cleanup.
- **D5 API.** `GET /v1/assets/{SAC C…}` aliases to the classic asset through `sac_address`.
- **D6 Guard.**
  - A `SacContractIdentities` metric in `rollup-freshness-probe`. Baseline is 36. Alarm at ≥ 1, enabled only after the cleanup.
  - Plus an ingest-side skip counter.
- **D7 Shape.**
  - **One PR** with ordered commit slices: resolver + live, then `events-backfill`, then probe + alarm, then API alias. This overrides the ~400-line norm for this task.
  - The heal is a runbook section, with no window.
  - The new `events-backfill` binary is swapped on ch-prod-01 only between phase-3 stages, never during an `amm` step.

Plan choices confirmed by Adam on 2026-10-05 (GSD plan `261005-htu`):

- **PC3 Fail closed at the live cold start.** If the Lambda cannot read BE's `is_sac` set, Init fails, like the other cold reads. An empty set would mint `Contract` identities, which D2 forbids. The cost: if BE's `default.soroban_contracts` breaks, live ingest stops until it is fixed.
- **PC6 A summary without the new line passes.** The orchestrator STOPs only when `unproven sac swaps:` is present and non-zero. A binary built before 0242 prints no such line, so phase 3 keeps running on it, which is why D4 holds.
- **PC9 Residual rows of a SAC with no classic.** Such a row is the only copy of its trades, so it is never deleted silently. The operator either re-runs that month through phase 3 or records the loss on this task. The rows are copied beside the 72 metadata rows for rollback. The 202404 Z/Q row, from stage B before the seed, is the expected case.

Open: **U3**. About 75.6k Soroswap swaps on BLTA/BLTB/BLTC/PPRIME/LumenJoule produce no candle under any id, and the cause is not established. It must be settled before AC3 promises numbers for those assets.
Stays in scope of 0242 (Adam, 2026-10-05): this task closes everything that concerns SACs.

**U3 settled (2026-10-05).** Neither the dust rule nor an ingest bug: those swaps were missing from the candle tables because of the 0139 re-key.
- Each of the five SAC contract identities shared its old UInt32 id with an unrelated classic asset: 123376 LumenJoule/BTC, 123377 BLTB/ETH, 123378 BLTA/AST, 123379 BLTC/ASC1148, 123380 PPRIME/KWD.
- `asset_id_map_0139` therefore marks all five `colliding`. The re-key copies only `mapped` and `sentinel` (`rekey.rs` `COPYABLE`), so their candles stayed in `price_ohlcv_*__pre0139`.
- Before 0139, the 1h counts matched BE's swaps exactly for 202603–202606.
- Jul–Sep were already short before 0139: the 07-09 Soroswap gap (0101) and burst minutes before the 0282 fix.
- Locally, five real transactions from 202603–202609 produce candles under the classic identity on the 0242 branch.

No code fix is needed. Phase 3 stage C, run with the 29-row seed, rebuilds them under the classic ids. AC3 for these five then rests on stage C's post-check; the BLT assets will have volume but no price, because their fills are dust.

The six latent SACs have other causes. SUSHI and HYPE traded before SushiSwap was indexed, and phase 3 covers that under 0290. STELLA, TESTTTT, TEST77 and TEST12 have pools with no swaps.

**October window (measured 2026-10-05, `dev_read`).** Since 2026-10-01, no candle on any tier is keyed on the 36 SAC ids, and no contract row has been minted since BURNMKR. No decision is needed now. The runbook re-checks months after 202609 before the cleanup, and any non-zero result goes through PC9.

## Acceptance Criteria

- [ ] A SAC whose classic asset is in the registry no longer mints a second
      identity — pinned by a test that interns the SAC first and the classic
      asset second, and vice versa
- [ ] The 11 existing pairs are resolved, or the decision to leave them with a
      cross-reference is recorded with its reasoning
- [ ] For a pair with candles on both sides (XCR is the sharpest), the API
      reports one asset with one volume, or the split is documented as known
- [ ] No new duplicate `asset_id` is introduced by whatever is chosen (count
      per natural identity before and after)
