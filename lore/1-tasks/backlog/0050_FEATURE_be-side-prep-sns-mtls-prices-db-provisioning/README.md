---
id: "0050"
title: "BE-side prep — SNS fan-out topic on the stellar-ledger-data bucket (mTLS cert + prices DB carved out to 0063)"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0045", "0047", "0011", "0038", "0063"]
tags: [layer-infra, priority-high, effort-medium, milestone-M1, cross-team, block-explorer, hetzner, clickhouse, mtls, sns]
milestone: 1
links:
  - "../../../../docs/prices-api-general-overview.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
  - "../0011_FEATURE_bootstrap-cdk-with-ssm-platform-lookups.md"
  - "../../active/0038_FEATURE_prices-ledger-processor-lambda/README.md"
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. The general-overview
      §9 Tranche 1 "BE-side prep (one-time)" bullet enumerates three
      cross-team artefacts (SNS topic, mTLS cert, prices DB) that have
      no owning task — they cross the BE/prices-api boundary and
      require BE-side CDK + operator action. Capturing them as one
      task makes the cross-team hand-off trackable; without it these
      slip into informal Slack threads and gate everything downstream.
  - date: 2026-06-10
    status: backlog
    who: oski
    note: >
      Converted to directory form. The 2026-06-10 cross-team meeting
      resolved the SNS-vs-direct-notification question (task 0038
      §C.1) in favour of **SNS fan-out**; item 1 now has a
      ready-to-implement BE ask at
      `notes/G-be-sns-fanout-ask.md` (topic + repoint with
      `rawMessageDelivery` + the `/platform/{env}/ledger-events-topic-arn`
      SSM key, grounded in BE's compute-stack.ts and 0038's CDK in
      PR #34). Note: the SNS item can ship independently of BE 0227 /
      task 0047 (it's S3+SNS+SSM, not Hetzner-CH) — only the mTLS-cert
      and `prices.*`-DB items stay gated. Task stays backlog pending
      BE scheduling.
  - date: 2026-06-17
    status: backlog
    who: oski
    note: >
      **Scope narrowed to the SNS fan-out item (item 1) only.** BE 0227
      shipped (archived 2026-05-19) and BE is granting prices-api admin
      access to the Hetzner CH box, so items 2 (mTLS cert issuance) + 3
      (`prices` DB/user/quota/profile) are now self-served and moved to
      the new task 0063. This task retains only the genuinely-still-BE-side
      SNS fan-out topic, which can ship independently.
---

# BE-side prep — SNS fan-out topic (S3 → SNS → tenant Ledger Processors)

> **Scope (2026-06-17):** narrowed to the **SNS fan-out** item only.
> The mTLS-cert and `prices` DB/user/quota items moved to **task 0063**
> (self-served now that admin access is granted + BE 0227 shipped).

## Summary

One coordinated cross-team task that drives the three BE-side
provisioning steps the general-overview §9 Tranche 1 plan lists as
prerequisites for live ingestion: (1) add an SNS fan-out topic
between the `stellar-ledger-data/` S3 bucket and both tenants'
Ledger Processors (one-time BE CDK change), (2) issue per-env mTLS
client cert + key pairs from BE's self-signed CA for the prices-api
Lambdas, and (3) provision a `prices` database + dedicated user +
quota + profile inside BE's shared Hetzner ClickHouse cluster
(per ADR 0007 §3.5 multi-tenant primitives).

## Context

Per the design doc §0, §1.2, §2.3, §5.2, and §11.1, prices-api
operates as a second tenant inside BE's existing infrastructure
rather than running its own. Three concrete BE-side changes have
to land before any prices-api ingestion or schema work can begin:

1. **SNS fan-out topic** (§1.2, §2.3, ADR 0007 §3.2 / 0045 Cluster
   A2): BE today notifies its own Ledger Processor directly on S3
   PutObject. Adding a second tenant requires interposing an SNS
   topic between the bucket and the subscribers. This is a
   one-time BE CDK change; once landed, adding/removing tenants
   becomes a subscription change rather than a bucket-config
   change.
2. **mTLS client cert issuance** (§5.2, §7, ADR 0007 §3.5): BE
   runs the self-signed CA and the per-AWS-service issuance
   script. Prices-api receives one client cert + key per env
   (dev / staging / prod), to be stored in AWS Secrets Manager
   (2 secrets per env). 1-year manual rotation cadence (BE
   Cluster C agreement). The issuance script invocation is the
   only BE-side operator step per cert lifecycle.
3. **`prices` database + user + quota + profile** (§3 intro,
   §11.1, ADR 0007 §3.5): BE creates the empty `prices` database
   in its Hetzner CH cluster, plus a dedicated CH user, plus a
   quota and a profile that scope the user's resource usage so a
   misbehaving prices-api consumer cannot exhaust BE's `default.*`
   throughput. The opening cost-share proposal is ~1-2% pro-rata
   (~$1-2/env/mo) per task 0046's empirical sizing, with D12
   commercial follow-up.

All three live on the BE side of the boundary, so prices-api owns
the spec, the acceptance verification, and the rollback plan, but
the implementation is BE's. The cross-team contract is the
agreement record at `archive/0045_.../notes/G-be-agreement-record.md`.

## Implementation Plan

### Step 1: Draft and confirm the BE-side artefact list

Produce a short written checklist (1–2 pages) under
`notes/G-be-prep-checklist.md` enumerating exactly what BE needs
to deliver for each of the three items, with success signals
prices-api can verify independently:

- SNS topic ARN per env, published to a known SSM key
  (`/platform/{env}/ledger-events-topic-arn` — canonical, per the
  SNS-fan-out ask in `notes/G-be-sns-fanout-ask.md`, which is the
  ready-to-implement spec for this item post the 2026-06-10 meeting).
- Per-env mTLS cert + key handed off via a secure channel; CA
  certificate exported as a static asset checked into prices-api
  for trust-chain verification.
- CH endpoint URL, DB name, username, password (or cert-mapped
  identity) published to SSM/Secrets Manager keys; quota and
  profile names documented for future reference.

> Items 2 + 3 (mTLS cert issuance + `prices.*` DB/user/grant/profile)
> now have a ready-to-implement BE ask at
> `notes/G-be-prices-db-rbac-ask.md` — grounded in BE's actual
> `users.d/services.xml` / `profiles.xml`, the Caddy `CLICKHOUSE_CN_USER_MAP`,
> and `issue-client-cert.sh`, with the two-layer (CH RBAC + AWS IAM)
> isolation rationale. Sibling to `G-be-sns-fanout-ask.md` (item 1).

### Step 2: BE-side execution (BE owns)

- BE 0227 (Hetzner Ansible playbook) ships first; this is the
  upstream gate per the 0045 agreement record. Track via the
  cross-repo task.
- BE updates the `stellar-ledger-data/` CDK stack to fan-out via
  SNS (mirrors ADR 0007 §3.2 sketch). Existing BE Ledger
  Processor subscription is preserved.
- BE runs the per-AWS-service mTLS issuance script for each env;
  hands off cert + key bundles via 1Password / equivalent secure
  channel.
- BE applies the `prices` DB + user + quota + profile DDL on the
  Hetzner CH cluster. Suggested user/quota names captured in the
  checklist; final names are BE's call.

### Step 3: Prices-api-side verification

For each env (dev → staging → prod):

- Subscribe a throwaway Lambda or local script to the SNS topic
  via the published ARN; observe that the next S3 PutObject
  triggers a delivery. Capture the message envelope as a
  fixture for 0038.
- Load the cert + key into Secrets Manager; connect to Caddy:443
  via `curl --cert ... --key ...` and run `SELECT version()` —
  expect 200 + a CH version string.
- Connect as the prices-api user and confirm `SHOW DATABASES`
  includes `prices`, that `CREATE TABLE prices.foo (...) ENGINE
  = Memory` succeeds, and that the same `CREATE TABLE` against
  `default.foo` fails with permission denied. Drop the
  throwaway table.

### Step 4: Document the hand-off in the checklist

Mark each item complete with the date of BE-side delivery and
the SSM/Secrets keys used. Link the checklist from 0011 (CDK
bootstrap) so the CDK stack's SSM lookups point at the right
keys.

## Acceptance Criteria

- [ ] SNS topic exists on BE's `stellar-ledger-data/` bucket
      fan-out for all three envs; ARNs published to SSM under
      a stable key prices-api's CDK can read
      (`/platform/{env}/ledger-events-topic-arn`)
- [ ] BE's existing Ledger Processor subscription preserved;
      prices-api can subscribe via the published ARN with
      `rawMessageDelivery`
- [ ] Throwaway-Lambda smoke test confirms an S3 PutObject triggers
      an SNS delivery; the recorded message envelope is captured as
      a fixture for 0038
- [ ] 0011 (CDK bootstrap) references the SSM key produced by
      this task; no CDK-side guess-and-hope for the topic ARN

> **Moved to task 0063** (self-served, admin access granted): per-env
> mTLS client cert issuance + storage, and the `prices` database /
> user / quota / profile provisioning + isolation smoke test.

## Blocked on

- Nothing hard. The SNS fan-out is S3 + SNS + SSM only — it does not
  depend on BE 0227 (shipped) or task 0047. Practical prerequisite is
  BE scheduling the one-time CDK change on the `stellar-ledger-data/`
  bucket stack.

## Out of scope

- mTLS cert issuance + `prices` DB/user/quota provisioning — **task 0063**.
- Schema migration tooling for `prices.*` tables — see 0051.
- Cost-share dollar-figure finalisation — Cluster D commercial
  follow-up per the 0045 agreement record.

## Notes

- This task is the **only** Tranche 1 work that cannot start
  without BE action. Scheduling-wise it should kick off in
  Week 1 in parallel with 0011 so the BE pipeline runs ahead
  of the prices-api code path.
- The throwaway-Lambda smoke test in Step 3 produces a recorded
  SNS event envelope; that fixture is consumed by 0038 (Prices
  Ledger Processor) integration tests, so do not skip it.
