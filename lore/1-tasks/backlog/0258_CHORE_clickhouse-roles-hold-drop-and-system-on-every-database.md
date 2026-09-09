---
id: "0258"
title: "api_reader and ingestion_writer hold DROP, TRUNCATE and SYSTEM on *.* — on the box shared with BE"
type: CHORE
status: backlog
related_adr: ["0007"]
related_tasks: ["0210"]
tags: [layer-infra, priority-high, effort-small, milestone-M2, security, clickhouse]
milestone: 2
history:
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

## Acceptance Criteria

- [ ] Every role's grants are scoped to the databases it actually uses
- [ ] No service role holds `DROP`, `TRUNCATE` or `SYSTEM` on `*.*`
- [ ] `displaySecretsInShowAndSelect` and the external-source privileges are
      removed unless a named requirement justifies each
- [ ] The service→role mapping is written down, since it was not discoverable
      from the repo during this investigation
