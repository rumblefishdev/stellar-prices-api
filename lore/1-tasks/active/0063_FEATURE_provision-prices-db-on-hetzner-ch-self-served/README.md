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
  - date: 2026-06-18
    status: active
    who: oski
    note: >
      DDL-apply identity decided — **Option 1**: loopback `default` admin
      applies all DDL; runtime certs (`prices_writer`/`prices_reader`) carry
      no DDL (`write_no_ddl`/`read_only`, grants on `prices.*` only). Chosen
      over the G-note's scoped-DDL writer and the hybrid migrator cert; matches
      BE (removed remote-DDL users in BE 0241) and keeps the 0051 loopback
      descope. Authored `notes/G-provisioning-plan.md` — the ready-to-execute
      runbook with drafted BE-PR XML (services/quotas + CN-map), the CREATE
      DATABASE one-shot, the cert→single-bundle-secret procedure (aligned to
      0052, reconciling the old two-secret assumption), verification, and a
      gated-action inventory. All authoring only — no Hetzner/AWS/BE-repo
      action taken.
  - date: 2026-06-23
    status: active
    who: oski
    note: >
      §5 mTLS certs issued + stored in Secrets Manager and §1d CN-map
      pushed to the operator env secret (both done by hand, recorded in
      the G-plan completion record). Expanded the runbook's §3 "Deploy the
      RBAC" from a one-liner into the full operator-run procedure (what
      RBAC means here, shared-box/gated warning, Steps 0–4 with dry run,
      post-deploy verification) so the BE-coordinated `--tags app` run is
      turnkey. Remaining: the gated §3 ansible deploy (BE-coordinated) then
      the §6 isolation smoke test — both operator-run. Two ACs sit at
      `[~]` (users + CN-map defined/pushed, live on deploy); task stays
      active until those flip and §6 passes.
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

Mirror the existing BE tenant shape. **Concrete drafted XML/CN-map/SQL
lives in `notes/G-provisioning-plan.md`** (Option 1). In summary:

- `users.d/services.xml`: add `prices_writer` (profile `write_no_ddl`,
  quota `prices_write`, grant `SELECT, INSERT, OPTIMIZE ON prices.*`) and
  `prices_reader` (profile `read_only`, quota `prices_read`, grant
  `SELECT ON prices.*`), both `<no_password/>` with networks restricted
  to loopback + the compose bridge subnet.
- `users.d/profiles.xml`: **no change** — reuse BE's `write_no_ddl`
  (8 GiB) + `read_only` (4 GiB/30 s). No `prices_write` profile (Option 1).
- `users.d/quotas.xml`: add dedicated `prices_write` / `prices_read`
  (caps copied from BE's `high_write` / `api_throttle`) so prices never
  draws down BE's per-service budget.
- `CLICKHOUSE_CN_USER_MAP` (env, not a file): append
  `prices-ingestion-{env}:prices_writer` and `prices-api-{env}:prices_reader`.

> **DDL identity — DECIDED 2026-06-18 (Option 1, Design Decisions → Emerged #1):**
> DDL is the box `default` admin over loopback; `prices_writer` stays
> `write_no_ddl` with **no** `CREATE`/`DROP`/`ALTER` grant. Schema is applied by
> 0051 over loopback (Step 4 below), **not** over mTLS and **not** by
> `prices_writer`. No migration cert is issued (revisit only if box-access-per-
> migration becomes friction — then add an Option-3 `prices_migrator` cert).

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
- Store each cert as a **single JSON bundle** `{cert,key,ca}` in AWS
  Secrets Manager (one secret per identity per env), named by the
  `MTLS_SECRET_NAME` 0052's client reads — **not** the two-secret
  cert/key split the 0050 G-note assumed. Reconcile 0011/0038 CDK to the
  single-bundle shape if it still emits `MTLS_CERT_SECRET_NAME` /
  `MTLS_KEY_SECRET_NAME` (see `notes/G-provisioning-plan.md` §5 + open
  item 2; likely a follow-up task).

### Step 5: Verify tenant isolation

For each env (dev → staging → prod):

- Connect via Caddy:443 with the prices cert; `SELECT version()` → 200.
- As `prices_writer`: `INSERT` into an existing `prices.*` table succeeds;
  `INSERT`/`SELECT` against `default.*` is **denied**; `CREATE TABLE
  prices.smoke …` is **denied** too (writer has no DDL — the Option-1
  proof). The tables themselves are created by 0051's loopback apply, not
  here.
- As `prices_reader`: `SELECT` from `prices.*` works; any write denied.

## Acceptance Criteria

- [x] `prices` database exists on the Hetzner CH box — created 2026-06-22
      under `default` admin on `ch-prod-01`; schema also applied (task 0051
      Step 4). See `notes/G-provisioning-plan.md` → Completion record.
- [~] `prices_writer` + `prices_reader` users exist via BE-repo
      `users.d/*.xml` (reproducible across deploys), with profile +
      quota scoping resource usage away from BE's `default.*`
      — **DEFINED 2026-06-23 by BE task 0314 (commit `87f24b76`)**:
      `services.xml` (+users, inline `<grants>`) + `quotas.xml`
      (+`prices_write`/`prices_read`) match our G-plan §1 byte-for-byte.
      Goes **live** only after the operator-run `ansible --tags app`
      (see AC below + §3); checkbox flips to [x] once deployed.
- [~] Caddy `CLICKHOUSE_CN_USER_MAP` maps the prices CNs to the prices
      users; unmapped CNs 403 — **entries PUSHED 2026-06-23**
      (`prices-ingestion-production:prices_writer`,
      `prices-api-production:prices_reader`) to `soroban/production/operator/env`;
      goes **live** on the §3 `ansible --tags app` run.
- [x] Per-env mTLS certs issued and stored in AWS Secrets Manager as a
      single `{cert,key,ca}` JSON bundle per identity (named by
      `MTLS_SECRET_NAME`, per 0052) — **DONE 2026-06-23**: CNs
      `prices-{ingestion,api}-production`, secrets
      `prices/production/clickhouse-mtls-prices-{ingestion,api}-production`
      (`eu-central-1`). Self-served from BE's CA; no hand-off needed.
- [ ] Smoke test confirms isolation: `prices.*` writable by
      `prices_writer`, `default.*` denied, `CREATE TABLE` denied to the
      writer; `prices_reader` read-only
- [x] DDL-apply identity for 0051 decided + documented — **Option 1:
      loopback `default` admin applies DDL; `prices_writer` is
      `write_no_ddl`** (Design Decisions → Emerged #1;
      `notes/G-provisioning-plan.md`)
- [x] Provisioning runbook authored — `notes/G-provisioning-plan.md`
      (drafted BE-PR XML/CN-map, SQL, cert/SM procedure, gated-action
      inventory). A per-env completion record is appended as steps run.

## Design Decisions

### Emerged

1. **Option 1 — loopback-admin DDL; no-DDL runtime certs** (chosen over
   Option 2 "scoped-DDL `prices_writer` applies over mTLS", the literal
   0050 G-note, and Option 3 "separate short-lived `prices_migrator`
   cert"). 0063 grants prices-api box admin access, so the loopback
   `default` path covers all DDL; the always-on runtime certs stay
   least-privilege (`write_no_ddl` / `read_only`, grants on `prices.*`
   only) — a leaked ingestion cert cannot `DROP TABLE prices.*` or touch
   `default.*`. Matches BE exactly (they removed their remote-DDL
   `migration_admin`/`partition_admin` users in BE 0241) and keeps the
   0051 loopback descope intact (no mTLS apply path). Trade-off: each
   schema change needs box access — acceptable for a low-churn OHLCV
   schema on the wholesale-idempotent apply; upgrade to Option 3's
   migrator cert only if that friction is ever felt. Full runbook +
   drafted artifacts in `notes/G-provisioning-plan.md`.
2. **No new `prices_write` profile; dedicated `prices_write`/`prices_read`
   quotas.** Profiles reuse BE's `write_no_ddl`/`read_only` (Option 1
   needs no DDL profile); quotas are prices-owned so prices can't draw
   down BE's `high_write`/`api_throttle` budget (mirrors BE's own
   `dev_read`-vs-`api_throttle` isolation). Quota naming is a minor
   BE-PR-time call (G-plan open item 4).
3. **Single `{cert,key,ca}` bundle secret per 0052**, not the two-secret
   cert/key split the G-note/0038-PR#34 assumed — reconcile the CDK
   (G-plan open item 2).

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
