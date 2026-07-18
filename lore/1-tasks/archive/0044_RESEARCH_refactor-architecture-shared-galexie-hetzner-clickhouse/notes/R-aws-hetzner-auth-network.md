---
title: "R: AWS Lambda ↔ Hetzner CH — auth, network path, latency, failure modes"
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [mtls, network, latency, failure-mode, lambda, hetzner, clickhouse, step-4]
links:
  - "./R-be-hetzner-ch-shape.md"
  - "./R-stellar-peers-galexie-live-feed.md"
  - "../../../../../soroban-block-explorer/lore/1-tasks/active/0227_FEATURE_infra-hetzner-ansible-playbook.md"
  - "../../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md"
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: >
      Synthesises the AWS↔Hetzner boundary from step-1's BE plan,
      step-3's live-feed contract, and externally-known Lambda /
      mTLS / network-latency characteristics.
---

# R: AWS Lambda ↔ Hetzner CH — auth, network path, latency, failure modes

## Purpose

Step 4 of task 0044. The AWS↔Hetzner hop is the only **new**
boundary the refactor introduces. Everything upstream (S3 → Lambda)
is unchanged from today's plan; everything inside Hetzner is
BE-owned. This note pins down the **wire-level contract** between
the prices-api Lambda and the Caddy:443 endpoint that fronts CH:

- How the Lambda gets a client certificate.
- Where the cert + private key live in AWS.
- How they reach the runtime at invocation time.
- What rotation looks like.
- What the network path costs in latency.
- What happens when Hetzner is unreachable.

---

## 1. mTLS identity — issuance and rotation

### 1.1 Issuance flow (recap from step 1)

BE owns a **self-signed CA** managed under `infra-hetzner/ca/`:

- **Public CA cert** committed to BE repo at `infra-hetzner/ca/ca.crt`.
- **Private CA key** lives only in the team password manager.
- **Issuance script** in `infra-hetzner/ca/` produces client certs
  **per AWS service, per developer** (quoted, BE task 0227).
- **Caddy** verifies any inbound TLS handshake against this CA via
  `tls.client_auth { mode require_and_verify }`.

Prices-api joining means BE runs the issuance script with a
`prices-api-{env}` subject (one cert per env: dev / staging /
prod), delivers the resulting cert + private key out-of-band
(password manager link, age-encrypted file, etc.), and the
prices-api side stores them in AWS.

### 1.2 Cert + key storage in AWS

Three plausible locations, in decreasing fit:

| Store | Pros | Cons | Verdict |
|---|---|---|---|
| **Secrets Manager** | Native rotation hooks; KMS-encrypted; per-Lambda IAM scope; first-class binary support | Cost (~$0.40/secret/mo × 2 fields × 3 envs ≈ $2.40/mo); rotation lambda is non-trivial | **Recommended** |
| **SSM Parameter Store SecureString** | Free; KMS-encrypted; IAM-scoped | Per-secret value cap (4 KiB standard, 8 KiB advanced); no built-in rotation; manual versioning | Acceptable fallback |
| **Lambda env vars** | Trivial | **Disqualified.** Env vars are visible in `aws lambda get-function-configuration`; the private key would leak via console access. CloudFormation/CDK templates would carry the value. |  |

Recommendation: **two Secrets Manager secrets per env** —
`prices-api/{env}/hetzner-ch/client-cert` (PEM) and
`prices-api/{env}/hetzner-ch/client-key` (PEM). Lambda IAM role
gets `secretsmanager:GetSecretValue` on those two ARNs only.

Cost note: even at the high end Secrets Manager pricing is ~$3/mo
across three envs. Negligible against the RDS line it replaces.

### 1.3 Loading at runtime

Lambda cold start: read both secrets via `aws-sdk-secretsmanager`
during the global init phase; build a `reqwest::Identity` from
the PEM pair; reuse the resulting HTTP client across warm
invocations.

**Critical:** read secrets at **init**, not per-invocation. Init
runs once per execution environment (warm container); a
per-invocation `GetSecretValue` adds ~5–15 ms baseline cost plus
Secrets Manager API throttling risk under burst.

Sketch (Rust):

```rust
// Lambda init — runs once per warm container
static CH_CLIENT: OnceCell<reqwest::Client> = OnceCell::new();

async fn init_client() -> Result<reqwest::Client> {
    let sm = aws_sdk_secretsmanager::Client::new(&aws_config::load_from_env().await);
    let cert_pem = sm.get_secret_value()
        .secret_id("prices-api/prod/hetzner-ch/client-cert")
        .send().await?.secret_string().to_owned();
    let key_pem  = sm.get_secret_value()
        .secret_id("prices-api/prod/hetzner-ch/client-key")
        .send().await?.secret_string().to_owned();
    let id = reqwest::Identity::from_pem(
        format!("{cert_pem}\n{key_pem}").as_bytes()
    )?;
    reqwest::Client::builder()
        .identity(id)
        .https_only(true)
        .timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(60))
        .build()
        .map_err(Into::into)
}
```

### 1.4 Rotation

**Server cert (Caddy → Let's Encrypt).** Auto-renewed by Caddy.
Prices-api side does nothing.

**Client cert.** BE's documented model is "issue once via script";
**no rotation runbook is documented today** (step-1 finding,
open question #1's neighbour). Two paths forward:

1. **Long-lived client cert** (1-year validity). Operator
   manually re-issues + re-publishes to Secrets Manager before
   expiry; CloudWatch alarm on cert NotAfter < now+30d.
2. **Short-lived client cert** (~24h-7d validity) with an
   automated re-issuance Lambda that drives the BE issuance
   script via SSH and writes the new pair to Secrets Manager.
   Strictly better security posture but introduces a cross-team
   automation dependency.

**Recommendation seed.** Start with **(1)** — 1-year cert,
calendar reminder + CloudWatch NotAfter alarm. Revisit (2) if
the security posture demands it.

**CA-key rotation.** Out of scope for prices-api; if BE rotates
the CA, prices-api re-fetches `ca.crt` from BE's repo and gets
re-issued. No prices-api-owned automation needed for the CA
itself.

### 1.5 Revocation

**No CRL, no OCSP** in BE's documented plan. Revocation == BE
removes the cert from acceptance (which they cannot do without a
CRL distribution point) OR rotates the entire CA. In practice,
client-cert revocation is "rotate the CA"; sufficient for two
known consumers, would not scale to many tenants.

Flag as a residual risk; same risk BE has today for itself.

---

## 2. Network path

### 2.1 Topology (post-BE-§5.6 migration)

```text
   ┌─────────────────────────────────┐
   │  AWS us-east-1 (N. Virginia)    │
   │                                 │
   │  prices-api Ledger Processor    │
   │  Lambda (no VPC)                │
   │       │                         │
   │       │ outbound HTTPS :443    │
   │       │ (public internet)       │
   └───────┼─────────────────────────┘
           │
           │ ~80–130 ms RTT
           │ TLS 1.3 + mTLS handshake
           │
   ┌───────┼─────────────────────────┐
   │   public internet (BGP)         │
   └───────┼─────────────────────────┘
           │
   ┌───────┼─────────────────────────┐
   │  Hetzner (FSN1 / NBG1 / HEL1 —  │
   │  not yet pinned by BE)          │
   │                                 │
   │  Caddy :443                     │
   │   - Let's Encrypt server cert   │
   │   - require_and_verify mTLS     │
   │       │                         │
   │       │ docker bridge → :8123   │
   │       ▼                         │
   │  ClickHouse (loopback-only)    │
   └─────────────────────────────────┘
```

Concrete settings:

| Element | Value |
|---|---|
| Source | Lambda outside VPC, us-east-1 |
| Egress route | Public internet (no NAT, no VPC endpoint) |
| Transport | HTTPS :443 only (CH native port 9000 not exposed) |
| Wire protocol | ClickHouse HTTP (`POST /` with `JSONEachRow` body and SQL in `query=` param) |
| TLS | TLS 1.3 + mTLS (Caddy enforces) |
| Server identity | Let's Encrypt cert chaining to ISRG Root X1 |
| Client identity | BE self-signed CA |

### 2.2 Latency budget

Compared to today's plan (Lambda-in-VPC writing to RDS-in-VPC):

| Hop | RDS-in-VPC | Hetzner CH |
|---|---|---|
| Network RTT | <1 ms (same AZ) | **~80–130 ms** (us-east-1 ↔ EU) |
| TLS handshake (cold) | Pooled by RDS Proxy, ~5 ms | ~2 × RTT = **~160–260 ms** (TLS 1.3 1-RTT + mTLS hello) |
| TLS handshake (warm) | n/a (reused) | 0 (reused) |
| Per-write driver overhead | sqlx prepared stmt: ~1 ms | HTTP request: **~5–10 ms in-process** + 1 RTT |
| Typical 1-row write | **~3–5 ms** | **~85–135 ms** (warm) / **~250–400 ms** (cold) |
| Typical batch write (100 rows) | ~5–10 ms | **~85–135 ms** (still 1 RTT) |

**Key observation.** The Hetzner path has **~25-40× higher
per-write latency on cold connections, ~25× on warm**. But the
Lambda's actual work isn't 1 row — it's a batch of OHLCV rows
extracted from one ledger (typically 10s–100s of rows on busy
ledgers). With **batched writes** the cost amortises: 1 RTT for
the whole batch.

**Implication for Lambda timeout.** Today's 0038 spec leaves the
timeout at 60s. With Hetzner CH, 60s is still ample even with
worst-case 400ms cold-start writes — there's ~150× headroom.

### 2.3 Connection reuse

`reqwest::Client` with default `pool_idle_timeout` reuses
connections across invocations as long as the Lambda execution
environment stays warm (~5-15 min idle). At Galexie's ~5–6s
cadence, the prices-api Lambda's container effectively never
cold-starts in production traffic. **The cold-start TLS cost is
paid once per container lifetime, then amortised across thousands
of invocations.**

### 2.4 Lambda outbound bandwidth

Lambda (outside VPC) has no published outbound bandwidth cap;
AWS bills standard egress (~$0.09/GB to internet). At ~1 KB
per OHLCV row and ~10 rows/ledger × ~15,700 ledgers/day,
**~150 MB/day outbound** — egress cost ~$0.40/month. Negligible.

### 2.5 DNS

The Hetzner endpoint is a dedicated server with a **fixed
public IP** (per BE task 0227's reverse DNS rule). A stable
DNS name (e.g. `clickhouse.{env}.stellar-explorer.example.com`)
should front it for client convenience — likely set up by BE in
their DNS provider. Prices-api side just consumes the name.
**No DNS rotation surprises** because BE owns the record and is
not load-balancing across multiple boxes (single dedicated host
per step-1).

---

## 3. Failure modes

The S3-event model already provides **structural durability** for
the upstream half. The Hetzner write is the new failure surface.

### 3.1 Existing structural durability

Recap from step 3:

- **S3 retention is indefinite** (BE ADR 0006). Files persist
  whether or not any Lambda has processed them.
- **Lambda async-invoke retry: 2× then DLQ** (BE pattern, same
  shape prices-api will adopt).
- **Replay is supported**: re-firing an S3 event re-invokes the
  Lambda against the same object key.

So the "is the data lost?" question has a structural answer:
**no**. Data sits in S3 until *some* Lambda invocation succeeds.

### 3.2 What happens when Hetzner is unreachable

Consider concrete failure modes:

| Failure | Lambda behavior | Recovery |
|---|---|---|
| Hetzner box rebooting (~minutes) | Lambda HTTPS timeout/connect-refused → throws | First retry (5min default) likely succeeds; if not, DLQ; replay later |
| Caddy down (~minutes) | TLS handshake fails → throws | Same as above |
| Hetzner network unreachable (~hours) | Both retries exhaust → DLQ accumulates | DLQ replay after restoration |
| TLS cert expired (server side) | Handshake fails → throws | Alarm should fire well before; BE Caddy auto-renews so unlikely |
| TLS cert expired (client side) | Handshake fails → throws | Prices-api side; CloudWatch NotAfter alarm avoids surprise |
| ClickHouse OOM / overload | HTTP 5xx → throws | Lambda retry + DLQ |
| Network partial — high packet loss | Slow / timeout → throws | Lambda retry |

**Key property:** every failure mode either (a) self-recovers on
next invocation when the issue clears, or (b) lands in DLQ for
replay. No data-loss path exists as long as S3 retains the source
file — which it does indefinitely per BE ADR 0006.

### 3.3 Backpressure: do we need an intermediate queue?

**Today's blocked plan (0038)** writes Lambda → RDS directly. No
queue.

**Hetzner refactor** options:

| Option | Description | Trade-off |
|---|---|---|
| Direct write | Lambda HTTPS POST to Caddy:443 | Simplest; failure → DLQ + replay |
| SQS in front of Lambda | S3 → SQS → Lambda (instead of S3 → Lambda direct) | Lambda batching; per-message visibility-timeout retry; DLQ tooling stronger |
| SQS between Lambda and Hetzner write | Lambda enqueues an internal "write to CH" job; second Lambda drains | Decouples ingest from write; over-engineered for two consumers |

**Recommendation seed.** **Direct write.** The S3-event-retry-
DLQ chain already gives structural durability; adding SQS in
front complicates the pipeline without addressing a real failure
mode that the existing retries don't cover. Reconsider only if
ops observability of "rows not yet in CH" turns out to be a
real operational gap.

### 3.4 Backfill / replay during Hetzner outages

Long outage scenarios deserve thought: if Hetzner is down for
hours, the DLQ accumulates failed events; backfill once restored.

The DLQ shape (SQS standard queue, BE pattern) supports
re-invocation by either:

1. **Source-mapping the DLQ back to the Lambda** with a small
   batch size, draining at controlled rate.
2. **Manual S3-event replay** via `aws s3 ls` + custom script
   that re-issues PutObject events.

Both are operational tools, not architecture. Document the
runbook as part of the implementation task, not this research.

### 3.5 Hetzner-side capacity limits

The Hetzner CH instance is **shared** with BE; capacity is a
two-tenant question:

- **Ingestion contention.** BE's indexer writes 17 tables per
  ledger; prices-api writes ~5–10 OHLCV rows per ledger.
  Prices-api's write rate is **~1–2 orders of magnitude smaller**
  than BE's. The added load is real but small relative to BE's
  baseline.
- **HTTP connection limits.** Caddy default `max_keepalive_conns`
  is 100; with both consumers in the dozens of warm Lambda
  containers, plus BE's own writers, this is the **first
  knob to size**. Capture as open question.
- **CH side.** `max_connections` in CH config defaults to 4096;
  not a concern at the scale of two tenants.

This is the real **shared constraint** that step 3 closed —
Lambda concurrency caps on the AWS side don't matter; CH-side
HTTP capacity at Caddy:443 does.

---

## 4. Library / driver choice

Three plausible Rust client paths:

| Library | Protocol | mTLS support | Notes |
|---|---|---|---|
| `clickhouse` crate (clickhouse-rs/clickhouse) | HTTP | Yes via `reqwest::Client` | **Recommended.** Maintained, async, supports `Identity` for mTLS, ergonomic insert/select |
| `klickhouse` | Native (TCP 9000) | Possible but unusual | **Disqualified** — Hetzner CH native port is loopback-only |
| `reqwest` raw + JSONEachRow | HTTP | Native reqwest | Fallback if `clickhouse` doesn't compose well with `lambda_runtime` |

The `clickhouse` crate accepts a pre-built `reqwest::Client`,
letting prices-api inject the mTLS-configured client from §1.3
verbatim. Same crate BE plans to use server-side.

**Recommendation seed.** `clickhouse` crate over HTTP with the
mTLS-configured `reqwest::Client`. Single client per warm
container, reused across invocations.

---

## 5. Implications and risks

1. **Latency budget is fine, in steady state.** Warm-container
   batch writes are ~85–135 ms per ledger — well inside the
   60 s Lambda timeout, and the Lambda is bottlenecked on XDR
   parsing CPU, not the CH write hop.

2. **Cold-start cost is paid rarely.** At Galexie's ~5–6 s
   cadence, the Lambda container is effectively always warm.
   Cold-start TLS cost (~250 ms) is irrelevant under steady
   traffic.

3. **No data-loss path** exists as long as BE ADR 0006's
   indefinite retention holds. The S3-event-retry-DLQ chain
   plus indefinite source retention is the durability story;
   no intermediate queue needed.

4. **Cert rotation is the real ops surface.** Auto-renewal on
   server side (Caddy + Let's Encrypt); manual + alarm on
   client side. Acceptable for a first cut; harden later.

5. **Caddy HTTP capacity is the shared constraint to size.**
   Two-tenant load on `max_keepalive_conns` is the operational
   variable that matters; Lambda concurrency on the AWS side
   does not. Document and size early.

6. **No VPC, no NAT, no SG.** This is a strict simplification
   versus the current RDS plan. Removes one whole class of
   networking misconfiguration (subnet routing, VPC endpoint
   policies, NACL drift) that the blocked tasks 0011/0038/0039
   would otherwise have to deal with.

---

## 6. Open questions surfaced by step 4 (forwarded to README)

15. **Client-cert rotation cadence.** 1-year manual vs.
    short-lived automated. Security-vs-ops trade. Default to
    1-year manual + alarm; revisit after first deploy cycle.
16. **DNS endpoint name.** Who owns the record fronting the
    Hetzner IP? Likely BE; needs an explicit handoff.
17. **Caddy `max_keepalive_conns` and `max_concurrent_streams`
    sizing.** The shared-tenant capacity dial. Needs BE to
    publish current values + agreed sizing for two tenants.
18. **DLQ ownership and replay runbook.** Prices-api owns its
    own DLQ; replay tooling lives prices-api side. Document
    as part of the implementation task.
19. **Library upgrade story.** The `clickhouse` Rust crate is
    on minor-version churn; pin the version and document the
    upgrade gating in the eventual ADR.

## 7. What step 4 does NOT cover

- Schema-ownership and per-tenant DB carve-out — step 5.
- Cost delta — step 6.
- Final go/no-go and per-task impact — step 7.
