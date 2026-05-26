---
title: 'Integration options — CDK, CI/CD, and backfill execution shapes'
type: idea
status: developing
tags: [cdk, ci-cd, backfill, options]
links: []
history:
  - date: 2026-05-11
    status: developing
    who: okarcz
    note: 'Sketched candidate integration shapes; trade-offs only, no decision yet.'
---

# Integration options

Three open design dimensions, each with 2–3 concrete shapes. Cross-cutting; some
combinations make more sense than others.

---

## Dimension A — CDK ownership

### Option A1: Single shared CDK app inside `soroban-block-explorer/infra/aws-cdk`

Add Prices API stacks to BE's CDK repo. Co-deploy.

**Pros:**

- Direct cross-stack references (CFN exports / `Stack.of()` lookups) for VPC ID, S3 ARN,
  Galexie task IAM, etc.
- One `cdk deploy` produces a coherent environment.
- Patterns/constructs shared by literal import, not copy-paste.

**Cons:**

- Tight repo coupling: PRs to Prices infra touch the BE repo; BE's CI runs Prices tests.
- Two teams' release cadences collide on one pipeline.
- "Stellar can fork & redeploy Prices" (Prices §0) becomes "fork two repos and reconcile."
- Changes Prices API's repo identity — it stops being self-contained.

### Option A2: Separate CDK app, cross-stack via SSM Parameter Store / CFN exports

Prices API owns its own CDK app under `infra/` in this repo. BE exports VPC ID, S3 bucket
name, NAT EIP, ECS cluster ARN, etc., via SSM (`/be/prod/vpc-id`, …). Prices CDK reads
those at synth or runtime.

**Pros:**

- Each repo deploys independently; team boundaries respected.
- Failures don't cross-contaminate (Prices deploy bug can't break BE).
- Clear ownership: Prices repo is self-contained from a fork-redeploy standpoint
  (third party publishes their own SSM keys).
- Mirrors typical multi-product AWS account structure.

**Cons:**

- Bootstrap order matters: BE must be up first to publish SSM keys.
- Cross-stack lookups by name are stringly-typed; renames break consumers silently until
  next deploy.
- Slight duplication of CDK construct patterns (mitigated by a shared npm package if it
  becomes painful).

### Option A3: Separate CDK apps, shared infra extracted to a third "platform" stack

Pull VPC, NAT, S3, Galexie, Secrets Manager into a `stellar-platform-infra` CDK app.
Both BE and Prices consume it via SSM.

**Pros:**

- Cleanest separation of concerns; truly multi-tenant platform layer.
- Either app can be replaced without disturbing the other.

**Cons:**

- New repo + ownership question (who owns the platform stack?).
- Disproportionate to a 2-service problem; adds an org-level abstraction with no obvious
  third tenant on the horizon.

---

## Dimension B — CI/CD shape

### Option B1: Independent GitHub Actions in each repo, both using OIDC

Mirror BE's ADR 0001 model: each repo has its own workflows, assumes its own per-env IAM
role, uses its own OIDC trust policy.

**Pros:** zero cross-repo coupling; matches BE's model exactly.
**Cons:** developers maintain two CI surfaces; environment promotion is manual.

### Option B2: Reusable workflow library

Publish BE's deploy workflow as a `workflow_call` reusable action; Prices repo consumes it.

**Pros:** DRY; updates propagate.
**Cons:** one team's change can break the other; needs versioning discipline.

### Option B3: Monorepo CI (only viable with Option A1)

Single workflow drives both deploys.

**Pros:** atomic releases.
**Cons:** see Option A1 cons; loses independent release cadence.

**Recommendation lean:** B1 — matches BE's documented pattern; trivial to set up.

---

## Dimension C — SDEX backfill execution (rows 6 + 7 of the matrix)

This is the highest-impact open question. Three shapes:

### Option C1: Stand up a dedicated Fargate service in Prices API account

Prices CDK declares its own ECS cluster (or task definition on a thin cluster) for the
SDEX backfill. Runs continuously per Prices §5.6.

**Pros:**

- Matches Prices API design exactly; no design rework.
- Independent of BE; BE doesn't have to host another team's workload.
- 13-week continuous run is appropriate for ECS, not Lambda.

**Cons:**

- Loses the "shared cluster" cost story (~$0 cluster overhead is already trivial; no
  meaningful financial loss).
- Operationally Prices API now owns ECS too.

### Option C2: Long-running EC2 instance + systemd

Run the backfill binary as a service on a dedicated EC2 (e.g. `m6g.large`).

**Pros:**

- Conceptually simpler than ECS; no task definitions / image registry.
- Comparable cost to Fargate at this duty cycle.

**Cons:**

- Manual OS patching; less ephemeral.
- Doesn't match BE's pattern; introduces a third runtime model.

### Option C3: Step Function orchestrating chunked Lambda backfill workers

Map state over ledger ranges; each Lambda invocation processes ~50k ledgers. Step Function
handles checkpointing and retries.

**Pros:**

- Pure serverless; matches the rest of the Prices stack.
- Auto-scales horizontally.

**Cons:**

- 15-min Lambda cap forces fine-grained chunking; orchestration overhead.
- Concurrency must be throttled to not overwhelm `db.m6g.large` writer.
- Net throughput uncertain vs. continuous task design.

**Recommendation lean:** C1 — least design rework; honors Prices API's existing throughput
estimates. Re-evaluate C3 if Fargate cost proves material.

---

## Dimension D — Soroban AMM backfill (row 8)

If BE's `soroban_events_appearances` does not store decoded topics+data:

### Option D1: Re-derive AMM swaps from public-archive XDR

Same source as the SDEX backfill, filtered to AMM contract IDs only. Single backfill code
path covering both streams. Loses the "fast Tranche 1 completion" benefit but simplifies
architecture (one stream, not two).

### Option D2: Add a Prices-funded enrichment step in BE's pipeline

Fork BE's Ledger Processor or co-deploy a sidecar Lambda that decodes AMM events into a
Prices-owned table. Cross-team coupling.

### Option D3: Re-investigate BE schema first (cheap)

Read BE's actual `soroban_events_appearances` schema and any companion tables before
committing. The Prices API design's claim might still hold if BE persists decoded data
for some events even though the read-path uses public archive for E14.

**Recommendation lean:** D3 first (1-day spike), then D1 if D3 disconfirms. D2 is a last
resort.

---

## Coherent combinations

| Combination              | Properties                                                                                                                       |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| **A2 + B1 + C1 + D3→D1** | Self-contained Prices repo; BE reused only for live ingestion infra; Prices owns its backfill tooling. **Most likely best fit.** |
| A1 + B3 + C1 + D2        | Maximum integration; minimum independence. Don't recommend.                                                                      |
| A2 + B2 + C3 + D1        | Pure-serverless Prices; reusable BE workflows. Defensible but risky on backfill throughput.                                      |

The recommended combination feeds into `S-shared-infra-recommendation.md`.
