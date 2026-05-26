---
title: 'R: BE Hetzner ClickHouse shape — what prices-api can externally consume'
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [hetzner, clickhouse, mtls, infra, block-explorer, step-1]
links:
  - '../../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md'
  - '../../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/README.md'
  - '../../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/notes/S-decisions.md'
  - '../../../../../soroban-block-explorer/lore/1-tasks/active/0227_FEATURE_infra-hetzner-ansible-playbook.md'
  - '../../../../../soroban-block-explorer/lore/2-adrs/0045_clickhouse-local-backfill-then-mirror-to-hetzner-via-freeze-rsync-attach.md'
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: 'Distilled from BE infra-overview §5.6, task 0216, task 0227, ADR 0045.'
---

# R: BE Hetzner ClickHouse shape — what prices-api can externally consume

## Purpose

Step 1 of task 0044. Capture the parts of BE's Hetzner-CH plan that
matter to a second tenant (prices-api) attempting to write live OHLCV
rows into the same data plane. This is a **distillation of BE
sources**, not a recommendation — the recommendation lives in the
later `S-*` note.

## 1. Physical host topology

| Aspect              | Value                                                                                    | Source                                 |
| ------------------- | ---------------------------------------------------------------------------------------- | -------------------------------------- |
| Provider            | Hetzner Dedicated Server                                                                 | 0216 `S-decisions.md` §1               |
| OS                  | Ubuntu 24.04 LTS                                                                         | 0227 `installimage.conf`               |
| Disk layout         | mdadm RAID 1, single ext4 root, separate ext4 `/boot`, no swap                           | 0227 `installimage.conf`               |
| Provisioning        | Hardware ordered manually via Hetzner Robot UI, **everything else IaC** (Ansible)        | 0216 `S-decisions.md` §5, 0227 `Scope` |
| Backups             | Daily `borg` cron → Hetzner **BX21 Storage Box**                                         | 0227 `Scope` + `Acceptance Criteria`   |
| Status (2026-05-18) | Box ordered + provisioned 2026-05-15; Ansible playbook + Caddy + mTLS work **in flight** | 0216 history line 4                    |
| Hardware class      | Not pinned in any committed source as of today                                           | (gap)                                  |

**Implication for prices-api.** The host is treated as a single
production CH data plane, not a multi-tenant managed cluster. There
is no "carve out a node for prices-api" knob; whatever prices-api
gets, it gets on the same physical box BE is operating.

## 2. Network ingress

```
                    ┌────────────────────────┐
                    │   Public internet      │
                    └───────────┬────────────┘
                                │ :443 mTLS  (only public ingress)
                                │ :80  ACME http-01 + redirect to :443
                                ▼
                    ┌────────────────────────┐
                    │  Caddy (TLS + mTLS)    │   server-side TLS = Let's Encrypt
                    │  - require_and_verify  │   client-side TLS = self-signed CA
                    └───────────┬────────────┘
                                │ docker bridge
                                ▼
                    ┌────────────────────────┐
                    │  ClickHouse (loopback) │  HTTP 8123 / native 9000 NOT publicly bound
                    └────────────────────────┘
```

Concrete points (all from 0227):

- **Only public ports: 22, 80, 443.** Host firewall denies inbound
  for everything else; Hetzner stateless firewall at the network-
  switch level enforces the same.
- **ClickHouse port binding is restricted to loopback only** — never
  directly reachable from the public internet. The Caddy reverse
  proxy is the single hop.
- **Caddy directive enforced:** `tls.client_auth { mode require_and_verify }`.
  A TLS handshake without a CA-signed client cert is rejected
  _before_ any HTTP request reaches ClickHouse.
- **Docker compose `ports: !override`** is mandatory for every
  rebound service in the production overlay — defensive against the
  base compose silently appending the dev-mode publicly-bound ports.

**Implication for prices-api.** The externally-consumable surface
is exactly one endpoint: `https://<hetzner-host>/...` over mTLS,
speaking the ClickHouse HTTP wire protocol. There is no second port,
no native-protocol fallback, no per-tenant separate ingress. A
prices-api Lambda would dial the same Caddy as BE does.

## 3. mTLS identity model

| Element           | Where it lives                                                                 | Lifecycle                                                      |
| ----------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------- |
| Public CA cert    | `infra-hetzner/ca/ca.crt` in the BE repo (committed)                           | Static — rotation requires re-issuing the file + Caddy reload  |
| Private CA key    | **Team password manager** — never in the repo                                  | Held by BE team, used only by the one-time CA bootstrap script |
| Client certs      | Issued by a script in `infra-hetzner/ca/` — **per AWS service, per developer** | Per-cert lifecycle; revocation model not explicitly documented |
| Server cert (TLS) | Let's Encrypt via Caddy, auto-renewed                                          | Automatic                                                      |

Key quote from 0227:

> One-time CA bootstrap script (run on a developer laptop)
> Client certificate issuance script (**per AWS service, per developer**)

The phrase "per AWS service" is the only externally-relevant signal
that BE's identity model is **already designed to issue distinct
certs to distinct AWS-side workloads**. That naturally accommodates
a second tenant like the prices-api Lambda — but it does _not_ imply
BE has agreed to issue one. The script is the mechanism; the policy
is a cross-team conversation.

**Implication for prices-api.** Joining means BE issues prices-api
its own client cert from the same self-signed CA. There is no
separate trust domain to negotiate, no PCA or ACM involvement. The
cost is:

1. A formal request to BE to issue a `prices-api-{env}` client cert.
2. Storing the cert + private key in AWS Secrets Manager (prices-api
   side).
3. Operationally agreeing how rotation works (BE's docs do not yet
   describe a rotation runbook).

## 4. Schema ownership boundary

| Aspect                 | Value                                                                                                    | Source                                              |
| ---------------------- | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| Schema source of truth | `crates/db-clickhouse/schema/init.sql` in BE repo                                                        | 0227 `ClickHouse configuration additions`, ADR 0044 |
| Schema content         | 17 tables + 1 Dictionary                                                                                 | BE infra-overview §5.2                              |
| Application mechanism  | `db-clickhouse-init` sidecar, idempotent, runs on every boot                                             | 0227 `Scope`, ADR 0045 §Consequences                |
| Engine choice          | `ReplacingMergeTree` for the table family that holds soroban events                                      | ADR 0045 §Rationale + §Alternatives                 |
| Replication            | **Not replicated.** `ReplicatedMergeTree` explicitly rejected for now (ADR 0045 alt. 3)                  | ADR 0045                                            |
| Default DB             | `default` (ClickHouse default) — no explicit per-tenant DB carve-out                                     | inferred from 0227 + init.sql shape                 |
| Internal user model    | One application user; a localhost-only no-password `dict` user is used by the Dictionary `SOURCE` clause | 0227 `ClickHouse configuration additions`           |

**Implication for prices-api.** Anything prices-api writes into this
cluster either:

1. Lives in **its own database** (e.g. `prices`) with its own
   `init.sql`-equivalent and its own sidecar migration job — clean
   boundary, requires BE to agree to a second database in their
   instance, and requires prices-api to write its own migration
   tooling.
2. Lives as **shared tables** with a discriminator column — couples
   prices-api's schema evolution to BE's repo (where `init.sql`
   lives), which is operationally awkward across two teams.

This is the question step 5 of task 0044 owes a recommendation on.
Surface the trade-off now so step 2 (mapping write targets) can
assume **option 1** as the working hypothesis until step 5 decides.

## 5. Multi-tenant assumptions (or lack thereof)

BE's committed sources never use the words "tenant", "shared",
"prices-api", or any equivalent. The Hetzner plan is written as a
single-application data plane.

What that means concretely:

- **No isolation primitive is in place** beyond ClickHouse's own
  `database`, `user`, `quota`, and `row_policy` features. Prices-api
  isolation would have to be configured on top, not relied on as
  pre-existing.
- **No documented capacity planning for a second tenant.** ADR 0045
  sizes disk at ~800 GB for BE's full pubnet backfill, on a single
  box with finite headroom; prices-api write volume is small
  (1-minute OHLCV rows, not raw events) but **not zero** and must be
  added to BE's sizing math.
- **No documented operational ownership of cross-tenant incidents.**
  If a prices-api query OOMs CH or a prices-api migration breaks
  something, the runbook does not say who pages or rolls back.

**Implication for prices-api.** Joining as a second tenant is
**not pre-blessed by the BE plan as written.** It is technically
possible, the auth model already accommodates it, but it requires
a cross-team agreement that does not yet exist. Capture this as
the central open question for the cost-share conversation (open
question #1 in the task README).

## 6. Backup / DR shape (relevant for whether prices-api data is recoverable)

- **Backups:** Borg daily → BX21 Storage Box. Configured per 0227.
  Backup destination is _Hetzner_, not AWS — egress for restore back
  to AWS would traverse public internet.
- **PITR:** Not documented. Borg gives daily granularity. RDS-style
  point-in-time-recovery is not on the Hetzner side.
- **No HA / failover:** Single box; no replica. The "what if the
  box dies" answer is "restore from Borg" — recovery time depends
  on backup size + uplink, on the order of hours.

**Implication for prices-api.** The RDS plan currently assumed by
0011/0038/0039/0040 gives PITR + automated snapshots for free.
Moving prices-api data to Hetzner CH **demotes the recovery guarantee
to daily-granularity Borg restore.** Whether that is acceptable is a
product decision (how much OHLCV loss can prices-api tolerate?). Flag
as open question #5 in the task README — confirmed live.

## 7. AWS-side topology change post-migration (re-stating BE §5.6)

Out of scope for "what does the CH instance look like" but in scope
for "what does the AWS side that talks to it look like":

- Lambdas **leave the VPC** entirely.
- Long-running ingestion task (Galexie) **moves to a public subnet**
  with a public IP.
- **NAT Gateway is removed.**
- Cross-cloud auth = **mTLS only** (cryptographic identity, not
  IP-based filtering).

**Implication for prices-api.** Whatever was assumed about a
Prices-Lambda-in-VPC writing to a Prices-RDS-in-VPC is invalid post-
migration. The prices-api Lambdas would also exit the VPC and talk
to Hetzner over the public internet. This is consistent with BE's
plan and removes the shared NAT Gateway cost line for prices-api
too (currently listed under §2.3 of `prices-api-general-overview.md`).

## 8. Summary — externally consumable surface

| Surface                                                 | Externally consumable?          | Notes                                                            |
| ------------------------------------------------------- | ------------------------------- | ---------------------------------------------------------------- |
| `https://<hetzner-host>:443/` ClickHouse HTTP over mTLS | **Yes**                         | Single endpoint, generic CH wire protocol                        |
| Native CH protocol port 9000                            | **No**                          | Loopback-only                                                    |
| Per-tenant database / user                              | **Mechanism exists, no policy** | Requires BE agreement (open question)                            |
| Per-AWS-service client cert                             | **Mechanism exists, no policy** | Issuance script supports it; needs BE issuance                   |
| Backup / restore                                        | **BE-owned**                    | Borg → BX21; no per-tenant restore primitive                     |
| Schema migration                                        | **BE-owned** today              | Prices-api would need its own migration story (step 5)           |
| Observability                                           | **BE-owned**                    | Native Prometheus endpoint on loopback (0227); no per-tenant cut |

## 9. Open questions surfaced by step 1 (forwarded to README)

1. **Hardware sizing margin.** BE has not published the box's CPU
   / RAM / disk class. Capacity planning for a second tenant is
   speculative until BE shares specs. _Action:_ ask BE.
2. **Per-tenant database vs. shared tables.** BE plan is silent.
   _Action:_ step 5 of task 0044 to decide; preferred default for
   step 2 is "separate `prices` database".
3. **Client-cert issuance policy.** Mechanism supports per-service
   certs; BE has not committed to issuing one for prices-api.
   _Action:_ cross-team request before any implementation work.
4. **Backup retention for prices-api rows.** Daily Borg vs. RDS
   PITR is a recovery-grade demotion. _Action:_ product decision
   (RPO target for OHLCV).
5. **Cross-tenant incident ownership.** No runbook covers the
   prices-api ↔ BE interaction during an incident. _Action:_
   document in the eventual ADR if recommendation is "go".

## 10. What step 1 does NOT cover

- The actual write-target mapping (RDS table → CH engine choice) —
  step 2.
- The Stellar peers → Galexie → S3 live-feed chain — step 3.
- The AWS-side mTLS network path and failure modes — step 4.
- The cost delta math — step 6.
- The go/no-go synthesis — step 7.

These are explicitly out of scope here so the note stays focused on
"what is the BE thing we would be joining".
