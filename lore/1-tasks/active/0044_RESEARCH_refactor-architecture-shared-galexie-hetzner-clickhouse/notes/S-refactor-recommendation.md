---
title: "S: Recommendation — prices-api live data sink on BE's Hetzner ClickHouse"
type: synthesis
status: developing
spawned_from: ../README.md
spawns: []
tags: [synthesis, recommendation, refactor, clickhouse, hetzner, step-7]
links:
  - "./R-be-hetzner-ch-shape.md"
  - "./R-ingest-target-mapping.md"
  - "./R-stellar-peers-galexie-live-feed.md"
  - "./R-aws-hetzner-auth-network.md"
  - "./I-schema-ownership-options.md"
  - "./R-cost-delta.md"
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: "Synthesis across the six prior steps."
---

# S: Recommendation — prices-api live data sink on BE's Hetzner ClickHouse

## 0. TL;DR

**Recommendation: conditional go.** The refactor is architecturally
sound; every technical question has a clear answer. Its readiness is
gated on **two external events**:

1. BE's Hetzner ClickHouse reaches production (BE tasks 0216 +
   0227 close).
2. The cross-team conversation with BE produces written commitments
   on schema-ownership, bucket fan-out, capacity sizing, cert
   issuance, and cost-share.

The right move **now** is to capture the architectural commitment
in an ADR, park the blocked tasks 0011 / 0038 / 0039 / 0040 against
that ADR, and open the cross-team conversation. The right move
**when both gating conditions clear** is to rewrite the blocked
tasks against this design and execute (~4–5 person-weeks).

This is not "go now" because implementation against a not-yet-shipped
Hetzner deployment is premature. It is also not "no-go" because the
refactor is cheaper than the RDS plan at any plausible scale and
strictly simpler in steady state. **Lock in the direction; defer
execution.**

---

## 1. Recommended topology (final)

```
┌──────────────────────────────────────────────────────────┐
│ Stellar mainnet — Public Global Stellar Network ; 2015     │
│                  validator overlay (SCP)                   │
└──────────────────────────┬───────────────────────────────┘
                           │  BE non-validating Captive Core
                           │  (KNOWN_PEERS / built-in discovery)
                           ▼
┌──────────────────────────────────────────────────────────┐
│ BE ECS Fargate (us-east-1a, BE-owned)                       │
│ stellar-core ── named pipe ──▶ Galexie ── PutObject ──▶ S3 │
│ (Captive watcher)              (append mode)               │
└──────────────────────────┬───────────────────────────────┘
                           │ .xdr.zst, ~1 file / 5–6 s, indefinite retention (BE ADR 0006)
                           ▼
┌──────────────────────────────────────────────────────────┐
│ S3: {env}-stellar-ledger-data (BE-owned)                   │
└──────────────────────────┬───────────────────────────────┘
                           │ OBJECT_CREATED → SNS topic (Shape B)
                           │
            ┌──────────────┴──────────────┐
            ▼                             ▼
┌───────────────────────┐      ┌────────────────────────┐
│ BE Ledger Processor   │      │ prices-api Ledger       │
│ Lambda                 │      │ Processor Lambda        │
│ (writes BE schema)     │      │ (writes prices.* schema)│
└───────────┬───────────┘      └────────────┬───────────┘
            │                                │
            └───────┬────────────────────────┘
                    │ HTTPS :443 mTLS over public internet
                    │ (no VPC, no NAT)
                    ▼
┌──────────────────────────────────────────────────────────┐
│ Hetzner dedicated server (BE-owned, multi-tenant)          │
│                                                            │
│  Caddy :443 — Let's Encrypt server cert + require_and_     │
│              verify mTLS against BE self-signed CA          │
│              │                                              │
│              ▼ docker bridge                                │
│  ClickHouse — single instance, loopback only                │
│   ├─ default.*    ── BE indexer data (BE-owned)            │
│   └─ prices.*     ── prices-api data (PRICES-OWNED)         │
│                                                            │
│  Borg → BX21 Storage Box (daily; ask BE for BACKUP DATABASE │
│  prices target)                                            │
└──────────────────────────────────────────────────────────┘
```

Subordinate Lambda set on the AWS side (unchanged shape, retargeted
write path):

- **Ledger Processor** (S3 event-driven) — writes `prices.price_ohlcv_1m`
  per-source rows.
- **Current Price Updater** (rate(1 min)) — writes
  `prices.current_prices`.
- **Oracle Fetcher** (rate(5 min)) — writes `prices.oracle_prices`.
- **Asset Discovery** (rate(1 hour)) — writes `prices.assets`.
- **Cleanup Worker** (cron daily) — `ALTER … DROP PARTITION` per
  table per granularity. (See §4.4 below.)
- **OHLCV Rollup Lambda — eliminated.** Replaced by MV chain
  `price_ohlcv_1m → 15m → 1h → 4h → 1d → 1w → 1M`.
- **Schema migration applier** (one-shot per deploy) — applies
  `packages/prices-ch-schema/migrations/NNNN_*.sql`.

---

## 2. Inherited working hypotheses (consolidated)

The six prior notes each landed working hypotheses. The synthesis
adopts them as the recommendation:

| # | Decision | Source |
|---|---|---|
| 1 | Single mTLS endpoint at Caddy:443; no second port, no direct CH native protocol | step 1 §8 |
| 2 | `price_ohlcv` uses **CH-B**: one row per `(ts, asset, quote, granularity, source)` on `ReplacingMergeTree(version)` | step 2 §2.3 |
| 3 | Rollups: **MV chain** `1m → 15m → 1h → 4h → 1d → 1w → 1M`. Rollup Lambda eliminated | step 2 §3 |
| 4 | `current_prices`: **mechanical Lambda port (CH-MV-C)** — Updater retained, writes `ReplacingMergeTree(updated_at)` | step 2 §4 |
| 5 | `oracle_prices`: native `MergeTree`. `assets` + `backfill_progress`: `ReplacingMergeTree(updated_at)` in `prices.*` (not DynamoDB) | step 2 §5 |
| 6 | Cleanup uses `ALTER … DROP PARTITION` (cheap); implies splitting OHLCV per-granularity into `price_ohlcv_1m`, `_15m`, … | step 2 §6 |
| 7 | Bucket fan-out: **Shape B (SNS topic)** between S3 and the two Lambdas | step 3 §6 |
| 8 | mTLS cert + key in **Secrets Manager** (2 secrets per env), loaded once at Lambda init via `OnceCell` | step 4 §1.2-3 |
| 9 | Client cert: **1-year manual rotation + CloudWatch NotAfter alarm** | step 4 §1.4 |
| 10 | No intermediate SQS between Lambda and Hetzner — S3 retention + DLQ already provide durability | step 4 §3.3 |
| 11 | Library: official `clickhouse` Rust crate over HTTP with `reqwest::Client` injected | step 4 §4 |
| 12 | Schema ownership: **separate `prices` database** in the same CH cluster (Option 1) | step 5 §2 |
| 13 | Migrations: **~200-line hand-rolled Rust applier** + versioned `migrations/NNNN_*.sql`. CI step for dev/staging, one-shot Lambda for prod | step 5 §3.4 |
| 14 | DDL coordination: announcement-not-approval inside `prices.*`; joint review only for cross-database reads | step 5 §4 |
| 15 | Resource isolation: `users.d/prices-api.xml` with profile + quota; numbers TBD per traffic-actuals | step 5 §5 |
| 16 | Cost-share: target **5–10% pro-rata** as opening proposal; accept free ride if BE offers it | step 6 §6 |

These are not separately re-litigated below. They are the recommendation.

---

## 3. Impact map on existing blocked tasks

| Task | Current shape | Refactor verdict | Action now |
|---|---|---|---|
| **0011** — Bootstrap CDK with SSM platform lookups | Provisions RDS + RDS Proxy + VPC integration + IAM | **Major rewrite.** RDS gone, RDS Proxy gone, no VPC integration. Replaced by: Secrets Manager mTLS material, no-VPC Lambda, IAM for `secretsmanager:GetSecretValue` on the 2 prices-api secrets | **Stay blocked.** Add history entry pointing at this note as the redesign source. Do not rewrite the spec until BE's Hetzner CH ships + ADR is accepted. |
| **0017** — Local ClickHouse for prices-api Tranche 1 backfill | Local laptop CH populated by BE's backfill-runner; consumed by Stream 1 backfill CLI | **Unchanged.** The backfill story (ADR 0001 / 0005) is workstation-local; the refactor only affects the live cloud sink. | **Leave as-is.** No frontmatter change. |
| **0038** — Prices Ledger Processor Lambda (live S3-event-driven RDS writer) | sqlx → RDS UPSERT with ON CONFLICT DO UPDATE; runs in VPC | **Major rewrite.** sqlx → `clickhouse` crate; UPSERT → ReplacingMergeTree INSERT; VPC → no-VPC; ADR 0004 row shape → CH-B per-source rows | **Stay blocked.** Add history entry; defer rewrite. |
| **0039** — Prices periodic workers Lambda set | EventBridge-scheduled Lambdas (Rollup, Current Price Updater, Oracle Fetcher, Asset Discovery, Cleanup) | **Major rewrite.** Rollup Lambda **deleted** (MV chain replaces it). Others retargeted to CH. Cleanup becomes `DROP PARTITION` per granularity table. | **Stay blocked.** Add history entry; defer rewrite. |
| **0040** — Prices API Gateway and read handlers | axum handlers reading from RDS via sqlx; partition-pruned queries | **Moderate rewrite.** Read handlers retargeted to CH (`clickhouse` crate + per-source GROUP BY). Endpoint contracts unchanged. Read latency budget needs re-validation (CH HTTP vs. local-VPC PG). | **Stay blocked.** Add history entry; defer rewrite. |

**Why "stay blocked + history entry" rather than rewrite-in-place
now:** the cross-team conversation may meaningfully change the
shape (e.g. if BE refuses Option 1, falls back to Option 4 sidecar
CH). Rewriting blocked task specs against a design that might shift
costs churn. The history-entry approach preserves traceability
without committing to text.

---

## 4. Cross-team conversation with BE (consolidated agenda)

The 28 open questions across the six R/I-notes cluster into four
agenda items for the BE conversation. Bring all four as one
written brief.

### 4.1 Cluster A — Architectural buy-in

**Asks:**

- (Q20) Approve Option 1: a separate `prices` database in BE's CH
  cluster, with a dedicated CH user.
- (Q11) Approve Shape B: rewire bucket → SNS topic, prices-api
  subscribes its own Lambda. One-time BE CDK change.
- Confirm the announcement-not-approval norm for DDL inside
  `prices.*`.

**Why one cluster:** these three commitments together unlock all
the technical work. If BE rejects any of the three, the refactor
shape changes materially.

### 4.2 Cluster B — Capacity, retention, backup

**Asks:**

- (Q1, Q26) Hetzner box hardware specs + monthly cost — needed to
  ground capacity math and cost-share negotiation.
- (Q17) Confirm Caddy `max_keepalive_conns` headroom for a second
  tenant in addition to BE's own writers. Surface BE's current
  default.
- (Q12) Confirm BE intent to keep BE ADR 0006 (indefinite S3
  retention). Prices-api's replay story depends on it.
- (Q23) Add `BACKUP DATABASE prices` as a separate daily Borg
  target so prices-api can be restored independently.
- (Q4) Backup RPO acceptable for prices-api rows (daily Borg
  granularity vs. RDS PITR). Product-side question, surface to BE
  as a heads-up.

### 4.3 Cluster C — Auth + secrets

**Asks:**

- (Q3) BE issues `prices-api-{env}` client certs (one per env)
  via the existing per-AWS-service issuance script.
- Rotation cadence: 1-year manual + NotAfter alarm; revisit in
  one year.
- Revocation model: rotate CA on compromise (BE pattern,
  inherited).

### 4.4 Cluster D — Money

**Asks:**

- (Q25) Cost-share number. Prices-api opens with 5–10% pro-rata
  proposal (~$3–$15/mo per env). Free ride is the friendly
  alternative; flat fee acceptable up to ~$15/env without changing
  the recommendation.
- Re-open if production scales materially (the at-scale savings
  give both sides room to renegotiate).

---

## 5. Follow-up tasks to spawn

### 5.1 Spawn now (gated on the ADR landing)

| ID | Type | Title | Notes |
|---|---|---|---|
| **next ADR (0007)** | ADR | `live-data-sink-on-shared-hetzner-clickhouse` | Captures this synthesis as the architectural commitment. Pre-condition for unblocking the other tasks. Status: proposed → accepted after cross-team conversation lands. |
| **next task (0045)** | RESEARCH or FEATURE | `cross-team-bundle-with-be-on-hetzner-ch-tenancy` | Drives clusters A–D from §4 to closure. Likely 1–2 weeks calendar time, mostly waiting. |

### 5.2 Spawn when both gates clear (BE Hetzner CH ships AND ADR accepted)

These are the **rewrites** of the existing blocked tasks. They
inherit the IDs (no new IDs) but get fully restated specs:

- **0011** rewrite — CDK bootstrap **without** RDS / VPC; with
  Secrets Manager mTLS material + IAM for prices-api Lambdas.
- **0038** rewrite — Ledger Processor Lambda targeting Hetzner CH
  via `clickhouse` crate + mTLS `reqwest::Client`.
- **0039** rewrite — Periodic workers retargeted; Rollup Lambda
  **deleted** entirely.
- **0040** rewrite — API read handlers retargeted; latency
  re-validation included.

### 5.3 Spawn opportunistically

| ID | Type | Title | Notes |
|---|---|---|---|
| (TBD) | FEATURE | `prices-ch-schema-migration-applier` | The ~200-line Rust applier from step 5. Can be built in parallel with the BE conversation since it has no BE dependency for design. |
| (TBD) | DOCS | `update-design-doc-§2-§5-§10-§11-for-hetzner-ch` | Update the design doc to match the new direction (similar to how task 0013 updated for ADR 0001/0005). Lands alongside the ADR's "accepted" transition. |

### 5.4 G-note to consider landing inside this task before close

A **`G-prices-init-sql.md`** capturing the actual DDL for `prices.*`
(all six tables + MV chain + users.d/prices-api.xml) would close
the loop on step 5's "what does the schema literally look like"
question. It's design output, not research — fits the G- prefix.
Recommended; not blocking on closure.

---

## 6. Risks and how to handle them

| Risk | Probability | Impact | Handling |
|---|---|---|---|
| BE rejects Option 1 (separate `prices` DB) | Low | High | Fall back to Option 4 sidecar CH instance. Adds ops surface; still cheaper than dedicated RDS. |
| BE refuses to add the prices-api Lambda as a notification target | Very low | High | Reverts to standalone Galexie + S3 for prices-api. Refactor abandoned. Highly unlikely because §11.1 already commits to this sharing pattern in the design doc. |
| Hetzner CH operational maturity issues post-cutover | Medium | Medium | Keep RDS-plan code path in a feature flag for the first quarter; cut over per-env to start. |
| Cost-share negotiation produces flat fee > $20/env | Low | Low | Refactor still has the at-scale case + non-monetary deltas. Pencils out long-term. |
| BE Hetzner CH ships much later than projected | Medium | Low | The conditional-go posture explicitly accepts schedule drift — no engineering investment is committed until BE ships. |
| `ReplacingMergeTree` eventual-consistency under high write rate produces user-visible inconsistencies on the API read path | Low | Medium | The read path uses `FINAL` or explicit `argMin/argMax + GROUP BY` per step 2 §1. Validate with load test in the implementation task; falls back to CH-A if CH-B's read semantics don't hold. |
| Migration applier becomes the bottleneck (mTLS in CI) | Low | Low | Run the applier as a one-shot Lambda in prod (skip CI complications). Documented in step 5 §3.4. |

---

## 7. Closing checklist for task 0044

Before this task moves to `archive/`, complete:

- [x] Step 1: BE Hetzner CH shape distilled (`R-be-hetzner-ch-shape.md`)
- [x] Step 2: Ingest target mapping (`R-ingest-target-mapping.md`)
- [x] Step 3: Stellar peers → S3 → Lambda live feed (`R-stellar-peers-galexie-live-feed.md`)
- [x] Step 4: AWS↔Hetzner auth + network (`R-aws-hetzner-auth-network.md`)
- [x] Step 5: Schema ownership options (`I-schema-ownership-options.md`)
- [x] Step 6: Cost delta (`R-cost-delta.md`)
- [x] Step 7: Synthesis recommendation (this note)
- [ ] (Optional) `G-prices-init-sql.md` — concrete DDL for `prices.*` schema and MV chain
- [ ] Spawn ADR 0007 — `live-data-sink-on-shared-hetzner-clickhouse`
- [ ] Spawn task 0045 — cross-team conversation bundle with BE
- [ ] Add history entries to blocked tasks 0011 / 0038 / 0039 / 0040 noting the redesign source (this synthesis note). 0017 untouched.
- [ ] Move 0044 to `archive/` with `status: completed` and a history note that follows the lore-framework-tasks completion checklist.

---

## 8. The one-line case

The Stellar live feed and the Hetzner storage plane are both
BE-funded; the prices-api refactor is one Lambda swap and one
CH database carve-out away from sharing both. The technical
case is small, well-bounded, and cheaper at scale. The work
on hand is the cross-team conversation, not the engineering.
