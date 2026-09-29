---
id: "0258"
title: "api_reader and ingestion_writer hold DROP, TRUNCATE and SYSTEM on *.* — on the box shared with BE"
type: CHORE
status: completed
related_adr: ["0007"]
related_tasks: ["0210"]
tags: [layer-infra, priority-high, effort-small, milestone-M2, security, clickhouse]
milestone: 2
links:
  - https://github.com/rumblefishdev/soroban-block-explorer/blob/develop/lore/1-tasks/archive/0591_OPS_api-reader-and-ingestion-writer-have-no-grants.md
  - https://github.com/rumblefishdev/soroban-block-explorer/pull/531
history:
  - date: 2026-09-29
    status: completed
    who: stkrolikiewicz
    note: >
      Fixed in the explorer: task 0591, PR #531. Its services.xml was
      deployed in place on 2026-09-29 06:33 UTC. SHOW GRANTS on ch-prod-01
      now gives api_reader SELECT on default.* and prices.*, and
      ingestion_writer SELECT, INSERT on default.*; nothing ON *.* for
      either. Probes on prod: INSERT into prices.* and url()/remote() are
      refused with 497.
  - date: 2026-09-25
    status: backlog
    who: claude
    note: >
      Quick-check done (Oskar's 0314 triage asked for it). Operator ran SHOW
      GRANTS on ch-prod-01: prices_reader / prices_writer / prices_admin are
      scoped to prices.* (explorer 0567–0569); api_reader and ingestion_writer
      still hold ALL ON *.* because their services.xml entries have no
      <grants> block — and they are the EXPLORER's Lambda API and Lambda
      Ingestion users, not ours. DDL is refused by their profiles
      (allow_ddl=0, readonly=1 on api_reader); INSERT anywhere incl. prices.*
      and the url()/s3()/remote() paths are not. Mapping written down
      (criterion 4). The fix is an explorer task on their services.xml.
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Found while checking, per [[0210]]'s runbook, whether the API reader's
      grant covered the new `asset_symbol` table. It did — because the role has
      essentially everything, everywhere.
---

# Two roles are named "reader" and "writer" but are effectively superusers

## Summary

`SHOW GRANTS` on the production ClickHouse returns, for **both** `api_reader`
and `ingestion_writer`:

```
GRANT CHECK, SHOW, SELECT, INSERT, ALTER, CREATE, DROP, UNDROP TABLE,
      TRUNCATE, OPTIMIZE, BACKUP, KILL QUERY, KILL TRANSACTION,
      MOVE PARTITION BETWEEN SHARDS, SYSTEM, dictGet,
      displaySecretsInShowAndSelect, INTROSPECTION, CLUSTER, FILE, URL,
      REMOTE, MONGO, REDIS, MYSQL, POSTGRES, SQLITE, ODBC, JDBC, HDFS, S3,
      HIVE, AZURE, KAFKA, NATS, RABBITMQ, SOURCES ON *.*
```

`ON *.*` — every database on the instance. Per ADR 0007 that instance is
**shared with BE**, so a role called `api_reader` can drop BE's tables.

## Why it matters beyond the name

- `DROP` / `TRUNCATE` on `*.*` — destructive reach far outside `prices`.
- `displaySecretsInShowAndSelect` — reveals credentials embedded in table
  definitions.
- `URL`, `REMOTE`, `S3`, `MYSQL`, `POSTGRES` — table functions that can read
  from and push to arbitrary external endpoints, i.e. an exfiltration path.

The API is internet-facing and its credentials live in SSM. A leak there is not
"someone can read our prices"; it is full control of a shared database.

## The pattern already exists

`prices_reader` and `prices_writer` are scoped correctly and are what these
should look like:

```
GRANT SELECT ON prices.* TO prices_reader
GRANT SELECT, INSERT, ALTER DELETE, OPTIMIZE ON prices.* TO prices_writer
```

## Implementation

- Establish which service actually authenticates as `api_reader` and as
  `ingestion_writer` — the names suggest prices-api and the ingest path, but
  `prices_reader`/`prices_writer` exist too, so this must be checked, not
  assumed, before anything is revoked.
- Re-grant at the narrowest level that keeps each service working, using the
  `prices_*` roles as the model.
- Coordinate with BE: shared box, and revoking is the kind of change that
  breaks things loudly if the mapping was guessed wrong.

## Checked on the box — 2026-09-25

`SHOW USERS; SHOW GRANTS FOR …` run by the operator on `ch-prod-01`
(`docker exec app-clickhouse-1 clickhouse-client`). Ten users:
`api_reader`, `default`, `dev_read`, `dev_shared`, `dict_reader`, `galexie`,
`ingestion_writer`, `prices_admin`, `prices_reader`, `prices_writer`.

**The mapping, from both repos' infra:**

| CH user | who authenticates as it | grants on the box | defined in |
|---|---|---|---|
| `prices_reader` | prices api-handler (mTLS bundle `clickhouse-mtls-prices-api-*`) | `SELECT ON prices.*` | explorer `crates/db-clickhouse/users.d/services.xml`, `<grants>` |
| `prices_writer` | prices ledger-processor + every scheduled worker (`…-prices-ingestion-*`) | `SELECT, INSERT, OPTIMIZE, ALTER DELETE ON prices.*`, `SELECT` on four `system.*` tables and on `default.soroban_events`, `default.soroban_contracts` (explorer 0569) | same, `<grants>` |
| `prices_admin` | the operator's cert (explorer 0567/0568) | `SELECT, INSERT, ALTER, CREATE TABLE, DROP TABLE, TRUNCATE ON prices.*`, `SELECT ON default.*`, four `system.*` | same, `<grants>` |
| `api_reader` | **the explorer's** Lambda API (Caddy CN `lambda-api-<env>`) | **everything `ON *.*`** + `TABLE ENGINE ON *`, `SET DEFINER ON *`, `SOURCES ON *.*` | same — **no `<grants>` block**, so ClickHouse grants ALL |
| `ingestion_writer` | **the explorer's** Lambda Ingestion (CN `lambda-ingestion-<env>`) | same as `api_reader` | same — no `<grants>` block |

So the Summary's premise ("the names suggest prices-api") was wrong: neither
wide user is ours. Every prices identity is scoped; the two wide ones belong to
the explorer, and their own RBAC document
(`docs/architecture/security/clickhouse-rbac.md`, "Per-service user matrix")
claims `SELECT on default.*` and `INSERT on tables Galexie does not touch` —
a scoping the box does not have.

**What the profiles still block, and what they do not.** `api_reader` runs
profile `read_only` (`readonly=1`, `allow_ddl=0`); `ingestion_writer` runs
`write_no_ddl` (`readonly=0`, `allow_ddl=0`). So `DROP`, `TRUNCATE`, `CREATE`
and `SYSTEM` are refused at the settings layer for both, despite the grant.
Not blocked: `ingestion_writer` can `INSERT` into any table in any database,
`prices.*` included, and can `INSERT INTO FUNCTION url()/s3()/remote()` (the
exfiltration path in "Why it matters"); both can `SELECT FROM url()/remote()`
into the network; `displaySecretsInShowAndSelect` stands on both.

**Where the fix is:** the explorer repo — a `<grants>` block on each of the two
users in `services.xml`, mirroring what their matrix already says, deployed
the way 0567–0569 were. Not this repo. 0258 closes when that task exists and
points here.

## Closed on the box — 2026-09-29

Explorer task 0591 (PR #531) added `<grants>` to every service user in
`services.xml`, and it was deployed in place on 2026-09-29 06:33 UTC.
`SHOW GRANTS` on `ch-prod-01` after the deploy:

| CH user | grants |
|---|---|
| `api_reader` | `SELECT ON default.*`, `SELECT ON prices.*` |
| `ingestion_writer` | `SELECT, INSERT ON default.*` |
| `galexie` | none (it writes to S3 only) |
| `dev_read` | `SELECT` on `default.*`, `prices.*` and `system.*` |
| `prices_reader` / `prices_writer` / `prices_admin` | unchanged, see the 2026-09-25 table |

Probes on prod, as each user:

- `ingestion_writer`: `INSERT INTO prices.asset_metadata … WHERE 0` is refused
  with 497
- `api_reader`: the same INSERT is refused with 497
- `url()` and `remote()` are refused for both

Still wide, by design or tracked elsewhere:

- `default` and `dev_shared` are admin users on purpose. `default` is
  password-protected and loopback-only. `dev_shared` is the target of the
  developer mTLS certs, and the explorer's RBAC doc names it as the one
  user without `<grants>`.
- `dict_reader` still holds `ALL` on `default` plus the external-source
  privileges. It is unused and loopback-only, and step 2 of explorer task
  0396's rollout removes it.

## Acceptance Criteria

- [x] Every role's grants are scoped to the databases it actually uses
      → every service user; the exceptions are listed in "Closed on the box"
- [x] No service role holds `DROP`, `TRUNCATE` or `SYSTEM` on `*.*`
      → `dict_reader` is the one exception, and it goes with explorer 0396
- [x] `displaySecretsInShowAndSelect` and the external-source privileges are
      removed unless a named requirement justifies each
      → gone from `api_reader` and `ingestion_writer`; `dict_reader` as above
- [x] The service→role mapping is written down, since it was not discoverable
      from the repo during this investigation
      → below, "Checked on the box — 2026-09-25"; the other three criteria are
      the explorer's to close (their `services.xml`), see the same section
