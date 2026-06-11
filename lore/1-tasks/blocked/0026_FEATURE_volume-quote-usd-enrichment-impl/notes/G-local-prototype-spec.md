---
id: "G-local-prototype-spec"
title: "Local-prototype scope + BE cross-team contract for the volume_quote_usd enrichment Lambda"
type: G
task: "0026"
status: developing
spawned_from: []
spawns: []
related_notes: []
links:
  - "../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md"
  - "../../../blocked/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
---

# Local-prototype spec + BE cross-team contract

> **Audience:** prices-api implementer (Part A), BE team reviewers (Part C).
> **Status:** draft for cross-team discussion.
> **Why this note exists:** task 0026's 2026-06-08 activation history
> entry promised a "local-only Rust crate + design document" deliverable
> mirroring task 0038's shape. The original 0024 design spec was
> authored against an RDS-Postgres data plane that ADR 0007
> (accepted 2026-05-20) supersedes — the PG-flavoured SQL needs
> translation to ClickHouse semantics. This document is that
> translation, plus the cross-team contract.

---

## 0. TL;DR

We are building a **local-only** Rust binary that exercises the
volume_quote_usd enrichment algorithm against fixture data: read
`price_ohlcv` candidates needing enrichment, ASOF-join the latest
`oracle_prices.price_usd` for the candidate's quote asset, multiply
`oracle_price * volume_quote`, emit enriched rows through a stub
sink (stdout JSON / SQL-file). It does NOT deploy to AWS, does
NOT register an EventBridge / Scheduler rule, does NOT connect to
the Hetzner ClickHouse.

The prototype's value is twofold:

1. **De-risk the CH translation.** The 0024 design spec's
   PG-flavoured algorithm (CTE + `FOR UPDATE SKIP LOCKED` +
   `UPDATE … FROM … RETURNING`) does not translate row-for-row to
   ClickHouse. CH has no row locks, asynchronous `ALTER … UPDATE`
   mutations, and the idiomatic enrichment pattern is **INSERT
   newer versions into a `ReplacingMergeTree`**. The prototype
   proves the idempotency and concurrency story under that pattern
   before any production code commits.
2. **Ground the BE meeting.** Part C below is the concrete list of
   cross-team commitments the production Lambda needs. Bundled with
   task 0038's parallel asks so BE answers both in one
   conversation.

When the gating events clear (BE 0227 ships the Hetzner mTLS
endpoint; task 0047 verifies cross-tenant throughput; task 0012
sets up the prices-api CDK + RDS or ClickHouse schema migration
tooling), the prototype's interior is reused; only the
candidate-source, the oracle-source, the sink, and the CDK
packaging change.

---

## 1. Reference: BE indexer + task 0038's prototype

Patterns we inherit from the same pool 0038 drew on
(`soroban-block-explorer/crates/indexer/`,
`crates/db-clickhouse/`):

- **mTLS to Hetzner CH** — `db_clickhouse::mtls::client_from_lambda_env`,
  same env-var contract (`MTLS_SECRET_NAME`, `CH_DOMAIN`, `ENV_NAME`).
- **Cold-start eager init** — surface missing env / unreachable
  Parameters and Secrets Extension as Lambda Init Errors rather
  than per-event panics.
- **Structured JSON tracing** with `EnvFilter::from_default_env()`
  on `RUST_LOG`.
- **Safe-error redaction** — never stringify upstream CH
  `BadResponse` bodies (can echo row data). Reuse the
  `safe_response_token` from 0038's `prices_ledger_processor::safe_log`.
- **Retry-with-backoff** `[50, 200, 800] ms` envelope for transient
  CH errors. Reuse 0038's `prices_ledger_processor::retry::retry_with_backoff`.

Patterns we do NOT inherit:

- **S3 → SQS doorbell trigger.** This Lambda runs on
  EventBridge Scheduler, not S3 events. No SQS queue, no
  doorbell-cursor reconcile.
- **`reservedConcurrentExecutions = 1` for ordering.** Concurrency
  is still 1 here, but for **idempotency simplicity**
  (single-writer enrichment) not ordering — concurrent UPDATEs are
  fine semantically since the `WHERE volume_quote_usd = 0` filter
  is the idempotency gate; we pin to 1 only so the CloudWatch
  per-invocation metrics aren't fragmented.
- **Galexie key derivation.** No S3 reads at all.

---

## Part A — Local prototype scope

### A.1 What the binary does

A single Rust binary, `enrichment-worker`, that on each invocation:

1. **Reads a batch of candidate rows** via a `CandidateSource`
   trait. In prototype mode, that's a JSON-lines file
   (`fixtures/candidates.jsonl`); in production it's a CH query
   against `prices.price_ohlcv FINAL WHERE volume_quote_usd = 0`
   (idempotency gate).
2. **Looks up the oracle price** for each candidate's
   `quote_asset_id` via an `OraclePriceLookup` trait. Prototype
   uses an in-memory map loaded from `fixtures/oracle_prices.jsonl`;
   production uses an ASOF-style CH query against
   `prices.oracle_prices`. Forward-fill window is configurable
   (default 300 s).
3. **Computes the enriched `volume_quote_usd`** as
   `oracle.price_usd × candidate.volume_quote`. Uses
   `rust_decimal::Decimal` to preserve the integer-quote-asset-units
   precision the writers and CH `Decimal(36, 18)` columns require.
4. **Classifies the outcome** per row:
   - Hit (oracle found within window) → produce an `EnrichedRow`.
   - Miss (no oracle in window) → drop, increment
     `EnrichmentOracleMiss` metric, row stays at 0 for the next
     pass.
5. **Writes enriched rows** through an `EnrichmentSink` trait —
   prototype emits stdout JSON-lines / SQL `INSERT INTO
   prices.price_ohlcv …` per row; production runs a single
   batched INSERT through the mTLS-backed `clickhouse::Client`.
6. **Emits metrics** — `EnrichmentRowsEnriched`,
   `EnrichmentOracleMiss`, `EnrichmentRowsRemainingAtVolumeZero`,
   `EnrichmentBatchDurationMs` (per 0024 §5). Prototype dumps to
   stdout JSON; production publishes via `aws_sdk_cloudwatch`.
7. **Caps work per invocation** at
   `MAX_BATCHES × BATCH_SIZE` rows (default 20 × 10 000 =
   200 000). Anything over is left for the next hourly run.

### A.2 PG → CH algorithm translation (the non-trivial part)

The 0024 G-note §2 reads candidates and runs an `UPDATE … FROM …
RETURNING` against PG, lock-bounded by `FOR UPDATE SKIP LOCKED`.
None of that idiom exists in CH.

**Two translation candidates considered:**

**Option 1 — `ALTER TABLE … UPDATE` mutation.**
```sql
ALTER TABLE prices.price_ohlcv
UPDATE volume_quote_usd = (
    SELECT price_usd FROM prices.oracle_prices
     WHERE asset_id = price_ohlcv.quote_asset_id
       AND oracle_name = {oracle_name:String}
       AND timestamp <= price_ohlcv.timestamp
       AND timestamp >  price_ohlcv.timestamp - INTERVAL {window_s:UInt32} SECOND
     ORDER BY timestamp DESC LIMIT 1
) * volume_quote
WHERE volume_quote_usd = 0 AND volume_quote > 0
  AND timestamp >= now() - INTERVAL 30 DAY;
```
- ✗ Asynchronous mutation. Returns immediately; the actual rewrite
  runs in background. `system.mutations` monitoring required.
- ✗ Rewrites the entire matched-part data files; expensive on a
  table that takes incremental UPSERTs.
- ✗ Concurrent ALTER UPDATEs queue and serialise in MergeTree,
  but error semantics on conflict are fuzzy. The PG-flavoured
  `FOR UPDATE SKIP LOCKED` has no CH equivalent here.

**Option 2 — INSERT newer versions into ReplacingMergeTree
(recommended).**

Per ADR 0007 Cluster A and the BE pattern of `ReplacingMergeTree`
(used in BE's 17 RMT tables — see `crates/db-clickhouse/schema/`),
`prices.price_ohlcv` is a ReplacingMergeTree with `ORDER BY
(timestamp, asset_id, granularity, quote_asset_id, source)`
(per ADR 0003 PK shape + ADR 0004 source column). A version
column — `_inserted_at: DateTime DEFAULT now()` — picks the
winner on background merge.

The enrichment Lambda then:

```sql
-- 1) Materialise enriched rows into a staging table.
INSERT INTO prices.price_ohlcv_enrichment_staging
SELECT
    p.timestamp,
    p.asset_id,
    p.granularity,
    p.quote_asset_id,
    p.source,
    p.open, p.high, p.low, p.close,
    p.volume_base,
    p.volume_quote,
    o.price_usd * p.volume_quote AS volume_quote_usd,
    p.trade_count, p.vwap_numerator, p.vwap_denominator,
    now()                       AS _inserted_at
FROM prices.price_ohlcv FINAL p
ASOF LEFT JOIN prices.oracle_prices o
    ON o.asset_id     = p.quote_asset_id
   AND o.oracle_name  = {oracle_name:String}
   AND o.timestamp   <= p.timestamp
WHERE p.volume_quote_usd = 0
  AND p.volume_quote > 0
  AND p.timestamp >= now() - INTERVAL 30 DAY
  AND o.price_usd IS NOT NULL
  AND p.timestamp - o.timestamp <= INTERVAL {window_s:UInt32} SECOND
LIMIT {batch_size:UInt32};

-- 2) Promote staging into the live table. Inserted rows have a
--    higher _inserted_at than the existing 0-value rows; the
--    ReplacingMergeTree picks the higher version on next merge.
INSERT INTO prices.price_ohlcv
SELECT * EXCEPT (_inserted_at), _inserted_at
FROM prices.price_ohlcv_enrichment_staging
WHERE _inserted_at = (SELECT max(_inserted_at) FROM prices.price_ohlcv_enrichment_staging);

-- 3) Truncate staging (or partition + DROP PARTITION per invocation).
TRUNCATE TABLE prices.price_ohlcv_enrichment_staging;
```

Pros:

- ✓ Idempotency comes from the **WHERE filter on read** (`FINAL` +
  `volume_quote_usd = 0`). Re-running picks no rows.
- ✓ Concurrency-safe: two enrichment passes that pick disjoint
  candidate batches (extremely likely given hourly cadence vs row
  density) produce non-overlapping INSERTs. Two passes that DO
  pick overlapping rows produce two equivalent enriched rows; the
  RMT collapses to one on merge.
- ✓ No async mutation queue. No `system.mutations` to monitor.
- ✓ Failure mode: a partial Lambda crash before step (2) leaves
  the staging table dirty but `prices.price_ohlcv` unchanged.
  Next invocation TRUNCATEs and retries.

Cons:

- ✗ Requires `price_ohlcv_*` to be a ReplacingMergeTree with a
  version column. **C.4 RESOLVED (2026-06-09): confirmed
  `ReplacingMergeTree(version)`.** The version column is the
  ledger-derived `version UInt64` (not the `_inserted_at` wall
  clock this note originally assumed) — see the Decision Log at
  the end of this note for how that changed the implementation.
- ✗ `SELECT FINAL` is required on read paths to merge in real
  time (background merges happen asynchronously); reads pay a
  small cost. Acceptable per task 0046's empirical numbers.

**Recommendation:** Option 2. The prototype hard-codes this
shape; the Part C.4 ask confirms it lands in production.

### A.3 Workspace placement

```
packages/
├── extractors-core/              # existing
├── ledger-processor/             # existing
├── phoenix-extractor/             # existing
├── soroswap-extractor/            # existing
├── aquarius-extractor/            # existing
├── sdex-backfill/                 # existing
├── prices-ledger-processor/        # 0038's prototype
└── enrichment-worker/             # NEW — this prototype
    ├── Cargo.toml
    ├── src/
    │   ├── main.rs               # Lambda entrypoint (EventBridge schedule event)
    │   ├── enrich.rs              # core algorithm + types
    │   ├── candidates/            # input abstraction
    │   │   ├── mod.rs            # trait CandidateSource
    │   │   └── jsonl_file.rs      # fixtures/candidates.jsonl
    │   ├── oracle/                # oracle lookup
    │   │   ├── mod.rs            # trait OraclePriceLookup
    │   │   └── in_memory.rs       # fixtures/oracle_prices.jsonl loaded eagerly
    │   ├── sink/                  # writer abstraction
    │   │   ├── mod.rs            # trait EnrichmentSink
    │   │   ├── stdout.rs         # JSON-lines
    │   │   └── sql_file.rs       # ALTER-friendly SQL dump
    │   └── bin/
    │       └── cli.rs            # CLI driver, named `enrichment-cli`
    ├── fixtures/                  # gitignored
    └── tests/
        └── enrich_e2e.rs         # in-memory pipeline
```

The three trait seams (`CandidateSource`, `OraclePriceLookup`,
`EnrichmentSink`) are the production-swap points. Same approach
as 0038. **Shared utility extraction (`safe_log`, `retry`) into
a workspace `prices-common` crate is deferred to a follow-up
task**: for now, this crate duplicates those modules from
0038's. Flagged in Issues Encountered.

### A.4 Trait seams

```rust
pub trait CandidateSource {
    fn next_batch(
        &mut self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Candidate>, CandidateError>> + Send;
}

pub trait OraclePriceLookup {
    fn lookup(
        &self,
        asset_id: &str,
        at: i64,
        window_s: u32,
    ) -> impl Future<Output = Result<Option<OraclePrice>, OracleError>> + Send;
}

pub trait EnrichmentSink {
    fn write(
        &self,
        rows: &[EnrichedRow],
    ) -> impl Future<Output = Result<(), SinkError>> + Send;
}
```

Production swaps:
- `JsonlCandidateSource` → CH query (`SELECT … FROM prices.price_ohlcv
  FINAL WHERE volume_quote_usd = 0 … LIMIT batch_size`)
- `InMemoryOracleLookup` → CH query (`SELECT price_usd FROM
  prices.oracle_prices WHERE asset_id = ? AND timestamp <= ?
  ORDER BY timestamp DESC LIMIT 1` — moved to ASOF JOIN in the
  single-pass query of A.2)
- `StdoutJsonSink` / `SqlFileSink` → batched INSERT via
  `clickhouse::Client::insert`

### A.5 Inputs — JSONL fixtures

```
packages/enrichment-worker/fixtures/
├── candidates.jsonl             # one Candidate per line
└── oracle_prices.jsonl          # one OraclePrice per line
```

Sample candidate line:
```json
{"timestamp": 1700000000, "asset_id": "CXLM", "granularity": "1m", "quote_asset_id": "CUSDC", "source": "phoenix", "volume_base": "10000000000", "volume_quote": "12500000000", "open": "1.25", "high": "1.26", "low": "1.24", "close": "1.255", "trade_count": 42, "vwap_numerator": "1.2487", "vwap_denominator": "100000000"}
```

Sample oracle line:
```json
{"asset_id": "CUSDC", "oracle_name": "reflector", "timestamp": 1700000000, "price_usd": "1.0012"}
```

Volumes and prices use string-encoded `rust_decimal::Decimal` to
match the CH `Decimal(36, 18)` columns the writers produce.

### A.6 Sinks — stub only

1. **`StdoutJsonSink`** — one JSON line per `EnrichedRow`. The
   same JSON shape the production writer would compute, useful
   for grep / diff across runs.
2. **`SqlFileSink`** — `INSERT INTO prices.price_ohlcv … VALUES
   (…), (…), …` batched per invocation. **The artefact for the BE
   meeting** — they can read the file and confirm the column shape
   matches what `prices.*` will host. Includes the version column
   (`_inserted_at`) so BE sees the ReplacingMergeTree pattern.

Explicitly out of prototype scope:

- No `clickhouse::Client` connection.
- No `aws_sdk_cloudwatch::Client::put_metric_data` (metrics go to
  stdout structured-JSON instead).
- No `aws_sdk_scheduler::Client` (the Lambda is not scheduled).

### A.7 Operator invocation surface

```bash
# CLI — fast inner-loop iteration
cargo run -p enrichment-worker --bin enrichment-cli -- \
    --candidates fixtures/candidates.jsonl \
    --oracle fixtures/oracle_prices.jsonl \
    --oracle-name reflector \
    --window-s 300 \
    --batch-size 1000 \
    --max-batches 5 \
    --sink stdout

# Lambda — runs the same algorithm under lambda_runtime, accepts
# an EventBridge schedule event payload (which is ignored in
# prototype mode — fixtures drive everything)
cargo lambda invoke enrichment-worker --data '{}'
```

### A.8 Prototype acceptance

- [ ] `cargo build -p enrichment-worker --release` succeeds.
- [ ] `cargo lambda invoke` with a stub event runs the enrichment
      against `fixtures/` and emits expected rows to stdout.
- [ ] `enrichment-cli --sink sql-file` produces an
      `out/enriched-{ts}.sql` containing `INSERT INTO
      prices.price_ohlcv …` rows that BE can review for column
      shape.
- [ ] Re-running the same invocation produces bit-identical
      output (idempotent: `volume_quote_usd = 0` filter excludes
      already-enriched rows from the candidate fixture).
- [ ] A candidate row whose `quote_asset_id` is absent from the
      oracle fixture is dropped (no row emitted) and an
      `EnrichmentOracleMiss` metric line lands on stdout.
- [ ] A candidate row whose oracle entry is older than
      `--window-s` is dropped (forward-fill window enforced).
- [ ] Unit tests cover: ASOF lookup picking newest-within-window,
      window expiry, decimal multiplication precision (no float
      drift on amounts > 10^18), and miss accounting.
- [ ] One `tests/enrich_e2e.rs` integration test runs the whole
      pipeline against in-memory fakes (no fixture I/O), asserts
      expected `EnrichedRow` and metric counts.
- [ ] This G-note's Part C reviewed by BE; answers captured below
      questions inline or as a follow-up note.

No deployment, no AWS calls, no live network.

---

## Part B — Out of prototype scope (explicit non-goals)

- **CDK stack.** No `infra/aws-cdk/` changes. The original 0024 §1.1
  Lambda specifics map cleanly to a CDK construct; landing that is
  the production-rewrite task.
- **EventBridge / Scheduler rule.** The prototype runs on operator
  invocation, not on a cron.
- **CloudWatch metric publish.** Metrics are emitted as JSON to
  stdout in the prototype.
- **`prices.price_ohlcv_enrichment_staging` table creation.** No
  DDL is issued. The G-note's SQL in A.2 is documentation only;
  applying it is production-rewrite work.
- **Historical (one-shot) backfill pass.** Per 0024 §4 (Option B1).
  Same binary in production, invoked without the 30-day window
  guard; out-of-scope here because the BE meeting may shift the
  one-shot's trigger (EventBridge rule on a `backfill_progress`
  table vs SNS vs manual).
- **Multi-oracle preference / fallback.** v1 picks one oracle
  name (default `reflector`).
- **Two-hop enrichment** for exotic-quote pairs (e.g. AQUA → XLM
  → USD). Spawns as a separate task per 0024 §3.1.
- **Shared `prices-common` crate.** `safe_log` and `retry` are
  duplicated from 0038's crate in this prototype; consolidation
  is a follow-up task.

---

## Part C — Cross-team contract (BE meeting agenda)

Each item is a concrete decision we need from BE. Bundled with
task 0038's Part C so BE answers both in one conversation.

### C.1 — Reuse of `db-clickhouse::mtls`

**Same ask as 0038 Part C.5.** This Lambda also runs outside the
VPC and writes to Hetzner CH via mTLS; it would consume
`db_clickhouse::mtls::client_from_lambda_env("prices")` identically.

**Position:** depend on the whole `db-clickhouse` crate via git
Cargo dep; Cargo dead-code-eliminates the unused modules.

### C.2 — Caddyfile `CLICKHOUSE_CN_USER_MAP` for prices-api

**Same ask as 0038 Part C.6.** No additional CNs needed — the
existing `prices-api-{env}` → `prices_writer_{env}` mapping
covers both Lambdas (they share an identity).

### C.3 — mTLS cert issuance

**Same ask as 0038 Part C.7.** Same cert bundle; same Secrets
Manager path under
`${mtlsSecretNamePrefix}/lambda-enrichment-{env}` per BE's
naming convention. **One sub-question:** does BE prefer one cert
per Lambda function (per BE's `compute-stack.ts:251, 305` shape)
or one shared cert for all prices-api Lambdas? Per-function is
cleaner for rotation; shared is simpler. Position: one per
function.

### C.4 — `prices.price_ohlcv` table engine + version column

**The biggest single item in this meeting (for the
enrichment-side).** The Option 2 enrichment pattern in A.2 requires
`prices.price_ohlcv` to be:

- **Engine:** `ReplacingMergeTree(_inserted_at)`
- **ORDER BY:** `(timestamp, asset_id, granularity, quote_asset_id, source)`
  — the ADR 0003 PK + ADR 0004 source column.
- **Version column:** `_inserted_at: DateTime DEFAULT now()` (or
  similar; field name negotiable).

**The ask:** BE confirms this engine choice fits what they
already operate (`crates/db-clickhouse/schema/init.sql` has 17
RMT tables already; this is one more in our `prices.*` database).

**Open sub-questions:**

1. Version column granularity — `DateTime` (second) vs `DateTime64`
   (millisecond) vs `UInt64` monotonic counter. Position:
   `DateTime` is sufficient (we never INSERT the same row twice
   in the same second).
2. `FINAL` cost on read. The Current Price Updater (task 0039)
   reads `price_ohlcv` every minute; `SELECT FINAL` per minute
   has bounded cost (task 0046's ~0.45 GB/yr empirical
   footprint). Acceptable per ADR 0007 Cluster A's
   announcement-not-approval norm; flagged for BE awareness.
3. Whether to add a materialised view that pre-merges
   `volume_quote_usd > 0` rows for read paths. Position: not for
   v1; consider once `current_prices` is the dominant reader.

### C.5 — `prices.oracle_prices` schema authority

The Oracle Fetcher Lambda (task 0039) writes this table; the
enrichment Lambda reads it. Both live in our `prices.*` database.

**The ask:** confirm BE has no read interest in
`prices.oracle_prices`. If they do, the schema is a soft
coordination item (we'd want stability for their dashboards).

**Position:** BE owns `default.*`; we own `prices.*`. Schema
decisions on `prices.*` are announcement-not-approval per ADR
0007 Cluster A. Surface the ask only so BE can flag if they
expect to consume the table; otherwise proceed unilaterally.

Proposed shape (for the meeting, not a decision):

```sql
CREATE TABLE prices.oracle_prices (
    asset_id     LowCardinality(String),
    oracle_name  LowCardinality(String),
    timestamp    DateTime,
    price_usd    Decimal(36, 18),
    _inserted_at DateTime DEFAULT now()
) ENGINE = ReplacingMergeTree(_inserted_at)
ORDER BY (asset_id, oracle_name, timestamp);
```

### C.6 — EventBridge Scheduler vs Rules

The 0024 spec mentions "EventBridge cron Lambda". AWS now offers
both **EventBridge Rules** (the original) and **EventBridge
Scheduler** (newer, more flexible). Both work for an hourly
trigger.

**Position:** EventBridge Scheduler — it's the recommended path
for new schedules, has a richer API for ad-hoc one-shot triggers
(the historical-pass invocation per 0024 §4 / B1), and is the
choice BE used for their own scheduled jobs.

**The ask:** confirm BE's preference matches, or point us at
whichever they want both teams to standardise on.

### C.7 — DLQ for failed enrichment passes

A failed enrichment pass (CH unreachable, malformed oracle data)
should retry. Two patterns:

1. EventBridge's `RetryPolicy` (max retries + max age) wraps the
   Lambda invocation; failed invocations land on a Scheduler-side
   DLQ.
2. The Lambda itself maintains a sidecar SQS queue for failed
   batches.

**Position:** EventBridge's RetryPolicy + a CloudWatch alarm on
sustained `EnrichmentRowsRemainingAtVolumeZero`. Sidecar SQS
adds operational complexity for marginal gain.

**The ask:** confirm BE has no preference forcing the sidecar
pattern.

### C.8 — Historical (one-shot) backfill enrichment

Per 0024 §4 Option B1, a one-shot pass runs after the SDEX
backfill (task 0027) completes. The trigger can be:

1. EventBridge Scheduler one-time run, manually configured by
   the operator once backfill finishes.
2. An automated trigger via SNS from the backfill completion
   path.
3. Polling the `backfill_progress` (or equivalent) table.

**Position:** (1) — manual operator trigger. The backfill
completion event is rare enough (one per env, per
historical-data-source) that automation overhead exceeds the
manual cost.

**The ask:** confirm BE has no objection to the manual trigger
or has an existing automation pattern they want us to adopt.

---

## Part D — Open questions for the meeting

Questions where we want BE input but haven't pre-staked a
position.

### D.1 — Read-side cursor for candidate selection

Production v1 reads `SELECT … FROM prices.price_ohlcv FINAL
WHERE volume_quote_usd = 0 … LIMIT batch_size`. Per
`MAX_BATCHES × BATCH_SIZE = 200 000` rows/hour, this scans the
WHERE-matching rows on every invocation, leaning on the planner's
PK prefix `(timestamp, …)` for the recency guard.

Alternative: maintain a cursor (`last_enriched_max_timestamp`) in
a tiny CH table to skip already-walked rows. Trade-off: cursor
adds state + a write per invocation; FINAL+LIMIT scan is
stateless but pays per-invocation cost. Lean: **stateless scan**
until empirical numbers say otherwise.

### D.2 — Multi-oracle prefer-then-fallback

v1 picks one `oracle_name` (default `reflector`). If the Oracle
Fetcher Lambda eventually writes multiple oracle sources (e.g.
also Chainlink), the enrichment Lambda picks one. Options:

- Pick highest-precedence available per row.
- Average across sources.
- Hard-pick one per env config.

Lean: hard-pick one per env until multi-oracle is a real signal.

### D.3 — Forward-fill window vs linear interpolation

v1 forward-fills (most-recent oracle bar within window). For
sparse oracle cadences (Reflector publishes ~5 min bars), linear
interpolation between two bracketing bars might be more accurate
than forward-fill — but only matters for sub-5-minute OHLCV bars
near the bracket boundaries.

Lean: forward-fill until operational signal says otherwise.

### D.4 — Staging table partitioning

`prices.price_ohlcv_enrichment_staging` in A.2 holds at most
one batch's worth of rows (200 000) between INSERT and
TRUNCATE. Engine choice is academic; `MergeTree ORDER BY
tuple()` is fine. Worth confirming we don't want it persisted
across invocations (we don't).

---

## Part E — Production rewrite punch list (when gates clear)

When task 0012 lands (CDK + RDS or CH schema migration tooling)
and BE 0227 ships the Hetzner mTLS endpoint, the prototype's
production swap items:

| # | Item | Est. days |
|---|---|---|
| 1 | Replace `JsonlCandidateSource` with `clickhouse::Client` SELECT against `prices.price_ohlcv FINAL`. | 0.5 |
| 2 | Replace `InMemoryOracleLookup` with the ASOF JOIN folded into the candidate query (single round-trip). | 1 |
| 3 | Replace `StdoutJsonSink` / `SqlFileSink` with the two-step staging-table INSERT + promotion. | 1 |
| 4 | CDK: Lambda function, role, EventBridge Scheduler rule, CloudWatch alarms, DLQ wiring. | 2 |
| 5 | Cross-stack handshake — publish Lambda role ARN under `/prices/{env}/...`, BE consumes if needed. | 0.5 |
| 6 | mTLS cert issuance + cert upload to Secrets Manager (shares C.7 with 0038's prod-rewrite). | 0.5 |
| 7 | Schema migration: confirm `prices.price_ohlcv` engine + version column; create
       `prices.price_ohlcv_enrichment_staging`; create `prices.oracle_prices`. | 1 |
| 8 | CloudWatch metrics publish via `aws_sdk_cloudwatch`. | 0.5 |
| 9 | Historical one-shot mode (`--no-window-guard` CLI flag + EventBridge one-time rule). | 1 |
| 10 | End-to-end smoke from a real dev-env Lambda invocation. | 1 |

**Total once gates clear: ~9 engineering days** (less than 0038's
~11 because no S3 / SQS / xdr-parser surface).

---

## Appendix — references

### Code in BE repo (`soroban-block-explorer/`)
- `crates/db-clickhouse/src/mtls.rs` — reusable mTLS client builder (shared with 0038)
- `crates/db-clickhouse/schema/` — 17 RMT tables we're matching the engine shape against
- `infra/src/lib/stacks/compute-stack.ts` — Lambda + cert + env-var pattern

### Local docs
- ADR 0003 — `price_ohlcv` PK shape with `quote_asset_id`
- ADR 0004 — multi-source merge columns
- ADR 0007 — live data sink on shared Hetzner ClickHouse
- Task 0024 (archived) — original PG-flavoured design spec
- Task 0038 — sibling prototype + cross-team contract (shared Parts C.1–C.3)

---

## Decision Log — C.4 resolved + production implementation (2026-06-09)

The BE cross-team C.4 question is resolved and the production Form-B
path is implemented (`packages/enrichment-worker/src/ch_enrich.rs`).
Resolving C.4 surfaced three places where the *real* ADR-0007 schema
diverges from what Part A.2 was drafted against; the implementation
follows the schema, not the original sketch:

1. **Engine confirmed `ReplacingMergeTree(version)`.** Recorded as a
   hard requirement in `docs/database-schema/database-schema-overview.md`
   §3.2 (callout) and `docs/prices-api-general-overview.md` §3.

2. **Version is `version UInt64` (ledger_seq×1000 + intra-ledger
   order), not `_inserted_at DateTime`.** Enriched re-inserts therefore
   carry `version = original_version + 1` (deterministic, wins the
   merge, self-heals if a later higher-version write to the same bucket
   resets `volume_quote_usd = 0`). The `now() AS _inserted_at` promote
   trick from A.2 is obsolete.

3. **`volume_quote` was missing from the live schema and is RESTORED**
   (empty/pre-production tables, so no backfill cost). The decoder
   (task 0048 spec) already computes `volume_quote = Σ|quote_amount|`
   to derive `vwap`, then discarded it; the column just stops
   discarding it. Enrichment now reads it directly —
   `volume_quote_usd = oracle_price × volume_quote` (exact) — instead
   of the lossy `vwap × volume_base` reconstruction. **Writer
   dependency:** task 0038 + the backfills must populate the restored
   column (`sdex-backfill/src/sink.rs` currently writes `volume_quote`
   into the `volume_quote_usd` slot — to be corrected there).

**Other deltas from A.2:**

- **Per-granularity tables.** Enrichment targets `price_ohlcv_1m` only.
  Rolled-up `_15m … _1M` are produced by the MV chain; how those views
  re-aggregate a *re-inserted* `_1m` row and what `version` they project
  onto their RMT targets is a **task 0051 dependency** — flagged, not
  solved here.
- **Direct `INSERT … SELECT`, no staging table.** With ledger-derived
  `version + 1`, the staging→promote→truncate dance (whose promote keyed
  on `max(_inserted_at)`) buys nothing: the single statement is already
  idempotent via the `FINAL WHERE volume_quote_usd = 0` read filter and
  self-healing on partial crash.
- **Trait seams dissolve in production.** `CandidateSource` /
  `OraclePriceLookup` / `EnrichmentSink` fold into the one set-based SQL
  statement; they remain the *prototype's* structure only.

**Caveat:** the production path is verified to compile + pass the
prototype test suite, but **not** run against a live ClickHouse (none
provisioned; prepare-only). Needs an integration test against a real CH
before it can be trusted end-to-end.
