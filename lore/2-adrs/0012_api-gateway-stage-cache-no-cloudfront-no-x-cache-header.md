---
id: "0012"
title: "Keep the API Gateway regional stage cache; do not adopt CloudFront and do not emit an X-Cache header"
status: accepted
deciders: [okarcz]
related_tasks: ["0122", "0262", "0126", "0128"]
related_adrs: ["0008"]
tags: [architecture, api-gateway, caching, cloudfront, edge, cost, scf, acceptance]
links:
  - "../../docs/prices-api-cache-verification.md"
history:
  - date: 2026-09-04
    status: accepted
    who: okarcz
    note: >
      Decided after [[0122]] verified the cache and costed the alternative.
      Operator instruction, verbatim in intent: do not create CloudFront and do
      not add an `X-Cache` header on API Gateway at this point in the project;
      record why in a document. This ADR is that record.
  - date: 2026-09-04
    status: accepted
    who: okarcz
    note: >
      **Confirmed by the team lead** on the same day, after review of this
      record. The decision is therefore the team's and not one person's, which
      is the standing it needs before it is put to the SCF reviewer in [[0262]].
      No change to the decision or the reasoning — this entry records
      ratification only.
---

# ADR 0012: The API Gateway stage cache stays; no CloudFront, no `X-Cache` header

**Related:**
- [Task 0122: API Gateway cache verification](../1-tasks/archive/0122_TEST_apigateway-cache-ttl-verification.md)
- [Task 0262: the Tranche 2 AC 3 wording](../1-tasks/active/0262_DOCS_x-cache-criterion-rewording.md)
- [Evidence: `docs/prices-api-cache-verification.md`](../../docs/prices-api-cache-verification.md)

---

## Context

The API's response cache is the **API Gateway REST stage cache** — 0.5 GB,
enabled per method, with per-route TTLs and per-route cache keys. [[0122]]
verified it end to end on production: TTLs match the code on all three
surfaces, hits and post-expiry misses were demonstrated on every key-gated
route, and `x-api-key` is in no cache key, so the cache is correctly shared
across callers.

One thing it cannot do is **say** that it served a hit.

**API Gateway emits no `X-Cache` header, on any route.** This is not a setting
that was left off; the feature does not exist. Verified three times
independently — 2026-08-20 during [[0121]], and twice on 2026-09-03.

That matters because the funded Tranche 2 acceptance criterion 3 is worded:

> *"Cache confirmed: consecutive identical requests within TTL window return
> `X-Cache: Hit` header."*

Almost certainly written with CloudFront in mind, which does emit one. So the
question this ADR answers is not "does the cache work" — it does — but **what,
if anything, we change in order to be able to point at a header.**

---

## Decision

**Three things, and the third is the one that is easy to get wrong.**

### 1. The API Gateway regional stage cache remains the caching layer

No change to the deployed architecture. The REST API stays `REGIONAL`, both
custom domains stay regional endpoints, and the 0.5 GB stage cache keeps
serving the per-route TTLs [[0122]] verified.

### 2. CloudFront is not adopted at this point in the project

Not rejected forever — **deferred, with a stated trigger** (see *When to revisit*
below). The project does not currently have the problem CloudFront solves.

### 3. No `X-Cache` header is emitted, by any mechanism

Including the cheap ones. A header written by the Lambda handler is not a
lesser version of the real thing; it is **actively wrong**, and that is the
finding that settles this.

---

## Rationale

### Why the stage cache is the right layer for this project *now*

CloudFront's value is putting a copy of the response in ~hundreds of edge
locations so a distant client is served locally. That is a **latency-for-distant-users**
optimisation, and it is real — but:

- Our origin, our Lambda and our ClickHouse cluster all sit in one European
  region. The expensive hop is the database, and **the stage cache already
  removes it**: [[0122]] measured hits at **45-53 ms** against misses at
  **78-145 ms**, with no overlap.
- Nothing in Tranche 2 asks for global edge latency, and no measured user
  problem points at it. Adding an edge tier to satisfy a *header* would be
  re-architecting for a documentation artefact.
- The stage cache is already correct on the properties that are hard to get
  right: per-route TTLs, per-route cache keys, and `x-api-key` deliberately
  excluded from every key so callers share one cache.

### 🔑 Why a handler-written `X-Cache` would lie — this was measured, not assumed

**API Gateway replays a cached response byte for byte, headers included, and
the Lambda runs only on a miss.** So a header the handler writes is frozen at
miss time and replayed verbatim on every subsequent hit.

Demonstrated on `/price` across a TTL boundary:

| request | body hash |
|---|---|
| first ask (miss) | `4694801b` |
| +2 s (hit) | `4694801b` — identical |
| +4 s (hit) | `4694801b` — identical |
| +13 s (miss, expired) | `039f54a6` — changed |

A handler-set header would therefore report **`Miss` on every genuine hit** —
the exact inverse of the criterion. **That is worse than no header**, because
it is wrong in a way a reviewer could reasonably act on. The same reasoning
rules out integration-response header mappings: they run on the integration,
and a hit never reaches it.

### 🔑 And CloudFront would not satisfy the wording either

**CloudFront emits `X-Cache: Hit from cloudfront`, not `X-Cache: Hit`.**

So after 3-5 days of edge work and a DNS migration, a reviewer reading the
criterion literally still sees a header that does not say `Hit`, and the same
conversation with the reviewer is still required. **Option (b) does not buy the
thing it is expensive for.** This is the deciding argument; cost is not.

### Cost is genuinely not the reason

Roughly neutral: ~\$12-13/mo of CloudFront against ~\$14-15/mo of stage cache
recovered if it were removed. A `*.sorobanscan.rumblefish.dev` certificate
already exists in `us-east-1`, so even that would have cost nothing. We are not
declining CloudFront to save money.

### What we present instead

Latency, labelled as the weaker claim it is. Hits **45-53 ms**, misses
**78-145 ms**, no overlap, with expiry demonstrated on both the 10 s and 60 s
tiers. This shows *behaviour consistent with a cache* rather than the cache
asserting itself, and [[0128]] must carry that distinction rather than blur it.

⚠️ Its honest weakness is published too: the miss/hit ratio tracks how expensive
the underlying query is — 2.7x on `/ohlcv` (120 ms of daylight) but only **1.4x
on `/backfill/status`** (19.5 ms), close enough to ordinary variance that a
single pair of requests would not settle it.

---

## Alternatives Considered

### Alternative 1: Handler-written `X-Cache`, flipped somewhere downstream

**Description:** Lambda sets `X-Cache: Miss`; something rewrites it to `Hit` on
a cached reply.

**Cons:** There is no "somewhere downstream". The gateway replays bytes and the
handler does not run on a hit — proven by the body hashes above.

**Decision: REJECTED — it does not merely fail, it reports the opposite of the
truth.**

### Alternative 2: Adopt CloudFront in front of the regional API

**Description:** Put a distribution in front, reproduce the per-route cache
behaviour at the edge, move DNS, and emit the CloudFront `X-Cache` header.

**Pros:** A real hit/miss header. Genuine latency improvement for distant users.
Cost roughly neutral. Certificate already in place.

**Cons:** ~6 cache behaviours to reproduce the per-route, per-parameter key; an
origin request policy that must keep `x-api-key` **out** of the key (otherwise
every API key gets a private cache and the hit rate collapses); a DNS move on a
live API; a decision about the existing stage cache; re-verification of
everything [[0126]] settled (CORS, gateway responses, the MOCK routes) behind a
new edge. **3-5 days plus edge deploy risk** — and it still emits
`Hit from cloudfront`, so the literal wording remains unmet.

**Decision: REJECTED for now — deferred, not refused. See *When to revisit*.**

### Alternative 3: Change the API Gateway endpoint type to `EDGE`

**Description:** API Gateway's `EDGE` endpoint type fronts the API with a
CloudFront distribution AWS manages.

**Cons:** It is CloudFront with less control — the distribution is not ours to
configure, so the per-route cache-key reproduction that Alternative 2 needs is
not available, and the same header-wording problem applies. It would also be a
larger change to [[0126]]'s settled edge behaviour than it appears.

**Decision: REJECTED — inherits Alternative 2's blocking flaw with less
control.**

---

## Consequences

### Positive

- **Nothing is re-architected to satisfy a documentation artefact.** The
  caching layer stays the one that is deployed, verified and correct.
- No DNS move, no edge deploy risk, no re-verification of [[0126]].
- The `x-api-key`-excluded shared cache — the property that makes the hit rate
  work at all — is preserved by not touching it.
- The evidence is already written and reviewer-facing:
  `docs/prices-api-cache-verification.md`.

### Negative

- 🔴 **Tranche 2 AC 3 cannot pass as literally worded, and this needs the
  reviewer's agreement.** That conversation is [[0262]] and must happen
  **before** submission, not be declared inside [[0128]] and hoped through. The
  risk being managed: a reviewer runs one `curl`, sees no `X-Cache`, and marks
  the criterion failed with the evidence document unread.
- The evidence is **inference from latency**, not assertion by the cache. Weaker,
  and route-dependent — barely 1.4x on `/backfill/status`.
- No hit/miss telemetry per response. CloudWatch's `CacheHitCount` /
  `CacheMissCount` remain the only aggregate view.
- Users far from the origin region keep paying the round trip. Acceptable now;
  it is the thing that will change first (below).

### When to revisit

CloudFront becomes the right call — on its own merits, not for a header — when
any of these is true:

1. **Measured latency for geographically distant clients** becomes a real
   complaint or a stated requirement.
2. **Origin egress or request volume** makes edge caching cheaper than the
   current shape rather than neutral.
3. A launch requirement needs something only an edge tier gives — WAF at the
   edge, custom error pages, request signing, or geo controls.

⚠️ **A future `X-Cache` request is NOT on that list.** If the header alone is
the motivation, re-read this ADR: CloudFront emits `Hit from cloudfront` and
does not satisfy the wording either.

---

## References

- `docs/prices-api-cache-verification.md` — the reviewer-facing evidence:
  measured TTLs, cache-key composition, hit and expiry demonstrations, the
  body-hash proof, and the proposed rewording.
- [[0122]] — the verification task; all seven acceptance criteria closed.
- [[0262]] — the outstanding reviewer conversation about AC 3's wording.
- [ADR 0008](./0008_single-axum-lambda-for-prices-api.md) — the single-Lambda
  runtime this cache sits in front of.
