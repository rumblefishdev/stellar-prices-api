---
id: "0005"
title: "Stream 2 SDEX historical backfill runs locally on a workstation (supersedes ADR 0002)"
status: accepted
deciders: [okarcz]
related_tasks: ["0012", "0027"]
related_adrs: ["0001", "0002", "0003", "0004"]
tags: [architecture, backfill, sdex, stream-2, local-backfill, workstation, cloud-push, block-explorer-pattern]
links:
  - "./0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "./0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md"
  - "../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md"
  - "../1-tasks/active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-fargate-design.md"
history:
  - date: 2026-05-14
    status: proposed
    who: okarcz
    note: >
      Drafted to reverse ADR 0002's Fargate commitment. After examining
      BE's local-backfill pattern (BE ADR 0010, crates/backfill-bench,
      crates/backfill-runner), local-workstation backfill is the
      simpler, cheaper, already-proven path for SDEX.
  - date: 2026-05-14
    status: accepted
    who: okarcz
    note: >
      Accepted same day. Supersedes ADR 0002. BE's local-backfill
      pattern is mirrored verbatim, adapted to SDEX filter + table
      shape. Cloud push of price_ohlcv is a separate post-backfill
      step landing in a follow-up task.
---

# ADR 0005: Stream 2 SDEX historical backfill runs locally on a workstation (supersedes ADR 0002)

**Related:**

- [ADR 0001: Stream 1 Soroban AMM historical backfill is sourced from BE's ClickHouse `soroban_events` (local instance)](./0001_stream1-clickhouse-sourced-amm-backfill.md) — Stream 1 complementary decision (also workstation-local)
- [ADR 0002 (superseded by this ADR): Stream 2 SDEX historical backfill is fully independent of BE — Fargate task variant](./0002_stream2-sdex-archive-backfill-independent-of-be.md) — the original Fargate commitment
- [BE ADR 0010: Local backfill via `backfill-bench` instead of AWS Fargate](../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md) — the upstream pattern this ADR mirrors
- [BE ADR 0040: Multi-laptop backfill snapshot merge — schema hazards and playbook](../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md) — informs the single-laptop v1 stance
- [Task 0012: Design SDEX local backfill (Stream 2)](../1-tasks/active/0012_FEATURE_design-prices-owned-backfill-fargate/README.md) — operational design landing for this ADR

---

## Context

ADR 0002 (accepted 2026-05-13) committed Stream 2 (SDEX, ledger 1 → tip,
~57M ledgers) to a prices-api-owned ECS Fargate task reading Stellar
public history archives. Independence from BE's runtime was the
explicit driver; "Fargate" was the assumed deployment shape.

In the design phase (task 0012), reviewing BE's actual backfill
implementation revealed a simpler shape already in production at BE:

- **BE ADR 0010** rejected Fargate for BE's own backfill in favour of
  a local CLI (`crates/backfill-bench`, since promoted to
  `crates/backfill-runner` with sink abstraction). Reasoning: backfill
  is a one-time operation, Fargate infra is over-engineered for it,
  and the local tool reuses the production `process_ledger` pipeline
  with zero code duplication.
- **BE's archive read path** uses `aws s3 sync --no-sign-request`
  against the anonymous public `aws-public-blockchain` bucket —
  no AWS IAM, no Fargate task role, no NAT egress costs.
- **BE's resumability** is partition-level: `Sink::load_completed(start, end)`
  returns the set of already-indexed ledgers and the runner skips any
  partition whose entire clamped range is already in the DB.
- **BE's `xdr-parser` crate** wraps upstream `stellar-xdr` with `.xdr.zst`
  decompression and batch deserialization helpers — exactly the glue
  the SDEX backfill needs, available today.

Adopting this pattern reverses one architectural choice from ADR 0002
(Fargate) while preserving everything else (BE-independence at
runtime, BE-authored XDR parser as a library dep, ledger-1-to-tip
coverage, prices-api-owned filter and write paths).

A second concern from BE ADR 0040 — multi-laptop merge hazards
(surrogate-id remap, watermark reconciliation) — is acknowledged but
deferred: prices-api's only growing table during backfill is
`price_ohlcv`, whose PK per ADR 0003 is `(timestamp, asset_id,
quote_asset_id, granularity)`. That is a natural composite key plus
two surrogate asset ids; multi-laptop parallelism would still need
the `assets`-table remap that BE faces. Single-laptop v1 sidesteps
this entirely.

---

## Decision

**Stream 2 SDEX historical backfill is a local Rust CLI on the
operator's workstation that mirrors BE's `backfill-bench` /
`backfill-runner` pattern.** Concretely:

1. **Source.** `aws s3 sync --no-sign-request` against the anonymous
   public bucket `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`.
   Same path BE consumes; no AWS account is needed to read.

2. **Sink.** Local PostgreSQL (Docker) on the operator's workstation.
   Same posture as BE's local backfill: every laptop has its own
   PG, no shared cloud DB during backfill.

3. **XDR parsing.** BE's `xdr-parser` crate consumed as a git Cargo
   dependency (`xdr-parser = { git = "https://github.com/rumblefishdev/soroban-block-explorer.git", branch = "main" }`).
   No BE-repo changes; BE crate is read-only from prices-api's view.
   Preserves ADR 0002 §3's "BE parser as library dep, no runtime
   coupling" stance.

   **Future direction (already decided, separately from this ADR):**
   `xdr-parser` will be published as a standalone versioned crate,
   independent of the BE workspace. Once published, the git Cargo
   dep above is replaced with a plain version pin
   (`xdr-parser = "X.Y.Z"`). Prices-api makes the cutover in a small
   follow-up edit to its `Cargo.toml` whenever BE publishes; no
   design or schema change is triggered, and v1 of the backfill CLI
   can ship against either form. The git pin is a transient
   convenience, not an architectural choice.

4. **Pipeline.** Partition-at-a-time: download partition N+1 in
   background (`aws s3 sync`) while indexing partition N. Single-slot
   prefetch, no worker pool. Matches BE `backfill-runner::run::execute`.

5. **Filter + extraction.** Per task 0022's spec — the five
   trade-shaped op types from `TransactionResultMeta`, `ClaimAtom`
   unpacking, `TradeTick` → 1-minute `price_ohlcv` bucketing with
   whole-row UPSERT.

6. **Resumability.** Per-ledger atomic transaction: all `price_ohlcv`
   UPSERTs for the ledger AND the `backfill_progress` checkpoint
   advance commit together. Crash mid-ledger leaves
   `current_ledger` pointing at the last fully-processed ledger;
   restart re-fetches and re-UPSERTs that ledger (idempotent under
   whole-row replacement). Partition-level pre-skip also supported:
   if a partition's clamped range is fully in `backfill_progress`'s
   processed set, the partition is not re-downloaded.

7. **Cloud push is a separate, post-backfill step.** Once local
   backfill completes (or reaches a "Tranche 1 release-ready"
   threshold), a small `sdex-cloud-push` tool streams the relevant
   prices tables (`price_ohlcv` + `assets`) from local PG to the
   cloud RDS. This is the **only** AWS-touching component on the
   Stream 2 path, and it depends on task 0011 (CDK bootstrap) only
   because the cloud RDS must exist before push is meaningful.
   Backfill itself is not blocked on 0011.

8. **Single-laptop v1.** Parallel multi-laptop backfill (BE ADR 0040's
   shape) is explicitly out of scope for v1 — the asset-id
   surrogate-remap problem BE faced applies to our `assets` table
   too, and v1 deliberately avoids it. v2 may revisit if 57M ledgers
   on one workstation proves too slow.

9. **Coverage and tranche stance unchanged from ADR 0002.** Ledger 1
   → current tip is still the target range. Tranche 1 acceptance
   ("≥ 6 months of recent history exposed by `GET /backfill/status`")
   is met by running the local backfill in tip-backward chunks: the
   operator first invokes `--start=tip-1.1M --end=tip` (≈ 6 months),
   runs the cloud-push step, then continues with older ranges in
   the background. Full historical completion still extends past
   Tranche 3 — also unchanged.

---

## Rationale

### Why local now, when ADR 0002 chose Fargate yesterday

ADR 0002 was accepted on 2026-05-13 against the user directive
"entirely independent of BE." That goal is satisfied by either
deployment shape — the BE-independence is about runtime/data
coupling, not about whether the binary runs in AWS or on a laptop.
ADR 0002 picked Fargate without examining BE's own backfill
implementation. The design phase (task 0012) closed that gap:
BE rejected Fargate for the same use case in ADR 0010, with reasons
that apply identically to prices-api.

### Why the BE pattern carries over cleanly

BE ADR 0010's five reasons map 1:1 to prices-api's situation:

| BE ADR 0010 rationale                                | Maps to prices-api as                                                                  |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Already built and proven (task 0117 benchmark)       | Pattern is proven; we reimplement in our repo (per "never modify BE" directive) for SDEX filter + table |
| Simpler infrastructure (no Fargate, ECS, IAM, NAT)   | Same — no CDK gate, no AWS infra for the backfill itself                               |
| Lower cost (no Fargate compute, no NAT egress)       | Same — workstation electricity + ISP bandwidth only                                    |
| Same pipeline (reuses `process_ledger`)              | Adapted — our pipeline is per task 0022's spec, same code path local and (future) cloud-streaming |
| Backfill is a one-time operation                     | Same — 57M ledgers is one-time; ongoing Lambda handles tip                             |

### Why a separate "cloud push" step instead of writing to cloud RDS directly

Three reasons:

1. **Bandwidth.** Streaming every UPSERT across the public internet to
   cloud RDS would be the dominant latency over the multi-week
   wall-clock. Local PG sustains thousands of UPSERTs/sec; remote
   PG over ~50 ms RTT would be an order of magnitude slower.
2. **Cloud RDS bootstrap.** Task 0011 (CDK bootstrap including the
   prices-api RDS instance) is still in backlog. Direct cloud-writing
   blocks the backfill on 0011. Local-then-push lets the backfill
   start today and lets the push step land whenever cloud RDS exists.
3. **Cost.** Backfill writes are heavy on cloud-DB IOPS billed by
   provisioned throughput. A bulk push (COPY-style streaming, or
   per-batch INSERT … SELECT over a CLI on the laptop with cloud-RDS
   credentials) is far cheaper than a multi-week live trickle.

### Why "never modify BE" still allows `xdr-parser` as a git Cargo dep

A git Cargo dependency points at a commit hash or branch on the BE
repo; cargo fetches the source read-only and compiles it into our
binary. No code is added or modified in BE. The only coupling is
that if BE renames or removes the `xdr-parser` crate, our pin needs
updating — which is exactly the coupling shape ADR 0002 §3 already
accepted ("library dependency, versioned, reproducible").

### Why single-laptop v1, not multi-laptop parallel

BE ADR 0040 documents the schema hazards of parallel-laptop backfill
in painful detail: surrogate-id remap for tables with FK referrers,
watermark reconciliation for current-state tables, partition layout
agreement across snapshots. Prices-api's `assets` table is a
surrogate-id table (`asset_id BIGSERIAL`) referenced by every
`price_ohlcv` row via the ADR 0003 PK — the same hazard. v1 avoids it
by running on one workstation; v2 can adopt BE's `db-merge`-shaped
solution if measured wall-clock motivates it.

---

## Alternatives Considered

### Alternative 1: Keep ADR 0002 — prices-api-owned ECS Fargate task

**Description:** Provision a Fargate cluster + task definition + IAM
roles + CloudWatch alarms + SNS topic for the SDEX backfill, as
designed in task 0012's first-pass G-note.

**Pros:**

- No workstation uptime dependency.
- Closer to cloud RDS, so direct-to-cloud writes are viable.
- Detached from operator's laptop power / network.

**Cons:**

- Over-engineered for a one-time operation (BE ADR 0010's primary
  reason for the same reversal).
- Blocked on task 0011 (CDK bootstrap), which is still in backlog.
- NAT egress costs for ~6 TB of archive reads.
- IAM contract design, CloudWatch alarm tuning, SNS subscription
  management — all non-trivial moving parts for a one-time job.
- Diverges from how the BE team — same engineering org — handles
  the structurally identical problem.

**Decision:** REJECTED — over-engineered; the BE-proven local
pattern is simpler and unblocks the work.

### Alternative 2: Multi-laptop parallel backfill (BE ADR 0040 shape) from day one

**Description:** Two or more laptops run disjoint ledger ranges,
each writing to its own local PG; a `prices-db-merge` tool then
consolidates them via surrogate-id remap on the `assets` table.

**Pros:**

- Wall-clock reduced by ~N for N laptops.

**Cons:**

- Surrogate-id remap on `assets` is non-trivial — BE's `db-merge`
  crate is hundreds of lines of careful SQL and a documented
  pre-condition gate (ADR 0040). Building the prices-api analog is
  ~1-2 weeks of work for a backfill that runs once.
- 57M ledgers single-laptop is ≈ 12-16 days per task 0022's
  measured 311 ledgers/s. Probably acceptable.
- Can be adopted as v2 if v1 measurement disproves the "acceptable"
  claim — no design commitments prevent it.

**Decision:** REJECTED for v1 — disproportionate to the gain.
Revisit if v1 wall-clock proves untenable.

### Alternative 3: Direct cloud-RDS writes from a workstation CLI

**Description:** Same local CLI as the chosen path, but the sink is
cloud RDS over SSH-tunnel / VPN — no local PG at all.

**Pros:**

- One DB, no merge or push step.

**Cons:**

- Still blocked on task 0011 (cloud RDS must exist).
- Latency-bound write path slows the backfill 10-50× vs. local PG.
- Cloud-RDS IOPS cost burned on the long-running backfill instead
  of a short bulk push.
- Defeats the "local Docker DB" pattern that BE proved.

**Decision:** REJECTED — slower and more expensive than local+push.

---

## Consequences

### Positive

- **Backfill is unblocked.** Task 0027 (impl) no longer needs to
  wait for task 0011 (CDK bootstrap). Operator can start the local
  backfill the moment the Rust CLI and schema migrations land.
- **Zero AWS infrastructure to provision for the backfill itself.**
  No Fargate cluster, no IAM contract design, no CloudWatch alarm
  tuning, no SNS topic. The only AWS touch on Stream 2 is the
  post-backfill cloud-push tool, which is small and scoped.
- **Same team pattern, same operator skillset.** BE engineers
  running their own `backfill-runner` and prices-api engineers
  running `sdex-backfill` look at the same shaped tool, the same
  partition layout, the same `aws s3 sync --no-sign-request`
  command line.
- **Lower cost during backfill.** No Fargate compute, no NAT egress,
  no CloudWatch ingestion fees. Cloud cost arrives only at push time.
- **Tranche 1 readiness gated only on prices-api artifacts plus the
  cloud-push step.** The push step depends on 0011 (cloud RDS), but
  not on Fargate, IAM, or any of the operational design that ADR
  0002 dragged in.

### Negative

- **Workstation uptime dependency during backfill.** A laptop that
  goes to sleep mid-run is a recoverable interrupt (resumable per
  §5.2 of the design G-note) but adds operator overhead. BE
  accepts the same trade in ADR 0010.
- **Public endpoint `GET /backfill/status` has two-step semantics.**
  During backfill, the cloud-side view shows whatever was last
  pushed; the freshest local progress is visible to the operator
  via a local SQL query but not to API consumers. Mitigation: the
  cloud-push tool runs in a "tip-backward chunk" cadence so the
  cloud view advances every push cycle. Documented in the design
  G-note §11.
- **Single-laptop wall-clock.** 57M ledgers × ~3 ms/ledger ≈ 12-16
  days continuous, plus archive sync time (probably ~1.5× that with
  network bottleneck). Acceptable per task 0022 §4's existing
  estimate.
- **Multi-laptop parallelism deferred.** If wall-clock proves
  unacceptable in measurement, v2 has to land a `prices-db-merge`
  tool (analog of BE's `db-merge`) before parallelism is safe.

---

## References

- [BE ADR 0010 — local backfill rationale](../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md)
- [BE ADR 0040 — multi-laptop merge hazards](../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md)
- BE crate `crates/backfill-bench/` — the simpler reference implementation
- BE crate `crates/backfill-runner/` — the production-grade variant (sink abstraction, prefetch pipeline)
- BE crate `crates/xdr-parser/` — the `.xdr.zst` parsing helper we pin via git Cargo dep
- Stellar public archive: `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` (anonymous, `--no-sign-request`)
- [Task 0012 design G-note](../1-tasks/active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-local-design.md) — operational landing of this ADR
- [Task 0022 archived spec](../1-tasks/archive/0022_RESEARCH_sdex-filter-and-extraction-spec/) — SDEX filter + decode + bucket spec consumed by both deployment shapes
- [ADR 0003: `price_ohlcv` PK includes `quote_asset_id`](./0003_price-ohlcv-pk-includes-quote-asset-id.md) — schema PK shape consumed by the backfill UPSERT
