---
title: "G: BE conversation brief — shared Hetzner CH for prices-api live writes"
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [generation, brief, cross-team, block-explorer, hetzner, clickhouse, mtls]
links:
  - "../README.md"
  - "../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md"
  - "../../../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/README.md"
  - "../../../../../../soroban-block-explorer/lore/1-tasks/active/0227_FEATURE_infra-hetzner-ansible-playbook.md"
history:
  - date: 2026-05-19
    status: developing
    who: okarcz
    note: "First draft. To be reviewed internally before sending to BE."
---

# G: BE conversation brief — shared Hetzner ClickHouse for prices-api live writes

> **Audience:** Block Explorer team (owners of BE tasks 0216 + 0227).
> **From:** prices-api team.
> **Status:** Draft — not yet sent. Track responses against §3 cluster outcomes.

---

## 1. TL;DR

We want prices-api's live data sink to **share your Hetzner ClickHouse
host** rather than stand up a dedicated AWS RDS for the same workload.
Concretely:

- We write into a **separate `prices.*` database** on your CH cluster
  (not into `default.*`).
- We piggy-back on **your existing S3 → ledger pipeline** by adding
  an SNS topic between the bucket and your Ledger Processor Lambda;
  we subscribe our own Lambda to that topic.
- We hit your **Caddy:443 mTLS endpoint** from AWS Lambdas using a
  per-env client cert your CA issues to us.

Everything else stays on our side of the boundary: our AWS account,
our Lambdas, our schema migrations, our cost-tracking. We are asking
for **four things** below — a single bundle, because they are
interdependent.

We expect a written outcome per cluster (accepted / counter / blocked).
Once all four land, we promote ADR 0007 (proposed → accepted) and
unblock our rewrites of tasks 0011 / 0038 / 0039 / 0040.

---

## 2. Topology we are proposing

```
┌────────────────────────────────────────────────────────────┐
│ Stellar mainnet (BE-owned non-validating Captive Core)      │
└──────────────────────────┬─────────────────────────────────┘
                           │ Galexie PutObject (BE-owned)
                           ▼
┌────────────────────────────────────────────────────────────┐
│ S3: {env}-stellar-ledger-data  (BE-owned)                   │
└──────────────────────────┬─────────────────────────────────┘
                           │ OBJECT_CREATED → SNS topic ◀── (one-time BE CDK change)
            ┌──────────────┴──────────────┐
            ▼                             ▼
┌────────────────────────┐      ┌────────────────────────┐
│ BE Ledger Processor    │      │ prices-api Ledger      │
│ Lambda (BE-owned)      │      │ Processor Lambda       │
│   → default.*          │      │   → prices.*           │
└──────────┬─────────────┘      └────────────┬───────────┘
           │                                  │
           └──────────────┬───────────────────┘
                          │ HTTPS :443 mTLS (public internet)
                          ▼
┌────────────────────────────────────────────────────────────┐
│ Hetzner dedicated server (BE-owned)                         │
│  Caddy :443 — Let's Encrypt server cert                     │
│              + require_and_verify against BE CA             │
│              │                                              │
│              ▼ docker bridge                                │
│  ClickHouse — single instance, loopback only                │
│   ├─ default.*    (BE-owned)                                │
│   └─ prices.*     (PRICES-OWNED)                            │
│                                                             │
│  Borg → BX21 Storage Box  (request: `BACKUP DATABASE prices`) │
└────────────────────────────────────────────────────────────┘
```

If anything in this picture diverges from your current plan, that
itself is a thread to pull — please flag.

---

## 3. The four asks

The four clusters are bundled because they are interdependent: capacity
feeds cost-share; cert issuance shape feeds rotation cadence; schema
ownership feeds backup target. We need them resolved together, not
sequentially.

### 3.1 Cluster A — Architectural buy-in

**Ask 1.** Approve a **separate `prices` database** in your CH
cluster, with a dedicated CH user (`prices_writer` + `prices_reader`,
or whichever names you prefer). DDL inside `prices.*` is announcement-
not-approval; DDL touching `default.*` or any cross-database read is
joint review.

**Ask 2.** Approve **inserting an SNS topic** between
`{env}-stellar-ledger-data` and your Ledger Processor Lambda. Both
Lambdas (yours + ours) subscribe to the topic. One-time CDK change on
your side; you keep ownership of the bucket and the topic. Reasoning
in the research note linked under §6.

**Ask 3.** Confirm the **announcement-not-approval** norm for DDL
inside `prices.*` — we open a PR in our repo, you get a Slack heads-up
+ the PR link, no blocking review unless cross-database reads are
involved.

**What we need from you:** yes / counter / no on each of the three.

**Fallback if blocked:** sidecar CH instance on a separate Hetzner
box, prices-owned. Adds ops surface; we'd rather not. (Option 4 in
the research note.)

---

### 3.2 Cluster B — Capacity, retention, backup

**Ask 4.** Hetzner box hardware specs + your monthly Hetzner invoice
amount (server + Storage Box + traffic, if any). We need this to
ground the capacity math and the cost-share conversation in §3.4.

**Ask 5.** Confirm Caddy `max_keepalive_conns` has headroom for a
second tenant's writers (we'd be adding ~6 Lambdas, mostly idle, with
batched HTTP `INSERT` traffic — peak ~10-15 req/s per env). What's
the current value, and would you tune it before we start writing?

**Ask 6.** Confirm BE intent to **keep BE ADR 0006 (indefinite S3
retention)**. Our replay story for OHLCV recompute depends on it; if
you ever flip to a retention policy, we'd need a heads-up so we can
mirror the S3 content into prices-api-owned storage.

**Ask 7.** Add `BACKUP DATABASE prices` as a separate daily Borg
target so prices-api can be restored independently of BE data (or in
case of `prices.*`-only corruption). Same Storage Box; we are not
asking for new infrastructure.

**Ask 8.** Confirm the backup RPO is **daily Borg granularity**
(no PITR). This is acceptable for prices-api; surfacing as a heads-up
so it isn't a surprise later.

**What we need from you:** hardware spec sheet + cost sheet for ask
4; yes/counter/no on 5-8.

**Fallback if blocked:** if Caddy headroom is tight, we can run a
small in-AWS write buffer (SQS) to smooth bursts. If `BACKUP DATABASE
prices` is rejected, we'd take logical exports to S3 ourselves;
non-blocking.

---

### 3.3 Cluster C — Auth + secrets

**Ask 9.** BE issues **`prices-api-{env}` client certs** (`dev`,
`staging`, `prod` — three certs) via the same per-AWS-service script
you use for your own Lambdas. We store the cert + key in our Secrets
Manager, loaded once at Lambda init via `OnceCell`. We do not need
access to your CA private key.

**Ask 10.** Rotation cadence: **1-year manual rotation with a
CloudWatch NotAfter alarm at T-30 days** on our side. Revisit in one
year. If you prefer shorter, we'll match.

**Ask 11.** Revocation model: **rotate the BE CA on compromise**
(we understand this is your existing pattern). If we suspect a
prices-api key has leaked, we ping you for a re-issue + add the old
cert to a deny list on Caddy.

**What we need from you:** yes/counter/no on cadence; commitment to
issue the three certs when ADR 0007 transitions to accepted.

**Fallback if blocked:** if you can't issue certs externally, we
could run a sidecar Caddy on the same host that uses our own CA; ugly
and we'd rather not.

---

### 3.4 Cluster D — Money

**Ask 12.** Cost-share. We open with a **5–10% pro-rata proposal**:
~$3–$15/env/month, three envs, ~$10–$45/month total flowing to BE.
Basis: our estimated share of storage rows (~5% steady state),
ingress traffic (~5%), and CPU (~10% during MV chain runs).

**Stance:**

- We are happy to pay the pro-rata number above.
- We are happy with a **flat fee up to ~$15/env/month** for simplicity.
- We are happy with a **free ride** if you offer it — both sides
  win on at-scale savings vs. the dedicated-RDS counterfactual (we're
  saving ~$6.9k/year at scale, of which we'd happily kick back a
  meaningful slice).
- Re-open the conversation if production scale materially shifts
  (10× rows, traffic, or CPU).

**Ask 13.** Agree on **how the money actually moves** — internal cost
allocation, monthly invoice, or zero-flow with a written acknowledgment?
We do not have a preference; whatever your finance side prefers.

**What we need from you:** a number (or "free ride") + a mechanism.

**Fallback if blocked:** if the cost ask sticks at >$20/env/month
flat, we'd re-evaluate the sidecar-CH path. Unlikely to come to that.

---

## 4. Timing and sequencing

We do not expect a decision tomorrow. The natural sequence:

1. **You finish BE 0227** (Ansible + Caddy + mTLS plumbing). The
   `max_keepalive_conns` value and final firewall/TLS config from
   that task feed asks 5 and 9.
2. **We get a 30-minute call** (or async thread, your preference) to
   walk through this brief. We bring all four clusters; you push
   back on whichever is shakiest.
3. **You respond in writing** with yes/counter/no per ask. Async is
   fine.
4. **We capture the outcome** in `notes/G-be-agreement-record.md`
   (this task's `notes/`), cross-link from ADR 0007, and transition
   the ADR proposed → accepted (or amend the design if any cluster
   counter-proposes materially).

Calendar estimate: **1–2 weeks elapsed** if you're responsive on
written follow-ups. We do not start any rewrites of tasks 0011 / 0038
/ 0039 / 0040 until step 4 lands.

---

## 5. What we explicitly are NOT asking for

- Access to your CA private key.
- Direct CH native protocol (9000) — we'll write via Caddy:443 + HTTP.
- Cross-database joins to `default.*` from our queries — if a use
  case ever needs that, we'll come back with a separate proposal.
- A new ClickHouse instance, separate cluster, or different host.
- Changes to your S3 bucket name, prefix layout, or Galexie config.
- Changes to your CH version, settings, or upgrade cadence (we
  match whatever you run).
- Schema-design opinions on `default.*`.

---

## 6. Backing research

Linked for context, not required reading:

- Task 0044 synthesis: ../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/S-refactor-recommendation.md
- Why a separate database (Option 1): ../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/I-schema-ownership-options.md
- Why SNS fan-out (Shape B): ../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-stellar-peers-galexie-live-feed.md §6
- mTLS + Secrets Manager + `OnceCell`: ../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-aws-hetzner-auth-network.md
- Cost basis for the 5-10% pro-rata number: ../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-cost-delta.md
- Architectural commitment (proposed): ../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md

---

## 7. Outcome tracking (filled in after response)

| Cluster | Ask | Outcome | Note |
|---|---|---|---|
| A | 1. Separate `prices` DB + dedicated user | — | |
| A | 2. SNS topic between S3 and Lambdas | — | |
| A | 3. Announcement-not-approval DDL norm | — | |
| B | 4. Hardware specs + monthly cost | — | |
| B | 5. Caddy `max_keepalive_conns` headroom | — | |
| B | 6. BE ADR 0006 retention confirmed | — | |
| B | 7. Daily `BACKUP DATABASE prices` Borg target | — | |
| B | 8. Daily RPO acceptable (heads-up) | — | |
| C | 9. Per-env client certs via BE script | — | |
| C | 10. 1-year manual rotation cadence | — | |
| C | 11. Revocation = CA rotation | — | |
| D | 12. Cost-share number | — | |
| D | 13. Money-movement mechanism | — | |

Once this table is fully populated, copy it (with outcomes) into
`G-be-agreement-record.md` and close out the cluster outcomes section
in this task's README. ADR 0007 then references that record.
