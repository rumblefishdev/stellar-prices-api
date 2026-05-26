# Profile run — 2026-05-13

Run on `.temp/FC47D9FF--62400000-62463999` (mainnet ledgers
~62,400,000–62,463,999, ≈ 2026-05-07, protocol 22). 2 000-ledger sample.

Hardware: developer laptop, single thread, release build.

## Input

- Files scanned: 2 000
- Ledgers decoded: 2 000
- Compressed bytes read: 325.86 MiB (mean ≈ 167 KiB / ledger)

## Timing (single-threaded)

| Phase               | Total | Mean/ledger | Throughput         |
| ------------------- | ----- | ----------- | ------------------ |
| Decompress + decode | 6.43s | 3.22 ms     | **311 ledgers/s**  |
| Claim-atom walk     | 0.02s | 0.009 ms    | (essentially free) |
| End-to-end (wall)   | 6.45s | 3.22 ms     | **310 ledgers/s**  |

Decode dominates wall time (>99%). The variant-discriminant walk
(`OperationResultTr` match + `ClaimAtom` variant tally) is free
once the structural decode is done.

§5.6's design-doc estimate was **150 000–200 000 ledgers/hour**
(≈ 42–55 ledgers/s) on a 2 vCPU / 4 GB Fargate task. The profile
shows **single-threaded > 7× that** on developer hardware, which
means the §5.6 estimate is conservative and the bottleneck on
Fargate will be archive transport / DB writes, not decode.

## Trade-bearing density

- Trade-bearing ledgers: **1 987 / 2 000 (99.35 %)**
- Mean claim-atoms per trade-bearing ledger: **98.4**
- Median: 105 · P95: 182 · Max: 396

Modern mainnet ledgers carry trades essentially every block, so a
"skip trade-empty ledgers" filter saves almost nothing for the
post-2024 range. The filter is structurally useful for early
history (pre-2018, sparse SDEX activity) but the cost we'd save
there is small relative to the total backfill (early ledgers
dominate count but barely move bytes).

## ClaimAtom variant distribution

| Variant        | Count   | Share   |
| -------------- | ------- | ------- |
| V0 (legacy)    | 0       | 0.00 %  |
| ORDER_BOOK     | 39 825  | 20.36 % |
| LIQUIDITY_POOL | 155 749 | 79.64 % |

V0 is empty as expected — V0 atoms are pre-protocol-18 only
(≈ pre-Aug 2022). The sample is protocol 22.

LIQUIDITY_POOL dominates 4:1 over ORDER_BOOK in the recent range:
path-payment routing through classic LPs (Stellar's
LiquidityPoolDeposit-shaped classic pools, distinct from Soroban
AMMs) is the bulk of on-chain "SDEX" trade volume now.

## Op-level

- Total ops in successful txs: 723 672
- Trade-bearing ops: 75 711 (**10.46 %** of all successful ops)
- Successful txs: 206 864
- Failed txs: 389 482 (**65.31 %** of all txs — most are MEV/bot retries)

The 65 % tx-failure rate is striking but expected for recent
mainnet (high-frequency offer-replacement bots). The protocol
filter (`TxSuccess` only) is critical — without it, the extractor
would emit phantom trades for ~190 k failed-tx atom-equivalents
in this 2 000-ledger window.

## Implications for the spec

1. **Decode cost is fixed.** The "filter strategy" choice
   collapses to: do the structural decode, then a free
   variant-tally walk. There is no cheaper pre-filter against
   archive bytes alone.

2. **Trade-bearing density is high enough that filtering on it
   barely helps.** A pre-2018 ledger range would have lower
   density; modern range is ~100 % trade-bearing.

3. **Throughput headroom is large.** 311 ledgers/s
   single-threaded means even one Fargate vCPU is ~7× the
   design-doc target. DB writes and archive S3 reads are the
   bottlenecks worth optimising, not decode.

4. **Variant distribution drives DB write shape.**
   ORDER_BOOK and LIQUIDITY_POOL are both present and must be
   first-class. V0 happens on old history; the decoder must
   support it but the per-claim cost is negligible.

5. **Failed-tx phantom-trade pitfall is real and large.** Spec
   must require `TxSuccess` filter before walking op results.
