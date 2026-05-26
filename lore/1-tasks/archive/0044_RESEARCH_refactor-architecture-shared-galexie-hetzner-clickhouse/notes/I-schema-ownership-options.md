---
title: "I: Schema-ownership boundary — prices-api inside BE's CH cluster"
type: idea
status: developing
spawned_from: ../README.md
spawns: []
tags: [schema, multi-tenant, migrations, ddl, clickhouse, step-5]
links:
  - './R-be-hetzner-ch-shape.md'
  - './R-ingest-target-mapping.md'
  - './R-aws-hetzner-auth-network.md'
  - '../../../../../soroban-block-explorer/lore/1-tasks/active/0227_FEATURE_infra-hetzner-ansible-playbook.md'
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: >
      Options analysis for "where do prices-api's CH tables live"
      and "who owns the DDL". Step 1 established BE plan is silent
      on multi-tenancy; step 2 needs a concrete answer to express
      the engine choices.
---

# I: Schema-ownership boundary — prices-api inside BE's CH cluster

## Purpose

Step 5 of task 0044. Pick a schema-ownership model and a migration
mechanism for prices-api's tables inside BE's Hetzner CH cluster.
This is the **dominant cross-team question** — every other step's
recommendation defers to whatever step 5 lands on.

Inputs:

- Step 1: BE plan is single-tenant; auth model already accommodates
  multiple AWS-side workloads but schema is BE-owned.
- Step 2: prices-api needs ~6 tables (`price_ohlcv` per-granularity,
  `current_prices`, `oracle_prices`, `assets`,
  `backfill_progress`) plus a chain of materialised views.
- Step 3: Live ingest contract is unchanged; only the storage half
  flips. Schema lives **inside** Hetzner CH.

Output: a recommended option + a migration tooling pick + a DDL
coordination process. Final go/no-go remains in the `S-*` note.

---

## 1. The four shapes

| #     | Shape                                                                        | Storage                                                         | Ownership                                          | Migration unit |
| ----- | ---------------------------------------------------------------------------- | --------------------------------------------------------------- | -------------------------------------------------- | -------------- |
| **1** | **Separate database `prices` in the same CH cluster**                        | `prices.*` tables alongside BE's `default.*`                    | Prices-api owns its database fully                 | Per-database   |
| **2** | **Shared tables with a `tenant` discriminator column**                       | BE's `default.*` tables gain a `tenant` column                  | BE owns DDL; prices-api proposes via PR to BE repo | Joint, BE-led  |
| **3** | **Same database, separate CH user with table-level grants**                  | Tables physically in `default.*` but logically prices-api-owned | Mixed: BE creates table, prices-api owns rows      | Per-table      |
| **4** | **Sidecar — separate `clickhouse-server` container on the same Hetzner box** | Two CH instances, two ports, two storage volumes                | Each fully independent                             | Per-instance   |

### 1.1 Option 1 — Separate database `prices`

The clean separation. BE creates `CREATE DATABASE prices` once,
issues a CH user `prices_api_writer` with full DDL/DML rights on
`prices.*`, and otherwise stops thinking about prices-api's tables.

```sql
-- One-time, BE-side
CREATE DATABASE prices;
CREATE USER prices_api_writer IDENTIFIED WITH ... ;
GRANT ALL ON prices.* TO prices_api_writer;
GRANT SELECT ON default.* TO prices_api_writer;  -- if prices-api needs to cross-read BE data
```

Prices-api's tables: `prices.price_ohlcv_1m`,
`prices.price_ohlcv_15m`, …, `prices.current_prices`, etc.

**Pros:**

- **Single-line BE involvement.** After the `CREATE DATABASE` +
  `GRANT`, BE never touches prices-api DDL again.
- **Prices-api migrations are unilateral.** No cross-team
  coordination per schema change.
- **Namespace isolation.** No risk of table-name collision with
  BE.
- **Backup is per-database** via Borg snapshot of
  `/var/lib/clickhouse/data/prices/` (or via CH-native
  `BACKUP DATABASE prices TO …`).
- **Cross-database reads work natively.** If prices-api ever
  needs to read BE's `default.soroban_events`, fully-qualified
  `default.soroban_events` works under the `GRANT SELECT ON
default.*` line.
- **Resource isolation knobs available.** CH `quota` and
  `profile` settings under `users.d/prices-api.xml` cap memory,
  concurrent queries, network traffic. Standard CH idiom.

**Cons:**

- Requires BE to agree to a per-tenant DB. Trivial in CH terms
  but new policy.
- Two migration tools in flight (BE's `db-clickhouse-init`
  sidecar + prices-api's equivalent). Both touch the same CH
  instance; coordination during deploy windows is operationally
  non-zero.
- `BACKUP DATABASE`-granular restore is a CH 23.4+ feature; for
  Borg-volume restore the unit is the whole instance.

### 1.2 Option 2 — Shared tables with `tenant` discriminator

BE's `init.sql` grows a `tenant LowCardinality(String)` column on
each table prices-api would otherwise duplicate. Both writers
insert with their own `tenant` value; read queries filter by
tenant.

**Pros:**

- One DDL surface. No risk of two tables drifting apart.
- Cross-tenant analytics are trivial (`GROUP BY tenant`).

**Cons:**

- **Couples prices-api's schema evolution to BE's repo.** Every
  prices-api schema change is a PR to BE; every BE schema change
  has to consider prices-api's tenant. Slow-moving.
- **The schemas don't actually share well.** BE's `soroban_events`
  is event-shaped; prices-api's `price_ohlcv` is candle-shaped.
  They're not the same row. Forcing them into one table needs
  ~80% nullable columns per row — pathological in a columnar
  store.
- **Different engines.** BE uses `ReplacingMergeTree` /
  `AggregatingMergeTree` per table; prices-api's per-granularity
  rollup chain (step 2) wants its own MV graph. Shared tables
  cannot express both.

This option only makes sense if prices-api's data shape
**literally overlaps** with BE's — which it does not. **Reject.**

### 1.3 Option 3 — Same database, separate CH user, table-level grants

BE owns the database (`default`); prices-api gets a CH user with
narrow grants on its own tables. Prices-api proposes table DDL via
PRs to BE's `init.sql`; BE applies them.

**Pros:**

- Less BE involvement than Option 2 (read DDL once, never read
  rows).
- No new database to provision.

**Cons:**

- **BE's `init.sql` carries prices-api's tables.** Every
  prices-api schema change still flows through BE's repo.
  Operationally equivalent to Option 2's coupling on the DDL
  axis; only the row coupling is removed.
- BE's `db-clickhouse-init` sidecar runs all DDL — prices-api
  has no independent migration path.
- Backup boundary is the same (whole instance), so no win there.

Option 3 trades the schema-evolution cost for nothing. **Reject
unless BE specifically prefers it for security review purposes.**

### 1.4 Option 4 — Sidecar `clickhouse-server` instance

Two CH containers on the same Hetzner box, different ports,
different data volumes. Caddy SNI-route or path-route between
them.

**Pros:**

- True isolation: prices-api can't OOM BE's CH, prices-api
  schema changes don't touch BE.
- Independent backup volumes.

**Cons:**

- **Doubles the operational surface.** Two CH versions to
  upgrade in lock-step (parts format compatibility), two backup
  cron jobs, two `users.d`/`config.d` directories, two systemd
  units.
- Defeats the cost-sharing premise. The reason to share is "one
  box, one CH"; a sidecar instance is "one box, two CHs" — less
  isolation cost than two boxes but more than option 1.
- BE plan doesn't include it. Asking BE to bolt on a sidecar
  instance is non-trivial Ansible work (touches task 0227's
  scope).

Reasonable fallback if BE refuses option 1, but strictly worse
on operational cost.

---

## 2. Recommendation: Option 1

**Separate database `prices` in the same CH cluster.**

Reasoning, ranked by load-bearing weight:

1. **Decoupled schema evolution.** Prices-api iterates without
   BE-blocking PRs. Given that 0038/0039/0040 + future work
   will land 5+ schema changes in the first quarter, decoupling
   is the dominant productivity factor.
2. **One BE commitment.** BE agrees once to `CREATE DATABASE
prices` + grants. Operationally inexpensive on BE side.
3. **Natural CH idiom.** Multiple databases in one CH instance
   is the language's first-class multi-tenant primitive. No
   tricks.
4. **Failure-mode containment.** A bad prices-api migration
   cannot break BE's tables. Symmetrically, a bad BE migration
   cannot mutate prices-api tables (only break shared physical
   resources, which is the same risk as any shared instance).

The cost — two migration tools, single-instance backup — is
real but small.

---

## 3. Migration tooling

Three viable mechanisms:

### 3.1 Sidecar pattern (BE-style), retargeted

Mirror BE's `db-clickhouse-init` shape. A small Rust binary
reads `packages/prices-ch-schema/init.sql` and applies it to
`prices.*` over the mTLS-protected HTTP endpoint. Run on every
deploy as a CI step (not on every container start — BE's pattern
runs server-side; prices-api would run client-side).

**Pros:**

- Symmetric with BE's mental model. Easy to explain.
- `CREATE TABLE IF NOT EXISTS` semantics make it idempotent for
  greenfield work.

**Cons:**

- Idempotent `init.sql` doesn't express `ALTER`s cleanly.
  Workaround: a small `prices._schema_versions` table tracking
  which numbered statements have been applied.
- Replays full file on every deploy; for a large schema this is
  slower than necessary.

### 3.2 Versioned migration files + tiny applier

Hand-rolled ~200-line Rust binary:

```text
packages/prices-ch-schema/
  migrations/
    0001_create_price_ohlcv_1m.sql
    0002_create_price_ohlcv_15m_mv.sql
    0003_add_current_prices.sql
    ...
  src/main.rs   -- the applier
```

The applier:

1. `SELECT name FROM prices._schema_migrations` (creates table
   if missing).
2. For each `migrations/NNNN_*.sql` not yet applied: execute,
   then `INSERT INTO _schema_migrations VALUES (...)`.
3. Run via `cargo run` in CI on deploy, or invoke as a one-shot
   Lambda.

**Pros:**

- Explicit version history.
- `ALTER` migrations express naturally.
- No third-party migration framework dependency.
- Same crate can run from CI **or** from a developer laptop for
  iteration.

**Cons:**

- Hand-rolled = "we own the bugs" — though at ~200 lines this is
  a feature, not a cost.
- No automatic rollback support (CH doesn't support
  transactional DDL anyway, so rollback would be advisory at
  best).

### 3.3 Third-party migration framework

| Tool                                         | Verdict                                                                                              |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `refinery` (Rust)                            | No first-class CH driver. Pass.                                                                      |
| `golang-migrate/migrate` (Go) with CH driver | First-class CH. Introduces a Go dependency in a Rust shop. Pass.                                     |
| `atlas` (ariga.io)                           | Declarative schema-as-code with CH support. Powerful but heavy. Overkill for a 6-table schema. Pass. |
| `clickhouse-migrations` (npm)                | JS dependency in a Rust shop. Pass.                                                                  |

None of these are bad; they're just **larger** than the project
needs at this scope.

### 3.4 Pick: §3.2 versioned migration files + tiny applier

Reasoning: explicit version history outweighs the convenience of
re-runnable idempotent SQL once the schema starts taking ALTERs.
Hand-rolled applier is cheap at ~200 lines and stays in-language.

**Where it runs:** CI step in the prices-api deploy pipeline.
Connects via mTLS (same secret material the Lambdas use),
applies pending migrations, exits. Run before the
Ledger-Processor Lambda is published so the schema is ready when
the Lambda starts firing.

---

## 4. DDL coordination process across two teams

Three operational policies to agree with BE:

### 4.1 Schema announcement, not approval

Prices-api **notifies** BE about a new migration (Slack channel,
PR cross-link, whatever channel exists) but does not require BE
approval for changes inside `prices.*`. Decoupling premise of
Option 1 only works if BE isn't on the critical path.

### 4.2 Joint review for cross-database reads

If prices-api ever uses `SELECT … FROM default.<be_table>`, the
**SQL** is reviewable by BE (because if BE renames `<be_table>`
that breaks prices-api). Suggested practice: cross-database
reads go through a small set of named views in `prices.*` that
wrap the BE join, so the breakage surface is narrow and
discoverable.

### 4.3 Deploy-window etiquette

Both sides run migrations during deploy. Concurrent migrations
on the same instance occasionally lock metadata. Suggested
practice: prices-api's migrations land **before** the Lambda is
re-published; BE's `db-clickhouse-init` runs at server boot.
Steady-state deploys do not collide. **Document** the rare
collision mode (initial bring-up + box recovery) in the runbook.

---

## 5. Resource isolation knobs

ClickHouse supports per-user quotas and profiles. Recommended
defaults to negotiate with BE for the prices-api user:

```xml
<!-- users.d/prices-api.xml -->
<clickhouse>
  <profiles>
    <prices_api_profile>
      <max_memory_usage>4000000000</max_memory_usage>          <!-- 4 GB per query -->
      <max_concurrent_queries_for_user>20</max_concurrent_queries_for_user>
      <max_threads>8</max_threads>
      <readonly>0</readonly>
    </prices_api_profile>
  </profiles>

  <quotas>
    <prices_api_quota>
      <interval>
        <duration>3600</duration>
        <queries>50000</queries>
        <read_rows>10000000000</read_rows>
        <result_rows>1000000000</result_rows>
      </interval>
    </prices_api_quota>
  </quotas>

  <users>
    <prices_api_writer>
      <profile>prices_api_profile</profile>
      <quota>prices_api_quota</quota>
      <networks>
        <ip>0.0.0.0/0</ip>  <!-- handshake gating is mTLS at Caddy, not IP -->
      </networks>
    </prices_api_writer>
  </users>
</clickhouse>
```

These numbers are placeholders to refine after step 6 (cost +
capacity sizing). Their existence is the architectural point:
prices-api cannot accidentally consume BE's headroom.

---

## 6. Backup / restore boundary

Borg backs up `/var/lib/clickhouse` (BE pattern from task 0227).
That **includes** prices-api's database files. Restore granularity
is whatever Borg + CH `ATTACH PART` semantics allow:

- **Whole-instance restore.** Always works. Restores both
  tenants' data.
- **Per-database restore.** Possible via CH-native `BACKUP
DATABASE prices TO …` (CH 23.4+) or selective Borg path
  restore of `/var/lib/clickhouse/data/prices/` plus
  `/var/lib/clickhouse/metadata/prices/`.

Negotiation point: ask BE to add `BACKUP DATABASE prices` as a
separate daily Borg target so prices-api can be restored
independently. Cheap on BE side; useful for prices-api recovery
drills.

---

## 7. Open questions surfaced by step 5 (forwarded to README)

20. **BE buy-in on Option 1.** Trivial in CH terms but a policy
    decision: does BE accept a second database in their
    instance? Asked alongside open question 11 (bucket-
    notification fan-out) and 17 (Caddy capacity) as the
    cross-team conversation bundle.
21. **Cross-database read pattern.** Is there ever a case where
    prices-api needs to read BE's `default.soroban_events`? If
    so, name the read-paths early so the GRANT scope is right
    from day one. (Initial guess: no — prices-api consumes XDR
    directly from S3, same as BE; no need to read BE's parsed
    table.)
22. **Migration applier deployment.** CI step (GitHub Actions
    runner with mTLS secrets) vs. one-shot Lambda (avoids
    embedding the cert in CI). Recommend Lambda for prod;
    CI for dev/staging.
23. **`BACKUP DATABASE prices` cadence.** Ask BE to add a
    per-database backup target to the Borg cron.
24. **Migration-collision runbook.** Document the rare
    initial-bring-up / box-recovery scenario where prices-api
    and BE migrations race.

---

## 8. What step 5 does NOT cover

- Concrete `init.sql` / migration file contents for the chosen
  shape — that's a G-note in the implementation task.
- Capacity sizing of the per-user quotas — step 6.
- Final go/no-go and the impact map across blocked tasks — step 7.
