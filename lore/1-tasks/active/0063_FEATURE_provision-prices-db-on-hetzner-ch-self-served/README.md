---
id: "0063"
title: "Provision the `prices` database + user + quota + profile + mTLS cert on Hetzner CH (self-served with admin access)"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0050", "0051", "0047", "0011", "0038"]
tags: [layer-infra, priority-high, effort-medium, milestone-M1, hetzner, clickhouse, mtls, rbac, tenancy]
milestone: 1
links:
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "./0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning/notes/G-be-prices-db-rbac-ask.md"
  - "./0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/archive/0227_FEATURE_infra-hetzner-ansible-playbook.md"
  - "../../../../soroban-block-explorer/infra-hetzner/README.md"
  - "../../../../soroban-block-explorer/crates/db-clickhouse/users.d/services.xml"
  - "../../../../soroban-block-explorer/infra-hetzner/ca/issue-client-cert.sh"
history:
  - date: 2026-06-17
    status: backlog
    who: oski
    note: >
      Spawned after confirming BE 0227 (infra-hetzner Ansible playbook +
      mTLS CA + prod compose overlay) shipped and was archived
      2026-05-19, and that BE will grant prices-api admin access to the
      Hetzner CH box. This flips the `prices` database + user + quota +
      profile provisioning (formerly task 0050 items 2+3, BE-owned) into
      self-served work prices-api executes directly. Task 0050 is now
      narrowed to the SNS fan-out item only. The table schema + MV chain
      itself remains in 0051, which applies into the database this task
      creates.
  - date: 2026-06-18
    status: active
    who: oski
    note: >
      Activated as the single gate now in front of both 0051 (live schema
      apply) and 0052 (live mTLS round-trip), both moved to blocked-on-0063.
      Scope this session is **authoring / PR-staging only**: draft the
      `prices_writer`/`prices_reader` `users.d` additions + CN-map entries
      (PR to soroban-block-explorer), the `CREATE DATABASE` one-shot, and
      the cert→Secrets-Manager procedure — grounded in BE's real RBAC files.
      Hard constraint (operator): **no Hetzner or AWS calls** — every remote
      action (clickhouse-client on the box, ansible-playbook, aws
      secretsmanager) is gated on explicit per-session approval.
---

# Provision the `prices` database on Hetzner CH (self-served)

## Summary

Now that BE has shipped the Hetzner ClickHouse production deployment
(BE task 0227, archived 2026-05-19) and is granting prices-api admin
access to the box, prices-api provisions its own tenant directly
rather than waiting on a BE-side hand-off. This task creates the
`prices` database, the scoped `prices_writer` / `prices_reader` CH
users, their profile + quota, the Caddy CN→user mapping, and the
per-env mTLS client cert — all per ADR 0007 §3.5 multi-tenant
primitives, mirroring BE's existing tenant pattern.

This is **tenancy provisioning only** — the empty database plus
access control. The `prices.*` table DDL + materialised-view rollup
chain is task 0051, which applies into the database this task creates.

## Context

BE's deployment model (verified in `soroban-block-explorer`):

- ClickHouse runs in Docker via `docker-compose.prod.yml`, fronted by
  Caddy:443 doing TLS + `require_and_verify` mTLS + a CN→user allowlist
  (`Caddyfile` + `CLICKHOUSE_CN_USER_MAP`).
- Tenant **users / profiles / quotas are config-file-defined** in
  `crates/db-clickhouse/users.d/{services,profiles,quotas}.xml`
  (not SQL-created), delivered to the box by the Ansible `app` role.
- Databases + table schema are applied by a sidecar
  (`db-clickhouse-init`) running `crates/db-clickhouse/schema/init.sql`.
- mTLS client certs are issued off-box with
  `infra-hetzner/ca/issue-client-cert.sh <CN>`, which needs the CA
  private key (BE password manager, never committed).

Because users are XML-defined, the durable path is a small PR to the
BE repo (add the prices users + CN map entry) plus an Ansible
`--tags app` run, rather than ad-hoc `CREATE USER` SQL that the next
deploy would not reproduce. The opening `prices` database itself can
be created with a one-shot `CREATE DATABASE IF NOT EXISTS prices`
under admin, then kept reproducible by 0051's migration runner.

## Implementation Plan

### Step 1: Confirm access + capture the live deploy state

- Confirm admin access works: `clickhouse-client --user=default
  --password=… -q "SHOW DATABASES"` on the box (loopback), or via
  Caddy:443 with an admin-mapped cert.
- Record the box's current `users.d/*.xml`, `Caddyfile`, and
  `CLICKHOUSE_CN_USER_MAP` so the prices additions are diffed against
  the real running config, not assumptions.

### Step 2: Add the prices tenant to BE's RBAC (PR to soroban-block-explorer)

Mirror the existing BE tenant shape (see `G-be-prices-db-rbac-ask.md`):

- `users.d/services.xml`: add `prices_writer` (profile `write_no_ddl`,
  quota `high_write`) and `prices_reader` (profile `read_only`, quota
  `api_throttle` or a new `prices_read` quota), both `<no_password/>`
  with networks restricted to loopback + the compose bridge subnet.
- Reuse BE's existing `write_no_ddl` / `read_only` profiles unless
  prices needs tighter caps; if so add a `prices_write` profile in
  `profiles.xml`. Decide at impl time; record in a short note.
- `CLICKHOUSE_CN_USER_MAP`: add `prices-ingestion-{env}:prices_writer`
  and `prices-api-{env}:prices_reader`.

> DDL caveat: `write_no_ddl` blocks `CREATE TABLE`. 0051's schema-apply
> runs under an admin/DDL-capable identity, **not** `prices_writer`.
> Resolve which identity applies schema as part of Step 2 (either a
> short-lived admin cert for migrations, or BE applies the initial DDL).

### Step 3: Create the database + deploy the RBAC

- One-shot under admin: `CREATE DATABASE IF NOT EXISTS prices`.
- Land the Step 2 PR; run `ansible-playbook -i inventory.ini site.yml
  --tags app` so CH picks up the new users and Caddy picks up the new
  CN map. (Prepare-only constraint: coordinate the actual playbook run
  with BE / explicit approval — see notes.)

### Step 4: Issue + store the mTLS client certs

- Issue per-env certs with `infra-hetzner/ca/issue-client-cert.sh`
  for CNs `prices-ingestion-{env}` and `prices-api-{env}`.
  - Requires the CA private key. If admin-on-the-box does **not**
    include CA-key access, this sub-step stays a BE ask (BE runs the
    script and hands over the bundle) — flag it explicitly.
- Store cert+key (+ CA cert) bundles in AWS Secrets Manager, 2 secrets
  per env, at the keys 0011's CDK + 0052's client crate expect.

### Step 5: Verify tenant isolation

For each env (dev → staging → prod):

- Connect via Caddy:443 with the prices cert; `SELECT version()` → 200.
- As `prices_writer`: `CREATE TABLE prices.smoke (x UInt8) ENGINE=Memory`
  + `INSERT` succeed; the same against `default.*` is **denied**.
- As `prices_reader`: `SELECT` from `prices.*` works; any write denied.
- Drop the smoke table.

## Acceptance Criteria

- [ ] `prices` database exists on the Hetzner CH box
- [ ] `prices_writer` + `prices_reader` users exist via BE-repo
      `users.d/*.xml` (reproducible across deploys), with profile +
      quota scoping resource usage away from BE's `default.*`
- [ ] Caddy `CLICKHOUSE_CN_USER_MAP` maps the prices CNs to the prices
      users; unmapped CNs 403
- [ ] Per-env mTLS cert+key pairs issued and stored in AWS Secrets
      Manager (2 secrets/env) at the keys 0011/0052 read; or, if CA-key
      access is withheld, the BE-issuance hand-off is recorded done
- [ ] Smoke test confirms isolation: `prices.*` writable by
      `prices_writer`, `default.*` denied; `prices_reader` read-only
- [ ] DDL-apply identity for 0051 decided + documented (admin cert vs
      BE applies initial DDL)
- [ ] `notes/G-provisioning-record.md` captures the SQL/XML applied,
      SSM/Secrets keys, CNs, and per-env completion dates

## Blocked on

- Nothing hard. BE 0227 (the upstream gate) is **shipped** and admin
  access is being granted. Practical prerequisites: (a) admin access
  actually handed over, (b) merge access to the BE repo for the
  `users.d` PR, (c) CA-key access for Step 4 (else that sub-step is a
  BE ask). Throughput gate 0047 (ADR 0007 GREEN/YELLOW) gates *going
  to prod volume*, not creating the empty dev database.

## Out of scope

- `prices.*` table schema + MV rollup chain — task 0051 (applies into
  the database this task creates).
- SNS fan-out topic — remains task 0050 (still genuinely BE-side).
- Shared ClickHouse mTLS client crate — task 0052.
- Cross-tenant throughput verification — task 0047.

## Notes

- **Prepare-only / approval**: per the standing local-only constraint,
  running the Ansible playbook and `aws secretsmanager put-secret-value`
  against real infra are mutating actions — get explicit per-session
  approval before executing; this task may be authored/PR-staged ahead
  of the approved run window.
- Source of truth for the concrete XML/SQL/cert snippets is
  `0050/notes/G-be-prices-db-rbac-ask.md` (already grounded in BE's
  real `services.xml` / `profiles.xml` / `issue-client-cert.sh`).
- This task supersedes task 0050 items 2 + 3. 0050 is narrowed to the
  SNS fan-out item.
