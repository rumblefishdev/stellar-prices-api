---
title: "SDEX filter strategy + checkpoint contract + asset-discovery side-effects"
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, filter, backfill, checkpoint, asset-discovery, spec, stream-2]
links:
  - "../README.md"
  - "../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/R-sdex-operation-xdr-shape.md"
  - "../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md"
  - "../../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "./profile/results.md"
  - "../../../../docs/prices-api-general-overview.md"
  - "../../../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      Closes task 0022 points (1) filter strategy, (6) checkpoint
      contract, (7) asset-discovery side-effects. Profile harness
      output is in ./profile/results.md.
---

# SDEX filter strategy + checkpoint contract + asset-discovery side-effects

This note pins the **filter + control-plane** half of the SDEX backfill
contract. The decode + bucket half is in
[G-sdex-decode-and-bucket-spec](./G-sdex-decode-and-bucket-spec.md).
Both notes are written so that task 0012's Rust implementation can be
graded against them clause-by-clause.

## TL;DR

| Concern                  | Decision                                                                              |
| ------------------------ | ------------------------------------------------------------------------------------- |
| Filter strategy          | Decode the ledger fully; walk `OperationResultTr` discriminant; emit only `TxSuccess` ops. The cheap pre-filter people imagine **does not exist** — `stellar-xdr` decodes atomically. |
| Filter CPU cost          | 3.22 ms / ledger (decode) + 9 µs / ledger (walk). 311 ledgers/s single-thread. ~7× §5.6's 42 ledgers/s target. |
| Trade-bearing density    | 99.35 % of recent-range ledgers carry ≥1 claim atom. Density filter saves almost nothing in the modern range. |
| Checkpoint granularity   | Per-ledger. `UPDATE backfill_progress SET current_ledger = $1 WHERE task_name = 'sdex_archive'` runs in the same DB transaction as the ledger's bucket flush. |
| Asset discovery          | Insert-on-fly with `ON CONFLICT (asset_type, asset_code, issuer_id, contract_id) DO NOTHING`. The Asset Discovery Lambda enriches metadata later. |
| End-to-end backfill time | **12–16 days** single-task for all ~57M ledgers (archive S3 GET is the bottleneck, not decode/walk/DB). Parallelisable across disjoint ranges: 2 tasks → ~6–8 d, 4 tasks → ~3–4 d. See §4. |

## 1. Filter strategy

### 1.1 The framing the README implied isn't there

Task 0022's README enumerated three filter options:

> parse `TransactionResultMeta` only and short-circuit on absence of
> `ClaimAtom` arrays; pre-filter on `operation.body.type` discriminant
> before touching results; ledger-level early-out via counts in
> `LedgerHeader`.

All three assume **partial XDR decoding** is cheaper than full decoding.
It isn't, with the `stellar-xdr` crate. `LedgerCloseMetaBatch::from_xdr`
performs a single structural pass over the whole payload — there is no
public API to decode a header without paying for the tx-processing
walk. The only ways to "filter cheaper than decode" would be:

- maintain an **out-of-band** index of trade-bearing ledger numbers
  (Option B from task 0020, vetoed by [ADR 0002](../../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md));
- write a **bespoke partial decoder** that reads `LedgerHeader` then
  scans for op-type magic bytes (high-cost engineering, low payoff —
  see §1.3).

This task is committed to ADR 0002's design (no out-of-band index) and
explicitly does **not** pursue a bespoke partial decoder. So the
filter is post-decode by construction.

### 1.2 Post-decode filter: structural variant-walk

After `LedgerCloseMeta` is decoded, the filter is the trivial walk:

```text
for tx in ledger.tx_processing:
    if tx.result.result.result is not TxSuccess(ops):
        continue                                # ← protocol filter
    for op in ops:
        if op is not OpInner(tr): continue
        match tr:
            ManageSellOffer(Success(s))             → emit s.offers_claimed
            ManageBuyOffer(Success(s))              → emit s.offers_claimed
            CreatePassiveSellOffer(Success(s))      → emit s.offers_claimed
            PathPaymentStrictReceive(Success(s))    → emit s.offers
            PathPaymentStrictSend(Success(s))       → emit s.offers
            _                                       → skip
```

This is essentially free relative to the structural decode itself. The
profile run measured **9 µs / ledger** for the walk vs **3.22 ms /
ledger** for the decode — three orders of magnitude difference. The
implication is that there is no efficiency gain to be had by being
clever about which sub-trees of the ledger to skip; do them all.

The `TxSuccess` filter is **not optional**. In the 2 000-ledger
sample, **65.3 % of transactions failed**. Most are MEV/bot retries
whose op-results carry "would-have-run" data; emitting trades from
those would produce phantom prices. Spec: filter `TxSuccess` at the
transaction layer before walking ops. See task 0020 R-note §2 for
the protocol justification.

### 1.3 Why a bespoke partial decoder isn't worth pursuing

Hypothesis: skip ~50 % of decode work by reading only `LedgerHeader`
and bailing on ledgers with `txCount == 0`.

Measured reality from `notes/profile/results.md`:

- Trade-bearing density: **99.35 %** of modern-range ledgers.
- Decode throughput: **311 ledgers/s** single-thread (vs §5.6 target of ~42 ledgers/s).
- 2 vCPU Fargate ≈ 600 ledgers/s upper bound from decode.

At 99.35 % density the optimisation saves <1 % of bytes. The
engineering cost (bespoke XDR walker, ongoing maintenance against
protocol upgrades) is far above the savings. For the early-history
range (pre-2018, sparse SDEX activity) density does drop — but
early ledgers are also small (low op-count → small encoded size),
so absolute byte savings stay marginal.

**Recommendation:** ship the post-decode variant-walk. Revisit only
if real-world Fargate throughput is bottlenecked by decode, which
the profile says it won't be.

### 1.4 What the bottleneck actually is

Per `notes/profile/results.md` and §5.6:

| Layer                          | Single-thread throughput | Notes |
| ------------------------------ | ------------------------ | ----- |
| Archive S3 read                | 5–10 MB/s sustained      | §5.6 figure; bytes-per-ledger ≈ 167 KiB compressed → ≈ 30–60 ledgers/s/connection |
| Zstd decompress + XDR decode   | 311 ledgers/s            | Profile measurement |
| `OperationResultTr` walk       | >100 000 ledgers/s       | Profile measurement |
| DB UPSERT into `price_ohlcv`   | tbd; bound by RDS IOPS   | Whole-row replacement per bucket spec §5 |

The expected bottlenecks on a 2 vCPU Fargate task are **archive
transport** (S3 read latency / sustained bandwidth) and **DB
writes** (per-minute candle UPSERTs into the partitioned PG table).
The §5.6 throughput estimate of 150 000 ledgers/hour (≈ 42 ledgers/s)
matches the archive-transport ceiling, not the decode ceiling.
Task 0012 implementers should focus optimisation effort there
(connection-pooling, S3 read parallelism, batched DB writes),
not on the filter path.

## 2. Checkpoint contract

The backfill is resumable. `backfill_progress.current_ledger` is the
crash-recovery anchor: on restart, the task resumes from
`current_ledger + 1`. Spec the write semantics so that no scenario
produces double-counted trades or skipped trades.

### 2.1 Atomicity unit: one ledger, one transaction

Each ledger's processing is one PG transaction:

```sql
BEGIN;
  -- Per-(asset, minute) UPSERTs for the candles this ledger contributed to
  INSERT INTO price_ohlcv (timestamp, asset_id, granularity, open, ...)
       VALUES (...)
  ON CONFLICT (timestamp, asset_id, granularity) DO UPDATE SET ...;
  -- (one statement per affected (asset, minute) row; see decode spec §5)

  -- Checkpoint advance
  UPDATE backfill_progress
     SET current_ledger = $ledger_seq,
         last_heartbeat = NOW()
   WHERE task_name = 'sdex_archive';
COMMIT;
```

The checkpoint advance is **in the same transaction** as the bucket
flush. Either both land or neither does. Crash mid-ledger leaves
`current_ledger` at the previous ledger; the resume re-processes
the current ledger, and the UPSERT semantics (see decode spec §5)
absorb the rewrite idempotently.

### 2.2 Why per-ledger, not per-chunk

A chunk-level checkpoint ("commit every 100 ledgers, advance
`current_ledger` by 100") would be ~100× fewer checkpoint writes
but introduces a re-do window: on crash, up to 99 ledgers of work
re-executes. With UPSERT-idempotent writes this is *correct* but
wasteful. Per-ledger is cheap (one UPDATE per ledger, against the
same row, with one connection — RDS handles this trivially) and
gives the tightest restart window.

Per-ledger also aligns with the §5.6 "heartbeat every 15 min"
expectation: an alive task will checkpoint thousands of times in
that window, so a stale heartbeat unambiguously means crashed.

### 2.3 Minute-boundary handling

Per-ledger checkpoint **per-ledger** is fine for crash safety, but
note that a 1-minute candle accumulates trades across ~10–12
ledgers. The decoder cannot finalise a 1m candle until the
**next** ledger crosses the minute boundary, because the in-flight
minute may still receive more trades.

Two execution shapes are valid (decode spec §5 picks one):

- **In-memory aggregate**: hold the in-flight 1m candle in
  process memory; flush to DB on minute rollover. Per-ledger
  checkpoint then advances `current_ledger` but **does not** flush
  the in-flight minute. On crash, the in-flight minute is lost;
  resume re-reads the relevant ledgers (idempotent under UPSERT)
  and rebuilds it. ✅ Decode spec §5 picks this.
- **Per-ledger partial-row UPSERT**: each ledger writes a partial
  1m candle merge (incremental). The candle converges as more
  ledgers in the minute arrive. Crash mid-minute leaves a partial
  candle in DB; resume re-processes the same ledgers and the
  incremental-merge expressions reconverge.

Both are correct. The in-memory aggregate is simpler (matches the
schema doc's "whole-row replacement" for backfill writers — see
[database-schema-overview.md L362–365](../../../../docs/database-schema/database-schema-overview.md)) and is what decode spec §5 commits to.

### 2.4 Start-of-stream + end-of-stream

- **Start of stream** (first run): `current_ledger = start_ledger - 1`
  is seeded at provisioning. The task resumes from
  `start_ledger - 1 + 1 = start_ledger`. The schema doc seeds both
  rows at `start_ledger = 1`, `target_ledger = tip-at-bootstrap-time`.
- **End of stream**: when `current_ledger == target_ledger`, the
  task writes `status = 'completed'` in the same transaction as
  the final ledger flush and exits 0. The API surfaces this via
  `GET /backfill/status` (see §5.6).
- **Catch-up to live**: if `target_ledger` advances during a run
  (because §5.6's catch-up mode re-targets the running task to the
  live tip), the task transitions to a polling mode (re-read
  `target_ledger` from DB on each ledger) until both converge.

The schema doc treats `target_ledger` as immutable per task row;
in practice the operator advances it manually for catch-up. This
is documented behaviour, not a design hole.

## 3. Asset discovery side-effects

A `ClaimAtom`'s `asset_sold` / `asset_bought` reference assets by
their canonical identity (4-tuple — see decode spec §2). The
`price_ohlcv.asset_id` foreign key requires a row in `assets`.
Backfill regularly encounters assets that the Asset Discovery
Lambda has not yet observed.

### 3.1 Strategy: insert-on-fly, idempotent, light metadata

When the backfill encounters an unknown asset:

```sql
INSERT INTO assets (
    asset_type,      -- 0=native, 1=alphanum4, 2=alphanum12, 3=contract (SAC)
    asset_code,      -- '' for native
    issuer,          -- '' for native; G... StrKey otherwise
    contract_address,-- NULL for non-SAC classic assets; resolved later if SAC
    first_seen_at    -- NOW() — backfill timestamp, not on-chain timestamp
)
VALUES ($1, $2, $3, NULL, NOW())
ON CONFLICT (asset_type, asset_code, issuer) DO NOTHING
RETURNING id;
```

If the row already exists, `RETURNING` is empty; the backfill
issues a follow-up `SELECT id FROM assets WHERE ...` to get the
surrogate id. This is two round-trips for unknown-then-known
assets but the unknown case is bounded (one per unique asset),
not per-claim. In practice the asset table fills out within the
first few thousand ledgers of backfill and the unknown-asset path
quiesces.

**Implementation tip** (task 0012): an in-process LRU cache keyed
by `(asset_type, asset_code, issuer)` → `asset_id` eliminates the
SELECT round-trip on the second-and-later occurrences within a
backfill session. Cache size 50 000 covers the long-tail asset
universe (Stellar mainnet ≈ 7 k distinct assets with non-trivial
trade activity per 2024 retro).

### 3.2 What's deferred to Asset Discovery Lambda

The backfill inserts a **skeleton** asset row only: type, code,
issuer, `first_seen_at`. The following columns stay NULL and are
filled by the Asset Discovery Lambda when it eventually scans the
issuer's `home_domain` and TOML:

- `name` (human-readable, from issuer TOML's `[[CURRENCIES]]`)
- `description`, `image_url`, `home_domain`
- `is_verified` (TOML-listed by the issuer)
- `contract_address` (SAC contract id) — backfill fills this for
  `asset_type = 3` claims; for classic assets, the SAC resolution
  happens later if the asset ever gets wrapped

The split is intentional: the backfill must not block waiting for
external HTTP fetches. The Asset Discovery Lambda runs hourly
(§5.3) and reconciles the metadata side asynchronously.

### 3.3 SAC vs classic for the same asset

A classic asset (e.g. `USDC:GA5Z…CIRCLE`) and its SAC contract
wrapper share economic identity but **different rows** in `assets`
(different `asset_type`, different surrogate id). The Asset
Discovery Lambda links them via `contract_address` on the classic
row once the wrap is detected.

The backfill does **not** auto-link: it inserts whichever form the
claim atom references. If a `ClaimAtom` carries a classic
`(asset_type=1, code="USDC", issuer="GA5Z…")`, only the classic
row gets created/touched. SDEX `ClaimAtom`s only ever reference
classic Asset variants (types 0/1/2 — see task 0020 R-note §4) —
SAC wraps only appear in Soroban events (Stream 1), not SDEX
results. So the backfill's asset-discovery path is classic-only by
construction.

### 3.4 Decimals normalisation

All `int64 amount_*` values are stroops (10⁻⁷). The backfill stores
amounts at full stroop precision (`NUMERIC(28,14)` after division
by 10⁷) — see decode spec §4. The backfill does **not** consult
`assets.decimals` because classic SDEX is uniformly 7-decimal.
This sidesteps a temporal-correctness pitfall: SAC wraps on the
same asset can adopt a different decimal count (SAC contracts can
override), but as noted in §3.3 SDEX never sees SAC variants. So
"all SDEX trades are 7-decimal" is a load-bearing invariant for
this backfill and is explicitly relied on.

## 4. End-to-end backfill runtime estimate

This section closes the loop from §1's "decode cost" through
§3's "DB write" to give a single end-to-end estimate of how long
the full historical backfill will take to finish: read each
ledger from the archive → decode XDR → extract `ClaimAtom`s →
bucket → write to `price_ohlcv`. The total is the sum of the
slowest stage at sustained pace.

### 4.1 Per-stage throughput

| Stage                                              | Throughput (per 2 vCPU Fargate task)     | Source                          |
| -------------------------------------------------- | ----------------------------------------- | ------------------------------- |
| Archive S3 GET (compressed bytes per ledger ≈ 167 KiB) | **~42–55 ledgers/s** (≈ 5–10 MB/s sustained, single connection) | [§5.6 design-doc figure](../../../../docs/prices-api-general-overview.md) — 150 000–200 000 ledgers/h |
| Zstd decompress + `LedgerCloseMetaBatch::from_xdr` | **~311 ledgers/s** on developer laptop, ≥ ~600 ledgers/s on 2 vCPU Fargate (linear scaling, well-behaved CPU work) | [`profile/results.md`](./profile/results.md) — 3.22 ms/ledger mean |
| `OperationResultTr` walk + `ClaimAtom` extract     | **>100 000 ledgers/s**                    | Profile measurement — 9 µs/ledger |
| Bucket aggregation (in memory, per minute)         | bounded by trade volume; **negligible**   | Decode spec §5.1                |
| Per-ledger UPSERT into `price_ohlcv` (whole-row replacement, ~6–12 candle rows per minute per task) | bounded by RDS IOPS on `db.t4g.small`; estimated **>500 ledger-writes/s** | RDS baseline; UPSERT semantics in decode spec §5.3 |

Three stages have ample headroom (decode, walk, DB). One stage
gates the wall clock: **archive S3 GET**. The other stages can
absorb whatever rate that throws at them.

### 4.2 Single-task estimate

§5.6's stated rate of **150 000–200 000 ledgers/hour** corresponds
to **42–55 ledgers/s** sustained. Plugging into the full
historical range:

| Range                                  | Ledgers      | At 42 ledgers/s | At 55 ledgers/s |
| -------------------------------------- | ------------ | --------------- | --------------- |
| Ledger 1 → tip (Nov 2015 → mid-2026)   | ~57 000 000  | **377 h ≈ 15.7 d** | **288 h ≈ 12.0 d** |

This matches §5.6's published "~380 hours (~16 days) of pure
compute" estimate for the SDEX archive backfill. The number is
ledger-count-dominated, not byte-dominated; early ledgers
(pre-2018) are tiny (low op-count, low compressed size) and
process meaningfully faster than the 167 KiB/ledger mean from the
modern sample, so the **real** runtime is probably 10–14 days
not 16. §5.6 calls the 16-day figure conservative for that
reason; this spec agrees.

### 4.3 Parallelism if 12–16 days is too slow

The extractor parallelises trivially across **disjoint ledger
ranges** because:

- Per-ledger checkpoint (filter spec §2.1) is per-row in
  `backfill_progress`; multiple tasks can claim distinct ledger
  ranges with their own progress rows (e.g. `sdex_archive_a`,
  `sdex_archive_b`, …).
- UPSERTs converge correctly under PG row-level locking — see
  decode spec §5.5 and the schema doc's "Concurrency" note.
- There is **no ordering requirement** for backfill writes: the
  in-memory minute aggregator emits whole 1m candles for past
  minutes, which are write-once final values, not partial merges.

| Tasks | Wall time at 42 ledgers/s/task | Wall time at 55 ledgers/s/task |
| ----- | ------------------------------ | ------------------------------ |
| 1     | 15.7 d                         | 12.0 d                         |
| 2     | 7.9 d                          | 6.0 d                          |
| 4     | 3.9 d                          | 3.0 d                          |
| 8     | 2.0 d                          | 1.5 d                          |

Practical ceiling: ~4 parallel tasks before archive-bucket S3
rate limits or DB IOPS start mattering. Going beyond is possible
but needs a separate scaling note.

### 4.4 What's NOT in this estimate

- **Cold-start / warm-up time** for the Fargate task itself
  (negligible — single-digit seconds).
- **The downstream `volume_quote_usd` enrichment pass** — see
  decode spec §5.3 and §6 item 2. That pass runs after the
  backfill completes and reads `oracle_prices`; its runtime is
  separate and small (one pass over already-written 1m rows).
- **OHLCV rollups** (15m / 1h / 4h / 1d / 1w / 1M) — produced by
  the existing OHLCV Rollup Lambda from the 1m base; not the
  backfill's responsibility.
- **Stream 1 (Soroban AMM) backfill** — independent task with
  its own ~hours-long runtime (§5.6, ADR 0001).

### 4.5 Recommendation

Run the SDEX backfill as a **single 2 vCPU / 4 GB Fargate task**
per §5.6, projecting 12–16 days of pure compute. If the project
timeline requires faster, scale to 2–4 parallel tasks on disjoint
ledger ranges before considering more invasive changes (e.g.
custom XDR walker, decode-throughput optimisation). The decode
stage is **7× faster than the archive read rate** on developer
hardware (§1.4), so there is no value in spending engineering
effort on the decoder; the gain is elsewhere.

## 5. Cross-references for task 0012

The Rust module shape this spec implies:

```
crate::sdex_backfill::filter        — §1.2 walk + §1 TxSuccess gating
crate::sdex_backfill::checkpoint    — §2 atomicity (one tx per ledger)
crate::sdex_backfill::asset_resolve — §3 insert-on-fly + LRU cache
```

The corresponding [decode-and-bucket spec](./G-sdex-decode-and-bucket-spec.md)
covers `extract` (variant decode, pair canonicalisation, price math)
and `bucket` (1m candle aggregation, UPSERT).

Both specs are written so the Rust submodules above map 1:1 to
spec sections — task 0012 implementers can review compliance
clause-by-clause.

## 6. Notes / open items

- The 65 % tx-failure rate observed in the modern sample range is
  much higher than I'd have guessed. If similar rates hold across
  history, this is interesting on its own (MEV / failed-arbitrage
  density) but does not change this spec.
- The profile harness lives in [`./profile/`](./profile/) and is
  not part of the main `stellar-prices-api` workspace (the Rust
  impl lands in task 0012). The harness is a research artifact
  that can be re-run against new sample ranges.
- V0 `ClaimAtom`s do not appear in the modern sample (V0 is
  pre-protocol-18). Decode-spec coverage of V0 is from the
  protocol XDR spec, not from observed mainnet data. This is a
  small gap that early-history backfill runs will close
  observationally.
