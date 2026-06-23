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
| Ingest candles (0038/0039) | cert `CN=prices-ingestion-production` → `prices_writer` | Caddy:443 mTLS from AWS | `SELECT, INSERT, OPTIMIZE ON prices.*` |
| API reads (0040) | cert `CN=prices-api-production` → `prices_reader` | Caddy:443 mTLS from AWS | `SELECT ON prices.*` |

No external CN maps to `default`; admin is reachable only from the box.

### Single production box — what `{env}` means here

There is **one** Hetzner CH box: BE's `production` dedicated server `ch-prod-01`
(one `CH_DOMAIN`; `infra-hetzner/README.md`: "exactly once per environment
(`production`)"). There is **no separate dev/staging Hetzner box.** prices-api's
own CDK is likewise single-env: `EnvironmentConfig.envName` is the literal
`'production'` (`infra/src/lib/types.ts:33`, validated 75-76) — there is no `dev`
/ `staging` AWS stage either.

> ✅ **Resolved 2026-06-23 (open item 5, re-opened then settled).** CNs are
> **env-suffixed, mirroring BE** (`prices-ingestion-production` /
> `prices-api-production`), not the bare `prices-ingestion` form. Rationale:
> prices shares **BE's CA**, so prices CNs live in the *same CA namespace* as
> BE's `lambda-ingestion-production` etc.; the `-production` suffix keeps them
> globally unique + self-documenting there, and matches BE's convention exactly.
> One box → one `prices` DB still holds: both CNs terminate at the same box and
> map to the same two users; there is simply no second env to disambiguate.

---

## 1. BE-repo RBAC — ✅ SHIPPED by BE task 0314 (2026-06-23)

> ✅ **DONE — BE task 0314, commit `87f24b76`** (branch
> `feat/0314_prices-tenant-clickhouse-rbac`, merged). BE added `prices_writer` /
> `prices_reader` to `services.xml` (with inline `<grants>`) and
> `prices_write` / `prices_read` to `quotas.xml` — verified **byte-for-byte
> identical** to the §1a/§1c drafts below (profiles unchanged, §1b). This was the
> one genuinely BE-owned item for 0063. **Not yet live on the box:** the change
> takes effect only on the operator-run `ansible --tags app` (§3). The drafts
> below are retained as the as-built record.

All three edits are **additive** — no existing BE user/profile/quota changes.

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
(`group_vars/all.yml:104`), supplied at playbook time (operator shell / GH
Secrets). The Caddy template renders each pair into
`map {http.request.tls.client.subject} {ch_user}` as `"CN=<cn>" <user>`
(`cn_user_map.snippet.j2`); unmapped → `__unmapped__` → 403.
**DECIDED 2026-06-23: env-suffixed CNs, mirroring BE** (open item 5). **Append**
exactly two pairs — no checked-in file to edit:

```
prices-ingestion-production:prices_writer,prices-api-production:prices_reader
```

> ⚠️ **Heads-up — BE doc lists these CNs *bare* (deploy-time mismatch to avoid).**
> BE's `docs/architecture/security/clickhouse-rbac.md` CN→user table (added by
> task 0314) lists the prices rows as `prices-ingestion` / `prices-api` (no
> `-<environment>`) — transcribed from our *pre-reversal* ask. **Every other**
> CN in that same BE table is `-<environment>` (`galexie-<environment>`,
> `lambda-api-<environment>`, …) and the doc says certs "get a CN matching their
> AWS role," so our env-suffixed form is the one that matches BE's own dominant
> convention; the bare prices rows are the anomaly. We own the CN-map append
> (`CLICKHOUSE_CN_USER_MAP` in `~/.config/soroban-prod.env`), so use the
> **env-suffixed** pairs above — they must match the issued cert CNs (§5) and our
> secret names. Optional cleanup: ask BE to fix their two doc rows to
> `prices-{ingestion,api}-<environment>`.

#### Procedure — edit + push the map (🔒 AWS, operator-run)

`CLICKHOUSE_CN_USER_MAP` lives **inside** the operator env secret
`soroban/production/operator/env` (the dotenv fetched to
`~/.config/soroban-prod.env` and `source`d). So "append" = edit that one var in
the file, then `put-secret-value` the **whole** file back (preserves the other
keys). Re-fetch first so a concurrent edit isn't clobbered:

```bash
export AWS_PROFILE=soroban-explorer

# (a) refresh local copy
aws secretsmanager get-secret-value --secret-id soroban/production/operator/env \
  --query SecretString --output text > ~/.config/soroban-prod.env

# (b) edit: append the two pairs to the EXISTING CLICKHOUSE_CN_USER_MAP value
#     (one line; don't drop existing CNs). Resulting tail:
#     CLICKHOUSE_CN_USER_MAP="…existing…,prices-ingestion-production:prices_writer,prices-api-production:prices_reader"
${EDITOR:-vi} ~/.config/soroban-prod.env

# (c) verify (the CN map is non-secret; safe to print just this var)
set -a; source ~/.config/soroban-prod.env; set +a
echo "$CLICKHOUSE_CN_USER_MAP" | tr ',' '\n' | grep prices
#   expect exactly: prices-ingestion-production:prices_writer
#                   prices-api-production:prices_reader   (no duplicates)

# (d) push the updated env back to Secrets Manager (new version)
aws secretsmanager put-secret-value --secret-id soroban/production/operator/env \
  --secret-string file://$HOME/.config/soroban-prod.env
```

This is the **durable** store; the Caddy snippet is *rendered* from this var
during the §3 `ansible --tags app` run (which sources the same env). Inert until
that deploy. Mutating AWS action — gated.

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

## 5. Issue + store the mTLS certs (🔒 Hetzner CA + 🔒 AWS) — **operator-run**

> **Hard constraint:** every step here touches the CA private key, the client
> private keys, or mutates AWS. Per the standing key rule + prepare-only rule,
> **the operator runs all of §5 by hand** — Claude authors the runbook and may
> only verify *public* certs (Step 3). The CA-key access is confirmed available
> to the operator (open item 1 closed), so this is **not** a BE ask.

Two identities, env-suffixed CNs (open item 5). The CN is the single thread that
ties together the cert subject, the §1d Caddy map key, the CH user, **and** the
Secrets-Manager secret-id — keep them byte-identical (the invariant that prevents
the BE-style drift in open item 2).

| Identity | CN | Secret-id = `MTLS_SECRET_NAME` |
|---|---|---|
| Writer (0038/0039) | `prices-ingestion-production` | `prices/production/clickhouse-mtls-prices-ingestion-production` |
| Reader (0040) | `prices-api-production` | `prices/production/clickhouse-mtls-prices-api-production` |

> **CA key location — corrected 2026-06-23.** BE's CA tooling does **not** fetch
> the CA key from Secrets Manager; per `infra-hetzner/ca/README.md` the CA private
> key lives in a **password manager**, and `issue-client-cert.sh` reads it from
> `/dev/shm/soroban-ca/ca.key` (override `$CA_KEY_PATH`). The operator + BE share
> one AWS account, so the whole flow runs under a single profile
> `AWS_PROFILE=soroban-explorer` (no `--profile` flags); the operator env is
> loaded from the SM secret `soroban/production/operator/env`.

### Step 0 — profile + load operator env
```bash
export AWS_PROFILE=soroban-explorer

aws secretsmanager get-secret-value --secret-id soroban/production/operator/env \
  --query SecretString --output text > ~/.config/soroban-prod.env
set -a; source ~/.config/soroban-prod.env; set +a   # load operator env vars

CA_DIR=/home/oski/Projects/stellar/soroban-block-explorer/infra-hetzner/ca
```

### Step 1 — put the CA private key at the tmpfs path
`issue-client-cert.sh` requires the CA key at `/dev/shm/soroban-ca/ca.key`
(or `$CA_KEY_PATH`). Materialize it from wherever the operator setup holds it
(the sourced operator env, or the password manager) — never `cat` it:
```bash
mkdir -p /dev/shm/soroban-ca && chmod 0700 /dev/shm/soroban-ca

# >>> the ONE operator-specific line — materialize ca.key from your setup, e.g.:
#   printf '%s' "$<YOUR_CA_KEY_VAR>" > /dev/shm/soroban-ca/ca.key
#   (or copy a password-manager export into that path)

chmod 0600 /dev/shm/soroban-ca/ca.key      # never cat this file
export CA_KEY_PATH=/dev/shm/soroban-ca/ca.key
```

### Step 2 — issue both certs (365-day, ECDSA P-256, clientAuth EKU)
```bash
cd "$CA_DIR"
./issue-client-cert.sh prices-ingestion-production
./issue-client-cert.sh prices-api-production
# → ./out/<CN>/{<CN>.crt,<CN>.key,ca.crt}  (out/ is gitignored)
```

### Step 3 — verify (public-cert only; Claude may run this)
```bash
for CN in prices-ingestion-production prices-api-production; do
  openssl verify -CAfile "$CA_DIR/ca.crt" "$CA_DIR/out/$CN/$CN.crt"
  openssl x509 -in "$CA_DIR/out/$CN/$CN.crt" -noout -subject -ext extendedKeyUsage
done   # expect: OK, subject CN matches, EKU = TLS Web Client Authentication
```

### Step 4 — single `{cert,key,ca}` JSON bundle → Secrets Manager (per task 0052)
0052's client reads **one** secret named by `MTLS_SECRET_NAME`, parses it as
`{cert,key,ca}` JSON, fetched via the Lambda Parameters & Secrets Extension
(`packages/prices-clickhouse/src/mtls.rs:106-146,232-236`). The CDK no longer
creates these secrets (operator-owned; open item 2), so use `create-secret`,
falling back to `put-secret-value` if the name already exists:
```bash
cd "$CA_DIR/out"
for pair in \
  "prices-ingestion-production:prices/production/clickhouse-mtls-prices-ingestion-production" \
  "prices-api-production:prices/production/clickhouse-mtls-prices-api-production"; do
  CN="${pair%%:*}"; SID="${pair##*:}"
  BUNDLE="$(jq -n --arg c "$(cat "$CN/$CN.crt")" --arg k "$(cat "$CN/$CN.key")" \
                  --arg a "$(cat "$CN/ca.crt")" '{cert:$c,key:$k,ca:$a}')"
  aws secretsmanager create-secret --name "$SID" --secret-string "$BUNDLE" \
    || aws secretsmanager put-secret-value --secret-id "$SID" --secret-string "$BUNDLE"
done
```

### Step 5 — wipe all secret material
```bash
shred -u /dev/shm/soroban-ca/ca.key && rmdir /dev/shm/soroban-ca
rm -rf "$CA_DIR/out/prices-ingestion-production" "$CA_DIR/out/prices-api-production"
# SM is now the source of truth
```

Secrets-Manager bytes only; per the SSM key contract, only secret **names** ride
in env/SSM, never the cert/key material. 1-year manual rotation cadence.

> ✅ **Cross-task reconciliation (open item 2) — DONE on PR#34 (2026-06-23).**
> Previously a confirmed mismatch: the shipped client (`mtls.rs`) reads **one**
> `{cert,key,ca}` bundle via `MTLS_SECRET_NAME`, but `secrets-stack.ts` created
> **two** secrets (`…-cert` + `…-key`, random placeholders, single identity).
> Reconciled in `feat/0038` commit `fed74bc`
> (`fix(lore-0038): use single mTLS bundle secret for CH client`): `secrets-stack`
> no longer creates the material (publishes the two bundle **names** to SSM,
> operator-issued out-of-band like BE); `compute-stack` sets each Lambda group's
> `MTLS_SECRET_NAME` (writer→ingestion, reader→api) and grants read on only its
> own by-name ARN; secret-id / `MTLS_SECRET_NAME` / CN / Caddy map key all derive
> from one CN via the `mtlsSecretName` helper (so they cannot drift). Lesson that
> drove this: BE's own CA README says upload to `soroban/<CN>-mtls` while their
> CDK reads `${prefix}/lambda-<role>-<env>` — a live doc-vs-code drift we avoid by
> single-sourcing the name.

---

## 6. Verify tenant isolation (🔒 Hetzner, per env)

Using the issued certs via Caddy:443:

- `prices-ingestion-production` cert: `SELECT version()` → 200; `SHOW DATABASES`
  includes `prices`; `INSERT` into a `prices.*` table succeeds; the same against
  `default.*` → **ACCESS_DENIED**; `CREATE TABLE prices.smoke …` → **DENIED**
  (writer has no DDL — the Option-1 proof). Schema itself was applied in Step 4.
- `prices-api-production` cert: `SELECT` on `prices.*` works; any `INSERT`/`CREATE`
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

1. ✅ **CLOSED 2026-06-22 — CA-key access:** the operator will issue the mTLS
   client certs self-served (`issue-client-cert.sh`) and store the
   `{cert,key,ca}` bundles in Secrets Manager. Step 5 is **not** a BE ask.
   (The §1 `users.d` RBAC — formerly the last BE-owned item — has since
   shipped via BE task 0314 `87f24b76`; see §1.)
2. ✅ **CLOSED 2026-06-23 — Secret shape reconciliation (done on PR#34).**
   Was: `secrets-stack.ts` emitted the two-secret (`…-cert`/`…-key`),
   single-identity shape with random placeholders, while `mtls.rs` reads one
   `{cert,key,ca}` bundle via `MTLS_SECRET_NAME`. Fixed in `feat/0038` commit
   `fed74bc`: `secrets-stack` publishes the two bundle **names** to SSM (no
   CDK-managed material; operator issues out-of-band), `compute-stack` sets each
   Lambda group's `MTLS_SECRET_NAME` + grants its own by-name ARN, and
   `mtls.ts`/`lambda-baseline.ts`/`app.ts` thread a single secret name. The
   reconciliation rode in PR#34 because that PR owns the live ledger Function
   whose env wiring was the broken half. See §5 warning.
3. **Backup scope** (G-note §4) — recommend **(b)**: `prices.*` is re-derivable
   from ledger history, so accept it is outside BE's `RESTORE DATABASE default`
   set rather than extending BE's snapshot. Flag as a decision, not an oversight.
4. ✅ **CLOSED 2026-06-22 — Quota naming:** dedicated `prices_write`/`prices_read`
   (§1c). Caps re-diffed against `develop` @ `8e4e705d` — exact match to the live
   `high_write`/`api_throttle` values.
5. ✅ **CLOSED 2026-06-23 — CN scheme: env-suffixed, mirroring BE**
   (`prices-ingestion-production` / `prices-api-production`). *Supersedes the
   2026-06-22 single-CN call.* Reason: prices shares BE's CA, so prices CNs
   share BE's CA namespace (which is env-suffixed, e.g.
   `lambda-ingestion-production`); the suffix keeps them globally unique +
   self-documenting and matches BE exactly. envName is the literal `'production'`
   (`infra/src/lib/types.ts:33`). secret-id ≡ `MTLS_SECRET_NAME` ≡ Caddy map key,
   all derived from the one CN (§0, §1d, §5).
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

---

## Completion record

### 2026-06-22 — §2 database created + §4 schema applied (operator, by hand)

- **§2 (G3) DONE.** `CREATE DATABASE IF NOT EXISTS prices` run on the single
  `production` box `ch-prod-01` (`168.119.73.161`) as the `default` admin via
  `docker exec app-clickhouse-1 clickhouse-client` (loopback). CH 26.3.10.
- **§4 / task 0051 Step 4 DONE (apply).** The full `prices.*` schema (tables,
  seed, views, refreshable MV chain) was applied immediately after, over the
  same loopback path — `init.sql` → `seed.sql` → `views.sql` → `rollups.sql`
  streamed from the local repo via `ssh … docker exec … clickhouse-client
  --multiquery` (Route A). `price_ohlcv_1m` engine + sort key verified live
  against ADR 0003/0004. Provenance: 0051 `notes/G-live-schema-state.md`
  (object-set / seed-count outputs + MV smoke still being captured).
- **AC #1 (`prices` database exists) — satisfied.**

**Still open on 0063 — updated 2026-06-23 (BE §1 shipped):**

- ✅ **BE-side §1 RBAC — SHIPPED** by BE task 0314 (commit `87f24b76`):
  `prices_writer` / `prices_reader` + `prices_write` / `prices_read` added to
  `users.d/{services,quotas}.xml`, byte-for-byte the §1a/§1c drafts. The one
  piece prices-api could not self-serve is **done**; it goes live on the §3
  `ansible --tags app` run.
- **Prices-api / operator-owned (self-served, remaining):** §1d Caddy
  `CLICKHOUSE_CN_USER_MAP` entries
  (`prices-ingestion-production:prices_writer`,
  `prices-api-production:prices_reader` — env-suffixed; see the §1d heads-up re
  BE's bare-CN doc rows) **and** §5 mTLS client-cert issuance + the single
  `{cert,key,ca}` Secrets-Manager bundles
  (`prices/production/clickhouse-mtls-prices-{ingestion,api}-production`). Not a
  BE ask (CA-key access available to the operator; open item 1 resolved).
- **Operator-coordinated infra actions:** §3 Ansible `--tags app` run (picks
  up the new users + CN map) and §6 tenant-isolation smoke test — now unblocked
  (the §1 BE dependency has landed).

Net: **the BE-side dependency (§1) is closed.** Everything remaining is
prices-api/operator work — issue certs (§5), append the CN map (§1d), run
`ansible --tags app` (§3), smoke-test isolation (§6).
