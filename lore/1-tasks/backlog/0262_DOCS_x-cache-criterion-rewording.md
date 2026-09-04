---
id: "0262"
title: "Tranche 2 AC 3 cannot pass as written — the `X-Cache: Hit` header does not exist and is not being added; the criterion needs the reviewer's agreement"
type: DOCS
status: backlog
related_adr: ["0008"]
related_tasks: ["0122", "0121", "0128", "0126"]
tags: [layer-infra, priority-high, effort-small, milestone-M2, api-gateway, caching, acceptance, scf]
milestone: 2
links:
  - "../../../docs/prices-api-cache-verification.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0122]] future work. 0122 verified the cache and closed all
      seven of its own ACs, but the Tranche 2 criterion it owns is worded around
      a response header that the deployed API does not emit and that we decided
      not to add. The engineering is finished; what is left is a conversation,
      so it is a task of its own rather than an open AC on a task that is done.
      To be discussed with the team before the milestone package is submitted.
---

# Tranche 2 AC 3 — the `X-Cache: Hit` criterion, and why we are not satisfying it literally

## Summary

Tranche 2 acceptance criterion 3 reads:

> *"Cache confirmed: consecutive identical requests within TTL window return
> `X-Cache: Hit` header."*

**The cache is real, verified and behaving correctly. The header does not
exist, cannot be produced honestly without rebuilding the edge, and would not
match the wording even then.** [[0122]] established all of this and wrote it up
in `docs/prices-api-cache-verification.md`.

The only thing outstanding is agreement — ours as a team, then the reviewer's —
that the criterion is graded against the reworded version below. **This is not
an engineering task.** It carries no code. If the answer comes back "add the
header anyway", that is a 3-5 day CloudFront project and gets its own task.

⚠️ **The risk this task exists to remove:** a reviewer runs one `curl`, sees no
`X-Cache`, and marks AC 3 failed — with the evidence document unread. That is a
cheap thing to lose to.

## The proposed rewording

> *"Cache confirmed: consecutive identical requests within the TTL window are
> served from the API Gateway stage cache, and a request after the window is
> not. Demonstrated by response latency, which separates cleanly, and by the
> deployed per-method cache configuration."*

Two changes, and both should be stated openly rather than slipped in:

1. **The observable changes** from a header to latency + deployed config.
2. **The claim weakens.** A header is the cache asserting itself. Latency is
   *behaviour consistent with a cache*. We should say so in [[0128]] in those
   words — the distinction must not be blurred at submission time.

**Precedent that this kind of amendment is normal here**: M1 declared
"In-tranche scope refinements"; Tranche 2 AC 2 already carries an in-place
amendment note about [[0157]]; AC 6 defers to "spot-check dates provided by
reviewer". Declaring a refinement is established practice on this milestone. It
is still better to raise it *before* submission than to have the reviewer find
it in [[0128]].

## What was found (0122, 2026-09-03)

### 1. There is no `X-Cache` header, on any route

Verified three times independently — 2026-08-20 during [[0121]], and twice on
2026-09-03. A live `/price` 200 carries `x-amzn-requestid`, `x-amz-apigw-id`,
`x-amzn-trace-id`, `cache-control`, `vary`, `access-control-allow-origin` — and
nothing else. **API Gateway's stage cache emits no hit/miss header at all.** It
is not a configuration we failed to switch on; the feature does not exist.

The criterion's wording almost certainly came from CloudFront, which does emit
one. See point 4 for why that does not rescue it either.

### 2. 🔴 The cheap fix was tested and it *lies*

The obvious idea — have the Lambda handler set `X-Cache: Miss`, and something
rewrite it to `Hit` on a cached reply — **cannot work, and this was measured,
not assumed.**

API Gateway replays a cached response **byte for byte, headers included**. The
Lambda runs only on a miss. So a header the handler writes is frozen at miss
time and is replayed verbatim on every subsequent hit.

Proof, from the TTL runs on `/price`:

| request | body hash |
|---|---|
| first ask (miss) | `4694801b` |
| +2 s (hit) | `4694801b` — identical |
| +4 s (hit) | `4694801b` — identical |
| +13 s (miss, expired) | `039f54a6` — changed |

The response is literally the same bytes until the entry expires. A
handler-written header would therefore report **`Miss` on every genuine hit** —
the exact inverse of what the criterion asks for. **That is worse than no
header**, because it would be wrong in a way a reviewer could reasonably act
on. The same reasoning rules out integration-response header mappings: they run
on the integration, and a hit never reaches it.

### 3. A truthful header needs a cache *in front of* the gateway. Nothing is.

The REST API is `REGIONAL`, and both custom domains are regional endpoints with
`distributionDomainName: null`. There is no CloudFront distribution in the path
today.

Putting one there would mean:

- ~6 cache behaviours to reproduce the current per-route, per-parameter key
- an origin request policy that keeps `x-api-key` **out** of the cache key
  (otherwise every API key gets a private cache and the hit rate collapses)
- a DNS move for `prices-api.sorobanscan.rumblefish.dev`
- a decision about the existing 0.5 GB stage cache — keep, shrink, or remove
- re-verification of everything [[0126]] settled (CORS, gateway responses, the
  MOCK routes) behind a new edge

**3-5 days, plus edge deploy risk.** Cost is roughly neutral, so cost is not the
argument: ~$12-13/mo of CloudFront against ~$14-15/mo of stage cache recovered.
One thing that would *not* have cost anything: a `*.sorobanscan.rumblefish.dev`
certificate already exists in `us-east-1`.

### 4. 🔑 The deciding argument — CloudFront does not satisfy the wording either

**CloudFront emits `X-Cache: Hit from cloudfront`, not `X-Cache: Hit`.**

So after a week of edge work and a DNS migration, a reviewer reading the
criterion literally still sees a header that does not say `Hit`, and **the same
conversation is still required**. That is what settles it: option (b) does not
actually buy the thing it costs 3-5 days to buy.

This is the single most important sentence to have ready for the team
discussion.

### 5. What we have instead — the latency evidence

Measured as server time only (`time_starttransfer - time_appconnect`), so the
~45 ms TLS handshake a fresh `curl` pays is excluded and the numbers are
comparable to [[0121]]'s k6 figures.

`/price`, declared 10 s TTL, same URL throughout:

| | server time |
|---|---|
| first ask (MISS) | 144.7 ms |
| +2 s (HIT) | 53.3 ms |
| +4 s (HIT) | 45.1 ms |
| +6 s (HIT) | 47.7 ms |
| **+13 s (MISS — expired)** | **139.7 ms** |
| +15 s (HIT) | 49.5 ms |

**Hits 45-53 ms, misses 78-145 ms, no overlap.** Expiry demonstrated on both
tiers — `/v1/assets` at 60 s was still hot at +32 s and expired at +64 s — and
both re-filled immediately. The hit figures reproduce 0121's 45-47 ms
independently, from a different tool on a different day.

All six key-gated cached routes show the same pattern: `/assets` 234→154 ms,
`/assets/{id}` 148.8→44.9, `/ohlcv` 190.3→70.0, `/oracles/{id}` 90.9→54.8,
`/backfill/status` 68.4→48.9.

### 6. ⚠️ The honest weakness — the evidence is not uniformly strong

The gap between a hit and a miss tracks **how expensive the underlying query
is**, so the method discriminates unevenly:

| route | miss/hit ratio | daylight |
|---|---|---|
| `/ohlcv` | 2.7x | 120 ms |
| `/price` | ~2.9x | ~95 ms |
| **`/backfill/status`** | **1.4x** | **19.5 ms** |

19.5 ms is real and repeatable but close enough to ordinary variance that a
single pair of requests would not settle it. **A header would not have this
property.** [[0128]] must not present the six routes as uniformly strong — if
we are going to argue the weaker evidence is sufficient, we have to be the ones
who name its limits.

`/api-docs-json` is a further gap: confirmed **configured** (3600 s TTL, no key
parameters) but deliberately **not** claimed as demonstrated, because its entry
only clears on a deploy flush, so no miss can be induced to compare against.

### 7. Everything else on the criterion's subject checks out

Worth having to hand, because it is what makes the rewording a fair trade
rather than a retreat:

- **TTLs**: the deployed stage, the CDK `CACHE_TTL` and the Rust handler's
  `cache_control.rs` tiers all agree. §6 of the overview was the only wrong
  copy and was corrected (PR #279).
- **Cache-key composition**: measured per method. `x-api-key` is in **no** cache
  key, so the cache is shared across callers — correct for identical public
  data, and now confirmed rather than assumed.
- **Negative cases**: `POST /v1/prices/batch` and `/health` both read
  `cachingEnabled: false` on the deployed stage.
- **Hit rate** under the 0121 load regimes: **≥99.90 %**, **≥98.20 %** (the AC
  scenario) and ~0 % — derived as lower bounds from pool size and TTL, since
  there is no header to count.

## Implementation

There is no code. The work is:

- Discuss with the team. The one-line version: *the header does not exist, the
  cheap fix would lie, and the expensive fix emits the wrong string anyway.*
- Decide as a team whether to (a) propose the rewording to the reviewer, or
  (b) commission the CloudFront work as a separate task. 0122 recommends (a).
- If (a): raise it with the reviewer **before** the milestone package is
  submitted, not by declaring it inside [[0128]] and hoping. Point at
  `docs/prices-api-cache-verification.md`, which is written for that reader.
- Record the outcome — in this task, and in [[0128]]'s criterion table.
- If (b): open a new task for the CloudFront migration and carry §3's list into
  it as the scope. Do not reopen [[0122]]; its verification work is done and
  independent of which way this goes.

## Acceptance Criteria

- [ ] The team has discussed the decision and chosen (a) rewording or (b) the
      CloudFront build.
- [ ] If (a): the reviewer has been asked, in writing, before submission, and
      the answer is recorded here.
- [ ] [[0128]]'s AC 3 entry states which observable the criterion was graded
      against, and labels the latency evidence as the weaker claim in those
      words — not blurred.
- [ ] If (b): a CloudFront task exists carrying the scope from §3, and this
      task records why the 3-5 days were judged worth spending given §4.

## Notes

- **Do not reverse 0122's decision by accident.** "We should just add the
  header" is the intuition this task exists to answer; §2 and §4 are why it was
  already tested and rejected. A new argument would have to defeat *both* —
  that the handler cannot see a hit, and that CloudFront emits the wrong string.
- The evidence document `docs/prices-api-cache-verification.md` is
  reviewer-facing and self-contained. It leads with a STATUS block, states the
  absence plainly, proves the handler cannot mark a hit, costs the CloudFront
  alternative, and proposes the rewording. It is the artefact to send, not this
  task file.
- 🔴 **Safety, inherited from [[0121]] and still in force**: do not drive cache
  misses at load. On 2026-09-03 that took the ClickHouse read path down for
  19-47 minutes ([[0260]]). Nothing in this task needs load — a re-run costs a
  handful of `curl`s.
- A cheaper re-run exists if [[0128]] wants fresh numbers close to submission: a
  caller granted `InvalidateCache` on the usage plan can force a miss on any
  route with `Cache-Control: max-age=0`, instead of the 63-second waits 0122
  used. It needs a deliberate permission grant, which is why 0122 did not take
  it.
