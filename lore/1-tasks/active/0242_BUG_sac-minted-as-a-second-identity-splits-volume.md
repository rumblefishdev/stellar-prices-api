---
id: "0242"
title: "A SAC is minted as a second identity for an asset we already hold — same token under two asset_ids, with volume split across both"
type: BUG
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0210", "0139", "0120", "0286", "0252"]
tags: [layer-backend, layer-database, priority-high, effort-medium, milestone-M2, ingest, identity, defect]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/canonical.rs"
history:
  - date: "2026-11-18"
    status: active
    who: okarcz
    note: >
      Plan to completion written: reprice 2026-10-01 → 2026-11-05 with a fresh
      events-backfill, pre-roll, then §5 cleanup. §6 waits for 0343 Phase 1
      and is the first release through develop → deploy/production → master.
  - date: "2026-11-18"
    status: active
    who: okarcz
    note: >
      Z's single 2024-04-18 Aquarius trade is recorded as a loss under PC9
      instead of being repriced (Heal log, 2026-11-18). The §5 backfill keeps
      one events-backfill window, 2026-10-01 → 2026-11-05.
  - date: "2026-11-17"
    status: active
    who: okarcz
    note: >
      Handed over by the previous assignee (akot), who approved PR #421 the
      same day and asked the new assignee to finish §5 (cleanup and backfill),
      §6 and §7. The plan agreed on 2026-11-12 stands: map every tier and month
      read-only, classify per row, backfill only uncovered months, back up, then
      DELETE, then the probe reads 0. Changed by task 0343: the three 0242
      alarms now sit behind `opsAlarms.sacIdentityAlarmsEnabled` (`false` in
      production, #421), so Observability is no longer held back (D8) and
      release #418 merged without waiting for §5. §6 sets the flag `true`
      together with `sacContractIdentitiesActionsEnabled`.
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
- **D8 Observability after the cleanup (Adam, 2026-10-06).** The PR deploys Compute and EventBridge only. Observability, with both 0242 alarms, is deployed in runbook §6, after §5, with the `SacContractIdentities` actions already on.
  - Why: deployed earlier, that alarm would sit in ALARM on the dashboard's Row-0 alarm strip through stage C, stage D and the cleanup. That strip is [[0294]]'s SCF evidence (all alarms OK).
  - Cost: until §6 no alarm watches `SacUnprovenSkipped`. The runbook has the operator read the metric by hand after the cold start and at least daily.
  - Constraint on the team: no `make deploy-production-observability` from `develop` between the merge and §6. Such a deploy creates the alarms early, and the red tile's description says it is expected.
- **D9 Compute after stage D (Adam, 2026-10-06, from Oskar's review of PR #390).** Compute (live + API) deploys after phase-3 stage D and before the §5 cleanup.
  - Why: the API alias answers a SAC address as its classic. Until stage C has rebuilt a SAC's months under the classic id, that classic holds only part of the history, so `GET /v1/assets/{C…}/ohlcv` would lose the pre-heal AMM series. Group B is the visible case: its classics were seeded on 2026-10-06.
  - Cost: the new live code lands two or three days later. The seed already resolves every known SAC on the old live after its next cold start.

Plan choices confirmed by Adam on 2026-10-05 (GSD plan `261005-htu`):

- **PC3 Fail closed at the live cold start.** If the Lambda cannot read BE's `is_sac` set, Init fails, like the other cold reads. An empty set would mint `Contract` identities, which D2 forbids. The cost: if BE's `default.soroban_contracts` breaks, live ingest stops until it is fixed.
- **PC6 A summary without the new line passes.** The orchestrator STOPs only when `unproven sac swaps:` is present and non-zero. A binary built before 0242 prints no such line, so phase 3 keeps running on it, which is why D4 holds.
Changed by Adam on 2026-10-06, after Oskar's review of PR #390:

- **PC3 reversed: fail open.** An unreadable `is_sac` set no longer fails Init. The container ingests with no candidates, logs a WARN and publishes `SacCandidatesUnavailable` on every run, with an alarm on it (deployed in §6, D8).
  - Why: the candidates guard only a SAC trade with no in-band proof, which happened 0 times in 338,601 legs. Failing closed stopped SDEX, oracle and AMM ingest over one BE table.
  - The probe's `SacContractIdentities` counts any identity minted meanwhile.
- **PC6 amended.** A month without the `unproven sac swaps:` line gets a visible note. Once a month of the same state has printed the line, a later month without it is a STOP: the host binary went back to a pre-0242 build. Stage C on the pre-0242 binary is unaffected.
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

## Heal log

- **2026-10-06, seed (runbook §3).** Stage B finished 202404. The seed ran at 10:29:11 UTC as `dev_shared`, straight from `seed-0242.sql` rather than through the staged variant of §3. There is therefore no `bak_0242_seed_stage`, and `bak_0242_seed` was created afterwards from the 29 rows with that `created_at`.
  - Check: `29 29 / 29 / 0`, with collisions at 0. No candle under any seeded id on any tier.
  - Comet's `pool_registry` row is present (1).
- **2026-11-02, a new split (found in [[0338]]).** BLNT's SAC `CCBNWP5L…` traded on Comet from 2026-10-26 14:22 UTC, before SDEX interned its classic at 18:42, so 10 trades sit on the `soroban` id 14236029787030528679. The "no contract row since BURNMKR" of the October window no longer holds.
  - The reprice in 0338 rewrites those trades under the classic id, so the rows left on that id are copies. Add it to the residual cleanup. It does not fall under PC9, because the classic exists.
- **2026-11-05, deploy (runbook §4e).** After stage D. Pre-check as `prices_writer` from fishuser-hero (Adam): 4,137 distinct `is_sac` contracts readable. `deploy-production-compute` (11:33 UTC) and `deploy-production-eventbridge` (11:35) from `develop` `43a28399`; the previous CodeSha256s are in `~/heal-0242/deployed-before.tsv` on stkrolikiewicz's laptop. Observability not deployed (§6).
  - Cold start: `loaded the is_sac contract set` with `sac_contracts` 4137, then `resumed cursor from ClickHouse` at 65318468. No WARN.
  - `SacUnprovenSkipped` and `SacCandidatesUnavailable`: no datapoints in the first hour.
  - Probe: `sac_contract_rows` **40**, not the runbook's baseline 36 (`sac_contract_scanned` 74, `be_sac_contracts` 4137). The old build ran until this deploy, so contract rows minted since the October baseline (BLNT above among them) count too. §5 counts its residual from today's number, not from 36.
- **2026-11-17, the map (runbook §5a and §5b, read-only).** Run as `dev_read` (`readonly = 1`) on `ch.sorobanscan.com`, SELECT only. The §5a predicate and the §5b byte-level test were inlined, so no `bak_0242_sac_ids` table was created. Files: `~/heal-0242/{sac_ids,ac4_before,residual}.tsv` on the local machine.
  - **40 SAC identities, all 40 with a classic** (`classic_id = 0`: 0). No PC9 case without a classic.
  - AC4 "before": 215,193 identities, 215,193 ids, 0 collisions.
  - **Residual: three months only, on every tier.** No other partition holds a row under a SAC id.

    | month | `1m` rows (byte-identical / only copy) | only-copy trades `1m` | `1d` rows (identical / only copy) | only-copy trades `1d` |
    | --- | --- | --- | --- | --- |
    | 202404 | 1 (0 / 1) | 1 | 1 (0 / 1) | 1 |
    | 202610 | 79 (12 / 67) | 75 | 35 (1 / 34) | 85 |
    | 202611 | 18 (0 / 18) | 19 | 11 (0 / 11) | 19 |

    `15m`, `1h`, `4h`, `1w` and `1M` hold the same three months (full table in `residual.tsv`).
  - By asset (`1d`): Z aquarius 2024-04-18 (1 trade, U2 as expected); BURNMKR aquarius 2026-10-06 (2); USDE sushiswap 10-16 → 11-04 (50); SUSDE sushiswap 10-16 → 11-04 (38); BLNT comet 10-26 (10); USDE aquarius 10-29 → 11-03 (5); SPY aquarius 11-01 (1). **The last row is 2026-11-04: nothing under a SAC id since the 11-05 deploy.**
  - BLNT's 10 trades are byte-identical on `1m` (the 0338 reprice) but not on the coarse tiers, which is why `1d` counts 10 more only-copy trades than `1m` in 202610.
  - `dev_read` has a 30 s `max_execution_time`, so the whole-table `1m` query timed out. `1m` was re-run per month over the months `1d` shows (a `1m` row always rolls into `1d`); each took ≤ 2 s.
  - **Consequence for the PC9 gate:** every only-copy row is from an AMM source, so the backfill is an `events-backfill` reprice (as in 0338), not phase 3, which also refuses the current month 202611. Ranges: 2026-10-01 → 2026-11-05, and 2024-04-18 for Z, or record Z's single trade as a loss.
- **2026-11-18, Z's trade recorded as a loss (PC9, decided by the operator).** Z on Aquarius, 2024-04-18, 1 trade: the only copy, under Z's SAC id. Stage B wrote 202404 before the 2026-10-06 seed, so the trade never reached the classic id. Z has a classic, but the row is still an only copy, so it takes PC9's path and is not deleted silently.
  - Not repriced. The backfill has one window: 2026-10-01 → 2026-11-05.
  - Why: a 2024-04-18 reprice needs its own `events-backfill` window and a second `preroll-amm-reprice.sql` run over 202404, a month phase 3 has already finished. If `1m` no longer holds the whole of 202404, that pre-roll rebuilds the month's `1w`/`1M` buckets from one day of `1m`. One trade does not justify that risk.
  - The 202404 row is copied in §5c with the rest of the residual (the rollback copy) and deleted in §5d.
  - After the reprice, the re-run map expects `only_copy = 0` for 202610 and 202611. The single 202404 row is this recorded loss.

- **2026-11-18, the reprice set-up (plan A).** Run from fishuser-hero, as the 0286 re-ingest was: fishuser-hero's key `~/.ssh/hero-ch-prod` still reaches `deploy@ch-prod-01` (CH answers `default`, 26.3.10.60). Each `events-backfill` runs on the host over SSH, with the password on stdin. No secret is stored on any disk.
  - Binary: `37e2498ef35adc7b88278182e6889c5cbc91ed86e917febc0f038fdfad470e40`, built from `develop` `00ad6090` with `cargo lambda build --release --target x86_64-unknown-linux-musl -p events-backfill` on the local machine. It replaced `3e0e521a…` (the 0338 build of 2026-11-02, before #390), which is kept as `~/events-backfill.pre-reprice-20261118` on the host.
  - One-ledger dry run at 51,500,460: `unproven sac swaps: 0`, the 0242 line.
  - `prices-production-cleanup`: DISABLED.
- **2026-11-18, the reprice and the pre-roll (plan B).** From fishuser-hero.
  - Dry run 64,705,404 → 65,318,468 (9 s): 1,558,996 events, the same count measured beforehand. Ticks: aquarius 504,153, comet 3,372, phoenix 1,465, soroswap 68,961 (= BE's raw swaps), sushiswap 30,631. `unproven sac swaps: 0`, `unresolved pools: 0`.
  - Two warnings, neither caused by this run. First, 8 Phoenix events fail on pool `CBCZGGNO…` with "unexpected topic shape": BE holds 1,469 Phoenix swaps, the tool and live both 1,465, so it is an existing extractor gap and no 0242 asset trades on Phoenix. Second, the coverage probe sees 12 contracts outside `pool_registry` (308,611 events); `prices-production-coverage-sweep-unclassified` has been in ALARM since 2026-11-09, outside this task.
  - Reprice at 12:48 UTC (10 s): 263,934 `1m` rows, the same tick counts.
  - Manual `prices-production-enrichment` run: 859 rows enriched. 99 % of the window's remaining `close_usd = 0` rows are in pairs never priced in the window (no USD route), so they are not repriced rows left unpriced.
  - The six `mv_reconcile_*` views were stopped (checked `Disabled`). `preroll-live-gap.sql` ran over `2026-10-01 00:00:00` → `2026-11-18 12:00:00` (`PREROLL OK`, a few minutes). The views were started again (checked `Scheduled`, no exception). The password was unset on fishuser-hero.
  - After the pre-roll, every tier equals `1m` per source over 10-01 → 11-18. Sushiswap went 37,917 → 38,005 and aquarius 734,695 → 734,701; nothing else moved. October and November `1M` equal their days, and the 7 weeks from 09-28 equal their days.
  - The pre-roll set about 120 `sdex` coarse rows to `close_usd = 0` (103 / 15 / 3 on `1h` / `4h` / `1d`), as in 0338, where the next coarse sweep restored them.
- **2026-11-18, §5a and the §5b gate (plan C).**
  - §5a: `bak_0242_sac_ids` created as `default` from fishuser-hero: `40 0`. AC4 "before": 215,216 identities, 215,216 ids, 0 collisions.
  - SAC-keyed rows sit in 202404, 202610 and 202611 only, on every tier.
  - Byte-level test: `1m`, `15m` and `1h` are fully byte-identical in 202610 and 202611 (79 + 18, 65 + 18, 59 + 18 rows). 23 coarse rows are not byte-identical: 1 on `4h`, 3 on `1d`, 9 on `1w`, 10 on `1M`. In each, the classic row also holds that pair's other trades in the bucket. All 23 were checked: the classic row equals the sum of its classic `1m` children, and holds at least the SAC row's trades. Since every SAC `1m` row is byte-identical, every SAC trade is in the classic coarse row. No trade in 202610 or 202611 is an only copy.
  - The only only-copy rows are 202404's, one per tier, Z's single trade: the recorded loss. **The gate passed.**
- **2026-11-18, §5c–§5g, the cleanup (plan C).** As `default` from fishuser-hero.
  - §5c: only `bak_0242_sac_ids` and `bak_0242_seed` existed before. `bak.sql` had 18 statements. The column lists left out only `assets.asset_id` (MATERIALIZED) and the six EPHEMERAL `base_*`/`quote_*` columns. Copies: `assets` 40, `asset_symbol` 40, and `1m` 98, `15m` 84, `1h` 78, `4h` 70, `1d` 47, `1w` 22, `1M` 16 rows. Each tier's copy equals the live SAC-keyed rows exactly: 107 trades and 12,911.293623 base volume, partitions 202404, 202610 and 202611.
  - §5d: 21 `DELETE … IN PARTITION ID` statements (7 tiers × 3 partitions, fine tiers first). After them, every tier reads 0 SAC-keyed rows (M4). Per source over 10-01 → 11-18, every tier dropped by exactly the SAC copies: aquarius 734,701 → 734,693 (8), comet 6,181 → 6,171 (10), sushiswap 38,005 → 37,917 (88). `1m` to `1d` still agree. Z's 202404 trade went with its partition: the recorded loss.
  - §5e and §5f: `assets` 40 → 0, then `asset_symbol` 40 → 0.
  - §5g: M1, the SAC contract rows, read 0. `identity_by_contract` for the 40: 40 contracts, 0 contract rows, `max(n) = 1`. AC4: 215,216 → 215,176 identities and ids (each −40), collisions 0. The 40 classic rows are all present. 42 contract rows remain, the genuine Soroban tokens. Manual probe: `sac_contract_rows: 0`, `collisions: 0`. API: XCR by its SAC is `XCR:GBLJ…`, `credit`, `contract ""`; `/ohlcv` by the SAC and by `CODE:ISSUER` is `SAME`; the XLM SAC answers `native` / `XLM`.
  - The `bak_0242_*` tables stay until this task is archived (§5h rollback).
  - Scheduled probe: `SacContractIdentities` 40 until 13:01 UTC, then 0 at 13:16 and 13:31. §6's pre-check (0 for at least two periods) is met.
  - The 13:30 coarse sweep restored the `sdex` coarse `close_usd = 0` counts the pre-roll had raised, to exactly their earlier values: `1h` 2,670,725, `4h` 1,527,763, `1d` 556,120.
- **2026-11-19, §6, the alarms (plan D).** The first release through `deploy/production` ([[0343]]).
  - #425 set `opsAlarms.sacIdentityAlarmsEnabled` and `opsAlarms.sacContractIdentitiesActionsEnabled` to `true` (approved by adamkoot, merged as `97b4cd62`). Release PR #426 `develop` → `deploy/production` merged as `c7dcd26d` (parents `7a85607b`, `97b4cd62`).
  - The release also carried [[0348]] (#422, #423). Compute was already live from `develop` `21b938fa` (deployed by its owner at 09:50 UTC, confirmed); `cdk diff Prices-production-Compute` from `c7dcd26d` showed only Lambda code hashes, so Compute was not redeployed.
  - Pre-check: `SacContractIdentities` `Maximum` 0 in every 15-minute period 09:24–10:24 and 11:22–11:52 UTC. `SacUnprovenSkipped` and `SacCandidatesUnavailable` had no datapoint in the previous two weeks.
  - Diff of `Prices-production-Observability` from `c7dcd26d`: the three alarms added, the dashboard strip 77 → 80 owned alarms, `DashboardAlarmCount` 87 → 90, nothing replaced or removed. `--strict` also lists 40 existing alarms, each an `AlarmDescription` whose deployed copy reads `?` for a non-ASCII character; no threshold, metric or action changes.
  - `make deploy-production-observability` from a clean detached checkout of `c7dcd26d`: `UPDATE_COMPLETE` at 12:55:28 UTC. Compute untouched (last update 09:50:48 UTC).
  - Post-check: `prices-production-sac-contract-identities`, `prices-production-ledger-processor-sac-unproven` and `prices-production-ledger-processor-sac-candidates-unavailable` each `ActionsEnabled = True`, `OK`, one alarm action.
  - `master` fast-forwarded `7a85607b` → `c7dcd26d` through the admin bypass; tag `prod-2026-11-19` on `c7dcd26d`.
  - From here the SAC metrics are watched by alarms; the daily read by hand (D8) ends.

## Plan to completion (2026-11-18)

State: stage D done, the 0242 code live since 2026-11-05, the map read on 2026-11-17. 40 SAC ids, all with a classic. Residual: 202404 (Z, a recorded loss) and 202610–202611 (94 only-copy trades, all AMM). Order: A → B → C, with [[0343]] Phase 1 done during B's runs, then D as the first release through the new flow, then E.

Three findings shape the plan:

- The `events-backfill` on the host was built for [[0338]] around 12:33 UTC on 2026-11-02, before #390 merged (13:28). A fresh build is needed.
- An enriched row outranks a reprice row of the same minute (0338). A classic-id minute that already holds a row keeps it, and the repriced trades of that minute are lost. The re-run map (step 9) catches this.
- A pre-roll that ends at 2026-11-05 rebuilds the week of 11-02 and November's `1M` from a partial slice. 0338 left those buckets to the reconcile MVs, but their 7-day look-back no longer reaches 11-05.

### A. Before the reprice

**Who runs what (decided 2026-11-18).** Every write runs on the Hetzner host as ClickHouse's `default` user (SSH, then `docker exec -i app-clickhouse-1 clickhouse-client`), the same place and user as `events-backfill`. The operator holds no `dev_shared` certificate, so the runbook's `chq` writes are replaced by this route. The agent prepares each write as a block with the real values and checks each result as `dev_read` before the next step. The operator pastes and runs every write.

1. **Host:** `SELECT currentUser()` prints `default`. The checks run as `dev_read` from the local machine.
2. **Ledgers (measured 2026-11-18, `dev_read`):** start **64,705,404** (closed 2026-10-01 00:00:02; 64,705,403 closed 09-30 23:59:57). End **65,318,468** (closed 2026-11-05 11:32:59): the cursor the 0242 build resumed from at its cold start (Heal log). Every earlier ledger was written by the old build. About 613k ledgers.
3. **Local machine → host:** build `events-backfill` from the current `develop` (`cargo lambda build --target x86_64-unknown-linux-musl`, as in 0338). It must contain #390. Record its sha256 here.
4. `prices-production-cleanup` is still DISABLED. It stays disabled until step 7 is done.

### B. The reprice, 2026-10-01 → 2026-11-05

5. **Host, as `default`, under `tmux`:** `--start 64705404 --end 65318468 --dry-run --verbose`; compare the tick counts with BE's swap count for the range; then run again without `--dry-run`.
6. **`1m`:** the repriced trades sit under the classic ids (USDE, SUSDE, BURNMKR, BLNT, SPY, per source and day). Every repriced row has `close_usd`; if not, `coarse-repair --table price_ohlcv_1m` for 202610 and 202611 (0338 step 7).
7. **Pre-roll with `preroll-live-gap.sql`.** Never `preroll-amm-reprice.sql`: it is historical since [[0286]].
   - `start_ts = '2026-10-01 00:00:00'`, a month edge; the script aligns the week itself.
   - `end_ts` = the start of the current hour at run time (decided 2026-11-18), not 2026-11-05. Ending at 11-05 would leave the classic rows of the week of 11-02 and of November without the repriced trades: the pre-roll's partial rows lose on version to live's rows, and the reconcile MVs no longer reach back to 11-05.
   - The window then overlaps the reconcile MVs' 7-day look-back. Stop the six `mv_reconcile_*` first ([[0286]] runbook §1a; six rows `Disabled`). START them after the checks below. `prices-production-mv-refresh-disabled` fires while they are stopped: expected.
   - After: each repriced day agrees on `1m`, `15m`, `1h`, `4h` and `1d`; each week equals its days; October and November equal their days.
   - Count `close_usd = 0` on the coarse tiers in the window. In 0338 the pre-roll left 72 `sdex` rows at 0. Let the enrichment worker price them, or run `coarse-repair`.

During steps 5–7: [[0343]] Phase 1 (the `production` environment and the two rulesets). It touches only GitHub settings.

### C. The gate and the cleanup (runbook §5)

8. **§5a:** create `bak_0242_sac_ids`. Expect **40** rows (the runbook says 36), with `classic_id = 0` at 0. Record AC4's "before": identities, ids, collisions.
9. **§5b, the re-run map:** `only_copy = 0` on every tier for 202610 and 202611. 202404 reads 1 only-copy row, Z's recorded loss. Any other only-copy row is a minute lost to an enriched row: STOP, and decide it under PC9 before step 10.
10. **§5c:** pre-check that only `bak_0242_sac_ids` and `bak_0242_seed` exist (there is no `bak_0242_seed_stage`). After the copies, each tier equals its `residual_rows`, and `assets` and `asset_symbol` hold 40 each.
11. **§5d → §5f:** the candles fine tiers first, then the 40 `assets` rows, then the 40 `asset_symbol` rows. Every post-check reads 0.
12. **§5g:**
    - M1: 0 SAC contract rows.
    - `identity_by_contract`: 40 contracts, 0 contract rows, `max(n) = 1`.
    - AC4: identities and ids each down by exactly 40, collisions unchanged.
    - The probe reads `sac_contract_rows = 0`.
    - The API: XCR by SAC and by classic print `SAME`; the XLM SAC answers as XLM.

### D. The alarms (runbook §6): the first release through the [[0343]] flow

Precondition: 0343 Phase 1 done. Until this deploy, no alarm watches `SacUnprovenSkipped` or `SacContractIdentities`: read both by hand at least daily (D8).

13. A PR to `develop` sets `opsAlarms.sacIdentityAlarmsEnabled` and `opsAlarms.sacContractIdentitiesActionsEnabled` to `true` in `infra/envs/production.json`.
14. Before the release PR: `git diff origin/deploy/production origin/develop -- . ':!lore'` holds only that flag change, or every other change in it is known and meant to ship. On 2026-11-18 the two branches differed only in `lore/`.
15. Pre-check: `SacContractIdentities` reads `0` for at least two periods (30 min).
16. A release PR `develop` → `deploy/production`, approved, merged with a merge commit. From a clean checkout at its tip: `cd infra && make deploy-production-observability`.
17. Post-check: all three alarms print `True`, then `OK`.
18. `master` fast-forwarded to the deployed commit through the admin bypass (no workflow until 0343 Phase 2), and the commit tagged `prod-YYYY-MM-DD`.

This release tests the branch rules. It does not test the `production` environment approval or the `promote` job: those come with 0343 Phase 2.

### E. Close

19. §7 evidence into this task: the outputs of §5a, §5b, §5g and §6, against AC1–AC4.
20. Tell BE: each SAC now resolves to one classic row in `identity_by_contract`.
21. Archive this task as `completed`. Then drop the `bak_0242_*` tables, one by one.

### Rollback

§5h restores the copies, metadata first. A reprice needs no rollback of its own: a re-run only replaces.

### Estimate

| Part | Hands-on |
| --- | --- |
| A + B (preparation, reprice, pre-roll, checks) | 2–4 h |
| C (§5a–§5g) | 1–2 h |
| §5h, only if a check fails | 15–30 min |
| D (§6, without review waits) | ~1 h |

Measured bases: 0338 enriched 334 rows in about 90 s; the 2026-11-17 map took ≤ 2 s per month on `1m`. No reprice or pre-roll wall time is on record, so those two are the largest unknowns.

### Settled 2026-11-18

- The pre-roll `end_ts`: the start of the current hour, with the reconcile MVs stopped (step 7).
- Both ledgers measured (step 2).
- The write user: `default` on the host. The split: the agent prepares, the operator runs (top of A).
- No conflict between the reprice and the reconcile MVs: they look back 7 days (11-11 → 11-18), and the reprice writes `1m` for 10-01 → 11-05.

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
