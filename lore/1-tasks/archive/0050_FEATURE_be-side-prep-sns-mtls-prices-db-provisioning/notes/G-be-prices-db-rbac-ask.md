---
id: "G-be-prices-db-rbac-ask"
title: "BE-side ask — prices DB + RBAC users/grants + mTLS cert (ready-to-implement)"
type: G
task: "0050"
status: mature
spawned_from: []
spawns: []
related_notes:
  - "G-be-sns-fanout-ask.md"
  - "G-prices-disk-footprint-for-be.md"
links:
  - "../../active/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
---

# BE-side ask — `prices` DB + RBAC + mTLS cert

> **Audience:** BE team (soroban-block-explorer infra).
> **Status:** ready-to-implement. Scopes **items 2 + 3** of task 0050
> (mTLS cert issuance + `prices.*` DB/user/quota) — the sibling
> `G-be-sns-fanout-ask.md` covers item 1 (SNS fan-out).
> **Why now:** prices-api task 0038 has authored its AWS side (PR #34):
> the ledger-processor `lambda.Function`, a prices-owned execution role,
> and an IAM grant scoped to two prices-owned Secrets Manager secrets
> (`prices/{env}/clickhouse-mtls-cert` / `-key`) fetched via the
> Parameters & Secrets extension. It is **prepare-only** until BE issues
> the cert and provisions the DB.
> **Gating:** unlike the SNS item, this is gated on **BE 0227** (Hetzner
> Ansible playbook) + **task 0047** (throughput verification) — it
> touches the live CH box, not just S3/SNS/SSM.

---

## TL;DR

Three additive changes on BE's Hetzner CH box, all in the same files BE
already uses for `galexie` / `ingestion_writer`:

1. **`CREATE DATABASE prices`** (admin, once).
2. **Two proxy-trust CH users** in `users.d/services.xml` —
   `prices_writer` (full DDL+DML on `prices.*`) and `prices_reader`
   (SELECT on `prices.*`) — each `<no_password/>`, loopback+bridge
   networks, with **grants scoped to `prices.*` and nothing on
   `default.*`**. Plus one capped `prices_write` profile.
3. **Caddy CN→user map** entries + **two issued mTLS certs**
   (`issue-client-cert.sh`) handed to prices-api via the password
   manager.

No change to any existing BE user, the `default` admin path, backups
(beyond optionally extending the snapshot scope — see §4), or the box's
resource tuning. The `prices` data footprint (1-min OHLCV candles) is a
rounding error against BE's full-chain index — **~0.45 GB/year**,
empirically measured; concrete numbers + horizons in the sibling
`G-prices-disk-footprint-for-be.md`.

---

## Architectural context — why this shape

prices-api is a **second tenant** on shared infrastructure: the same
Hetzner CH box *and* the same AWS account as BE (ADR 0007). The goal is
for prices to own read/write/edit on `prices.*` while being **unable to
touch BE's `default.*` tables**. Two independent isolation layers give
that, and the honest trust boundary is recorded so nobody over-claims.

**The Lambda's identity to CH is its mTLS client cert, not an IAM role
or a DB password.** The cert CN is what Caddy maps to a CH user. So
"which Lambda may write `prices.*`" reduces to "which Lambda holds the
cert," and the chain is:

```
prices Lambda (shared AWS account)
  exec role: GetSecretValue scoped to prices/{env}/clickhouse-mtls-{cert,key}   ← Layer 1 (AWS IAM)
  → presents CN=prices-ingestion-{env} to Caddy:443
Caddy: verify vs BE CA → CN map → X-ClickHouse-User: prices_writer
ClickHouse: prices_writer granted ONLY on prices.*                              ← Layer 2 (CH RBAC)
  → writes land; default.* = ACCESS_DENIED
```

- **Layer 1 — AWS IAM (already built, PR #34).** The prices Lambda role
  can read *only* the two prices-owned cert/key secrets (exact ARNs,
  wildcard-guarded in `infra/src/lib/mtls.ts`). BE's Lambda roles can't
  read them; the prices role can't read BE's `galexie`/`ingestion` cert.
  No Lambda in the shared account can present prices' CH identity unless
  it is the prices Lambda.
- **Layer 2 — CH RBAC (this ask, the backstop).** ClickHouse grants are
  **default-deny**: a user touches only what it is granted. With grants
  *only* on `prices.*`, every SELECT/INSERT/ALTER/DROP against
  `default.*` is refused at the engine. Even a leaked prices cert is
  blast-radius-capped to `prices.*` — BE data stays untouchable. The
  wall is two-way: BE's users have no grant on `prices.*` either.

**Honest trust boundary.** IAM scoping isolates prices from BE's
*services*, not from an **AWS account-admin**; CH grants isolate prices
from BE's *queries*, not from a **box-admin** (SSH + `default`). Both
admins remain in prices' trust base — identical in shape to today's
shared-tenancy posture, and the CH grant always caps the blast radius
regardless of who holds the cert. Hard isolation from BE *admins* would
require a separate AWS account / separate box — explicitly **not** what
"share the Hetzner" means here.

**Resource isolation ≠ data isolation.** Grants stop data damage
absolutely; they do **not** stop a heavy prices query from competing for
the shared 100 GiB / 24-thread pool. That is the job of the capped
`prices_write` profile (`max_memory_usage`, `max_execution_time`). Note
per the RBAC doc's known limitation that CH **quotas are not enforced on
the Caddy-proxied path** (CH 26.3) — so the per-query **profile** caps,
not quotas, are the real noisy-neighbour guard. Quotas are still defined
for forward-compat / the host-side path.

---

## 1. `prices` database + users + grants

Applied wherever BE defines the existing proxy-trust users
(`users.d/services.xml` `<grants>` blocks, or the `db-clickhouse-init`
sidecar `init.sql` — **BE's call**; the GRANT statements below are the
source of truth, the XML is the suggested placement to match
`api_reader` / `ingestion_writer`).

```sql
-- Admin (default), once:
CREATE DATABASE IF NOT EXISTS prices;
```

`prices_writer` — the ledger processor (0038) and future writers (0039):

```xml
<!-- users.d/services.xml -->
<prices_writer>
    <no_password/>
    <networks>
        <ip>127.0.0.1</ip>
        <ip>::1</ip>
        <ip>172.30.0.0/16</ip>   <!-- pinned compose bridge -->
    </networks>
    <profile>prices_write</profile>
    <quota>high_write</quota>     <!-- reuse existing; not enforced on proxy path -->
    <grants>
        <query>GRANT SELECT, INSERT, ALTER, CREATE TABLE, DROP TABLE, OPTIMIZE, TRUNCATE ON prices.* TO prices_writer</query>
    </grants>
</prices_writer>
```

`prices_reader` — the API read handlers (0040):

```xml
<prices_reader>
    <no_password/>
    <networks>
        <ip>127.0.0.1</ip>
        <ip>::1</ip>
        <ip>172.30.0.0/16</ip>
    </networks>
    <profile>read_only</profile>  <!-- reuse existing 4 GiB / 30 s read profile -->
    <quota>api_throttle</quota>   <!-- reuse existing -->
    <grants>
        <query>GRANT SELECT ON prices.* TO prices_reader</query>
    </grants>
</prices_reader>
```

New capped write profile (`users.d/profiles.xml`) — `readonly=0` +
`allow_ddl=1` so the writer can manage its own schema, but memory/exec
capped so it can't starve `default.*`. `access_management=0` so it can't
self-escalate:

```xml
<!-- users.d/profiles.xml -->
<prices_write>
    <readonly>0</readonly>
    <allow_ddl>1</allow_ddl>
    <access_management>0</access_management>
    <max_memory_usage>8589934592</max_memory_usage>   <!-- 8 GiB, matches write_no_ddl -->
    <max_execution_time>60</max_execution_time>
</prices_write>
```

> `allow_ddl=1` enables DDL *at all*; the **grant** still constrains it
> to `prices.*`. `prices_writer` therefore can `CREATE/DROP TABLE
> prices.x` but is `ACCESS_DENIED` on `DROP TABLE default.blocks` and
> `DROP DATABASE default`. Final user/profile/quota **names are BE's
> call** (parent README §Step 2).

**Why a writer with DDL, not just INSERT:** prices-api owns its schema
and runs its own table migrations — the same `prices_writer` identity
creates the `prices.*` tables (it has `CREATE TABLE ON prices.*`), so
"tables first, then Lambda writes" needs no extra admin cert. BE only
runs the one-time `CREATE DATABASE`.

---

## 2. Caddy CN → CH user map (`CLICKHOUSE_CN_USER_MAP`)

Append two entries (the `<cn>:<ch_user>` form, per the RBAC doc), then
`ansible-playbook ... --tags caddy_reload`:

| Caddy CN (verified by mTLS) | Mapped CH user  | Consumer (prices-api)                  |
| --------------------------- | --------------- | -------------------------------------- |
| `prices-ingestion-{env}`    | `prices_writer` | Ledger Processor Lambda (task 0038)    |
| `prices-api-{env}`          | `prices_reader` | API read handlers (task 0040, future)  |

> The map IS the allowlist — an unmapped CN yields empty `{ch_user}` →
> 403 at Caddy. No prices CN maps to `default` or `dev_shared`, so the
> prices certs can never reach an admin user.

---

## 3. mTLS cert issuance

For each env (dev → staging → prod), issue both certs from BE's CA and
hand the bundles to prices-api via the password manager:

```bash
./infra-hetzner/ca/issue-client-cert.sh prices-ingestion-{env}
./infra-hetzner/ca/issue-client-cert.sh prices-api-{env}
```

prices-api loads each `prices-ingestion-{env}` cert+key into the
prices-owned Secrets Manager secrets `prices/{env}/clickhouse-mtls-cert`
and `-key` (already referenced by `ComputeStack` env vars
`MTLS_CERT_SECRET_NAME` / `MTLS_KEY_SECRET_NAME`). Per the SSM key
contract (`/platform/{env}/*` BE-owned, `/prices/{env}/*` prices-owned —
**identifiers only, never bulk trust material**), only the secret
**names** travel in env/SSM; the cert/key bytes live in Secrets Manager,
never SSM. 1-year manual rotation cadence (BE Cluster C agreement,
parent README item 2).

---

## What BE does NOT need to do

- **No prices Lambda ARN / no cross-account anything** — same AWS
  account; prices scopes its own role to its own secrets (PR #34).
- **No change to existing users** (`default`, `galexie`, `api_reader`,
  `ingestion_writer`, `dev_shared`) — this is purely additive.
- **No resource re-tuning** — `prices_write` is capped *below* the
  existing write profile; the shared memory ceiling is untouched.

---

## 4. Backup scope (one decision to make)

The DR flow restores `RESTORE DATABASE default` (Hetzner README §Total
loss). Two options, **BE's call**:

- **(a)** extend the snapshot/restore to include `prices` (BE owns
  prices' durability), or
- **(b)** prices-api accepts that `prices.*` is **not** in BE's backup
  set and treats it as re-derivable (live ingestion can replay from the
  Galexie ledger objects + backfill streams).

Given `prices.*` is re-derivable from the ledger history, **(b)** is the
low-effort default; flag it explicitly so it is a decision, not an
oversight.

---

## Verification (joint) — mirrors parent README §Step 3

Per env, using the issued `prices-ingestion-{env}` cert via Caddy:443:

- `SELECT version()` → 200 + CH version (cert + CN map wired).
- `SHOW DATABASES` includes `prices`.
- `CREATE TABLE prices.smoke (x UInt8) ENGINE = Memory` → **succeeds**;
  same against `default.smoke` → **ACCESS_DENIED** (the isolation
  proof). Drop the throwaway table.
- With the `prices-api-{env}` reader cert: `SELECT` on `prices.*`
  succeeds; `INSERT`/`CREATE` on `prices.*` → **ACCESS_DENIED**.

---

## Cost

Item 2+3 add **€0/month** incremental Hetzner cost — shared box, shared
account; the marginal disk/RAM/CPU for OHLCV is negligible. Cost is the
one-time BE provisioning + the ~1–2%/env/mo pro-rata cost-share already
tracked in the parent README (Cluster D commercial follow-up). The only
infra alternative with real isolation from BE *admins* (separate AWS
account or a second Hetzner box, ~€25–110/mo) was considered and
rejected as contrary to the shared-tenancy decision in ADR 0007.
