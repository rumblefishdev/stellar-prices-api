---
id: "0335"
title: "Bump stellar-xdr 28→30 before the protocol 30 vote — every ledger after it carries a millisecond StellarValue arm that 28 cannot decode"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0277", "0330", "0325", "0098", "0091"]
tags: [layer-backend, priority-high, effort-small, phase-live, resilience, ingestion, deployment]
links:
  - "../archive/0277_FEATURE_stellar-xdr-28-protocol-28-bump.md"
  - "../../../docs/runbooks/xdr-protocol-watch.md"
  - "https://github.com/rumblefishdev/soroban-block-explorer/pull/673"
  - "https://github.com/stellar/rs-stellar-xdr/releases/tag/v30.0.0"
history:
  - date: "2026-10-19"
    status: active
    who: akot
    note: >
      Created after BE announced their stellar-xdr 30 bump (BE #673, lore-0649,
      merged; BE develop head d1251f30). Our lock still pins xdr-parser at
      31be5f7, so nothing is broken today, but the next `cargo update -p
      xdr-parser` fails to build unless stellar-xdr moves to 30 in the same
      change. No vote date announced yet.
  - date: "2026-10-19"
    status: active
    who: akot
    note: >
      Bump done on feat/0335_stellar-xdr-30-protocol-30-bump, not yet
      committed. The lock resolves one stellar-xdr (30.0.0) and xdr-parser is
      at d1251f30. The only compile break was BE's event API rework
      (ADR 0059), not XDR. On 485 real mainnet ledgers the adapted extraction
      gives the same 360,847 events as develop. Workspace tests: 1448 passed,
      0 failed.
  - date: "2026-10-20"
    status: active
    who: akot
    note: >
      PR #406 merged to develop as af504c37 (approved). CI green, including
      328/328 ClickHouse integration tests. Not deployed yet: ledger-processor
      and the EventBridge Lambdas are next, then the post-vote crossing
      measurement.
  - date: "2026-10-20"
    status: active
    who: akot
    note: >
      Deployed on the XDR 30 binary: Compute (ledger-processor) at 08:31 UTC
      and EventBridge (asset-discovery and the rest) at 09:02 UTC. CodeSha256
      changed on both, with 0 errors after. Stays active until the candle
      frontier is measured across the protocol-30 activation ledger.
---

# stellar-xdr 28 → 30 for protocol 30

## Summary

Protocol 30 (CAP-0088) makes the millisecond `StellarValueExt` arms
(`SignedMs`, `EmptyTxSetMs`) mandatory from the first ledger after the vote.
Our workspace pins `stellar-xdr = "=28.0.0"` (`Cargo.toml:30`), and the
ledger-processor decodes every Galexie object with it. On 28 the first
protocol-30 ledger fails to decode, the batch errors, and the cursor stops
moving. That is the protocol-27 freeze (0091) all over again. The bump has to
ship and be **deployed** before the vote.

## Context

- **BE side is done.** `xdr-parser` on BE `develop` now requires
  `stellar-xdr` 30 (BE `Cargo.toml:41`, `version = "30"`). We track it with
  `branch = "develop"`, and our lock pins rev `31be5f7` (the proto-28 rev from
  0277). Bumping one without the other puts two XDR majors in the graph and
  `decode_object` stops type-checking. The two move together.
- **The compiler will not prove readiness** (0277's correction). CAP-0088
  changes `StellarValueExt`, and we only read `scp_value.close_time`, a plain
  field. So a clean `cargo check` says nothing. Proof has to be a decode of a
  ledger carrying the new arm through our own path.
- **`closed_at` stays in seconds.** CAP-0088 keeps
  `closeTime == closeTimeMs / 1000`, and BE asserts the same in
  `crates/xdr-parser/tests/protocol_30_arms.rs`.
- **CAP-0084** adds `ScAddress::MuxedContract` (`W…`). We only construct
  `ScAddress::Contract` and never match `ScAddress` exhaustively, so no compile
  break is expected. It matters only if a pool swap names a muxed contract.
- **The watch** (`xdr-protocol-watch`, 0098/0330) goes LAGGING once Horizon
  reports `core_supported_protocol_version = 30`, because crate 30.0.0 is
  already published. That alarm is a backstop, not the plan.
- **Out of our hands:** BE's Galexie on captive core 30 (BE lore-0650).
  Protocol 29 stalled on Galexie, not on XDR (0325).

## Implementation Plan

1. `Cargo.toml`: `stellar-xdr = "=28.0.0"` → `"=30.0.0"`, keeping the exact pin.
2. `cargo update -p xdr-parser -p stellar-xdr`. The lock must resolve exactly
   one `stellar-xdr`, and `xdr-parser` lands on BE `develop`.
3. Fix whatever the compiler reports.
4. Decode test: a ledger with `StellarValueExt::SignedMs` and one with
   `EmptyTxSetMs`, packed as Galexie writes them (zstd `LedgerCloseMetaBatch`),
   through `decode_object` → `ledger_sequence` / `ledger_close_time` in seconds.
5. Workspace tests, plus the ClickHouse `#[ignore]` suite.
6. Deploy every binary that links `stellar-xdr`. That is the ledger-processor
   first (it decodes ledgers), then the EventBridge Lambdas. Confirm
   `CodeSha256` changed (0091: a merged bump with a stale binary changes
   nothing).
7. After the vote: measure the candle frontier crossing the activation ledger,
   as 0277 did.

## Findings 2026-10-19

**The break was BE's event API, not XDR.** Between `31be5f7` and `d1251f30`, BE
replaced `xdr_parser::extract_events` with `LedgerEvents::new(..).extract(tx,
hash)` (ADR 0059: stellar-rpc event ids). `EventSource` is gone, and
`ExtractedEvent` now carries `body: EventBody { event_type, contract_id,
topics, data }`. The new `TxEvents` returns consensus events in `.events`
(tx-level first, then per-op) and diagnostic events separately in
`.diagnostic`. The old function returned the same consensus list with the
diagnostic events appended after it. So an event's position in `.events` is
exactly the old per-tx `event_index`. `prices-ingest-core/src/soroban.rs` is
the only consumer that needed changes. `decompress_zstd`, `deserialize_batch`,
`ParseError` and `scval.rs` are byte-identical between the two revs.

**Equivalence measured, not assumed.** A scratch dump ran on both `develop`
and the branch over 485 mainnet ledgers from the public data lake (`65,021,700+`
on protocol 29 with V4 meta, `57,000,000+` and `60,000,000+` older). It printed
every consensus event (tx, index, type, contract, topics, data) plus the full
`process_ledger` output. All 360,847 events match, and so do all `amm_ticks`
and `oracle`. The one diff line is the order of two `unresolved` rows in ledger
65,021,722. They come from iterating the `amm_groups` `HashMap`, whose order is
random per process on `develop` too.

**XDR 30 gates nothing.** There is no `cap_*` feature in `stellar-xdr`
30.0.0, so `SignedMs`/`EmptyTxSetMs` are ungated. 28 rejects both
(`stellar_value_ext.rs:135`, `_ => Err(Invalid)`). `stellar-xdr` 30 also
pulls in `stellar-strkey` 1.0.0 beside our 0.0.13. No types cross between
them.

**Deploy scope.** Only the ledger-processor decodes ledgers now
(`reconcile.rs:292`). `asset-discovery` no longer calls `decode_object`, which
was 0277's scope correction. The EventBridge Lambdas still link `stellar-xdr`,
so they get rebuilt too.

**Pre-existing, not ours:** `cargo clippy --workspace --all-targets -D
warnings` fails only on `prices-clickhouse/tests/current_mv_it.rs:1187`
(`too_many_arguments`), a file this branch does not touch. Excluding that
crate, clippy is clean.

## Deploy 2026-10-20

Preflight, read 2026-10-20 at about 08:00 UTC:
`npm run xdr:verify-protocol-gap` reports pinned 30, mainnet current 29, and
**core supports 30**. Validators already run core 30, so the vote can come at
any time.

| Lambda | `CodeSha256` before | `LastModified` before |
| --- | --- | --- |
| `prices-production-ledger-processor` | `ZFuumECCwyZNKDt1B6lH48YeFxgwH6aHT+1t36FsQzg=` | 2026-10-19 08:26:51 UTC |
| `prices-production-asset-discovery` | `Zaeo0F6RslW+kdc2e5nNxScnikNbjNGkjGIks0ESIos=` | 2026-10-19 08:59:03 UTC |

Candle frontier before: sdex and aquarius 88 s behind now (07:54 UTC candle).

**Compute deployed** (`make deploy-production-compute`, stage cache flushed):

| Lambda | `CodeSha256` after | `LastModified` after |
| --- | --- | --- |
| `prices-production-ledger-processor` | `WCokytgM7mqlxS/+KiQk+QTf665qHwB7a+ssUGTD26g=` | 2026-10-20 08:31:01 UTC |

Read at 09:00 UTC, about 30 minutes later: Lambda `Errors` = 0 in every
10-minute bucket from 07:30 to 09:10 UTC. sdex and aquarius were 86 s behind,
with 29 and 28 one-minute candles since the deploy. soroswap (5) and sushiswap
(2) also wrote candles since the deploy, so the Soroban event path works on the
new `LedgerEvents` extraction. comet (1.7 h behind) and phoenix (2.7 h) had no
candle since the deploy. That is normal for them: in the previous 24 h they
wrote 12 and 8 candles, and phoenix's longest gap was 16 h.

**EventBridge deployed** (`make deploy-production-eventbridge`), every Lambda
in the stack at 2026-10-20 09:02:43–44 UTC:

| Lambda | `CodeSha256` after |
| --- | --- |
| `prices-production-asset-discovery` | `m7eceW0ZGn+gykUhHb3yhn6/haa1q86CFj+LsiH2u18=` (before `Zaeo0F6R…`) |
| `prices-production-oracle` | `+D6AZ+rytOpspELZ37v4iRBiIgj342g6NACnEvrgnY0=` |
| `prices-production-enrichment` | `M6YAUmixrmN6FlTdZkVfr60l2+yX1k97Oe9zYtdcMpc=` |
| `prices-production-supply` | `q8UWEMWZcI0gdsgRSg/VjqaH+91FoUV5w2/h38YTmc0=` |
| `prices-production-coarse-sweep` | `VLMFBYOXdBTXNDEBxS5t7ykJmCnpPCgYnYnm3B2iPX8=` |

The probes and `cleanup` were rebuilt too. The `prices-production-cleanup` rule
is still `DISABLED` (0200). By 09:13 UTC, oracle (2 runs) and the backfill and
rollup freshness probes (1 run each) had run with 0 errors, and the
ledger-processor had run 119 times with 0 errors. asset-discovery, enrichment
and supply run `rate(1 hour)` and had not run since the deploy yet.

## Acceptance Criteria

- [x] Workspace pin `=30.0.0`; `xdr-parser` on BE `develop` at `d1251f30`; one `stellar-xdr` in the lock
- [x] `cargo test --workspace` (1448 passed / 0 failed locally) and the CH ignored suite green (CI on #406: 328/328)
- [x] A test proves a `SignedMs` / `EmptyTxSetMs` ledger decodes through `decode_object` with `close_time` in seconds (`decode.rs`, table-driven)
- [x] The `xdr-parser` event API change preserves extraction: 360,847 events identical on 485 real ledgers
- [x] Ledger-processor deployed on the 30 binary, `CodeSha256` before/after recorded (2026-10-20 08:31 UTC)
- [x] EventBridge Lambdas linking `stellar-xdr` deployed on the 30 binary (2026-10-20 09:02 UTC)
- [ ] Candle frontier measured across the protocol-30 activation ledger (after the vote)
