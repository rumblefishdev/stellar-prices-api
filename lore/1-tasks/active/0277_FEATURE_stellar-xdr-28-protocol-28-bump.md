---
id: "0277"
title: "Bump stellar-xdr 27→28 before Protocol 28 (Adapter) activates on 2026-09-16 — BE must bump xdr-parser first"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0091", "0094", "0098", "0064"]
tags: [layer-backend, priority-high, effort-small, phase-live, clickhouse, resilience, ingestion, deployment]
links:
  - "https://stellar.org/blog/developers/adapter-protocol-28-upgrade-guide"
  - "../archive/0091_BUG_stellar-xdr-27-proto27-decode.md"
  - "../archive/0094_FEATURE_proto27-deploy-replay-verify.md"
  - "0098_FEATURE_xdr-protocol-version-gap-ci-guard.md"
history:
  - date: 2026-09-11
    status: backlog
    who: okarcz
    note: >
      Filed after the Protocol 28 "Adapter" upgrade guide was published.
      Mainnet vote 2026-09-16 17:00 UTC. This is the same shape as [[0091]],
      which was only found AFTER proto27 froze live ingestion for six days —
      filed proactively this time. ⚠️ Blocked in practice on BE bumping
      `xdr-parser`; see the Blocker section.
  - date: 2026-09-11
    status: backlog
    who: okarcz
    note: >
      The BE ask is raised — the operator notified BE the same day. Their
      answer (date, and the rev their `xdr-parser` bump lands on) is still
      outstanding and is what unblocks our half.
  - date: 2026-09-14
    status: active
    who: okarcz
    note: >
      Activated — the blocker is CLEARED. BE bumped `xdr-parser`'s `stellar-xdr`
      to 28 in `840f2b58` ("feat(lore-0548): bump stellar-xdr to 28 for the
      protocol-28 vote", 2026-09-10), merged and now an ancestor of their
      `develop` head `31be5f74`. Their own task is 0548. Two days before the
      pubnet vote.
---

# stellar-xdr 27 → 28 for Protocol 28 "Adapter"

## Summary

Stellar mainnet votes to **Protocol 28 "Adapter" on 2026-09-16 at 17:00 UTC**.
Our workspace is exact-pinned to `stellar-xdr = "=27.0.0"`. `stellar-xdr 28.0.0`
is published on crates.io. Protocol 27 froze live ingestion for six days on
exactly this mechanism, so the bump is the whole task — plus the deploy, because
[[0091]] proved that a merged bump with an undeployed binary changes nothing.

## Context

### The precedent this task exists to avoid repeating

Protocol 27 "Zipper" (2026-07-09) stalled the live candle frontier at ledger
`~63,384,067` / 2026-07-08 12:31 and it stayed frozen for **six days**. The
deployed ledger-processor was on `stellar-xdr 26` while mainnet moved to 27, and
it hit a decode wall at ledger `63,401,875`.

The failure was silent in every way that mattered:

- SQS drained normally; the main queue and the DLQ were both **empty**, so
  nothing dead-lettered and nothing could be redriven.
- The Lambda logged **zero** parse / XDR / panic / ClickHouse errors.
- The doorbell-lag alarm was blind to it — it watches queue age, and the queue
  was healthy. The pass drained and simply wrote no rows.

It was caught by reading `max(timestamp)` per source out of `price_ohlcv_1m` by
hand, six days in.

### What Protocol 28 actually changes for us

Three CAPs ship. Only one reaches this codebase:

| CAP | change | reaches us? |
|---|---|---|
| **CAP-83** | new `StellarValue` type `STELLAR_VALUE_EMPTY_TX_SET` | **yes** |
| CAP-85 | new `CONTRACT_EXECUTABLE_EXTERNAL_REF` executable type | no — zero `ContractExecutable` matches in the repo |
| CAP-86 | new `sparse_map_*` host functions | no — we do not run the Soroban host |

The upgrade guide names the affected class directly: *"indexers, analytics
pipelines, custom tooling"* consuming raw ledger data, which must *"treat ledgers
carrying this value as empty ledgers and handle the new type."* We consume raw
`LedgerCloseMeta` from Galexie objects, so we are squarely in it.

`stellar-xdr 28.0.0`'s release notes say it **ungates CAP-83 and CAP-85** and
re-generates the XDR. The concrete Rust API delta is not enumerated there — it is
`cargo check` that will tell us, exactly as it did for 26→27.

### ⚠️ The break may not land on the activation ledger

Proto27's wall was a clean boundary. This one probably is not: CAP-83 adds a new
union discriminant, so `stellar-xdr 27` should decode Protocol 28 ledgers fine
**until the first ledger that actually carries an empty tx set**. That could be
hours or days after the vote, and it will look unrelated to the upgrade when it
arrives.

**Confirm this against the CAP text before choosing a deploy window.** If it
holds, it removes the "nothing broke at 17:00, we're fine" false all-clear — and
it is an argument for deploying *before* the vote rather than watching after it.

## ✅ Blocker CLEARED 2026-09-14 — BE has bumped `xdr-parser`

**We cannot do this unilaterally.** `prices-ingest-core/src/decode.rs` calls
BE's `xdr_parser::decompress_zstd` + `deserialize_batch` and consumes the
`LedgerCloseMeta` values they return. The type crosses the crate boundary.

Current state, read 2026-09-11:

| | pin |
|---|---|
| ours, `Cargo.toml:28` | `stellar-xdr = { version = "=27.0.0" }` |
| ours, `Cargo.toml:31` | `xdr-parser = { git = "…soroban-block-explorer.git", branch = "develop" }` |
| locked `xdr-parser` rev | `d61b359f` — the BE #325 merge from the proto27 bump; unmoved since |
| BE, `Cargo.toml:40` | `stellar-xdr = { version = "27" }` — i.e. `^27`, will **not** resolve 28 |

If we bump to 28 while `xdr-parser` still compiles against 27, Cargo pulls
**both** major versions into the graph and `decode_object` stops type-checking —
a hard compile error, not a warning. So the order is fixed:

1. **BE bumps `xdr-parser`'s `stellar-xdr` to 28** and merges to `develop`.
2. We re-pin `=28.0.0` and re-pin `xdr-parser` to the resulting merge rev.

⚠️ `xdr-parser` is deliberately tracked on `branch="develop"`, not a rev — see
[[xdr-parser-develop-branch-intentional]]. That means BE's bump lands in our
build on the next `cargo update`, whether or not we are ready, so the two halves
want to move together rather than drift.

✅ **BE delivered, and it is already merged.** Read from their repo
2026-09-14:

| | value |
| --- | --- |
| BE bump commit | `840f2b58b67c842295b2ad0f16cb2778a5edf294` |
| message | `feat(lore-0548): bump stellar-xdr to 28 for the protocol-28 vote` |
| authored | 2026-09-10 12:14 +0200 |
| on `origin/develop`? | **yes** — ancestor of develop head `31be5f74` (2026-09-14 14:16) |
| BE `Cargo.toml:40` now | `stellar-xdr = { version = "28" }` |
| their task | BE 0548 — *Protocol 28 readiness: Galexie 28.0.1 pin + stellar-xdr 27→28* |

⚠️ **BE's `master` still reads `"27"`** — only `develop` carries the bump. That
is the branch we track, so it reaches us, but it means "BE is on 28" is true of
`develop` only.

⚠️ Our lock was still pinned at the **proto27** rev
`d61b359f39994c7ef5f5bde8a0d709cf81a1026c` — unmoved since BE #325 — so the bump
did **not** reach our build on its own. `cargo update -p xdr-parser` is a
required step, not a side effect.

⚠️ BE also confirmed **Galexie 28.0.1 is exporting** after a restart
(`31be5f74`), so the objects we read are already proto-28 capable.

## Implementation

- Ask BE to bump `xdr-parser` to `stellar-xdr 28`; agree a rev.
- Bump the workspace pin to `=28.0.0`, keeping the exact-pin style 0091 chose.
- Re-pin `xdr-parser` to BE's merge rev; keep the `branch="develop"` tracking.
- `cargo check --workspace` and fix the API delta. Known surface — the four
  crates that name the dependency:
  - `packages/prices-ingest-core` (`decode.rs`, `filter.rs`, `soroban.rs`,
    `canonical.rs`, `error.rs`, `examples/decode_probe.rs`)
  - `packages/asset-discovery` (`lib.rs`, `symbols.rs`)
  - `packages/sdex-backfill` (`ingest.rs`, `error.rs`)
  - `packages/oracle-worker` (`lib.rs`)
- Check the three `LedgerCloseMeta` match sites in particular — `decode.rs:23`,
  `filter.rs:125` / `:146`, `soroban.rs:164` / `:184`. They match `V0/V1/V2`
  exhaustively, so a new variant is a compile error, which is the good case.
- Decide what an empty-tx-set ledger should do on our path. It carries no
  trades, so the honest handling is "decode it, extract nothing, advance the
  cursor past it" — the cursor must still advance or we stall on a ledger that
  was never going to produce a candle.
- Deploy the ledger-processor. ⚠️ [[0141]] / [[deploy-ships-stale-lambda-assets]]
  — `deploy-production-compute` does **not** build; confirm the asset changed.
- Verify the crossing, as [[0094]] did: watch `max(timestamp)` per source in
  `price_ohlcv_1m` advance past the activation ledger rather than freeze at it.

## 🔑 The bump is done — and a clean compile proves NOTHING here

Bumped 2026-09-14 on `feat/0277_stellar-xdr-28-protocol-28-bump`.
`Cargo.toml:28` → `=28.0.0`; `cargo update -p xdr-parser` moved the lock from
`d61b359f` (proto27) to BE's develop head `31be5f74`. The lock resolves
**exactly one** `stellar-xdr`, so the dual-major hazard this task warned about
did not materialise.

`cargo check --workspace --all-targets`: **clean, zero warnings.**
`cargo test --workspace`: **905 passed, 0 failed**, 190 ignored (the CH
integration tests).

### ⛔ CORRECTION — this task's central prediction was wrong

The Implementation section says of the `LedgerCloseMeta` match sites:

> They match `V0/V1/V2` exhaustively, so a new variant is a compile error,
> which is the good case.

**There is no compile error, and there was never going to be one.** CAP-83 does
not touch `LedgerCloseMeta` — it adds `EmptyTxSet` to **`StellarValueExt`**, the
union nested inside `StellarValue`:

| type | 27 | 28 |
| --- | --- | --- |
| `LedgerCloseMeta` | `V0/V1/V2` | `V0/V1/V2` — **unchanged** |
| `StellarValueExt` | `Basic`, `Signed`, `EmptyTxSet` behind `#[cfg(feature = "cap_0083")]` | `Basic`, `Signed`, `EmptyTxSet` — **ungated** |

Our exhaustive matches are all on `LedgerCloseMeta` (`decode.rs:25-27`,
`filter.rs:127-137`, `filter.rs:148-158`, `soroban.rs:166-176`), and the only
thing we read out of the SCP value is `scp_value.close_time` — a plain field of
`StellarValue`, unaffected by the `ext` discriminant. **Nothing we match on
changed shape, so the compiler had nothing to say.**

⚠️ **Therefore "it builds" is not evidence of Protocol 28 readiness**, and must
not be reported as such. The next protocol bump should not expect the compiler
to catch it either.

### ✅ The exposure was real, and it was decode — proven, not assumed

`stellar-xdr 27`, `stellar_value_ext.rs:142-147`:

```rust
#[cfg(feature = "cap_0083")]
StellarValueType::EmptyTxSet => {
    Self::EmptyTxSet(StellarValueProposedValue::read_xdr(r)?)
}
#[allow(unreachable_patterns)]
_ => return Err(Error::Invalid),
```

We never enabled `cap_0083`, so under 27 the **first ledger carrying an empty
tx set** hits `_ => Err(Error::Invalid)`. BE's parser fails the **whole batch**
(`xdr-parser/src/lib.rs:111`), `decode_object` returns `ReconcileError::Decode`,
the run returns on the `?` — **cursor unmoved**. That is the proto27 freeze,
mechanism for mechanism. Under 28 the variant is ungated and decodes.

🔑 **This confirms the task's own warning that the break need not land at
17:00 UTC.** Protocol 28 activating does not by itself produce an empty tx set;
the wall is the first *empty* ledger, which may be hours or days later. So
"nothing broke at the vote" is a false all-clear, and it is an argument for
deploying **before** 09-16 rather than watching after it.

### ✅ An empty-tx-set ledger already advances the cursor — verified, no change needed

`reconcile.rs:129-158`: the cursor is driven by **objects fetched**, not by
trades found. `current = obj_max.max(next)` and `persisted += 1` run regardless
of whether `extract_trades` or `process_ledger` returned anything, and
`ledger_sequence` reads `ledger_header.header.ledger_seq`, which every variant
carries. A trade-less ledger therefore decodes, extracts nothing, and the run
still writes the advanced cursor at `:223`. **No code change is required for the
second half of AC 3** — recorded as verified rather than assumed.

### ⚠️ Pre-existing clippy noise, deliberately not fixed here

`cargo clippy --workspace --all-targets -- -D warnings` fails locally with 10
warnings (`collapsible_if` ×5, `no_effect` ×3, `items_after_test_module`,
`div_ceil`, …). **Byte-for-byte the same set on clean `develop`**, none
XDR-related — a newer local clippy than CI's, which is green. Left alone so the
bump diff stays two files.

## 🔴 SCOPE CORRECTION — `asset-discovery` has the same decode wall

**Found 2026-09-14 while diffing the deploy.** This task, its runbook and its
acceptance criteria named **only** the ledger-processor. That is wrong:
**exactly two deployed Lambdas call `decode_object`**, and the second is
`asset-discovery`.

```rust
// packages/asset-discovery/src/lib.rs:228
let key = ledger_s3_key(ledger as i64);
let Some(bytes) = fetcher.fetch(&key).await? else { break };
let metas = decode_object(&bytes)?;
```

It is a live EventBridge Lambda on `rate(1 hour)`, **ENABLED**, so on the first
empty-tx-set ledger it fails exactly as the ledger-processor would. `oracle-worker`
is clear — it depends on `stellar-xdr` transitively but never decodes a ledger.

⚠️ **It is on proto-27 and this deploy does not fix it.** The EventBridge stack
was deployed **2026-09-14 14:00:50 UTC**, and PR #314 merged at **14:19:29 UTC** —
19 minutes later. The running binary cannot contain the bump.

### ⛔ Do NOT deploy the EventBridge stack from the local machine as-is

Measured against production, not assumed:

| | |
| --- | --- |
| EventBridge stack last updated | 2026-09-14 14:00:50 UTC, by principal **`aws-cdk-fish`** (a different machine) |
| Compute stack last updated | 2026-09-11 12:52:50 UTC |
| local `target/lambda/*` (except the ledger-processor) | **2026-09-11 14:47** |

Local is **behind** production for all nine EventBridge Lambdas, so a deploy from
this tree would **roll back** the 14:00 deploy. `cdk diff Prices-production-EventBridge`
confirms all nine would change. **Rebuild every Lambda crate from current
`develop` first** — the canonical list is `tools/scripts/lambda-assets.sh`
(11 crates), and `--features lambda` plus explicit `-p` are both mandatory.

❓ **Open question for the operator:** was the 14:00 `aws-cdk-fish` deploy from
`develop`, or from a branch? If `develop`, rebuilding from current `develop` is
strictly forward (it is that same code plus this bump). If a branch, rebuilding
would drop whatever was on it.

### ✅ The ComputeStack deploy itself is clean — verified, not assumed

`cdk diff Prices-production-Compute --method=template` (template-only, no
changeset) returns **exactly one** changed resource:

```
[~] AWS::Lambda::Function LedgerProcessorFunction
 └─ Code.S3Key: ae261f9a… → 73535f98…
✨ Number of stacks with differences: 1
```

`ApiHandlerFunction` does **not** change. No IAM, SQS or env-var edits.

⚠️ **The runbook's "expect only the ComputeStack asset to change" is misleading**
because `make diff-production` runs `cdk diff` **unscoped** — it diffs every
stack, so it will always show the EventBridge assets too. Diff the stack you are
about to deploy, by name, with `--method=template`.

# 📕 DEPLOY RUNBOOK — 0277

Merged as **`bcc82f7`** on `develop` (PR #314, squashed, 2026-09-14). **Not yet
deployed.** The generic procedure is
[`docs/runbooks/deploy-ledger-processor.md`](../../../docs/runbooks/deploy-ledger-processor.md);
this section carries only what is specific to the protocol crossing.

⚠️ **Deploy BEFORE the vote (2026-09-16 17:00 UTC), not after.** `stellar-xdr 28`
decodes protocol-27 ledgers perfectly well, so shipping early costs nothing —
whereas the decode wall lands on the **first empty-tx-set ledger**, which may be
hours or days after activation and will look unrelated when it arrives.
⛔ **"Nothing broke at 17:00" is NOT an all-clear** and must not be recorded as one.

## Step 0 — [local machine, repo root] preflight

```bash
export AWS_PROFILE=soroban-admin
export AWS_REGION=eu-central-1
git checkout develop && git pull --ff-only
git log --oneline -1                  # expect bcc82f7 or later
npm run xdr:verify-protocol-gap       # expect: pinned 28 | mainnet current 27 | core supports 28
```

Before the vote the guard reads **"current"** because our pin (28) is no longer
behind mainnet (27). After the vote mainnet's `current` becomes 28 and it still
reads current. Either is fine; **BEHIND** at any point means the wall is live.

## Step 1 — [local machine, repo root] build the bootstrap

🔴 **This is the step that gets skipped, and skipping it deploys the OLD binary
with a green result.** `--features lambda` is mandatory — the bin is
`required-features = ["lambda"]` and builds to nothing without it.

```bash
cargo lambda build -p prices-ledger-processor --release --arm64 --features lambda
ls -l target/lambda/prices-ledger-processor/bootstrap   # mtime MUST be seconds ago
file target/lambda/prices-ledger-processor/bootstrap    # ELF 64-bit ... ARM aarch64
```

## Step 2 — [local machine, `infra/`] preview, then deploy the ComputeStack only

```bash
cd infra
make diff-production            # expect ONLY the Lambda code asset hash to change
make deploy-production-compute  # NOT `make deploy-production` (that is all stacks)
```

If the diff proposes IAM, SQS, env-var or other-stack edits — **stop**.

## Step 3 — [local machine] prove the RUNNING binary changed

```bash
aws lambda get-function-configuration \
  --function-name prices-production-ledger-processor \
  --query '[LastModified,Runtime,Architectures[0],CodeSha256]' --output text
```

`LastModified` seconds ago, `arm64`. **Record `CodeSha256` in this task file** —
that is the artefact 0091 lacked and 0094 had to supply days later.

## Step 4 — [prod CH, as `dev_read`] the crossing measurement

Capture **before** the vote and again **after**, so the criterion is a diff and
not an impression:

```sql
SELECT source, max(timestamp) AS latest_candle, now() - max(timestamp) AS behind_sec
FROM prices.price_ohlcv_1m GROUP BY source ORDER BY source
```

`behind_sec` near 0 = healthy. `price_ohlcv_1m` has **no ledger column** —
freshness is by candle `timestamp` ([[proto27-xdr26-live-freeze]]).

## Step 5 — [AWS] confirm the alarm actually covers this

`prices-production-rollup-freshness-1m` is claimed to cover halted upstream
ingestion ([[0137]]). This crossing is its **first real test against this
specific failure**. Record whether it stayed OK, or fired and cleared — the
criterion is "confirmed", not "assumed".

⚠️ Watch the DLQ (`prices-ingest-dlq-production`) through the crossing too. A
cold-start Init failure DLQs rather than gapping, and is recoverable by redrive.

## Expected disturbance from the deploy itself: none

The doorbell is a **queue** (14-day retention), the cursor is written **last**
(`reconcile.rs:223`) so an interrupted run redoes ledgers rather than skipping
them, `reservedConcurrentExecutions = 1` and `batchSize = 1` prevent racing
runs, and `maxReceiveCount = 10` absorbs swap churn. Expect one cold start and a
few seconds of lag. **No stop, no gap.**

## Acceptance Criteria

- [x] **BE has bumped `xdr-parser` to `stellar-xdr 28` and the rev is recorded here** — `840f2b58`, on their `develop`. See §Blocker CLEARED.
- [x] **Workspace pin is `=28.0.0`; `cargo check --workspace` and the full test
      suite are green** — 905 passed / 0 failed, one `stellar-xdr` in the lock.
- [x] **Any new `LedgerCloseMeta` / `StellarValue` variant is handled explicitly,
      and an empty-tx-set ledger advances the cursor** — no new `LedgerCloseMeta`
      variant exists (the change is `StellarValueExt::EmptyTxSet`, ungated in 28),
      and the cursor advance is driven by objects fetched, not trades found.
      Both verified against the source, not assumed. See §CORRECTION.
- [ ] The bumped ledger-processor is **deployed** and the deployed asset is
      confirmed changed — not merely merged. 0091 merged on 2026-07-14 and prod
      stayed frozen until 0094 deployed it.
- [ ] 🔴 **`asset-discovery` is deployed on the 28 binary too** — it is the only
      other deployed Lambda that calls `decode_object`, it runs hourly, and it
      was shipped on proto-27 19 minutes before this bump merged. See
      §SCOPE CORRECTION.
- [ ] The live candle frontier is measured crossing the Protocol 28 activation
      ledger, recorded before/after — the same check that resolved proto27's
      active-vs-latent question.
- [ ] `prices-production-rollup-freshness-1m` stayed OK through the crossing
      (or fired and was cleared), so the alarm is confirmed to cover this failure
      rather than assumed to.

## Notes

### The alarm coverage is better than it was in July — verify, don't assume

The rollup-freshness alarms from [[0137]] measure **data**, not MV exit status,
and `observability-stack.ts` describes them as covering *"the rollup chain
feeding it is stalled, **or upstream ingestion has halted**"*. So a repeat of the
proto27 freeze should now page within the staleness bound rather than run six
days unseen. That is worth confirming during the crossing rather than trusting —
it is the first real test of that alarm against this specific failure.

### This is the second time the detection mechanism was "somebody read a blog"

[[0098]] — *"Version-gap CI guard: surface stellar-xdr protocol lag before it
freezes prod"* — was spawned from 0094 precisely so the next protocol bump would
surface on its own. It is still in `backlog/`, unbuilt, and Protocol 28 reached
us the same way Protocol 27 did: by chance. Worth pulling 0098 forward alongside
this task; it is tagged `effort-small`.
