# Prices API — Technical Design Document (Post-2nd-Review)

> This document supersedes `prices-api-design-after-review.md`. It incorporates changes
> required by second-round reviewer feedback: shared infrastructure with the funded Soroban
> Block Explorer is explicitly catalogued and removed from the Prices API budget; the
> technology stack is updated to Rust (axum + sqlx) to match the block explorer codebase;
> a concrete historical backfill plan with milestones in Tranches 2 and 3 is added; and a
> `GET /backfill/status` API endpoint is introduced to allow progress monitoring.

## Revision History

Substantive revisions only. Minor wording / schema-comment / index tweaks land via normal
commits and are not tracked here. Append a new row when a change touches the architecture,
the API surface, or the cost / budget framing.

| Date       | Sections touched                                                                                                                                                          | Driver                                                                                                                                                                                                                                                                                    | Summary                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-09-02 | §5.7 (new)                                                                                                                                                                | [Task 0248](../lore/1-tasks/active/0248_DOCS_blend-is-named-in-the-rfp-but-is-not-a-price-source.md)                                                                                                                                                                                      | **Venue coverage recorded against the RFP's named markets.** The RFP's Price Aggregation bullet names four markets (Soroswap, Aquarius, SDEX, Blend); we ingest three of them plus Phoenix, which it does not name. New §5.7 states the count plainly and records why **Blend cannot be a price source**: it is a lending protocol with no swap, and a price is a property of a trade. The decisive point is that Blend pool creators choose an _oracle_ to price collateral, which places Blend downstream of a service like this one — a consumer of price data, not a producer. Its 80/20 BLND:USDC backstop AMM is the only part that trades and its volume is **unmeasured**, stated rather than implied. No extractor, no `Venue` arm, no registry seeding: pricing BLND from the backstop pool would be a feature of its own. Deliberately **not** generalised into a rule about lending protocols.                                                                                                                                                                                                                                                                                                                 |
| 2026-05-20 | §0, §1.1, §1.2, §2.1, §2.3, §3, §4.5, §5.2–§5.4, §5.6, §6, §7, §8, §9, §10, §11 (all-table refresh)                                                                       | [ADR 0007](../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md) (accepted) · [Task 0045](../lore/1-tasks/archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/README.md) · [Task 0049](../lore/1-tasks/active/0049_DOCS_overview-rewrite-for-adr-0007.md) | **Live data sink flipped from Prices-owned RDS PostgreSQL to BE's shared Hetzner ClickHouse cluster** (separate `prices` database). All live OHLCV / current-prices / oracle / asset registry / backfill-progress data now lives in ClickHouse, written over HTTPS-mTLS to Caddy:443 by Lambdas running outside any VPC. The S3 → Lambda path gains an SNS topic between the bucket and both tenants' processors (one-time BE CDK change). Schema rewritten to per-source `ReplacingMergeTree(version)` rows on per-granularity tables (`price_ohlcv_1m`, `_15m`, …, `_1M`); rollups become a CH materialised-view chain, **eliminating the OHLCV Rollup Lambda**. Prices-api VPC, NAT Gateway, and RDS line items removed; mTLS cert lifecycle added (per-env certs, 1-year manual rotation, CA-rotation revocation). Cost lines: $12/mo RDS removed; ~$1-2/env/mo Hetzner CH cost-share added (basis: [task 0046](../lore/1-tasks/archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md) empirical ~0.45 GB/yr, 14.8× compression). Local backfill sections (Stream 1 ADR 0001, Stream 2 ADR 0005) preserved — only their cloud-push targets shift RDS → CH. |
| 2026-05-15 | §2.3, §5.3, §5.6 Stream 1 (two-stream design table, architecture diagram, processing-rate sub-table, schema-coupling note), §9 (Tranche 1 work), §10, §11.1, §11.2, §11.4 | [ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md) · [Task 0029](../lore/1-tasks/active/0029_DOCS_update-design-doc-stream-1-adr-0001.md)                                                                                                                         | Stream 1 (Soroban AMM) backfill reconciled with ADR 0001: source moved from BE's PG `soroban_events` to a **local ClickHouse** instance populated upfront by BE's `backfill-runner --target=clickhouse`; deployment shape moved from ECS Fargate to a local Rust CLI (`soroban-amm-backfill`) on the operator's workstation, ScVal decoding via `stellar-xdr` crate, one-shot completion push to cloud RDS. Stream 1 Fargate cost line removed; backfill total now ~$30. BE coupling reframed as a transient prep-step tool invocation (not runtime DB read); §11.1 `soroban_events` row removed and its development-savings counterpart added to §11.2. Closes out the design-doc sweep started in [Task 0013](../lore/1-tasks/archive/0013_DOCS_update-design-doc-to-match-be-reality.md).                                                                                                                                                                                                                                                                                                                                                                                                                               |
| 2026-05-14 | §2.3, §3.5, §4.5, §5.3, §5.6 Stream 2, §6, §8, §9, §10, §11.1, §11.4                                                                                                      | [ADR 0005](../lore/2-adrs/0005_stream2-sdex-local-workstation-backfill.md) (supersedes ADR 0002) · [Task 0013](../lore/1-tasks/archive/0013_DOCS_update-design-doc-to-match-be-reality.md)                                                                                                | Stream 2 (SDEX) backfill moved from continuous ECS Fargate to a local Rust CLI on the operator's workstation with a separate `sdex-cloud-push` step to cloud RDS. `backfill_progress` schema swapped from heartbeat fields to `last_push_at`. `GET /backfill/status` response and tranche acceptance criteria reframed around push cadence. Backfill compute cost dropped ~95%. Stream 1 (Soroban AMM) reconciliation per [ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md) is tracked separately under [Task 0029](../lore/1-tasks/active/0029_DOCS_update-design-doc-stream-1-adr-0001.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| (earlier)  | whole document                                                                                                                                                            | second-round reviewer feedback                                                                                                                                                                                                                                                            | Post-2nd-Review baseline: stack switched to Rust + axum + sqlx (matching the Block Explorer codebase); BE-shared infrastructure explicitly catalogued (§11); historical backfill plan with Tranche 2 / Tranche 3 milestones added (§5.6); `GET /backfill/status` endpoint introduced (§4.5).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |

---

## 0. Deployment & AWS Account

The service is deployed to the **same dedicated AWS sub-account** as the already-funded and
operational Soroban Block Explorer (Rumble Fish, awarded March 2026). The two services share
the ingestion side of the platform — Galexie ECS, the S3 ledger bucket (now with SNS fan-out
between the bucket and both tenants' processors) — and the **live data plane**: per
[ADR 0007](../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md), prices-api
writes into a separate `prices` database inside BE's Hetzner-hosted production ClickHouse
cluster (Caddy:443 HTTPS-mTLS endpoint), with per-database isolation via ClickHouse's native
multi-tenant primitives (database, user, quota, profile). See Section 11 for the full
sharing accounting.

Because the live data plane lives on Hetzner (BE-owned, prices-api joins as a second tenant),
the prices-api AWS footprint is deliberately small: no Prices-api VPC, no NAT Gateway, no
RDS instance. Lambda functions run **outside any VPC**, reach Caddy:443 over the public
internet, and authenticate with per-environment client certificates issued by BE's CA and
stored in AWS Secrets Manager.

Infrastructure is managed via AWS CDK and deployed through a shared CI/CD pipeline (GitHub
Actions). The codebase is fully open source — Stellar retains the ability to fork and redeploy
to their own infrastructure at any time if needed.

---

## 1. Architecture Overview

### 1.1 API Layer

```
                         ┌────────────────────────────────┐
                         │       AWS API Gateway           │
                         │  (REST API, rate limiting,      │
                         │   API keys, throttling,         │
                         │   built-in response caching)    │
                         └────────────┬───────────────────┘
                                      │
                         ┌────────────▼─────────────┐
                         │      AWS Lambda           │
                         │  (API handler functions)  │
                         │   Rust / axum             │
                         │   (no VPC; outbound only) │
                         └────────────┬─────────────┘
                                      │ HTTPS-mTLS (public internet)
                                      │ client cert from Secrets Manager
                                      ▼
                  ┌─────────────────────────────────────┐
                  │  Caddy:443 reverse proxy             │
                  │  (Hetzner, BE-managed; mTLS termin.) │
                  └────────────┬────────────────────────┘
                                      │
                  ┌─────────────────────────────────────┐
                  │  ClickHouse `prices` database        │
                  │  (shared Hetzner cluster, BE-funded; │
                  │   separate database from BE's        │
                  │   `default.*`, ADR 0007)             │
                  └─────────────────────────────────────┘
```

### 1.2 Data Ingestion Layer

```
┌─────────────────────────────────────────────────────────────┐
│  SHARED WITH SOROBAN BLOCK EXPLORER (no additional cost)    │
│                                                             │
│  Stellar Network (mainnet peers)                            │
│          │                                                  │
│          ▼ (Captive Core / ledger stream)                   │
│  ┌──────────────────────────────────┐                       │
│  │  Galexie — ECS Fargate (1 task)  │                       │
│  │  Continuously running            │                       │
│  │  Exports one file per ledger     │                       │
│  │  (~1 file every 5–6 seconds)     │                       │
│  └──────────────┬───────────────────┘                       │
│                 │ LedgerCloseMeta XDR (zstd-compressed)     │
│                 ▼                                           │
│  ┌──────────────────────────────────┐                       │
│  │  S3: stellar-ledger-data/        │                       │
│  │  ledgers/{seq_start}-            │                       │
│  │         {seq_end}.xdr.zstd       │                       │
│  └──────────────┬───────────────────┘                       │
│                 │ S3 PutObject event notification           │
│                 ▼                                           │
│  ┌──────────────────────────────────┐                       │
│  │  SNS topic (BE-owned, fan-out)   │                       │
│  │  Subscribers: BE Ledger Proc.    │                       │
│  │              Prices Ledger Proc. │                       │
│  └──────────────┬───────────────────┘                       │
└─────────────────┼───────────────────────────────────────────┘
                  │ SNS delivery
                  ▼
┌──────────────────────────────────────────────────────┐
│  Lambda "Prices Ledger Processor" (per file, Rust)   │
│  No VPC; outbound HTTPS only                         │
│  1. Download + decompress XDR                        │
│  2. Parse LedgerCloseMeta via stellar-xdr crate      │
│  3. Extract SDEX trades (ManageSellOfferResult /     │
│     ManageBuyOfferResult → offersClaimed[])          │
│  4. Extract Soroban swap events (CAP-67):            │
│     Soroswap, Aquarius, Phoenix — all in one stream  │
│  5. INSERT per-source 1-min OHLCV rows into          │
│     ClickHouse `prices.price_ohlcv_1m`               │
│     over HTTPS-mTLS to Caddy:443                     │
└──────────────────────────────────────────────────────┘
               │
               ▼ HTTPS-mTLS (public internet)
       ┌──────────────────────┐      ┌───────────────────────┐
       │  Hetzner ClickHouse  │◄─────│ EventBridge-triggered │
       │  `prices` database   │      │ Lambda workers (Rust):│
       │  (shared cluster,    │      │  - Current Price Upd. │
       │   BE-funded; ADR 0007)│     │  - Oracle Fetcher     │
       └──────────────────────┘      │  - Asset Discovery    │
                  ▲                  │  - Cleanup Worker     │
                  │ MV chain         └───────────────────────┘
                  │ (1m → 15m → 1h →
                  │  4h → 1d → 1w → 1M;
                  │  replaces OHLCV Rollup Lambda)

       ┌────────────────────────────────────────────────┐
       │  External read-only (metadata / cross-ref)     │
       │  Reflector Oracle (Soroban RPC simulate)       │
       │  Soroswap API   (pool pair metadata only)      │
       │  Aquarius API   (pool pair metadata only)      │
       └────────────────────────────────────────────────┘
```

---

## 2. AWS Services Breakdown

### 2.1 Components Hosted by Prices API (Prices API budget)

| Service                              | Role                        | Details                                                                                                                                                                                                                                                                                           |
| ------------------------------------ | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Lambda — Prices Ledger Processor** | Primary ingestion           | SNS-triggered (one message per S3 PutObject on BE's `stellar-ledger-data/` bucket). Rust binary, no VPC. Parses XDR via `stellar-xdr` crate, extracts SDEX trades and Soroban swap events, INSERTs per-source 1-min OHLCV rows to ClickHouse `prices.price_ohlcv_1m` over HTTPS-mTLS to Caddy:443 |
| **Lambda — Current Price Updater**   | Price aggregation           | EventBridge rate(1 min). Reads latest candles from `prices.price_ohlcv_1m`, computes cross-source VWAP per §5.5, writes to `prices.current_prices` (`ReplacingMergeTree(updated_at)`)                                                                                                             |
| **Lambda — Oracle Fetcher**          | Oracle cross-reference      | EventBridge rate(5 min). Reads Reflector via Soroban RPC `simulateTransaction`. Writes to `prices.oracle_prices`. Failures do not block primary ingestion                                                                                                                                         |
| **Lambda — Asset Discovery**         | Asset registry              | EventBridge rate(1 hour). Detects new SEP-41 contract deployments and classic asset issuances; UPSERTs into `prices.assets`                                                                                                                                                                       |
| **Lambda — Cleanup Worker**          | Data retention              | EventBridge cron(02:00 UTC daily). `ALTER TABLE … DROP PARTITION` on old monthly partitions of each per-granularity OHLCV table                                                                                                                                                                   |
| **Lambda — API handlers**            | Public API                  | Individual functions per route group. Rust / axum via `lambda_runtime`, 256–512 MB, 15s timeout. No VPC; outbound HTTPS-mTLS to Caddy:443                                                                                                                                                         |
| **API Gateway**                      | Public API entry point      | REST API, usage plans, API key auth, rate limiting (1 req/s sustained, burst 5, 100 000 req/month per self-service key — task 0157), request validation. Built-in response cache (0.5 GB) with per-endpoint TTLs                                                                                  |
| **EventBridge Scheduler**            | Scheduled triggers          | Cron/rate rules for all periodic Lambda workers                                                                                                                                                                                                                                                   |
| **Secrets Manager**                  | Credentials & mTLS material | Per-env client `{cert,key,ca}` for Caddy:443 mTLS (single JSON bundle secret per identity, named by `MTLS_SECRET_NAME`); Soroswap/Aquarius API keys; oracle contract address                                                                                                                      |
| **CloudWatch + X-Ray**               | Observability               | API latency, error rates, ingestion lag, Lambda duration/concurrency, backfill progress; mTLS cert NotAfter alarm                                                                                                                                                                                 |
| **S3** (API docs)                    | Documentation hosting       | self-service onboarding portal + API reference, served from the block explorer's bucket and CloudFront distribution at `sorobanscan.rumblefish.dev/api/`; the OpenAPI document is served by the API itself                                                                                        |

**Components no longer in the Prices API budget** (eliminated by ADR 0007):

- **RDS PostgreSQL** — live data plane moved to BE's shared Hetzner ClickHouse cluster (§2.3 + §11.1).
- **VPC, subnets, security groups, NAT Gateway** — Lambdas run outside any VPC; mTLS over the public internet gates access at Caddy. The earlier "shared with BE's VPC" arrangement is no longer needed prices-side.
- **OHLCV Rollup Lambda** — rollups are now a ClickHouse materialised-view chain (`1m → 15m → 1h → 4h → 1d → 1w → 1M`) maintained inside the shared cluster.

### 2.2 External Services Consumed (read-only)

| External service               | Purpose                                           | Failure impact                                                    |
| ------------------------------ | ------------------------------------------------- | ----------------------------------------------------------------- |
| Reflector Oracle (Soroban RPC) | Price cross-reference only — not a primary source | Non-critical — oracle column shows `null` or last known value     |
| Soroswap API                   | Pool pair metadata and discovery                  | Non-critical — existing pairs continue; new pair discovery pauses |
| Aquarius API                   | Pool pair metadata and discovery                  | Non-critical — same as above                                      |

### 2.3 Components Shared with Soroban Block Explorer (no additional charge)

These components are funded by the Soroban Block Explorer grant and already operational. They
are listed here to confirm there is no double-billing.

| Component                               | Block Explorer context                                                                                                                  | Prices API usage                                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Galexie ECS Fargate task**            | 1 task, continuous, 1 vCPU / 2 GB, writes to `stellar-ledger-data/` S3 every ~5–6 s                                                     | One Galexie serves both services; no second Galexie needed                                                                                                                                                                                                                                                                                                                                                         |
| **S3 bucket `stellar-ledger-data/`**    | Owned and funded by Block Explorer                                                                                                      | Prices API Lambda reads the same files; no additional S3 storage cost                                                                                                                                                                                                                                                                                                                                              |
| **SNS topic on `stellar-ledger-data/`** | One-time BE CDK change: S3 PutObject events fan out to an SNS topic instead of a direct Lambda target (ADR 0007 §3.2 / 0045 Cluster A2) | Prices API Lambda subscribes its own queue to the SNS topic — same delivery semantics as the legacy direct-S3 target, with fan-out as a first-class primitive                                                                                                                                                                                                                                                      |
| **Hetzner ClickHouse cluster**          | Production data plane for BE's `default.*` (BE ADR 0044 / 0045) — single CH instance on a single Hetzner box behind Caddy:443           | Prices API joins as a **second tenant** with its own `prices` database, isolated via ClickHouse's first-class multi-tenant primitives (database, user, quota, profile). Writes and reads over HTTPS-mTLS through the same Caddy. No second CH instance, no second Hetzner box. Cost-share: opening proposal ~1-2% pro-rata ($1-2/env/mo) per task 0046's empirical ~0.45 GB/yr footprint; D12 commercial follow-up |
| **mTLS Certificate Authority**          | BE-managed self-signed CA + per-AWS-service client-cert issuance script (BE Cluster C asks)                                             | Prices API receives per-env client cert + key (one pair per env), stored in AWS Secrets Manager. Issuance script invocation is the only BE-side operator step per cert lifecycle (1-year manual rotation)                                                                                                                                                                                                          |
| **GitHub Actions CI/CD patterns**       | Shared pipeline structure and CDK conventions                                                                                           | Prices API pipeline reuses the same CDK deployment pattern                                                                                                                                                                                                                                                                                                                                                         |

**Confirmed: none of the components in 2.3 appear in the Prices API budget.**

**Removed rows.** Earlier versions of this table listed components that no longer reflect the
chosen architecture:

1. An **"ECS Fargate cluster"** row claiming Prices API historical backfill tasks ran in
   BE's shared cluster. Per [ADR 0005](../lore/2-adrs/0005_stream2-sdex-local-workstation-backfill.md),
   the SDEX backfill is a local workstation CLI — not a Fargate task — so nothing is
   shared with BE's cluster on the Stream 2 path.

2. A **"Block Explorer `soroban_events` table (read-only)"** row claiming the Soroban AMM
   backfill held a read-only connection to BE's RDS within the shared VPC. Per
   [ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md), Stream 1
   consumes a locally-run ClickHouse instance populated by BE's
   `backfill-runner --target=clickhouse`, not BE's production RDS. Nothing on the Stream 1
   path is shared runtime infrastructure either; the only BE artefacts consumed are the
   `backfill-runner` CLI (a one-shot transient invocation) and the production ClickHouse
   schema — captured in §11.2's development-savings table.

3. The **"VPC (us-east-1a)"** and **"NAT Gateway"** rows that listed Prices API resources
   deploying into BE's shared VPC and routing egress through BE's NAT Gateway. Per
   [ADR 0007](../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md), Lambdas
   now run **outside any VPC** and reach Caddy:443 over the public internet, gated by
   mTLS rather than IP. No prices-api Lambda runs in BE's VPC, so neither row applies.
   The monthly savings previously attributed to VPC/NAT sharing have shifted into the
   Hetzner CH row above (and the matching §11.1 update).

See §11.1 for the matching cost-savings table and §5.6 Stream 1 for the reconciled
architecture.

---

## 3. Database Schema (ClickHouse on shared Hetzner cluster, ADR 0007)

All tables live in the `prices` database inside BE's Hetzner ClickHouse cluster. Schema
ownership: prices-api owns `prices.*` migrations unilaterally; cross-database reads
against `default.*` (if any) are wrapped in named `prices.*` views (ADR 0007 §3.7).

**Engine choices recap (ADR 0007 §3.3 + ADR 0004):**

- **`ReplacingMergeTree(version)`** on `price_ohlcv_*` — per-source rows, one row per
  `(timestamp, asset_id, quote_asset_id, granularity, source)`. Idempotent re-writes from
  ledger replay collapse on background merge. Read path uses `FINAL` or `argMax/argMin +
GROUP BY` to handle eventual consistency.
- **`ReplacingMergeTree(updated_at)`** on `current_prices`, `assets`, `backfill_progress`
  — last-write-wins on the timestamp column.
- **`MergeTree`** on `oracle_prices` — append-only.
- **Per-granularity tables** (`price_ohlcv_1m`, `_15m`, `_1h`, `_4h`, `_1d`, `_1w`, `_1M`)
  feeding a **materialised-view chain** that aggregates 1m → 15m → … → 1M. This replaces
  the OHLCV Rollup Lambda.
- **Monthly partitions** on every OHLCV table via `PARTITION BY toYYYYMM(timestamp)`.
  Cleanup = `ALTER TABLE … DROP PARTITION`, instant.

### 3.1 Assets Table

```sql
CREATE TABLE prices.assets (
    asset_id         UInt32,            -- application-assigned surrogate id
    asset_code       FixedString(12),
    asset_type       Enum8('classic' = 1, 'soroban' = 2),
    issuer_address   FixedString(56),   -- G-address; empty string for XLM
    contract_address FixedString(56),   -- C-address (SAC or native contract); empty if N/A
    home_domain      String,            -- classic assets only; stored verbatim
    is_active        UInt8 DEFAULT 1,   -- soft-delete flag; readers filter `is_active = 1`
                                        -- unless ?include_inactive=true is set
    created_at       DateTime DEFAULT now(),
    updated_at       DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address)
SETTINGS index_granularity = 8192;
```

Surrogate `asset_id` is application-assigned (small `UInt32` counter inside the prices-api
write path), not the asset's on-chain identity — same idea as the prior SERIAL but materialised
outside the DB. The natural-key tuple `(asset_code, issuer_address, contract_address)` is the
sort key so reads by identity are O(log N).

### 3.2 Price Snapshots (OHLCV) — Per-Source, Per-Granularity Tables

One table per granularity. Each carries **per-source rows** (ADR 0004) — the cross-source
merge happens at read time (§4.2 / ADR 0007 §3.3), not at write time.

```sql
CREATE TABLE prices.price_ohlcv_1m (
    timestamp        DateTime CODEC(DoubleDelta),
    asset_id         UInt32,
    quote_asset_id   UInt32,            -- ADR 0003: PK includes the quote leg
    source           LowCardinality(String),  -- 'sdex', 'soroswap', 'aquarius', 'phoenix', ...
    open             Decimal(38, 14),
    high             Decimal(38, 14),
    low              Decimal(38, 14),
    close            Decimal(38, 14),
    volume_base      Decimal(38, 14) DEFAULT 0,
    volume_quote     Decimal(38, 14) DEFAULT 0,  -- native quote-asset volume (sum of
                                                  -- quote-leg amounts); decoder already
                                                  -- computes it to derive vwap. Oracle-
                                                  -- multiplied into volume_quote_usd by
                                                  -- the enrichment Lambda (task 0026)
    volume_quote_usd Decimal(38, 14) DEFAULT 0,  -- USD-denominated; filled by task 0026
    vwap             Decimal(38, 14),   -- single-source, single-minute VWAP
                                         -- (volume_quote / volume_base);
                                         -- see §5.5 layering for cross-source weighting
    trade_count      UInt32 DEFAULT 0,
    version          UInt64             -- monotonic per-row version for ReplacingMergeTree
                                         -- (ledger sequence × 1000 + intra-ledger order)
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, source, timestamp)
SETTINGS index_granularity = 8192;

-- Identical shape for the rolled-up granularities; populated by the MV chain below.
CREATE TABLE prices.price_ohlcv_15m AS prices.price_ohlcv_1m;
CREATE TABLE prices.price_ohlcv_1h  AS prices.price_ohlcv_1m;
CREATE TABLE prices.price_ohlcv_4h  AS prices.price_ohlcv_1m;
CREATE TABLE prices.price_ohlcv_1d  AS prices.price_ohlcv_1m;
CREATE TABLE prices.price_ohlcv_1w  AS prices.price_ohlcv_1m;
CREATE TABLE prices.price_ohlcv_1M  AS prices.price_ohlcv_1m;
```

**Rollup MV chain** (sketch — one MV per step; full DDL lives in
`docs/database-schema/clickhouse-prod-schema.sql` once landed):

```sql
CREATE MATERIALIZED VIEW prices.mv_ohlcv_1m_to_15m
TO prices.price_ohlcv_15m AS
SELECT
    toStartOfInterval(timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id,
    quote_asset_id,
    source,
    argMin(open,  timestamp) AS open,
    max(high)                 AS high,
    min(low)                  AS low,
    argMax(close, timestamp)  AS close,
    sum(volume_base)          AS volume_base,
    sum(volume_quote_usd)     AS volume_quote_usd,
    sum(volume_quote_usd) / nullIf(sum(volume_base), 0) AS vwap,
    sum(trade_count)          AS trade_count,
    max(version)              AS version
FROM prices.price_ohlcv_1m
GROUP BY timestamp, asset_id, quote_asset_id, source;
-- ... repeat for 15m→1h, 1h→4h, 4h→1d, 1d→1w, 1w→1M.
```

**Why per-granularity tables + MV chain (ADR 0007 §3.4):**

- Cleanup is `ALTER TABLE prices.price_ohlcv_1m DROP PARTITION '202503'` — instant, no
  per-row DELETE, no vacuum.
- Each granularity gets a sort key tuned for its read pattern; no `WHERE granularity = ?`
  filter on hot queries.
- Rollups run inside the DB on insert; no scheduled Lambda needs to re-derive them.
- Backfill writes into old monthly partitions of the historical granularity tables (1d, 1h,
  …) directly via the same INSERT path; the MV chain only fires on `_1m` writes for live
  ingestion. Backfill scripts produce already-aggregated rows for the granularities they
  target (matches the prior Postgres design's behaviour).

**Eventual consistency (ADR 0007 §Negative).** `ReplacingMergeTree` collapses duplicate
PK tuples in the background; reads see un-merged rows briefly. Read handlers use
`SELECT … FROM prices.price_ohlcv_1m FINAL` or an explicit `argMax/argMin … GROUP BY`
re-aggregation (ADR 0007 §3.3). Both patterns are verified workable in task 0044 §2.

### 3.3 Current Prices

```sql
CREATE TABLE prices.current_prices (
    asset_id         UInt32,
    price_usd        Decimal(38, 14),
    price_xlm        Decimal(38, 14),
    change_24h_pct   Decimal(10, 4),
    change_7d_pct    Decimal(10, 4),
    volume_24h_usd   Decimal(38, 14),
    market_cap_usd   Decimal(38, 14),    -- token_supply × price_usd, NULL if supply
                                          -- unavailable; supply read from the asset's
                                          -- token contract (Soroban `total_supply` /
                                          -- SEP-41) or Horizon /assets for classic
    vwap_24h         Decimal(38, 14),
    sources          String,             -- JSON: per-source {price, volume_24h}
                                          -- numeric values serialised as strings to
                                          -- preserve Decimal(38,14) precision; sources
                                          -- excluded by min_volume_usd or outlier
                                          -- detection are absent from the object
                                          -- (the min_volume_usd system default is
                                          -- applied CONDITIONALLY — see §5.5)
    updated_at       DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_id)
SETTINGS index_granularity = 8192;
```

Sorting on `(volume_24h_usd, asset_id)` etc. is **not** the right CH idiom for `GET /assets`
sorted/paginated reads — CH does not have B-tree secondary indexes the way Postgres does. The
read handlers instead `ORDER BY` + `LIMIT` on the chosen sort column against the merged view
of `current_prices`; with <100k tracked assets the scan is bounded and fast. If sorted reads
become hot, a per-sort-column materialised view can be added.

### 3.4 Oracle Prices

```sql
CREATE TABLE prices.oracle_prices (
    timestamp     DateTime CODEC(DoubleDelta),
    asset_id      UInt32,
    oracle_name   LowCardinality(String),  -- 'reflector', 'chainlink', 'redstone', 'band'
    price_usd     Decimal(38, 14),
    raw_data      String                   -- JSON blob, unparsed for forensic value
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, oracle_name, timestamp)
SETTINGS index_granularity = 8192;
```

Append-only; oracle reads are forensic / cross-reference, not in the hot path. Cleanup is
`ALTER TABLE … DROP PARTITION` on old months.

### 3.5 Backfill Progress Tracking

One row per backfill stream (`sdex_archive`, `soroban_amm`). Both rows are
seeded at provisioning time. Per ADRs 0001 and 0005, both canonical streams
are populated by workstation-local processes, so the cloud row is updated by
a push step — not by a continuously-running cloud-side task. For
`sdex_archive` the writer is the `sdex-backfill` CLI itself, writing directly to
Hetzner over mTLS (ADR 0009; runs in tip-backward chunks);
for `soroban_amm` the writer is the one-shot AMM CLI when it completes its
push to cloud. The `GET /backfill/status` endpoint reads both rows and
returns them as the nested `sdex` and `soroban_amm` objects (see Section 4.5).

```sql
CREATE TABLE prices.backfill_progress (
    task_name        LowCardinality(String),  -- 'sdex_archive', 'soroban_amm'
                                               -- (canonical streams; additional task_names
                                               -- can be inserted for targeted gap-fills
                                               -- or future AMM reindexes)
    start_ledger     UInt64,
    target_ledger    UInt64,
    current_ledger   UInt64,                   -- boundary ledger reflected in the cloud
                                                -- DB after the most recent push: oldest
                                                -- pushed for sdex_archive (high→low),
                                                -- newest pushed for soroban_amm (low→high)
    status           Enum8('running' = 1, 'paused' = 2, 'completed' = 3, 'error' = 4)
                     DEFAULT 'running',
    last_push_at     Nullable(DateTime),       -- timestamp of the most recent push that
                                                -- advanced current_ledger; NULL until the
                                                -- first push. Used by the GET /backfill/status
                                                -- freshness alarm (see §5.6)
    started_at       DateTime DEFAULT now(),
    completed_at     Nullable(DateTime),
    updated_at       DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (task_name);

-- Seed both stream rows at provisioning time so GET /backfill/status always
-- has a row to read for each stream. target_ledger is updated to the current
-- realtime tip when each task starts.
INSERT INTO prices.backfill_progress
    (task_name, start_ledger, target_ledger, current_ledger, status)
VALUES
    ('sdex_archive', 1,        0, 0, 'running'),
    ('soroban_amm',  48500000, 0, 0, 'running');
```

**Removed from earlier schema versions.** `last_heartbeat`, `rate_per_hour`, and
`eta_hours` were on the original Fargate-shaped schema (one continuous task per
stream, heartbeating into the cloud row every 15 minutes). ADR 0005 made
Stream 2 a local workstation CLI; ADR 0001 had already done the same for
Stream 1. Neither stream has a continuously-running cloud-side process, so
neither field has a meaningful value to write. Operators inspect live CLI
progress (rate, ETA) via direct SQL on the local workstation ClickHouse; the cloud
DB carries only the most recent push state. If a future stream reintroduces a
continuous cloud-side writer, these columns can be added back at that time.

### 3.6 Retention Policy (Cleanup Worker Lambda)

```
Fine-grained data retention:
  prices.price_ohlcv_1m   → keep 7 days  (ALTER TABLE DROP PARTITION for months > 7d old)
  prices.price_ohlcv_15m  → keep 30 days

Coarse-grained data (1h, 4h, 1d, 1w, 1M) → keep forever (per-table cleanup not needed)

Partition lifecycle (per-table):
  Drop partitions older than the retention window:
    ALTER TABLE prices.price_ohlcv_1m  DROP PARTITION '<YYYYMM>'
    ALTER TABLE prices.price_ohlcv_15m DROP PARTITION '<YYYYMM>'
  Oracle table: drop partitions older than 13 months
    ALTER TABLE prices.oracle_prices   DROP PARTITION '<YYYYMM>'

  No CREATE-partition-ahead step — ClickHouse creates partitions implicitly on
  the first INSERT that lands in a new month.

Cleanup-worker Lambda runs daily at 02:00 UTC and issues `ALTER TABLE … DROP
PARTITION` statements over HTTPS-mTLS to Caddy:443.
```

---

## 4. API Endpoints Design

**Base URL:** `https://api.prices.stellar.example.com/v1`

### 4.1 Assets

#### `GET /assets`

List all tracked assets with metadata and current price.

**Query parameters:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `type` | string | all | Filter: `classic`, `soroban`, or `all` |
| `search` | string | — | Search by asset code (prefix match) |
| `sort` | string | `volume_24h` | Sort by: `price`, `volume_24h`, `change_24h`, `code` |
| `order` | string | `desc` | `asc` or `desc` |
| `cursor` | string | — | Pagination cursor (Base64-encoded, see below) |
| `limit` | int | 50 | Max 200 |

**Cursor pagination mechanism:**

The cursor is a Base64-encoded JSON object containing the sort column value and the asset ID of the
last returned row (ID breaks ties when sort values are equal):

```
cursor = base64({ "volume_24h": 1523400.50, "id": 42 })
       → "eyJ2b2x1bWVfMjRoIjoxNTIzNDAwLjUwLCJpZCI6NDJ9"
```

On the first request (no cursor), the query is:

```sql
SELECT * FROM current_prices JOIN assets ON assets.id = current_prices.asset_id
ORDER BY volume_24h DESC, id DESC
LIMIT 51;  -- limit + 1 to determine has_more
```

On subsequent requests, the server decodes the cursor and uses a **keyset condition**:

```sql
SELECT * FROM current_prices JOIN assets ON assets.id = current_prices.asset_id
WHERE (volume_24h, id) < (1523400.50, 42)  -- decoded from cursor
ORDER BY volume_24h DESC, id DESC
LIMIT 51;
```

`has_more` is determined by fetching `limit + 1` rows.

**Response:**

```json
{
  "data": [
    {
      "asset_code": "USDC",
      "asset_type": "classic",
      "issuer_address": "GA5ZSE...XYZ",
      "contract_address": "CABC...DEF",
      "home_domain": "centre.io",
      "price_usd": "1.0001",
      "change_24h_pct": "-0.02",
      "change_7d_pct": "0.01",
      "volume_24h_usd": "1523400.50",
      "vwap_24h": "1.0002",
      "sources": {
        "sdex": { "price": "1.0001", "volume_24h": "800000" },
        "soroswap": { "price": "1.0002", "volume_24h": "500000" },
        "aquarius": { "price": "1.0001", "volume_24h": "223400" }
      },
      "updated_at": "2026-02-10T12:00:00Z"
    }
  ],
  "cursor": "eyJpZCI6NTB9",
  "has_more": true
}
```

#### `GET /assets/{asset_identifier}`

Get single asset details. `asset_identifier` can be:

- `{code}:{issuer}` for classic assets (e.g. `USDC:GA5ZSE...XYZ`)
- `{contract_address}` for Soroban tokens (e.g. `CABC...DEF`)
- `native` for XLM

### 4.2 Prices / OHLCV

#### `GET /assets/{asset_identifier}/ohlcv`

Historical OHLCV candlestick data.

**Query parameters:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `timeframe` | string | `24h` | `1h`, `24h`, `7d`, `30d`, `1y`, `all` |
| `granularity` | string | auto | `1m`, `15m`, `1h`, `4h`, `1d`, `1w`, `1M`. Auto-selected from timeframe if omitted |
| `start` | ISO8601 | — | Custom range start (overrides timeframe) |
| `end` | ISO8601 | — | Custom range end |
| `base_currency` | string | `USD` | Quote currency: `USD` or `XLM` |

**Auto-selected granularity mapping:**
| Timeframe | Default Granularity | Max Data Points |
|-----------|-------------------|-----------------|
| `1h` | `1m` | ~60 |
| `24h` | `15m` | ~96 |
| `7d` | `1h` | ~168 |
| `30d` | `4h` | ~180 |
| `1y` | `1d` | ~365 |
| `all` | `1d` | variable |

When `timeframe=all` is requested but the backfill has not yet reached the asset's inception
date, the response includes a `backfill_note` field indicating how far back data is available:

```json
{
  "asset": "USDC:GA5ZSE...XYZ",
  "granularity": "1d",
  "base_currency": "USD",
  "backfill_note": "Historical data available from 2022-01-01. Backfill in progress — see GET /backfill/status.",
  "data": [...]
}
```

**Response:**

```json
{
  "asset": "USDC:GA5ZSE...XYZ",
  "granularity": "15m",
  "base_currency": "USD",
  "data": [
    {
      "timestamp": "2026-02-10T11:00:00Z",
      "open": "1.0001",
      "high": "1.0005",
      "low": "0.9998",
      "close": "1.0003",
      "volume_base": "125000.00",
      "volume_quote_usd": "125037.50",
      "vwap": "1.0003",
      "trade_count": 47
    }
  ]
}
```

#### `GET /assets/{asset_identifier}/price`

Current real-time price (latest snapshot from `current_prices`).

`price_usd` is the **latest priced close**: a candle whose USD value has not
been computed yet (enrichment is a separate, lagging pass) is skipped rather
than reported as `0`. It is **not age-bounded** — for an asset that has
stopped trading it is simply its last priced close, up to the 24 h window
old — and `updated_at` is the snapshot time, **not** the price's age. No
field carries that age today. `"0"` means no priced close exists in the
window at all. `price_usd` is also **not** outlier-filtered: it reports the
newest priced close regardless of venue, while `vwap_24h` is the de-noised
figure.

`sources` and `vwap_24h` are bounded where `price_usd` is not, and the
asymmetry is deliberate: a per-venue entry asserts "this venue is quoting X",
so a venue whose last priced close is more than **2 h** old is dropped rather
than carried. A source is therefore absent when it has no recent priced
close **or** the §5.5 outlier filter excluded it. One consequence worth
planning for: an asset can legitimately return a `price_usd` alongside an
empty `sources` and a `vwap_24h` of `0` — we hold a price, but no venue is
currently quoting. (Task 0135.)

**Response:**

```json
{
  "asset": "USDC:GA5ZSE...XYZ",
  "price_usd": "1.0001",
  "price_xlm": "8.33",
  "vwap_24h": "1.0002",
  "volume_24h_usd": "1523400.50",
  "change_24h_pct": "-0.02",
  "sources": {
    "sdex": { "price": "1.0001", "volume_24h": "800000" },
    "soroswap": { "price": "1.0002", "volume_24h": "500000" },
    "aquarius": { "price": "1.0001", "volume_24h": "223400" }
  },
  "updated_at": "2026-02-10T12:00:30Z"
}
```

### 4.3 Batch / Multi-asset

#### `POST /prices/batch`

Fetch current prices for multiple assets in one call.

**Request body:**

```json
{
  "assets": ["native", "USDC:GA5ZSE...XYZ", "CABC...DEF"]
}
```

### 4.4 Oracle Data

#### `GET /oracles/{asset_identifier}`

Oracle-specific prices for an asset. Oracle data is exposed here for reference only and does not
feed the `price_usd` field in any other endpoint.

**Response:**

```json
{
  "asset": "USDC:GA5ZSE...XYZ",
  "oracles": [
    {
      "name": "reflector",
      "price_usd": "1.0000",
      "updated_at": "2026-02-10T11:55:00Z"
    },
    {
      "name": "redstone",
      "price_usd": "1.0001",
      "updated_at": "2026-02-10T11:58:00Z"
    }
  ]
}
```

### 4.5 Backfill Progress

#### `GET /backfill/status`

Returns the current state of the historical all-time backfill. This endpoint is the primary
mechanism for the Tranche 3 reviewer to validate that backfill is progressing correctly.

Per ADRs 0001 and 0005, both canonical streams run as workstation-local processes; the
cloud-side `prices.backfill_progress` row (ClickHouse, `ReplacingMergeTree(updated_at)`)
advances only when each stream's push step runs. The response therefore reflects the most
recent push state, not a live task heartbeat. Live CLI progress is visible to the operator
via direct SQL on the local workstation ClickHouse but is not surfaced to API consumers. A
CloudWatch alarm fires if `last_push_at` falls behind the configured push-cadence threshold
for the stream's tranche (operator-tunable; see §5.6 "GET /backfill/status Freshness").

The backfill is split into two independent streams with different sources and timelines (see
Section 5.6). The response reflects both.

**Response:**

```json
{
  "realtime_tip_ledger": 57234198,
  "sdex": {
    "status": "running",
    "current_ledger": 34891234,
    "start_ledger": 1,
    "target_ledger": 57234198,
    "progress_pct": 39.2,
    "ledgers_remaining": 34891233,
    "last_push_at": "2026-06-15T11:30:00Z",
    "earliest_data_available": "2019-08-22T00:00:00Z"
  },
  "soroban_amm": {
    "status": "completed",
    "last_push_at": "2026-04-14T08:23:11Z",
    "completed_at": "2026-04-14T08:23:11Z",
    "earliest_data_available": "2023-11-01T00:00:00Z"
  }
}
```

| Field                                 | Description                                                                                                                                                                                                                                                                                                |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sdex.status`                         | `running`, `paused`, `completed`, or `error` — SDEX archive backfill                                                                                                                                                                                                                                       |
| `sdex.current_ledger`                 | Oldest ledger reflected on Hetzner after the most recent backfill write (ADR 0009 — direct write, no separate push step). ⚠️ It is the lowest completed run **start**, so it asserts a floor, not proven contiguous coverage up to `target_ledger` (task 0263).                                            |
| `sdex.progress_pct`                   | `(target_ledger - current_ledger) / (target_ledger - start_ledger) * 100`, computed at read time                                                                                                                                                                                                           |
| `sdex.ledgers_remaining`              | `current_ledger - start_ledger`, computed at read time                                                                                                                                                                                                                                                     |
| `sdex.last_push_at`                   | Timestamp of the most recent successful **direct write** to Hetzner by the backfill CLI (ADR 0009; the column name predates that ADR and is retained). The CloudWatch freshness alarm fires when this is older than the configured cadence threshold for the active tranche. `null` until the first write. |
| `sdex.earliest_data_available`        | Stored timestamp of the oldest SDEX OHLCV row known for this stream — recorded by the push step when it first lands a candle for a given timestamp, **not** computed live via `MIN(timestamp)`. Returned as-is, so reads are O(1).                                                                         |
| `soroban_amm.status`                  | Typically `completed` from Tranche 1 onwards                                                                                                                                                                                                                                                               |
| `soroban_amm.last_push_at`            | Timestamp of the one-shot AMM CLI's completion push (ADR 0001). `null` until the push happens.                                                                                                                                                                                                             |
| `soroban_amm.earliest_data_available` | Same semantics as `sdex.earliest_data_available` — stored, not computed. Lands at the Soroban activation date (~Nov 2023) once the one-time backfill completes.                                                                                                                                            |

---

## 5. Data Ingestion Pipeline

### 5.1 Galexie Operation

Galexie is the SDF's Composable Data Platform exporter. It runs as a shared ECS Fargate task
(funded under the Block Explorer grant) that connects to Stellar mainnet peers via Captive Core
and writes one `LedgerCloseMeta` XDR file to S3 on every ledger close (~every 5–6 seconds).

The Prices API does not deploy or fund a second Galexie instance. Instead, the Prices API
Ledger Processor Lambda is registered as a second S3 event notification target on the shared
`stellar-ledger-data/` bucket, alongside the Block Explorer's Ledger Processor.

| Property       | Value                                                                        |
| -------------- | ---------------------------------------------------------------------------- |
| Runtime        | ECS Fargate, 1 vCPU / 2 GB RAM, continuously running (Block Explorer funded) |
| Output         | `s3://stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`            |
| Format         | `LedgerCloseMeta` XDR, zstd-compressed                                       |
| Recovery       | Checkpoint-aware — resumes from last exported sequence on restart            |
| Lag monitoring | CloudWatch alarm fires if S3 file timestamps fall >60s behind current ledger |

### 5.2 Prices Ledger Processor (Rust)

The Prices Ledger Processor is triggered by an SNS topic that fans S3 PutObject events out
to both BE's and prices-api's Lambdas (ADR 0007 §3.2). It is implemented in Rust using the
`stellar-xdr` crate (official SDF Rust XDR types) and deployed as a Lambda function using
the `lambda_runtime` crate. It runs **outside any VPC**; outbound traffic flows over the
public internet to Caddy:443. It downloads the file, parses it, and extracts:

- **SDEX trades** from `OperationResult.ClaimAtom[]` across the five trade-shaped op types
  (price, base volume, quote volume, asset pair) — per task 0048's decoder spec.
- **Soroban AMM swap events** (CAP-67) from `SorobanTransactionMeta.events` — Soroswap,
  Aquarius, Phoenix, plus oracle updates from Reflector/Redstone — per task 0048.

The XDR parsing logic is implemented as a shared Rust workspace crate, compiled into both
the Block Explorer's Ledger Processor and the Prices Ledger Processor, eliminating
duplication.

**Write semantics into `prices.price_ohlcv_1m` (ADR 0007 §3.3 / ADR 0004).** The writer
issues plain `INSERT` statements that produce **one row per
`(timestamp, asset_id, quote_asset_id, source)`** per minute the source contributed to.
Duplicate-PK rows from re-ingestion of the same ledger (replay, retry, backfill overlap)
are collapsed by the `ReplacingMergeTree(version)` engine on background merge —
no `ON CONFLICT … DO UPDATE` and no incremental-merge expressions are needed at write
time, because the engine treats `version` (per-row, ledger-sequence-derived) as the
ordering key when collapsing duplicates.

Cross-source weighting (the §5.5 VWAP formula) is **not** computed here; it happens one
layer up in the Current Price Updater Lambda (§5.3 row, ADR 0007 §3.3 + the §5.5 layering
table). The decoder writes the per-source bucket `vwap = volume_quote / volume_base` and
nothing more.

**mTLS write path.** The Lambda holds a `clickhouse` Rust client configured with the
per-env client `{cert,key,ca}` loaded from AWS Secrets Manager (a single JSON bundle
secret per identity, named by `MTLS_SECRET_NAME`, per ADR 0007 §3.5). Connections to Caddy:443 are warmed in the global init of the Lambda
runtime and reused across invocations to amortise TLS handshake cost — important because
the cross-cloud RTT (AWS → Hetzner, ~80-130 ms) dominates per-request latency if every
INSERT opens a fresh connection.

### 5.3 Ingestion Workers

> **⚠️ Backfill sink model superseded by ADR 0009 (direct-write).** The two SDEX
> rows below (`sdex-backfill` writing to a **local ClickHouse** on the
> workstation, then a separate **`sdex-cloud-push`** step streaming those rows to
> Hetzner) and the AMM row's "completion cloud push" describe the **original
> ADR 0005 / ADR 0001** local-stage-then-push design. **[ADR 0009](../lore/2-adrs/0009_backfill-direct-write-to-hetzner-clickhouse.md)
> retired it:** both backfill CLIs now write **directly to Hetzner `prices.*`
> over the 0052 mTLS client as they decode** — there is **no local ClickHouse
> mirror and no separate `sdex-cloud-push` step**. §9 (Tranche-1 Work) already
> reflects the delivered direct-write path. Throughout §5.3–§5.6 and the §5.5
> data-flow diagram, read every `sdex-cloud-push` / "cloud-push cadence" /
> "local ClickHouse sink" reference as the backfill CLI's **direct write** to
> Hetzner; `backfill_progress.last_push_at` (a real, retained column) is the
> timestamp of the most recent such write.

| Worker                                                          | Trigger                                                                                  | Source                                                                                                                                          | Data                                                                                                                                                                                                                               |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Prices Ledger Processor**                                     | SNS message (per S3 PutObject; ~every 5–6 s)                                             | `LedgerCloseMeta` from BE's S3                                                                                                                  | SDEX trades + all Soroban AMM swap events → per-source 1-min OHLCV rows in `prices.price_ohlcv_1m`                                                                                                                                 |
| **Oracle Fetcher**                                              | EventBridge rate(5 min)                                                                  | Reflector Oracle (Soroban RPC `simulateTransaction`)                                                                                            | Oracle reported prices → `prices.oracle_prices`                                                                                                                                                                                    |
| **Asset Discovery**                                             | EventBridge rate(1 hour)                                                                 | Ledger account entries in `LedgerCloseMeta`                                                                                                     | New classic asset issuances; new SEP-41 contract deployments → `prices.assets`                                                                                                                                                     |
| **Current Price Updater**                                       | EventBridge rate(1 min)                                                                  | `prices.price_ohlcv_1m` (after ingestion)                                                                                                       | Cross-source VWAP per §5.5 → `prices.current_prices`                                                                                                                                                                               |
| **Cleanup Worker**                                              | EventBridge cron(02:00 UTC)                                                              | `prices.*`                                                                                                                                      | `ALTER TABLE … DROP PARTITION` for expired month-partitions of `price_ohlcv_1m`, `_15m`, `oracle_prices`                                                                                                                           |
| **SDEX Backfill CLI** (`sdex-backfill`, ADR 0005)               | Local Rust CLI on operator workstation, run in tip-backward chunks during the project    | `s3://aws-public-blockchain` (anonymous `--no-sign-request`)                                                                                    | Historical SDEX trades → per-source 1-min rows in **local ClickHouse** on the workstation                                                                                                                                          |
| **SDEX Cloud Push** (`sdex-cloud-push`, ADR 0005)               | Operator-invoked between chunks; only Hetzner-CH-touching component on the Stream 2 path | Local ClickHouse `price_ohlcv` + `assets`                                                                                                       | Streams accumulated rows to `prices.price_ohlcv_*` over HTTPS-mTLS to Caddy:443; advances `prices.backfill_progress` row for `sdex_archive` (`current_ledger`, `last_push_at`)                                                     |
| **Soroban AMM Backfill CLI** (`soroban-amm-backfill`, ADR 0001) | One-shot Local Rust CLI on operator workstation, run once during Tranche 1               | Local ClickHouse `soroban_events` (populated upfront by BE's `backfill-runner --target=clickhouse`); ScVal decoding via the `stellar-xdr` crate | Historical Soroswap/Aquarius/Phoenix swaps → per-source 1-min rows; on completion runs the cloud push that lands all rows into `prices.*` on Hetzner CH and sets the `soroban_amm` `backfill_progress` row to `status='completed'` |

**Worker removed: OHLCV Rollup Lambda** (eliminated by ADR 0007 §3.4). The 1m → 15m → 1h →
4h → 1d → 1w → 1M roll-up chain is implemented inside ClickHouse as a chain of
materialised views attached to `prices.price_ohlcv_1m` (sketch in §3.2). Each MV's `INSERT
SELECT` runs at the time the source row is appended; backfill writes that land directly
in a higher granularity (e.g. `_1d`) skip the chain by writing into the target table
directly.

### 5.4 EventBridge Scheduler Rules

```
prices-ledger-processor:  SNS message → Lambda "prices-ledger-processor"
oracle-ingest:             rate(5 minutes)       → Lambda "oracle-worker"
asset-discovery:           rate(1 hour)          → Lambda "discovery-worker"
price-update:              rate(1 minute)        → Lambda "price-updater"
retention-cleanup:         cron(0 2 * * *)       → Lambda "cleanup-worker"
```

Note: the Prices Ledger Processor is event-driven (SNS subscription on BE's bucket
fan-out topic), not schedule-driven. The previously-scheduled `ohlcv-rollup` rule is
**removed** — rollups now run inside ClickHouse via the materialised-view chain (§3.2 /
ADR 0007 §3.4). All other workers remain on EventBridge schedules.

### 5.5 VWAP Calculation Logic

```
Weighted Price = Σ(source_price × source_volume_24h) / Σ(source_volume_24h)

Where sources = [SDEX, Soroswap, Aquarius, ...]
Only include sources where volume_24h > configurable_min_threshold_usd (e.g. $100)
```

Volume threshold is configurable per-request via `?min_volume_usd=` query param or defaults to
the system setting.

> **As implemented (task 0118).** The system default is **$100**, and it is applied
> **conditionally**: a below-threshold source is dropped only when the asset still has a source
> _above_ the threshold. The rule exists to stop a dust venue skewing a real market, and on an
> asset whose every venue is dust there is no real market to defend — dropping them all would
> blank `vwap_24h`/`sources` while the row still carries a usable `price_usd`. This is a
> deliberate deviation from the literal reading above, taken after a pre-merge production
> measurement: the unconditional form would have blanked **2,960 of 3,068 priced assets (96.5%)**,
> the same failure shape as the 2026-08-21 liveness-guard rollback. An **explicit**
> `?min_volume_usd=` is different: it always filters strictly at exactly the value sent, and can
> empty `sources` — the caller asked for that cut. The threshold is a **weighting rule only**;
> `price_usd` and `volume_24h_usd` are never filtered by it.

**Outlier detection:** before a source's price is included in the VWAP, it is compared against the
inter-source median. Sources deviating by more than a configurable percentage are excluded from
that update cycle.

#### Where this formula is implemented (layering)

The §5.5 weighted-price formula is **not** computed by the ingestion path (Prices Ledger
Processor / soroban-amm-backfill / sdex-backfill). It runs one layer up, in the **Current
Price Updater Lambda** (`price-updater`, EventBridge rate(1 min) — see §5.3 row "Current Price
Updater" and §5.4. Implementation tracked in lore task `0039_FEATURE_prices-periodic-workers-lambda-set`,
Step 2).

| Layer                             | Owner                                                                              | Input                                                                     | Formula                                                                                                                        | Output                                                         |
| --------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------- |
| **L1 — Ingestion / decoding**     | Prices Ledger Processor (live) + soroban-amm-backfill + sdex-backfill (historical) | Raw `LedgerCloseMeta` (SDEX) or decoded `soroban_events` swap/trade ticks | Per-tick price `(amount_bought / 10^dec_bought) / (amount_sold / 10^dec_sold)`; per-bucket `vwap = volume_quote / volume_base` | `price_ohlcv_1m` row **per (timestamp, asset, quote, source)** |
| **L2 — Cross-source aggregation** | **Current Price Updater Lambda**                                                   | `price_ohlcv` rows summed over a trailing 24h window per source           | **§5.5 formula** — `Σ(price × volume_24h) / Σ(volume_24h)` across sources, with outlier filter vs inter-source median          | One row per `(asset, quote)` in `current_prices`               |
| **L3 — Read-time merging**        | Rust/axum read handlers                                                            | `price_ohlcv` rows for a window                                           | Per-ADR-0007 §3.3 `GROUP BY` across sources (mostly a SELECT helper, not a re-weighting)                                       | API response                                                   |

The decoder's per-source candle `vwap` (one minute, one source) and §5.5's cross-source
weighted price (twenty-four hours, all sources) are different quantities by design — they
solve different problems and live in different Lambdas. See lore task
`0048_RESEARCH_soroban-events-pricing-decoder-spec` §3 for the decoder-layer definition and
its rationale.

### 5.6 Historical All-Time Backfill Plan

#### Scope and Two-Stream Design

The two price data sources are fundamentally different in where their historical data lives,
which drives a two-stream backfill design:

| Stream                                              | Data location                                                                                                                                                                                                                                                                                                                    | Era                                                     | Method                                                                                                                                                                                                                                                                          |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **SDEX trades**                                     | `TransactionResultMeta` (`ClaimAtom` from the five trade-shaped op types) in `LedgerCloseMeta` XDR                                                                                                                                                                                                                               | All-time (2015 → present, ~57M ledgers)                 | Local Rust CLI on operator workstation (`s3://aws-public-blockchain` anonymous reads) → local ClickHouse → post-backfill cloud push to Hetzner ClickHouse `prices.*` (see ADR 0005)                                                                                             |
| **Soroban AMM swaps** (Soroswap, Aquarius, Phoenix) | `SorobanTransactionMeta.events` (CAP-67), inlined `topics_xdr` + `data_xdr` per row in BE's ClickHouse `soroban_events` (BE ADRs [0033](../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md), [0044](../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md)) | Soroban activation (Nov 2023) → present (~8.5M ledgers) | Local Rust CLI on operator workstation, consuming a local ClickHouse instance populated upfront by BE's `backfill-runner --target=clickhouse` (see [ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md)) → one-shot push to Hetzner ClickHouse `prices.*` |

The Soroban AMM stream is handled first (Tranche 1). A short BE-tooling prep step runs BE's
`backfill-runner --target=clickhouse` against the Soroban-activation-onward ledger range
(~8.5M ledgers) into a Docker-hosted ClickHouse instance on the operator's workstation; the
prices-api `soroban-amm-backfill` CLI then queries that local CH copy (decoded via the
`stellar-xdr` crate), bucketizes into per-source 1-min `price_ohlcv` rows, and pushes the
result to Hetzner ClickHouse `prices.*` in a single completion push. The whole run
completes in hours, not weeks, and the local CH instance is torn down once the push lands.
The SDEX stream requires reading all 57 million ledgers from Stellar's public history
archives and is the long-running backfill that extends beyond the project duration.

#### Architecture

```
STREAM 1 — Soroban AMM (fast, Tranche 1, local CH-sourced, ADR 0001)
─────────────────────────────────────────────────────────────────────
BE `backfill-runner --target=clickhouse` (BE task 0205)
  populates local CH `soroban_events` upfront from
  s3://aws-public-blockchain (~8.5M ledgers, ~hours)
        │
        ▼
Local ClickHouse (Docker) on operator workstation
  soroban_events WHERE signature = 'swap'
    AND contract_id IN (Soroswap, Aquarius, Phoenix)
  JOIN ledgers ON closed_at  (per BE CH prod schema)
        │
        ▼
┌──────────────────────────────────────────────────────────┐
│  soroban-amm-backfill — local Rust CLI                   │
│  - Queries local CH by signature + contract_id            │
│  - Decodes topics_xdr + data_xdr (ScVal) via              │
│    `stellar-xdr` crate (shared BE-authored library)       │
│  - Extracts token pair + amounts                          │
│  - Buckets to 1-minute price_ohlcv (ADR 0003 PK shape)    │
│  - Writes to local CH prices.* mirror (Docker)            │
└──────────────────────────────────────────────────────────┘
        │
        ▼ one-shot completion push (only Hetzner-CH-touching step on Stream 1)
Hetzner ClickHouse `prices.*` (HTTPS-mTLS to Caddy:443)
  - Lands all per-source rows into `prices.price_ohlcv_1m`
    (historical month-partitions) and the higher granularities
    that the CLI pre-rolls
  - Sets `prices.backfill_progress` row for `soroban_amm`:
    current_ledger, last_push_at, status='completed', completed_at
        │
        ▼
Local ClickHouse instance is torn down after the push lands
(one-shot job; CH is a snapshot tool, not a long-running engine)

STREAM 2 — SDEX (local backfill + post-backfill cloud push, ADR 0005)
─────────────────────────────────────────────────────────────────────
s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/
(Stellar public history archive, anonymous `--no-sign-request`)
        │
        ▼ `aws s3 sync` (partition-at-a-time prefetch)
┌─────────────────────────────────────────────────────────┐
│  sdex-backfill — local Rust CLI on operator workstation │
│  (mirrors BE `backfill-runner` pattern, BE ADR 0010)    │
│  - Decompresses `.xdr.zst` via BE `xdr-parser` crate    │
│    (git Cargo dep; library only, no runtime coupling)   │
│  - Filter + extract per task 0022's spec:               │
│    5 trade-shaped op types → ClaimAtom → TradeTick      │
│  - Buckets to 1-minute price_ohlcv (ADR 0003 PK shape)  │
│  - Per-ledger checkpoint: INSERT rows, then             │
│    record ledger in backfill_sdex_ledgers               │
└─────────────────────────────────────────────────────────┘
               │
               ▼
       Local ClickHouse (Docker) on workstation
       (operator-owned; backfill writes here, not Hetzner CH)
               │
               ▼ `sdex-cloud-push` (separate post-backfill tool, HTTPS-mTLS)
       Hetzner ClickHouse `prices.*`
       (historical price_ohlcv_* month-partitions; runs in
        tip-backward chunks so the cloud view advances every
        push cycle)
```

Neither backfill task conflicts with live ingestion: ClickHouse's monthly partition layout
separates historical writes (old month partitions) from live writes (current month
partition), and `ReplacingMergeTree(version)` is safe under concurrent inserts.

**Schema coupling note (ADR 0001):** the Soroban AMM backfill consumes a **local** ClickHouse
instance populated upfront by BE's `backfill-runner --target=clickhouse` against the canonical
production CH schema (`docs/database-schema/clickhouse-prod-schema.sql` in this repo, mirroring
BE's). There is no runtime read against BE's database; the only coupling is the one-time,
transient invocation of BE's `backfill-runner` tool and the BE-authored CH schema the local
instance ingests. If BE evolves the CH `soroban_events` schema after the Tranche 1 backfill
window, the prices-api consumer is unaffected — its CH instance was a snapshot and is torn
down post-push. This dependency is documented in Section 11.

**Stream 2 coupling note (ADR 0005):** Stream 2 has zero runtime or data coupling with the
Block Explorer. The only BE artefact consumed is the `xdr-parser` crate, pinned as a git
Cargo library dependency (`xdr-parser = { git = "https://github.com/rumblefishdev/soroban-block-explorer.git", branch = "main" }`)
and compiled read-only into the `sdex-backfill` binary. No BE database is read, no BE
service is called, no BE infrastructure is shared on the Stream 2 path. (The git pin is a
transient convenience; once BE publishes `xdr-parser` as a standalone versioned crate it
becomes a plain `xdr-parser = "X.Y.Z"` pin — no design change.)

#### Processing Rate and Compute Estimates

**Stream 1 — Soroban AMM (local CH-sourced workstation CLI, ADR 0001):**

| Metric                                   | Value                                                                                                       | Notes                                                                                                          |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Data source                              | Local ClickHouse `soroban_events` (Docker, populated upfront by BE's `backfill-runner --target=clickhouse`) | Per-event rows with inlined `topics_xdr` + `data_xdr` + hoisted `signature` column                             |
| Ledger range                             | ~48.5M–57M (Nov 2023 to present)                                                                            | ~8.5M ledgers worth of events                                                                                  |
| Runtime                                  | Local Rust CLI on operator workstation (`soroban-amm-backfill`)                                             | No AWS infrastructure for the backfill itself; mirrors §5.6 Stream 2's local-CLI pattern                       |
| Workstation prep step                    | BE `backfill-runner --target=clickhouse` populates local CH                                                 | One-shot; runs against `s3://aws-public-blockchain` anonymous reads — no AWS account required                  |
| Local CH footprint                       | Hundreds of GB pre-compression; substantially smaller post-ZSTD on disk                                     | Sized for an operator workstation per ADR 0001 §Rationale (dev-laptop, not Fargate/EC2)                        |
| Sink during backfill                     | Local ClickHouse (Docker) on workstation                                                                    | Hetzner ClickHouse `prices.*` is **not** written until the one-shot completion push                            |
| Estimated wall-clock (including CH prep) | A few hours, dominated by `backfill-runner` archive ingestion                                               | CH query + extraction + OHLCV write is fast against an indexed local store                                     |
| Cloud-push cadence                       | One-shot completion push only                                                                               | `prices.backfill_progress` row for `soroban_amm` advances from `running` to `completed` in a single transition |
| Expected completion                      | During Tranche 1 (Week 2–3)                                                                                 | After the push, the local CH instance is torn down                                                             |

**Stream 2 — SDEX (local workstation CLI, ADR 0005):**

| Metric                               | Value                                                                                                    | Notes                                                                                                               |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| Total ledgers                        | ~57 million                                                                                              | Ledger 1 (Nov 2015) to current tip                                                                                  |
| Runtime                              | Local Rust CLI on operator workstation                                                                   | No AWS infrastructure for the backfill itself; mirrors BE `backfill-runner`                                         |
| Source                               | `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`                                                | Anonymous `--no-sign-request`; no AWS account needed to read                                                        |
| Sink during backfill                 | Local ClickHouse (Docker) on workstation                                                                 | Hetzner ClickHouse `prices.*` is **not** written during backfill — only by the post-backfill `sdex-cloud-push` step |
| Measured CLI rate                    | ~311 ledgers/s (~1.12M ledgers/hour)                                                                     | Per task 0022's measurement against the SDEX filter                                                                 |
| Effective wall-clock (network-bound) | ~12–16 days continuous on one laptop                                                                     | Archive sync is the bottleneck; CPU rarely saturates                                                                |
| Cloud-push cadence                   | Tip-backward chunks (e.g. `--start=tip-1.1M --end=tip` for the first Tranche 1 chunk, then older ranges) | The cloud `GET /backfill/status` view advances at push cadence, not CLI cadence                                     |
| Expected completion                  | Full historical coverage extends past Tranche 3                                                          | Tranche 3 acceptance is "progressing", not "complete" — unchanged from ADR 0002                                     |

The `sdex-backfill` CLI is resumable at per-ledger granularity: each ledger's `price_ohlcv`
UPSERTs and its `backfill_progress` checkpoint advance commit atomically in a single
transaction. A crash mid-ledger leaves `current_ledger` pointing at the last fully-processed
ledger; restart re-fetches and re-UPSERTs that ledger idempotently (whole-row replacement,
per ADR 0004's merge semantics). Partition-level pre-skip is also supported: if a
partition's clamped range is fully present in the processed set, the partition is not
re-downloaded. Early ledgers (pre-2018) have very few DEX trades and process faster; the
estimate above is conservative.

Single-laptop v1 is deliberate (ADR 0005 §8). Multi-laptop parallelism is feasible but
requires the `assets`-table surrogate-id remap that BE's `db-merge` solves — out of scope
for v1, revisitable as v2 if measured wall-clock proves untenable.

#### Backfill Milestones

| Tranche           | Stream      | Milestone                                                                                  | Validation                                                                                                                                                                                      |
| ----------------- | ----------- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **1** (Week 4)    | Soroban AMM | Full AMM history from Soroban activation (Nov 2023) available                              | `soroban_amm.status: "completed"` in `GET /backfill/status`; OHLCV data for Soroswap pairs verifiable for Nov 2023 dates                                                                        |
| **1** (Week 4)    | SDEX        | First tip-backward chunk (~6 months) processed locally and pushed to Hetzner CH `prices.*` | `sdex.earliest_data_available` ~6 months ago in `GET /backfill/status` (reflects what the most recent push delivered); `sdex.last_push_at` within the last push-cadence window                  |
| **2** (Week 9)    | SDEX        | 4+ years of SDEX history pushed (back to Jan 2022)                                         | `sdex.earliest_data_available` ≤ 2022-01-01 after a fresh push; reviewer spot-checks XLM/USDC OHLCV for 2022 dates                                                                              |
| **3** (Week 13)   | SDEX        | 8+ years of SDEX history pushed (back to Jan 2018)                                         | `sdex.earliest_data_available` ≤ 2018-01-01 after a fresh push; `sdex.last_push_at` within the last push-cadence window; operator reports a credible remaining estimate from local CLI progress |
| **Post-delivery** | SDEX        | Full all-time SDEX history (ledger 1 to present) pushed                                    | `sdex.status: "completed"`; Stellar notified                                                                                                                                                    |

Note (ADR 0005): Stream 2's cloud-side `GET /backfill/status` is fed by the post-backfill
`sdex-cloud-push` step, not by a live Fargate task. The view advances every push cycle. The
operator can inspect the local CLI's freshest progress via a direct SQL query on the local
Postgres at any time, but API consumers see only the most recently pushed state.

#### `GET /backfill/status` Freshness

Per ADR 0005, the SDEX backfill runs locally on the operator's workstation; the cloud
`backfill_progress` row for `sdex_archive` is updated by the `sdex-cloud-push` step, not by
the running CLI. The cloud view therefore advances at push cadence, not CLI cadence.

The relevant freshness signal on `GET /backfill/status` is `sdex.last_push_at` (the timestamp
of the most recent successful push). A CloudWatch alarm fires if `sdex.last_push_at` is older
than the configured push-cadence threshold during the backfill window (e.g. 7 days for
Tranche 1, looser post-delivery as completion approaches). The threshold is operator-tunable
because push cadence is driven by tip-backward chunk size, not by a continuous heartbeat.

A laptop-side staleness check is **not** wired into AWS alarms — workstation uptime is an
operator-managed concern (BE accepts the same trade in BE ADR 0010). Operators inspect local
CLI progress via direct SQL on the local workstation ClickHouse.

---

### 5.7 Venue coverage — the markets we ingest, and why Blend is not one

The SCF RFP's Core Requirements name four markets:

> _"Price Aggregation: Weighted average across major markets (Soroswap,
> Aquarius, SDEX, **Blend**)"_

**We ingest three of the four named markets, plus one the RFP does not name.**
Stated plainly so the count is not something a reader has to reconstruct:

| Venue        | Ingested | Named in the RFP | Path                                           |
| ------------ | -------- | ---------------- | ---------------------------------------------- |
| **SDEX**     | Yes      | Yes              | Ledger-close trades (§5.2), plus §5.6 backfill |
| **Soroswap** | Yes      | Yes              | Soroban AMM swap events (§5.2)                 |
| **Aquarius** | Yes      | Yes              | Soroban AMM swap events (§5.2)                 |
| **Phoenix**  | Yes      | **No**           | Soroban AMM swap events (§5.2)                 |
| **Blend**    | **No**   | Yes              | — see below                                    |

_Table 5.7 — Venue coverage against the RFP's named markets._

#### Why Blend is not a price source

**Blend is a lending protocol, not an exchange.** Users deposit assets to earn
interest or borrow against collateral; there is no swap. **A price is a property
of a trade**, so a lending pool has none to give — this is a mechanical fact
about the protocol, not a scoping preference. From the protocol's own
description ([Meru Wallet case study](https://stellar.org/case-studies/meru-wallet-uses-blend-defi-protocol-for-yield),
read 2026-09-01):

- **Isolated lending pools.** A pool creator sets supported assets, collateral
  requirements, interest rates and utilisation caps. None of these is a traded
  price.
- **Pool creators specify which _oracle_ prices the collateral.** This is the
  decisive fact: **Blend is a consumer of price data, positioned downstream of a
  service like this one** — not a producer of it. Feeding Blend's own numbers
  back into an aggregate would be circular.
- **The backstop module is an 80/20 BLND:USDC AMM.** This is the only part of
  Blend that trades. Its volume is **unmeasured** — we have not assessed whether
  it clears enough to contribute a meaningful price, and no claim is made either
  way.

So the RFP bullet groups a lending protocol with three DEXes under "major
markets". Aggregating it is not something we chose not to do; there is no trade
stream to aggregate.

⚠️ **This is not a rule about lending protocols as a category.** The reasoning
is "no trades, therefore no prices", which happens to cover lending today. It
does not generalise to a claim about protocol types.

#### What would change the answer

Only one technically coherent version of "add Blend" exists: **price the BLND
token from the backstop pool's 80/20 AMM**, treating it as another Soroban AMM
venue. That starts with measuring the backstop pool's volume, and is a feature
in its own right — a `Venue` arm, an extractor, pool-registry seeding and a
historical backfill. It is not in scope here, and this section is not a deferral
of it.

Recorded under [task 0248](../lore/1-tasks/active/0248_DOCS_blend-is-named-in-the-rfp-but-is-not-a-price-source.md).
Whether the headline `price_usd` should even be the weighted cross-venue
aggregate this same RFP bullet describes is a separate question, tracked as
[task 0217](../lore/1-tasks/backlog/0217_FEATURE_decide-whether-price-usd-is-outlier-protected.md).

---

## 6. Performance & Scaling Strategy

### Target: <100ms p95 API response time

| Layer                                    | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **API Gateway caching**                  | Built-in response cache (0.5 GB). Per-endpoint TTLs, as deployed and verified 2026-09-03 (task 0122): `/assets` list 60s, `/assets/{id}` 60s, `/ohlcv` 60s, `/price` **10s**, `/oracles/{id}` 60s, `/backfill/status` **60s**, `/api-docs-json` 3600s. `/health` and POST `/prices/batch` uncached. **The cache key is per-route, not "the query string"**: API Gateway keys only on parameters declared as `cacheKeyParameters`, so `/assets` keys on 7 query params, `/price` on the path plus `min_volume_usd`, `/ohlcv` on the path plus 5, and `/assets/{id}`, `/oracles/{id}` and `/backfill/status` on the path alone. `x-api-key` is in no key, so the cache is shared across callers. Source of truth is `CACHE_TTL` in `infra/src/lib/stacks/api-gateway-stack.ts`, which mirrors the handler tiers in `packages/prices-api/src/common/cache_control.rs` |
| **API Gateway throttling**               | Request throttling (1 req/s sustained, burst 5, 100 000 req/month per self-service key — task 0157; 200 req/s per method stage-wide)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| **Lambda**                               | Rust binary with `lambda_runtime`. Sub-millisecond cold starts. Stateless, auto-scales to concurrency limit. No VPC, so no ENI provisioning latency on cold start                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| **Database client (`clickhouse` crate)** | Warm connection pool reused across Lambda invocations to amortise mTLS handshake (~80-130 ms cross-cloud RTT to Caddy). Per-request payloads batched per-ledger so a typical invocation issues 1–2 INSERTs, not one per trade                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| **Sort key & partitioning**              | Per-granularity tables sorted by `(asset_id, quote_asset_id, source, timestamp)`; monthly partitions on `timestamp`. Partition pruning + sort-key skip eliminate irrelevant months and assets on hot reads                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| **Query optimization**                   | `prices.current_prices` avoids real-time aggregation. OHLCV reads target the granularity table that already holds the requested resolution. Read handlers issue `SELECT … FINAL` or `argMax/argMin + GROUP BY` to handle `ReplacingMergeTree` eventual consistency                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| **Cross-cloud latency mitigation**       | Public-internet hop AWS → Hetzner is ~80-130 ms RTT. Mitigated by (a) warm-container connection reuse, (b) per-ledger write batching, (c) API Gateway response caching for read-heavy endpoints, (d) the API handlers' query patterns favouring single-round-trip CH calls. Single-digit-ms p50 SELECTs over the public hop are routine once the connection is warm; the >100 ms p95 budget remains comfortable                                                                                                                                                                                                                                                                                                                                                                                                                                                    |

### ClickHouse Sizing (shared Hetzner cluster, BE-owned)

The live data plane is BE's production Hetzner ClickHouse cluster (single CH instance on a
single Hetzner box behind Caddy:443). Prices-api joins as a second tenant via its own
`prices` database. Hardware sizing, OS-level tuning, and any vertical/horizontal scaling
decisions are owned by BE. Prices-api's contribution to the box is empirically light: per
task 0046, ~0.45 GB/year flat-growth footprint and a write rate dominated by 1 INSERT
per ledger (~12k INSERTs/day per env at mainnet cadence).

If shared-host capacity becomes contended (the open question task 0047 is verifying), the
fallback is the **Option 4 sidecar CH** from ADR 0007 Alternative 3 — a second CH container
on the same Hetzner box, separate port, separate data volumes, separate Caddy route. Cost
delta: +~€39-69/mo for a second Hetzner tier if a second box becomes preferable. No
prices-api code changes; only the Caddy endpoint and the cert pair would swap.

**No RDS scaling path applies.** The previously-documented `db.t4g.micro → db.r6g.large +
Multi-AZ + read replica + RDS Proxy` escalation ladder is removed; with the live data plane
on ClickHouse, that machinery is not part of the prices-api budget at any traffic level
(ADR 0007 Consequences).

---

## 7. Security Considerations

- **API keys** via API Gateway usage plans — required for all public-facing requests
- **Rate limiting** per API key to prevent abuse
- **Input validation:** Asset identifiers validated against known patterns (G-address: 56 chars
  starting with G, C-address: 56 chars starting with C)
- **Price manipulation protection:**
  - Outlier detection: reject source price data deviating >configurable% from inter-source median
    before including in VWAP
  - Volume-weighted averaging: sources with low 24h volume are down-weighted or excluded via
    `min_volume_usd` threshold
  - Oracle cross-reference: Reflector data available in `/oracles/{asset}` for consumers who
    wish to cross-check; it does not feed primary price fields
- **HTTPS only** (enforced at API Gateway for inbound traffic; mTLS-only for the Lambda →
  ClickHouse hop)
- **No PII stored** — only blockchain-public data
- **IAM least-privilege:** each Lambda role scoped to only the resources it needs. Notably
  `secretsmanager:GetSecretValue` for the per-env mTLS cert + key pair, `sns:Subscribe` /
  Lambda invocation permissions on the BE-owned bucket-fan-out SNS topic, and nothing else
- **mTLS to ClickHouse (ADR 0007 §3.5):**
  - **Endpoint:** Caddy:443 on BE's Hetzner box; Caddy terminates mTLS and proxies to the
    local CH instance
  - **Trust:** BE-managed self-signed CA; prices-api receives per-env client cert + key
    issued via BE's per-AWS-service issuance script
  - **Storage:** `{cert,key,ca}` live in AWS Secrets Manager as a single JSON bundle secret per identity (named by `MTLS_SECRET_NAME`), loaded into the
    Lambda runtime on cold start, held in memory for the container's lifetime
  - **Rotation:** 1-year manual rotation cadence (BE Cluster C agreement); CloudWatch
    alarm on cert NotAfter approaching expiry; re-issuance is a single operator step on
    BE's side, secret rotation is a single CDK deploy on prices-api's side
  - **Revocation:** CA rotation on compromise (BE Cluster C agreement) — drastic but
    operationally simple given the small number of issued certs
- **Network exposure:** Lambdas run outside any VPC. There is **no inbound network surface
  on the prices-api side** beyond API Gateway; the ClickHouse endpoint is BE-managed and
  gated by mTLS at Caddy

---

## 8. Tech Stack Summary

| Component         | Technology                                                                                                                                                                                                                                       |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Language          | Rust (edition 2021)                                                                                                                                                                                                                              |
| Runtime           | AWS Lambda (Rust, custom `provided.al2` runtime via `lambda_runtime` crate) for API + ingestion workers (no VPC); ECS Fargate for shared Galexie (BE-funded); local Rust CLI on operator workstation for both backfill streams (ADRs 0001, 0005) |
| API Framework     | `axum` (HTTP router, shared with Block Explorer backend)                                                                                                                                                                                         |
| XDR parsing       | `stellar-xdr` crate (official SDF Rust XDR types) — shared workspace crate with Block Explorer                                                                                                                                                   |
| ClickHouse client | [`clickhouse`](https://crates.io/crates/clickhouse) Rust crate (async, native protocol over HTTPS-mTLS)                                                                                                                                          |
| Database          | ClickHouse on BE's shared Hetzner cluster (`prices` database, ADR 0007); engines per §3 — `ReplacingMergeTree`, `MergeTree`, materialised views                                                                                                  |
| TLS termination   | BE-managed Caddy:443 reverse proxy; mTLS using BE's self-signed CA + per-env client cert in AWS Secrets Manager                                                                                                                                  |
| Event fan-out     | SNS topic on BE's `stellar-ledger-data/` S3 bucket (one-time BE CDK change; subscribers: BE Ledger Processor + Prices Ledger Processor)                                                                                                          |
| Infrastructure    | AWS CDK (TypeScript) — shared CDK app with Block Explorer stacks; prices-api stacks deploy no VPC/RDS/NAT                                                                                                                                        |
| CI/CD             | GitHub Actions → `cdk deploy` — shared pipeline with Block Explorer                                                                                                                                                                              |
| Monitoring        | CloudWatch Logs + Metrics + Alarms + X-Ray tracing; mTLS cert NotAfter alarm; ingestion-lag alarm                                                                                                                                                |
| API Docs          | OpenAPI 3.0 spec, auto-generated from axum routes; the API reference (Swagger UI's shape, the portal's design system) is a route of the onboarding portal (`/api/docs`) on the block explorer's CloudFront                                       |

**Shared with Block Explorer codebase (same Rust workspace):**

- `stellar-xdr` parsing logic — compiled into both the Block Explorer Ledger Processor and the
  Prices Ledger Processor from the same workspace crate
- CloudWatch metric and alarm patterns
- CDK stack patterns (IAM, Lambda configuration, Secrets Manager material)
- ClickHouse client config patterns (mTLS material loading, connection reuse)

---

## 9. Delivery Plan — Three Tranches

### Tranche 1 — Infrastructure & Real-time Ingestion (Weeks 1–4)

**Work:**

- AWS CDK stack provisioned: Prices API Lambda execution roles (no VPC), API Gateway,
  EventBridge rules, CloudWatch alarms, Secrets Manager entries (including per-env mTLS
  cert + key pair for Caddy:443)
- BE-side prep (one-time): SNS topic added to BE's `stellar-ledger-data/` bucket fan-out;
  per-env client cert issued from BE's CA for the prices-api Lambda; prices-api's `prices`
  database + user + quota provisioned inside the shared Hetzner ClickHouse cluster
- `prices.*` schema applied on the Hetzner CH cluster: all tables from Section 3
  (`assets`, `price_ohlcv_1m` and the rolled-up granularity tables, MV chain,
  `current_prices`, `oracle_prices`, `backfill_progress`)
- Prices Ledger Processor Lambda deployed and subscribed to the SNS fan-out topic;
  confirmed processing live ledgers (decoded XDR → INSERT into
  `prices.price_ohlcv_1m` over HTTPS-mTLS)
- Asset Discovery Lambda running; `prices.assets` populated for at least 20 major assets
- Local SDEX backfill CLI (`sdex-backfill`, ADR 0005) operating on the operator's
  workstation against `s3://aws-public-blockchain`, decoding historical SDEX trades and
  writing them **directly to Hetzner ClickHouse `prices.*` over the 0052 mTLS client**
  (ADR 0009 — no local mirror, no separate `sdex-cloud-push` step)
- Soroban AMM Stream 1 fully delivered (ADR 0001): the `soroban-amm-backfill` Rust CLI
  reads BE's `soroban_events`, extracts Soroswap/Aquarius/Phoenix swaps (ScVal decoded via
  `stellar-xdr`), buckets them to per-source 1-min rows, and **writes directly to Hetzner
  ClickHouse `prices.*` over the same mTLS path the live Lambda uses** (ADR 0009). The asset
  registry is loaded from the target at run start so surrogate ids align with the live path;
  there is no intermediate local ClickHouse mirror or completion push
- `GET /backfill/status` endpoint live and returning valid progress data
- CloudWatch alarms: `sdex.last_push_at` older than the Tranche 1 push-cadence threshold
  (e.g. 7 days) → SNS notification; mTLS cert NotAfter < 30 days → SNS notification

**Acceptance criteria:**

1. `cdk deploy` from a clean AWS account produces the full Prices API stack with no manual
   steps. The CDK app has **no RDS, no VPC, no NAT Gateway** in its synth output; secrets for
   the mTLS material are present and IAM allows `secretsmanager:GetSecretValue` for them
2. `prices.*` schema on Hetzner CH matches Section 3 (verifiable via `clickhouse-client
--query "SHOW TABLES FROM prices"` and `SHOW CREATE TABLE prices.price_ohlcv_1m`
   etc., issued by the operator over mTLS)
3. After 24 hours of live operation: `prices.price_ohlcv_1m` contains continuous 1-min
   per-source rows for at least 20 major assets (XLM, USDC, EURC, AQUA, BTC, ETH) with
   no gaps >2 candles (verified via `FINAL` SELECT against the table)
4. `GET /backfill/status` returns `sdex.status: "running"`, `sdex.last_push_at` within the
   configured Tranche 1 push-cadence window, and `sdex.current_ledger` decreasing across
   successive pushes (tip-backward direction). `soroban_amm.status` is `"running"` early in
   Tranche 1 and transitions to `"completed"` once the AMM stream finishes (see Section 4.5
   for the canonical response shape)
5. CloudWatch alarm test: let a scheduled backfill write cycle lapse → freshness alarm fires
   once `sdex.last_push_at` exceeds the configured Tranche 1 threshold. (Tool name
   corrected per ADR 0009 — there is no `sdex-cloud-push` step; `last_push_at` is
   the timestamp of the CLI's most recent **direct write** to Hetzner. The test
   itself is unchanged: it exercises the freshness alarm, not the tool.)
6. `sdex.earliest_data_available` in `GET /backfill/status` shows a date approximately 6 months ago

**Budget: $XX,XXX (Tranche 1)**

---

### Tranche 2 — Public API (Weeks 5–9)

**Work:**

- Core API endpoints implemented and deployed:
  - `GET /assets` (paginated, sortable, filterable)
  - `GET /assets/{asset_identifier}` (single asset)
  - `GET /assets/{asset_identifier}/price` (current price with `sources` breakdown)
  - `GET /assets/{asset_identifier}/ohlcv` (OHLCV with timeframe/granularity params, with `backfill_note` when history is partial)
  - `POST /prices/batch` (multi-asset current price)
  - `GET /oracles/{asset_identifier}` (Reflector cross-reference data)
  - `GET /backfill/status`
- API Gateway: response caching (TTLs per Section 6), usage plans, API key issuance, throttling
- Full VWAP formula wired into Current Price Updater Lambda (Section 5.5)
- Outlier detection: sources deviating >configurable% from inter-source median excluded
- Aquarius pool metadata integration: Aquarius appearing as a named source in VWAP
- Input validation: asset identifier format enforced, param ranges validated, 400 on invalid input

**Backfill milestone for Tranche 2:**
By the end of Week 9, the operator has run additional tip-backward `sdex-backfill` chunks —
writing **directly to Hetzner `prices.*` over mTLS** (ADR 0009; there is no separate
`sdex-cloud-push` step) — covering approximately **January 2022 to present** (4+ years of
SDEX history, including all of the Soroban era plus 2 years of pre-Soroban SDEX data). The
covered range and freshness are visible via `GET /backfill/status` (`sdex.earliest_data_available`,
`sdex.last_push_at`). Local CLI per-ledger rate (~311 ledgers/s per task 0022) is well above
what is required for this coverage given the workstation uptime through Tranche 2; see §5.6
for the full local-CLI metrics.

**Acceptance criteria:**

1. All 7 endpoint groups return correct, schema-valid responses for at least 20 major assets
2. Load test (k6 or Locust, script provided): 100 req/s sustained for 5 minutes on
   `GET /assets/{id}/price` → p95 latency <200ms, error rate <0.1%
   — the target is unchanged, but since task 0157 no key in the account can sustain
   it: the default plan is 1 req/s (§6). The run needs a usage plan created for it,
   per `docs/runbooks/manual-api-key-tier.md`; the report must state which plan the
   key was on (task 0121)
3. Cache confirmed: consecutive identical requests within TTL window return `X-Cache: Hit` header
4. VWAP calculation verifiable against raw `price_ohlcv` rows for at least 3 assets
5. `GET /backfill/status` shows `earliest_data_available` ≤ 2022-01-01
6. OHLCV data for `?timeframe=all` on USDC returns data points from at least January 2022,
   with correct 1d candles verifiable against known USDC price history (spot-check dates
   provided by reviewer)

**Budget: $XX,XXX (Tranche 2)**

---

### Tranche 3 — Production Launch & Validation (Weeks 10–13)

**Work:**

- OpenAPI 3.0 specification covering all endpoints
- Self-service onboarding portal (S3 + CloudFront): API key request form, quickstart guide,
  example queries
- Integration test suite (automated, runs in CI): covers all 7 endpoint groups
- Load test report: k6 or Locust, documented test plan, results at 100/s, 500/s, 1000/s
- Security review checklist: IAM least-privilege, no secrets in env vars, input sanitization
- X-Ray tracing enabled end-to-end
- CloudWatch dashboards: API latency, error rate, ingestion lag, ClickHouse write latency, mTLS cert NotAfter, backfill progress
- GitHub repository made public with README, architecture docs, deploy instructions

**Backfill milestone for Tranche 3:**
By the end of Week 13, the operator has run additional tip-backward `sdex-backfill` chunks —
direct-write per ADR 0009, as above — covering approximately **January 2018 to present**
(8+ years of SDEX history). Per ADR 0005 §9, full historical completion (ledger 1 to current tip) is
**not** a Tranche 3 deliverable — the operator continues pushing older ranges in the
background post-delivery.

**The Tranche 3 review validates that backfill is progressing correctly, not that it is complete.**
The reviewer should confirm (against the response shape in Section 4.5):

- `GET /backfill/status` returns `sdex.status: "running"` and `sdex.last_push_at` is fresh
  (within the Tranche 3 push-cadence window)
- `sdex.earliest_data_available` ≤ 2018-01-01
- `sdex.current_ledger` is strictly decreasing across successive `GET /backfill/status`
  observations (visible as more pushes complete during the review window)
- `soroban_amm.status` is `"completed"` (carried over from Tranche 1)
- OHLCV data for `?timeframe=all` on XLM returns data points from 2018 or earlier
- Operator narrates a credible remaining estimate from local CLI progress (not exposed
  through the cloud API — see §5.6 freshness subsection)

The local backfill continues post-delivery. When the final tip-backward chunk is pushed and
`sdex.status` transitions to `"completed"`, the `GET /backfill/status` endpoint records
`sdex.completed_at`. The team will share a link to this endpoint with Stellar for
post-delivery monitoring.

**Acceptance criteria:**

1. `GET /backfill/status` shows `sdex.status: "running"`, `sdex.last_push_at` within the
   Tranche 3 push-cadence window, and `sdex.earliest_data_available` ≤ 2018-01-01
2. OpenAPI spec passes `openapi-validator` lint with no errors; Swagger UI deployed
3. Onboarding portal accessible; self-service API key request flow functional
4. Integration test suite: all tests pass on CI (GitHub Actions link provided)
5. Load test report: p95 <100ms at 100 req/s confirmed — same caveat as Tranche 2
   AC 2: a purpose-built usage plan is required, and the report names it
6. Security checklist signed off: no wildcard IAM, ClickHouse endpoint reachable only via
   mTLS through Caddy:443, mTLS cert + key in Secrets Manager (not env vars), all inputs
   validated
7. GitHub repository public; `cdk deploy` from README works in a fresh AWS account
8. CloudWatch dashboard accessible to Stellar team (read-only IAM role); all alarms OK
9. 7-day post-launch monitoring report: uptime %, error rate, p95 latency, SDEX push
   cadence and `earliest_data_available` trajectory

**Budget: $XX,XXX (Tranche 3)**

---

## 10. Cost Estimate (monthly)

### Monthly Running Cost (low traffic, post-backfill)

| Service                                                                      | Estimated Cost       | Notes                                                                                                                                                                                                                                          |
| ---------------------------------------------------------------------------- | -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hetzner ClickHouse share (`prices` DB on BE's cluster)                       | ~$1–$2               | Cost-share with BE per task 0046's empirical ~0.45 GB/yr, ~74 bytes/ledger, 14.8× compression — opening proposal ~1-2% pro-rata. D12 commercial follow-up; flat fee acceptable up to ~$5/env per the brief without changing the recommendation |
| ECS Fargate — Galexie                                                        | **$0**               | Shared with Block Explorer (see Section 2.3)                                                                                                                                                                                                   |
| SNS topic on `stellar-ledger-data/`                                          | ~$1                  | Per-message + delivery cost across both subscribers; negligible at ledger cadence                                                                                                                                                              |
| Lambda — API handlers                                                        | ~$20                 | Rust binaries, sub-ms cold starts; no VPC, so no ENI provisioning latency                                                                                                                                                                      |
| Lambda — Ingestion workers (~500K invocations)                               | ~$10                 |                                                                                                                                                                                                                                                |
| API Gateway (10M requests + 0.5 GB cache)                                    | ~$50                 |                                                                                                                                                                                                                                                |
| CloudWatch + X-Ray                                                           | ~$20                 |                                                                                                                                                                                                                                                |
| Secrets Manager (mTLS material × 2 per env + a few API keys) + S3 (API docs) | ~$6                  |                                                                                                                                                                                                                                                |
| **Total (low traffic, post-backfill)**                                       | **~$108/mo per env** | Down from ~$117/mo in the RDS-shaped prior design; down ~$70/mo from the original pre-shared-infra estimate                                                                                                                                    |

### Backfill Period Additional Costs (one-time, during 13-week project)

Per ADRs 0001 and 0005, **both** historical backfill streams run as local Rust CLIs on the
operator's workstation, not as continuous ECS Fargate tasks. The Hetzner CH cluster sees
only bursty write traffic: the Stream 2 `sdex-backfill` CLI's tip-backward chunks and the
Stream 1 `soroban-amm-backfill` one-shot run, both writing **directly** over mTLS
(ADR 0009 — the `sdex-cloud-push` step this paragraph described no longer exists; the
burst profile is unchanged, only its source). AWS-billed line items are
essentially unchanged from steady-state since the writes happen against Hetzner, not
AWS-side storage. Workstation electricity, ISP bandwidth, and local ClickHouse disk for
the Stream 1 prep step are operator-paid and outside this table.

| Item                                             | Configuration                                                                                                                                                     | One-time Cost   |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| ECS Fargate — Soroban AMM backfill task          | — (no Fargate per ADR 0001)                                                                                                                                       | **$0**          |
| ECS Fargate — SDEX archive backfill task         | — (no Fargate per ADR 0005)                                                                                                                                       | **$0**          |
| Cloud DB during push windows                     | No RDS (ADR 0007); the bursty pushes hit Hetzner CH instead — no extra AWS line item. The previously-budgeted `db.t4g.small` upgrade window is removed            | **$0**          |
| Hetzner CH cost-share during backfill            | Empirically the backfill spike adds <1 GB to the box's footprint (task 0046) — does not move the cost-share number. D12 follow-up may revise upward once measured | **$0** marginal |
| S3 archive reads (Stellar public history)        | Anonymous reads via `--no-sign-request` against `s3://aws-public-blockchain` — not billed to the Prices API AWS account. Applies to both streams                  | **$0**          |
| **Total one-time backfill compute (AWS-billed)** |                                                                                                                                                                   | **~$0**         |

Compared with the prior ADR 0002 / Fargate-era estimate of ~$636 (Stream 2) plus the legacy
Stream 1 Fargate line, the local-workstation pattern across both streams (already costed at
~$30 in the RDS-shaped design) collapses further to ~$0 marginal AWS-billed cost under
ADR 0007. The trade is operator workstation uptime, accepted as a deliberate design choice
(ADR 0005 §"Negative" and ADR 0001 §Consequences).

### Scaled Up (high traffic)

| Service                                                         | Added Cost                                        |
| --------------------------------------------------------------- | ------------------------------------------------- |
| Hetzner CH cost-share re-opened if production scales materially | +~$3-15/env (per task 0045 D12 escalation clause) |
| Add Lambda provisioned concurrency                              | +~$45                                             |
| **Total at scale**                                              | **~$156-168/mo per env**                          |

The previously-documented `db.r6g.large + Multi-AZ + read replica + RDS Proxy` escalation
(~$525/mo additional in the prior design) is **eliminated** under ADR 0007 — at any
realistic prices-api traffic level, the shared Hetzner CH absorbs the read/write load. If
task 0047's throughput verification returns RED, the sidecar-CH fallback adds
~€39-69/mo for a dedicated Hetzner box (one box covers all 3 envs) — still substantially
below the prior RDS scaling ladder.

---

## 11. Infrastructure Sharing with Soroban Block Explorer

This section explicitly enumerates all shared infrastructure to confirm there is no
double-billing between the two grants.

### 11.1 Shared Components (Block Explorer funded, Prices API does not charge)

| Component                                                                 | Saving vs. standalone Prices API                         | How sharing works                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ------------------------------------------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Galexie ECS Fargate task                                                  | ~$36/mo                                                  | One Galexie serves both services                                                                                                                                                                                                                                                                                                                                                                                                            |
| S3 bucket `stellar-ledger-data/` + SNS fan-out topic                      | ~$2/mo                                                   | Same files read by both Lambdas; SNS fan-out replaces the previously-direct Lambda target so adding/removing tenants is a subscription change, not a bucket-config change                                                                                                                                                                                                                                                                   |
| Hetzner ClickHouse data plane (`prices` database tenant on BE's box)      | ~$11–$12/mo (avoided RDS) net of ~$1–$2 cost-share to BE | Prices-api stores all live OHLCV / current-prices / oracle / asset registry / backfill-progress in a separate `prices` database, isolated via ClickHouse's native multi-tenant primitives. Cost-share opening proposal ~1-2% pro-rata per task 0046's empirical sizing; basis lives in [agreement record](../lore/1-tasks/archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md) (Cluster D) |
| mTLS Certificate Authority + per-AWS-service issuance script (BE-managed) | ~$0 (avoids prices-api running its own CA)               | One client cert per env, issued by BE; prices-api stores the keypair in AWS Secrets Manager                                                                                                                                                                                                                                                                                                                                                 |
| **Monthly saving**                                                        | **~$49/mo**                                              | Total saving comparable to the prior design once the avoided RDS line is netted against the new CH cost-share; the per-env baseline is **~$108/mo** (see §10)                                                                                                                                                                                                                                                                               |

**Components no longer shared (and no longer needed).** Two rows present in earlier
versions of this table are gone — not because BE stopped offering them, but because the
prices-api architecture no longer requires them:

- **VPC (subnets, security groups, route tables)** — prices-api Lambdas run outside any
  VPC (ADR 0007 §3.6). Joining BE's VPC saved nothing meaningful before; not joining it
  removes the cross-service coupling on networking.
- **NAT Gateway** — same reason: no VPC means no egress through a NAT Gateway. The
  ~$35/mo saving previously attributed to sharing BE's NAT has shifted into the
  no-VPC-at-all column.

**Components removed from earlier history** (kept here for traceability):

1. An **"ECS Fargate cluster"** row claiming Prices API backfill tasks ran in BE's shared
   cluster. Per [ADR 0005](../lore/2-adrs/0005_stream2-sdex-local-workstation-backfill.md)
   and [ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md), both
   historical backfill streams run as local workstation CLIs.

2. A **"Block Explorer `soroban_events` table (read-only)"** row claiming the Soroban AMM
   backfill queried BE's RDS at runtime. Per
   [ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md), Stream 1
   consumes a local CH instance populated by `backfill-runner --target=clickhouse` —
   captured in §11.2.

### 11.2 Development Savings

| Artifact                                                                                        | Shared from Block Explorer                                                                                                                               | Saving                                                                                                                                                                                                           |
| ----------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `stellar-xdr` Rust parsing crate                                                                | Written once, compiled into both Ledger Processors and the `soroban-amm-backfill` CLI (ScVal decoding for ADR 0001)                                      | ~5–7 dev days of XDR parsing logic not duplicated                                                                                                                                                                |
| `backfill-runner --target=clickhouse` (BE task 0205)                                            | One-shot prep tool invoked on the operator's workstation to populate a local CH copy of `soroban_events`; consumed as-is                                 | ~3–5 dev days vs. building a prices-api-side `LedgerCloseMeta → CH` writer for the Stream 1 backfill window                                                                                                      |
| BE production ClickHouse schema (`docs/database-schema/clickhouse-prod-schema.sql`, mirrors BE) | DDL adopted unchanged for the local Stream 1 CH instance; also reused as the engine + sort-key reference for the prices-api `prices.*` schema (ADR 0007) | ~1–2 dev days of schema design + indexing decisions not duplicated                                                                                                                                               |
| BE-managed mTLS CA + per-AWS-service issuance script                                            | Per-env client cert issued via BE's existing tooling; prices-api avoids building its own CA / issuance pipeline                                          | ~2–3 dev days plus ongoing CA-operations savings                                                                                                                                                                 |
| Hetzner CH operational tooling (Caddy config, backup Borg job, OS-level tuning)                 | BE owns the box; prices-api inherits the operational primitives without building them                                                                    | ~5–10 dev days (one-time) + ongoing ops time not spent on CH operation                                                                                                                                           |
| BE Cluster A/B/C/D agreement record (task 0045)                                                 | Single written cross-team contract for tenancy, capacity, auth, cost-share                                                                               | Avoids ad-hoc renegotiation per implementation task; lives at [`G-be-agreement-record.md`](../lore/1-tasks/archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md) |
| ClickHouse client config patterns (mTLS material loading, connection reuse)                     | Copy-adapted from BE's own Lambda → CH writers                                                                                                           | ~1–2 dev days                                                                                                                                                                                                    |
| CDK IAM + Secrets Manager patterns                                                              | Reused CDK constructs (no VPC / NAT patterns needed)                                                                                                     | ~2–3 dev days                                                                                                                                                                                                    |
| Observability configuration (CloudWatch dashboards, alarm patterns)                             | Copy-adapted from Block Explorer; includes mTLS NotAfter alarm pattern                                                                                   | ~1–2 dev days                                                                                                                                                                                                    |

### 11.3 What Is Not Shared

The following components are **separate** and funded exclusively by the Prices API grant:

- Prices API Lambda functions (separate function definitions, separate IAM roles, no VPC)
- Prices API API Gateway + usage plans + response cache
- Prices API EventBridge rules
- Prices API Secrets Manager entries (per-env mTLS material + a few external-API keys)
- Prices API `prices.*` schema + migrations on the shared Hetzner CH cluster
- Prices API onboarding portal (S3 + CloudFront)

### 11.4 Cross-Service Dependency and Risk

Per [ADR 0007](../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md), the
prices-api **live data plane** runs as a tenant inside BE's Hetzner ClickHouse cluster.
The two services share a host and an mTLS-fronted endpoint, but logically each owns its
own database (`prices` vs `default`) and its own schema migrations. Per
[ADR 0001](../lore/2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md), the
Soroban AMM backfill consumes a **local** ClickHouse instance on the operator's
workstation populated by `backfill-runner --target=clickhouse` — separate from the
production Hetzner CH.

| Risk                                                                                      | Mitigation                                                                                                                                                                                                                                   |
| ----------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cross-tenant throughput contention on the shared Hetzner CH                               | **Task 0047** verifies cross-tenant throughput (GREEN/YELLOW/RED) before implementation begins. RED supersedes ADR 0007 to Alternative 3 (sidecar CH on the same Hetzner box — same code shape, different host).                             |
| BE evolves `default.*` schema in ways that conflict with prices-api views                 | ADR 0007 §3.7: prices-api owns `prices.*` unilaterally; any cross-DB read into `default.*` is wrapped in a named `prices.*` view so the breakage surface is narrow and reviewable.                                                           |
| Cross-cloud network outage (AWS ↔ Hetzner)                                                | Lambdas tolerate connect failures with exponential-backoff retry; SNS delivery retries the message; BE's S3 retention (indefinite per BE ADR 0006) supports replay of arbitrary windows by re-firing PutObject events. No data loss path.    |
| mTLS cert expiry not detected                                                             | CloudWatch NotAfter alarm fires 30 days before expiry; 1-year manual rotation cadence (Cluster C agreement); revocation = CA rotation.                                                                                                       |
| Hetzner box backup RPO is daily Borg, not RDS PITR                                        | Accepted in ADR 0007 §Negative. OHLCV data has natural replay path from BE's S3 archive; daily-granularity restore is operationally acceptable given the reconstruction cost.                                                                |
| BE's `backfill-runner` produces incorrect or incomplete rows in the **local** Stream 1 CH | Gap detection: after the AMM CLI's cloud push, prices-api checks for contiguous OHLCV coverage from Soroban activation to present. Any gaps trigger a targeted archive-read for the missing ledger ranges.                                   |
| BE's `backfill-runner` evolves between the prep step and the AMM CLI run                  | Pin a known-good `backfill-runner` version for the duration of the workstation prep. Re-pin and re-populate the local instance if BE ships an incompatible change.                                                                           |
| BE's `backfill-runner` is unavailable                                                     | Stream 1 is gated on operator workstation; a delay of a day or two is recoverable within Tranche 1's window. Fargate-based fallback documented in [task 0017](../lore/1-tasks/backlog/0017_FEATURE_local-clickhouse-for-prices-backfill.md). |

The Prices API never writes to BE's `default.*` schema; BE never reads from `prices.*`.
The runtime coupling is exactly the shared-host + shared-Caddy endpoint, gated by mTLS,
with strict per-database isolation via ClickHouse's native multi-tenant primitives.

**Stream 2 (SDEX) coupling.** Per ADR 0005, the SDEX historical backfill has **zero**
runtime or data coupling with the Block Explorer at the backfill layer. The only BE
artefact consumed is the `xdr-parser` crate, pinned as a git Cargo library dependency
and compiled read-only into the `sdex-backfill` binary on the operator's workstation. The
CLI writes rows directly into the shared Hetzner CH as it decodes (ADR 0009, replacing the
`sdex-cloud-push` step this paragraph described) — that hop introduces the same shared-host
coupling as the live path, governed by the same Cluster A/B/C/D agreements.
