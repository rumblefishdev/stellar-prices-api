---
id: "G-provisioning-plan"
title: "prices tenant provisioning runbook — Option 1 (loopback-admin DDL, no-DDL runtime certs)"
type: G
task: "0063"
status: mature
spawned_from: ["G-be-prices-db-rbac-ask"]
spawns: ["G-64k-sizing-remeasure", "G-be-rbac-pr-description", "S-schema-deploy-verification"]
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

### Single production box — what `{env}` means here

There is **one** Hetzner CH box: BE's `production` dedicated server `ch-prod-01`
(one `CH_DOMAIN`; `infra-hetzner/README.md`: "exactly once per environment
(`production`)"). There is **no separate dev/staging Hetzner box.** So the
`{env}` placeholder throughout this runbook is the **AWS-side** environment of
the *connecting client* (the Lambda stage), not a second CH box — every cert CN
terminates at the same one box.

> ⚠️ **Open implication — confirm with BE.** One box → one `prices` database.
> If per-env certs (`prices-ingestion-dev` vs `-production`) are all issued, they
> map to the **same** `prices_writer`/`prices_reader` users writing the **same**
> `prices.*` tables. Decide before issuing certs: (a) only `-production` CNs for
> now (recommended — least surface, matches the single box), or (b) per-env CNs
> with an agreed story for whether dev/staging clients should touch prod
> `prices.*` at all. Tracked as Open item 5.

---

## 1. BE-repo PR content (author locally → 🔒 PR to soroban-block-explorer)

All three edits are **additive** — no existing BE user/profile/quota changes.
Drafted here; do **not** push to the BE repo without approval.

### 1a. `crates/db-clickhouse/users.d/services.xml` — two new users

```xml
<!-- prices tenant (prices-api task 0063). Additive; cert-as-credential.
     Element shape matches BE's 5 live users (no_password / networks /
     profile / quota / access_management). The <grants> block is the ONE
     addition no current BE user has — see re-diff note + open item 6. -->
<prices_writer>
    <no_password/>
    <networks>
        <ip>127.0.0.1</ip>
        <ip>::1</ip>
        <ip>172.30.0.0/16</ip>   <!-- compose bridge, match existing users -->
    </networks>
    <profile>write_no_ddl</profile>   <!-- REUSED; allow_ddl=0 -->
    <quota>prices_write</quota>
    <access_management>0</access_management>   <!-- match BE's 5 users -->
    <grants>
        <!-- XML grants take NO "TO <user>" — the user is implied by context. -->
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
    <profile>read_only</profile>      <!-- REUSED; allow_ddl=0, 4 GiB / 30 s -->
    <quota>prices_read</quota>
    <access_management>0</access_management>
    <grants>
        <query>GRANT SELECT ON prices.*</query>
    </grants>
</prices_reader>
```

What is intentionally **absent** vs the G-note's writer grant: no `CREATE TABLE`,
`DROP TABLE`, `ALTER`, `TRUNCATE`. The writer appends + dedups (`OPTIMIZE` for
`ReplacingMergeTree`); structural change is the loopback admin's job. Even if a
grant slipped through, `write_no_ddl`'s `allow_ddl=0` blocks DDL at the profile.

> **Re-diff vs BE `develop` @ `8e4e705d` (2026-06-22).** The two users slot in
> alongside `dev_shared` / `galexie` / `api_reader` / `ingestion_writer` /
> `dev_read` and copy their element shape exactly, with two deliberate deltas:
> (1) `<access_management>0</access_management>` — added to match all 5 BE users;
> (2) a `<grants>` block — which **no** BE user currently has. BE's service users
> are unscoped (implicit all-database access; fine when `default` is the only DB).
> The `<grants>` block both scopes prices to `prices.*` **and** flips these users
> into explicit-grant mode, so `prices_writer` is denied `default.*` (the
> isolation AC). Corollary: BE's own unscoped users (`ingestion_writer` etc.) can
> still reach `prices.*` — acceptable, that's inside BE's own trust boundary; the
> isolation that matters (a leaked prices cert → `prices.*` only) holds.

### 1b. `crates/db-clickhouse/users.d/profiles.xml` — NO change

Option 1's core simplification: **no `prices_write` profile.** The writer reuses
`write_no_ddl` (8 GiB cap), the reader reuses `read_only` (4 GiB / 30 s). The
G-note's DDL-capable `prices_write` profile is not created.

### 1c. `crates/db-clickhouse/users.d/quotas.xml` — two new quotas

Dedicated quotas (not reusing BE's `high_write` / `api_throttle`) so prices can
never consume BE's per-service budget — the same isolation reasoning BE used to
split `dev_read` from `api_throttle`. **DECIDED 2026-06-22: dedicated naming
`prices_write` / `prices_read`** (open item 4 closed). Caps copied from the BE
siblings — **re-diff vs `develop` @ `8e4e705d` confirms an exact match to the
live values** (`prices_write` ≡ `high_write`'s `written_bytes`
`1125899906842624`; `prices_read` ≡ `api_throttle`'s `queries 10000` /
`result_rows 10000000000` / `read_rows 50000000000` / `read_bytes
1099511627776` / `execution_time 1000`, incl. the post-0243 `errors 0`):

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
Secrets). **DECIDED 2026-06-22: single-CN, no env suffix** (open item 5 closed) —
one prod box, one `prices` DB, so identity is per-role not per-env. **Append**
exactly two pairs — no checked-in file to edit:

```
prices-ingestion:prices_writer,prices-api:prices_reader
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

Single-CN (open item 5 closed): **one cert per identity, no `-{env}` suffix** —
the CN matches the §1d CN map. The same cert is reused by clients in every AWS
env (one prod box / one DB), so only two certs are issued total:

```bash
./infra-hetzner/ca/issue-client-cert.sh prices-ingestion
./infra-hetzner/ca/issue-client-cert.sh prices-api
```

**Storage format — single JSON bundle, per task 0052 (NOT two secrets).** 0052's
client reads one Secrets Manager secret holding `{cert, key, ca}` JSON, named by
`MTLS_SECRET_NAME`, fetched via the Lambda Parameters & Secrets Extension. Store
one bundle secret per identity per env:

```bash
# Secret PATH stays env-scoped (each AWS env account has its own SM), but the
# bundle CONTENT is the same single cert across envs (single-CN). One secret per
# identity per env, JSON {cert,key,ca}:
aws secretsmanager put-secret-value --secret-id prices/{env}/clickhouse-mtls-ingestion \
  --secret-string "$(jq -n --arg c "$(cat prices-ingestion.crt)" \
                            --arg k "$(cat prices-ingestion.key)" \
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

- `prices-ingestion` cert: `SELECT version()` → 200; `SHOW DATABASES`
  includes `prices`; `INSERT` into a `prices.*` table succeeds; the same against
  `default.*` → **ACCESS_DENIED**; `CREATE TABLE prices.smoke …` → **DENIED**
  (writer has no DDL — the Option-1 proof). Schema itself was applied in Step 4.
- `prices-api` cert: `SELECT` on `prices.*` works; any `INSERT`/`CREATE`
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
4. ✅ **CLOSED 2026-06-22 — Quota naming:** dedicated `prices_write`/`prices_read`
   (§1c). Caps re-diffed against `develop` @ `8e4e705d` — exact match to the live
   `high_write`/`api_throttle` values.
5. ✅ **CLOSED 2026-06-22 — CN scheme:** single-CN, no env suffix
   (`prices-ingestion` / `prices-api`). One prod box / one `prices` DB; identity
   is per-role. The same two certs are reused by clients in every AWS env; only
   the Secrets-Manager **path** is env-scoped (§1d, §5).
6. **`<grants>` element validity (OPEN — pre-PR check).** Re-diff confirms BE's
   live `services.xml` @ `8e4e705d` uses **no** `<grants>` on any of its 5 users
   (they're unscoped by design). The prices users are the first to use it, so
   before the PR: confirm the box's CH version applies user-XML `<grants>` at
   startup (supported since CH ~21.4; BE's local mirror is 25.6). If it does not,
   fall back to moving the two `GRANT … ON prices.*` statements into the
   `db-clickhouse-init` `init.sql` (run once under loopback admin) — same end
   state, just SQL-applied rather than XML-declared. Two syntax fixes already
   applied to §1a from the re-diff: XML grants carry **no** `TO <user>` clause,
   and `<access_management>0</access_management>` was added to match BE's users.
