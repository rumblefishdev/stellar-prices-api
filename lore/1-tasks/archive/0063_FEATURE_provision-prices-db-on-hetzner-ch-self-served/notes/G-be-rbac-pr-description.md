---
id: "G-be-rbac-pr-description"
title: "BE-repo PR description — add prices tenant RBAC (services.xml + quotas.xml + CN map)"
type: G
task: "0063"
status: mature
spawned_from: ["G-provisioning-plan"]
spawns: []
related_notes:
  - "G-provisioning-plan.md"
links:
  - "../../../../../../soroban-block-explorer/crates/db-clickhouse/users.d/services.xml"
  - "../../../../../../soroban-block-explorer/crates/db-clickhouse/users.d/quotas.xml"
---

# BE-repo PR description (paste-ready)

> Target repo: `soroban-block-explorer`. Re-diffed against `develop` @ `8e4e705d`
> (2026-06-22). All edits additive — no existing BE user/profile/quota touched.
> Source runbook: prices-api task 0063 [[G-provisioning-plan]].

---

## Title

```
feat(clickhouse): add prices tenant RBAC (prices_writer / prices_reader)
```

## Body

### Why this PR is needed (context for reviewers / AI agents)

**prices-api** is a separate service that indexes Stellar DEX/AMM prices and
writes per-source OHLCV candles to ClickHouse. Per **ADR 0007**, it does **not**
run its own database — it lands its data in **this** cluster as a second tenant
(a dedicated `prices` database), so BE's in-cluster analytics (e.g. the
LP-analytics `price_usd_series` JOIN) can read prices data without a network hop.

The reason this requires a PR **in your repo** specifically: on this deployment,
ClickHouse tenant **users / quotas are defined as checked-in XML** under
`crates/db-clickhouse/users.d/` and delivered to the box by Ansible — they are
**not** created with `CREATE USER` SQL. So an ad-hoc `CREATE USER prices_writer`
on the box would be wiped/drift on the next deploy. The only durable,
deploy-reproducible way to create the prices service identities is to add them
to this XML. That one piece is the sole thing prices-api cannot self-serve.

Everything else is already handled on the prices-api side: BE granted box-admin
access (your task 0227), and prices-api creates the `prices` database, applies
its own schema over loopback admin, and issues/stores its mTLS client certs
itself. **This PR is purely the access-control config** those certs map onto.

### Summary

Adds a second ClickHouse tenant — **`prices`** — to the shared Hetzner cluster,
owned by the prices-api service (their task 0063, per ADR 0007 §3.5 multi-tenant
primitives). This is **access-control only**: two scoped users, two dedicated
quotas, and two Caddy CN→user mappings. The `prices` database itself and its
schema are created/owned by prices-api over loopback admin (not in this PR).

All changes are **additive** and mirror the existing per-service tenant pattern
(`task 0240`). No change to `default`, `dev_shared`, `galexie`, `api_reader`,
`ingestion_writer`, or `dev_read`.

### What changes

| File | Change |
|------|--------|
| `crates/db-clickhouse/users.d/services.xml` | + `prices_writer`, `prices_reader` users |
| `crates/db-clickhouse/users.d/quotas.xml` | + `prices_write`, `prices_read` quotas |
| `crates/db-clickhouse/users.d/profiles.xml` | **no change** — reuses `write_no_ddl` + `read_only` |
| `CLICKHOUSE_CN_USER_MAP` (deploy env var, not a file) | append 2 CN pairs (see Deploy step) |

### Isolation model

- **`prices_writer`** — profile `write_no_ddl` (your existing 8 GiB / `allow_ddl=0`
  profile), quota `prices_write`, granted **only** `SELECT, INSERT, OPTIMIZE ON
  prices.*`. **No** `CREATE`/`DROP`/`ALTER`/`TRUNCATE` and no access to `default.*`.
  Schema changes are applied by prices-api over loopback admin, not by this user.
- **`prices_reader`** — profile `read_only` (4 GiB / 30 s), quota `prices_read`,
  granted **only** `SELECT ON prices.*`.
- **Dedicated quotas** (`prices_write`/`prices_read`, caps copied verbatim from
  your `high_write`/`api_throttle`) so prices traffic can never draw down a BE
  service's per-user budget — same isolation reasoning behind your `dev_read`
  vs `api_throttle` split.

### ⚠️ One thing to confirm before merge — first use of `<grants>` in `services.xml`

Your current service users are unscoped (implicit all-database access — correct
when `default` is the only DB). The prices users are the **first** to carry an
inline `<grants>` block, which is what both scopes them to `prices.*` and flips
them into explicit-grant mode (so `prices_writer` is denied `default.*`).

Please confirm the box's ClickHouse version applies user-XML `<grants>` at
startup (supported since ~21.4; should be a non-issue on the current image). If
you'd rather not introduce inline grants here, we can instead apply the two
`GRANT … ON prices.*` statements from the prices `db-clickhouse-init` step under
loopback admin — identical end state, just SQL-applied. Happy to go whichever
way you prefer.

> Note: your own unscoped service users (`ingestion_writer`, etc.) will be able
> to reach `prices.*`. That's inside your trust boundary and expected; the
> isolation that matters here is that the prices certs are confined to
> `prices.*` and cannot touch `default.*`.

### Deploy

After merge, the prices CNs must be appended to the `CLICKHOUSE_CN_USER_MAP`
deploy env var (operator shell / GH Secrets — single-CN, no env suffix):

```
prices-ingestion:prices_writer,prices-api:prices_reader
```

Then `ansible-playbook … --tags app` so CH reloads `users.d/*` and Caddy reloads
the CN map. Unmapped CNs continue to 403 at Caddy (fail-closed); no prices CN
maps to `default`/`dev_shared`. (prices-api will coordinate the playbook run /
cert issuance with you.)

### Test plan (run by prices-api post-deploy)

Via Caddy:443 with the issued prices certs:
- `prices-ingestion` cert → `INSERT` into `prices.*` succeeds; `INSERT`/`SELECT`
  on `default.*` → `ACCESS_DENIED`; `CREATE TABLE prices.smoke …` → `ACCESS_DENIED`.
- `prices-api` cert → `SELECT` on `prices.*` succeeds; any write → `ACCESS_DENIED`.

### Reviewer checklist

- [ ] `<grants>` applies at startup on the running CH version (or agree on the
      `init.sql` fallback)
- [ ] Quota names `prices_write`/`prices_read` OK (vs reusing existing names)
- [ ] Networks ACL (`127.0.0.1` / `::1` / `172.30.0.0/16`) matches the compose bridge
- [ ] Single-CN scheme OK (one prod box / one `prices` DB)

---

## File diffs

### `crates/db-clickhouse/users.d/services.xml` — add inside `<users>…</users>`

```xml
        <!-- prices tenant (prices-api task 0063). Additive; cert-as-credential.
             Scoped to prices.* via <grants> (first user here to use it) so a
             leaked prices cert cannot touch default.* or run DDL. -->
        <prices_writer>
            <no_password/>
            <networks>
                <ip>127.0.0.1</ip>
                <ip>::1</ip>
                <ip>172.30.0.0/16</ip>
            </networks>
            <profile>write_no_ddl</profile>
            <quota>prices_write</quota>
            <access_management>0</access_management>
            <grants>
                <query>GRANT SELECT, INSERT, OPTIMIZE ON prices.*</query>
            </grants>
        </prices_writer>

        <prices_reader>
            <no_password/>
            <networks>
                <ip>127.0.0.1</ip>
                <ip>::1</ip>
                <ip>172.30.0.0/16</ip>
            </networks>
            <profile>read_only</profile>
            <quota>prices_read</quota>
            <access_management>0</access_management>
            <grants>
                <query>GRANT SELECT ON prices.*</query>
            </grants>
        </prices_reader>
```

### `crates/db-clickhouse/users.d/quotas.xml` — add inside `<quotas>…</quotas>`

```xml
        <!-- prices tenant (prices-api task 0063). Caps copied from high_write /
             api_throttle; dedicated names so prices never draws down a BE
             service's per-user budget. -->
        <prices_write>   <!-- mirrors high_write -->
            <interval>
                <duration>3600</duration>
                <queries>0</queries>
                <errors>0</errors>
                <result_rows>0</result_rows>
                <read_rows>0</read_rows>
                <execution_time>0</execution_time>
                <written_bytes>1125899906842624</written_bytes>
            </interval>
        </prices_write>

        <prices_read>    <!-- mirrors api_throttle -->
            <interval>
                <duration>3600</duration>
                <queries>10000</queries>
                <errors>0</errors>
                <result_rows>10000000000</result_rows>
                <read_rows>50000000000</read_rows>
                <read_bytes>1099511627776</read_bytes>
                <execution_time>1000</execution_time>
            </interval>
        </prices_read>
```

### `crates/db-clickhouse/users.d/profiles.xml`

No change. `prices_writer` reuses `write_no_ddl`; `prices_reader` reuses `read_only`.
