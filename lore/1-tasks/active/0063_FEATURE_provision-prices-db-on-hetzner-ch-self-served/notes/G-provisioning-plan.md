---
id: "G-provisioning-plan"
title: "prices tenant provisioning runbook — Option 1 (loopback-admin DDL, no-DDL runtime certs)"
type: G
task: "0063"
status: mature
spawned_from: ["G-be-prices-db-rbac-ask"]
spawns: []
related_notes:
  - "../../../backlog/0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning/notes/G-be-prices-db-rbac-ask.md"
links:
  - "../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../../../../soroban-block-explorer/crates/db-clickhouse/users.d/services.xml"
  - "../../../../../../soroban-block-explorer/crates/db-clickhouse/users.d/profiles.xml"
  - "../../../../../../soroban-block-explorer/crates/db-clickhouse/users.d/quotas.xml"
  - "../../../../../../soroban-block-explorer/infra-hetzner/ca/issue-client-cert.sh"
---

# prices tenant provisioning runbook — Option 1

> **Decision (2026-06-18):** Option 1 — **DDL is the box `default` admin over
> loopback; the runtime certs (`prices_writer` / `prices_reader`) carry no DDL.**
> Chosen over the G-note's scoped-DDL writer (Option 2) and the hybrid migrator
> cert (Option 3). Rationale: 0063 grants prices-api box admin access, so the
> loopback path covers DDL; runtime certs stay least-privilege (a leaked
> ingestion cert cannot `DROP TABLE prices.*`); matches BE exactly (they
> *removed* their remote-DDL users in BE 0241); keeps the 0051 loopback descope
> intact (no mTLS apply path). Upgrade to Option 3's `prices_migrator` cert only
> if box-access-per-migration ever becomes friction.

This is the **ready-to-execute** runbook. Every step is either local authoring
or a 🔒 **gated remote action** (Hetzner / AWS / BE-repo) that needs explicit
per-session operator approval before running.

---

## 0. Identity model (recap)

| Job | Identity | Path | Powers |
|---|---|---|---|
| Install / migrate schema | `default` admin | loopback on the box (SSH) | full DDL |
| Ingest candles (0038/0039) | cert `CN=prices-ingestion-{env}` → `prices_writer` | Caddy:443 mTLS from AWS | `SELECT, INSERT, OPTIMIZE ON prices.*` |
| API reads (0040) | cert `CN=prices-api-{env}` → `prices_reader` | Caddy:443 mTLS from AWS | `SELECT ON prices.*` |

No external CN maps to `default`; admin is reachable only from the box.

---

## 1. BE-repo PR content (author locally → 🔒 PR to soroban-block-explorer)

All three edits are **additive** — no existing BE user/profile/quota changes.
Drafted here; do **not** push to the BE repo without approval.

### 1a. `crates/db-clickhouse/users.d/services.xml` — two new users

```xml
<!-- prices tenant (prices-api task 0063). Additive; cert-as-credential. -->
<prices_writer>
    <no_password/>
    <networks>
        <ip>127.0.0.1</ip>
        <ip>::1</ip>
        <ip>172.30.0.0/16</ip>   <!-- compose bridge, match existing users -->
    </networks>
    <profile>write_no_ddl</profile>   <!-- REUSED; allow_ddl=0 -->
    <quota>prices_write</quota>
    <grants>
        <query>GRANT SELECT, INSERT, OPTIMIZE ON prices.* TO prices_writer</query>
    </grants>
</prices_writer>

<prices_reader>
    <no_password/>
    <networks>
        <ip>127.0.0.1</ip>
        <ip>::1</ip>
        <ip>172.30.0.0/16</ip>
    </networks>
    <profile>read_only</profile>      <!-- REUSED; allow_ddl=0, 4 GiB / 30 s -->
    <quota>prices_read</quota>
    <grants>
        <query>GRANT SELECT ON prices.* TO prices_reader</query>
    </grants>
</prices_reader>
```

What is intentionally **absent** vs the G-note's writer grant: no `CREATE TABLE`,
`DROP TABLE`, `ALTER`, `TRUNCATE`. The writer appends + dedups (`OPTIMIZE` for
`ReplacingMergeTree`); structural change is the loopback admin's job. Even if a
grant slipped through, `write_no_ddl`'s `allow_ddl=0` blocks DDL at the profile.

### 1b. `crates/db-clickhouse/users.d/profiles.xml` — NO change

Option 1's core simplification: **no `prices_write` profile.** The writer reuses
`write_no_ddl` (8 GiB cap), the reader reuses `read_only` (4 GiB / 30 s). The
G-note's DDL-capable `prices_write` profile is not created.

### 1c. `crates/db-clickhouse/users.d/quotas.xml` — two new quotas

Dedicated quotas (not reusing BE's `high_write` / `api_throttle`) so prices can
never consume BE's per-service budget — the same isolation reasoning BE used to
split `dev_read` from `api_throttle`. Caps copied from the BE siblings:

```xml
<prices_write>   <!-- mirror high_write: writes unbounded, written_bytes sanity cap -->
    <interval>
        <duration>3600</duration>
        <queries>0</queries> <errors>0</errors> <result_rows>0</result_rows>
        <read_rows>0</read_rows> <execution_time>0</execution_time>
        <written_bytes>1125899906842624</written_bytes>
    </interval>
</prices_write>

<prices_read>    <!-- mirror api_throttle -->
    <interval>
        <duration>3600</duration>
        <queries>10000</queries> <errors>0</errors>
        <result_rows>10000000000</result_rows>
        <read_rows>50000000000</read_rows>
        <read_bytes>1099511627776</read_bytes>
        <execution_time>1000</execution_time>
    </interval>
</prices_read>
```

> Quotas are **not enforced on the Caddy-proxied path** (CH limitation noted in
> the G-note), so the real noisy-neighbour guard is the per-query *profile* cap
> (`read_only` 4 GiB/30 s). Quotas are defined for forward-compat + the host path.
> Reuse-vs-dedicated quota naming is a minor call — confirm with BE at PR time.

### 1d. Caddy CN→user map (env, not a file)

`clickhouse_cn_user_pairs` is derived from the `CLICKHOUSE_CN_USER_MAP` env var
(`group_vars/all.yml:75`), supplied at playbook time (operator shell / GH
Secrets). **Append** two pairs per env — no checked-in file to edit:

```
prices-ingestion-{env}:prices_writer,prices-api-{env}:prices_reader
```

Unmapped CN → `__unmapped__` → 403 at Caddy (fail-closed). No prices CN maps to
`default`/`dev_shared`.

---

## 2. Create the database (🔒 Hetzner, loopback admin, once)

```sql
-- as `default` admin over loopback (SSH to box):
CREATE DATABASE IF NOT EXISTS prices;
```

`CREATE DATABASE` is a box-admin one-shot; it is not granted to any scoped user.

---

## 3. Deploy the RBAC (🔒 Hetzner, Ansible)

After the 1a–1d PR merges in the BE repo + `CLICKHOUSE_CN_USER_MAP` is extended:

```bash
ansible-playbook -i inventory.ini site.yml --tags app   # CH picks up users.d, Caddy picks up the CN map
```

Prepare-only: coordinate the actual run with BE / explicit approval.

---

## 4. Apply the schema (🔒 Hetzner, loopback — this is task 0051 Step 4)

On the box (or SSH tunnel to `localhost:8123`), as `default` admin, run the
existing **plaintext** apply — no mTLS, no DDL cert:

```bash
CLICKHOUSE_URL=http://localhost:8123 \
  cargo run -p prices-clickhouse --bin prices-clickhouse-init -- --rollups
# (or: clickhouse-client --queries-file=init.sql / seed.sql / views.sql)
```

Idempotent (`CREATE … IF NOT EXISTS`). Owned by 0051; listed here for the full
provisioning sequence.

---

## 5. Issue + store the mTLS certs (🔒 Hetzner CA + 🔒 AWS)

Per env, from BE's CA (needs the CA private key — **if box-admin does not include
CA-key access, this is a BE ask**: BE runs the script, hands over the bundle):

```bash
./infra-hetzner/ca/issue-client-cert.sh prices-ingestion-{env}
./infra-hetzner/ca/issue-client-cert.sh prices-api-{env}
```

**Storage format — single JSON bundle, per task 0052 (NOT two secrets).** 0052's
client reads one Secrets Manager secret holding `{cert, key, ca}` JSON, named by
`MTLS_SECRET_NAME`, fetched via the Lambda Parameters & Secrets Extension. Store
one bundle secret per identity per env:

```bash
# one secret per identity per env, JSON {cert,key,ca}:
aws secretsmanager put-secret-value --secret-id prices/{env}/clickhouse-mtls-ingestion \
  --secret-string "$(jq -n --arg c "$(cat prices-ingestion-{env}.crt)" \
                            --arg k "$(cat prices-ingestion-{env}.key)" \
                            --arg a "$(cat ca.crt)" '{cert:$c,key:$k,ca:$a}')"
```

> ⚠️ **Cross-task reconciliation:** 0038's earlier AWS side (PR #34) + the 0050
> G-note assumed **two** secrets (`…-cert` / `…-key`). 0052 standardised on the
> **single-bundle** shape. Whichever 0011/0038 CDK provisions the secret + env
> vars must match 0052: `MTLS_SECRET_NAME` → one `{cert,key,ca}` JSON secret,
> plus `CH_DOMAIN`. Flag a follow-up to align the CDK if it still emits the
> two-secret `MTLS_CERT_SECRET_NAME` / `MTLS_KEY_SECRET_NAME` pair.

Secrets-Manager bytes only; per the SSM key contract, only secret **names** ride
in env/SSM, never the cert/key material. 1-year manual rotation cadence.

---

## 6. Verify tenant isolation (🔒 Hetzner, per env)

Using the issued certs via Caddy:443:

- `prices-ingestion-{env}` cert: `SELECT version()` → 200; `SHOW DATABASES`
  includes `prices`; `INSERT` into a `prices.*` table succeeds; the same against
  `default.*` → **ACCESS_DENIED**; `CREATE TABLE prices.smoke …` → **DENIED**
  (writer has no DDL — the Option-1 proof). Schema itself was applied in Step 4.
- `prices-api-{env}` cert: `SELECT` on `prices.*` works; any `INSERT`/`CREATE`
  → **ACCESS_DENIED**.

---

## Gated-action inventory (each needs explicit approval before running)

| # | Action | Target | Step |
|---|---|---|---|
| G1 | `clickhouse-client … SHOW DATABASES` (confirm admin) | 🔒 Hetzner | 0063 Step 1 |
| G2 | Open PR to `soroban-block-explorer` (1a–1d) | GitHub (BE repo) | §1 |
| G3 | `CREATE DATABASE IF NOT EXISTS prices` | 🔒 Hetzner | §2 |
| G4 | `ansible-playbook … --tags app` | 🔒 Hetzner | §3 |
| G5 | Apply schema (plaintext loopback) | 🔒 Hetzner | §4 / 0051 |
| G6 | `issue-client-cert.sh` (CA key) | 🔒 Hetzner CA | §5 |
| G7 | `aws secretsmanager put-secret-value` | 🔒 AWS | §5 |

---

## Open items / decisions to confirm

1. **CA-key access** — does box-admin include the CA private key? If not, Step 5
   issuance is a BE ask (BE runs `issue-client-cert.sh`, hands over the bundle).
2. **Secret shape reconciliation** — align 0011/0038 CDK to 0052's single-bundle
   `MTLS_SECRET_NAME` (see §5 warning). Likely a follow-up task.
3. **Backup scope** (G-note §4) — recommend **(b)**: `prices.*` is re-derivable
   from ledger history, so accept it is outside BE's `RESTORE DATABASE default`
   set rather than extending BE's snapshot. Flag as a decision, not an oversight.
4. **Quota naming** — dedicated `prices_write`/`prices_read` (recommended, §1c)
   vs reusing BE's `high_write`/`api_throttle`. Confirm at BE PR time.
