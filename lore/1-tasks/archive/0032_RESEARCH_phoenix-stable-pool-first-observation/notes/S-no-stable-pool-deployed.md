---
title: "No Phoenix stable pool exists on mainnet (2026-05-15) — 11/11 factory pools are XYK"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [phoenix, stable-pool, negative-result, factory-survey, stream-1-consumer]
links:
  - "R-phoenix-xyk-pool-interface.md"
  - "evidence/phoenix_pool_inventory_2026-05-15.txt"
  - "../../0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md"
history:
  - date: 2026-05-15
    status: mature
    who: oski
    note: >
      Surveyed the entire Phoenix factory via query_pools(); all 11 pools
      are XYK. No stable pool deployed. Side-finding: two distinct XYK
      WASM builds in production with identical Soroban interface and
      meta strings.
---

# No Phoenix stable pool exists on mainnet (2026-05-15)

## Headline

**As of 2026-05-15 the Phoenix mainnet factory
(`CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI`) has zero
stable pools deployed.** All 11 pools returned by `query_pools()` are
XYK constant-product pools. The consumer's stable-pool decoder
described in task 0018 §3 remains source-only and unverified —
correctly so, because there is nothing to verify against yet.

## Method

1. Fetched factory WASM and inspected its interface via
   `stellar contract info interface`. Factory exposes
   `query_pools() -> Vec<Address>`, which is more direct than scanning
   `("create", "liquidity_pool")` events as the task originally
   proposed.
2. `stellar contract invoke ... -- query_pools` returned 11 addresses.
3. For each address, fetched the pool's WASM with
   `stellar contract fetch` and hashed the binary with `sha256sum`.
   See [evidence/phoenix_pool_inventory_2026-05-15.txt](evidence/phoenix_pool_inventory_2026-05-15.txt).
4. Two distinct WASM hashes appeared. The non-XYK candidate was
   `CD5XNKK3B6BEF2N7ULNHHGAMOKZ7P6456BFNIHRF4WNTEDKBRWAE7IAA`. To
   classify it, dumped its `query_config`, `query_pool_info`,
   `query_version`, and resolved its token symbols by calling
   `symbol()` / `name()` on each token contract.

## Findings

### 1. No stable pool

The token pair on the only "different-WASM" pool is **PHO/USDC**:
- `token_a` = `CBZ7M5B3Y4WWBZ5XK5UZCAFOEZ23KSSZXYECYX3IXM6E2JOLQC52DK32`
  → `symbol = "PHO"`, `name = "PHO:GAX5TXB5RYJNLBUR477PEXM4X75APK2PGMTN6KEFQSESGWFXEAKFSXJO"`
  (Phoenix's own governance token).
- `token_b` = `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75`
  → `symbol = "USDC"`, `name = "USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"`
  (Centre USDC).

A stable curve only makes sense for two pegged-to-the-same-target
assets. PHO is a volatile governance token, so a PHO/USDC pool using a
stable curve would be a misconfiguration — overwhelmingly likely that
this pool is still XYK, just a different build.

### 2. Two distinct XYK WASM builds in production

Despite reaching the same answer (XYK), the two WASM hashes differ:

| WASM SHA-256 | Size | Count | Example pool |
|---|---|---|---|
| `167ab414...506c` | 36810 B | 10 | XLM/USDC (CBHCRSVX...BIZX) |
| `13b158655e...f2ca` | 37047 B |  1 | PHO/USDC (CD5XNKK3...IAA) |

Indistinguishable by external observation:

- Same `stellar contract info interface` output (byte-identical).
- Same `contract meta` description: `"Phoenix Protocol XYK Liquidity Pool"`.
- Same `rsver` (1.85.1) and `rssdkver`
  (`22.0.7#211569aa49c8d896877dfca1f2eb4fe9071121c8`).
- Same `query_version()` → `"2.0.0"`.
- Same `Config.pool_type` field value: `0`.

Difference is in implementation bytes (237 B larger). Likely a minor
build delta — could be a code path that's slightly different on the
newer-deployed pool, or a non-determinism in the build pipeline. Not
investigated further in this task.

### 3. `pool_type` is in scope but not yet discriminative

`Config.pool_type: 0` is the same for both observed XYK builds. The
field exists in the schema — when Phoenix eventually deploys a stable
pool it should presumably return a non-zero value here. This gives the
consumer a cheaper future check than counting events: read
`query_config(pool_id).pool_type` once at pool-registration time.

## So what?

For the prices-api consumer (task 0018 / stream-1):

1. **Stable-pool decoder remains source-only.** Keep the 6-event
   grouping spec from task 0018 §3 as written; do not try to
   "validate" it against onchain data because there is no onchain
   data. Mark this clearly in the consumer's extractor docs so future
   maintainers do not think it is a deferred TODO.

2. **The XYK extractor must tolerate two WASMs.** The consumer
   should not key off WASM hash for XYK detection unless it accepts
   *both* observed XYK hashes (and any future XYK build that Phoenix
   ships). Better discriminator: `Config.pool_type == 0` AND the
   `swap` event grouping has 8 events. The hash-set approach risks
   silently dropping the PHO/USDC pool from price feeds.

3. **Cheap forward check.** When the consumer registers a Phoenix
   pool, call `query_config(pool_id)` once and store `pool_type`
   alongside the pool address. Route 8-event groupings through the
   XYK extractor when `pool_type == 0`; route 6-event groupings
   through the stable extractor when `pool_type != 0`. This makes the
   classifier robust to additional XYK WASM revisions without
   requiring re-scanning.

## Acceptance criteria mapping for task 0032

- [ ] At least one mainnet stable-pool deployment identified
  → **Negative result**: none exist as of 2026-05-15.
- [ ] One real stable-pool swap event grouping decoded and archived
  → **Cannot satisfy**; no stable pool exists to decode from.
- [x] Consumer's stable-pool decoder spec status documented
  → This note serves as the documentation: source-only, no
  observation possible until Phoenix actually ships a stable pool.

The task is therefore appropriate to close as a **negative-result
synthesis** rather than left active waiting for a stable pool to
appear. A new task should be spawned to "Re-survey Phoenix factory
periodically and capture first stable pool when it appears" — that
work picks up exactly where this one stops.

## Followups (worth spawning as backlog tasks)

- **Two-XYK-WASM tolerance in the consumer's pool registry** — make
  sure the venue lookup table in task 0018 §3 does not key off a
  single XYK hash. Recommend the `pool_type + event count` approach
  above. Should be small, concrete.
- **Periodic factory re-survey** — schedule a re-run of the
  `query_pools` + WASM-hash inventory (e.g., monthly) so the moment a
  stable pool is added it gets caught. This replaces task 0032 as the
  ongoing concern.
- **What is the 237-byte delta?** Low priority. Useful only if it
  later turns out to materially change event emission order or
  payload. Note: if this delta does change event emission, the
  consumer would silently mis-parse the PHO/USDC pool today. Worth a
  quick spot-check by dumping a real swap on `CD5XNKK3...IAA` and
  confirming the 8-event grouping holds.
