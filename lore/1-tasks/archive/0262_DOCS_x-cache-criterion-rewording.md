---
id: "0262"
title: "Tranche 2 AC 3 cannot pass as written — the `X-Cache: Hit` header does not exist and is not being added; the criterion is amended in place and justified in the repo"
type: DOCS
status: completed
related_adr: ["0008", "0012"]
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
  - date: 2026-09-04
    status: active
    who: okarcz
    note: >
      Activated, and the engineering question is **decided by the operator**:
      do **not** create CloudFront and do **not** add an `X-Cache` header on API
      Gateway at this point in the project. Record the reasoning instead —
      why the API Gateway stage cache is the right layer for where the project
      is, and why a gateway with no built-in cache header means the header is
      skipped rather than faked. Written up as **ADR 0012**. What remains is the
      conversation this task always owned: the team, then the reviewer, on AC
      3's wording — no code, and it must happen before submission.
  - date: 2026-09-07
    status: active
    who: okarcz
    note: >
      🔒 **Second decision by the operator: the reviewer is not messaged.** The
      criterion is amended in place and the amendment is justified in the repo,
      inside the submission package. Delivered: `milestone-2-rfp-deviations.md`
      §2 expanded from 22 lines to the full argument (~146), an in-place
      amendment note on AC 3 in the overview's criteria list matching the one
      AC 2 already carries, and the cache-verification document reframed from
      requesting a rewording to declaring one. [[0128]] activated the same day
      so the remaining AC has somewhere to land. ⚠️ The engineering decision of
      2026-09-04 is untouched — no CloudFront, no header; see ADR 0012.
  - date: 2026-09-07
    status: completed
    who: okarcz
    note: >
      Closed. 4 of 5 criteria met; the fifth is **deferred to [[0128]]**, where
      it is now written as a numbered acceptance criterion rather than left as
      prose, because the artefact it applies to (`milestone-2-evidence.md`) is
      0128's deliverable and does not exist yet. PR #288 merged (4 files,
      +238/-44): `milestone-2-rfp-deviations.md` §2 22 → ~146 lines,
      an in-place amendment note on AC 3 in the overview's criteria list, and
      `prices-api-cache-verification.md` reframed from requesting a rewording
      to declaring one. No code, as designed. ADR 0012 unedited.
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

**The engineering question is settled — 2026-09-04, by the operator.** No
CloudFront, no `X-Cache` header, at this point in the project. Recorded as
**[ADR 0012](../../2-adrs/0012_api-gateway-stage-cache-no-cloudfront-no-x-cache-header.md)**,
which carries the reasoning, the three rejected alternatives, and — the part
that matters most for a future session — an explicit **"when to revisit"**
list, with a warning that *"someone asked for `X-Cache` again"* is **not** on
it.

**The delivery question is settled too — 2026-09-07, by the operator.** The
reviewer is **not** messaged. The amendment is declared and justified *in the
repo*, carried to the reviewer by the submission package itself. The reasoning
is that the argument is long and evidence-led and already written down; a chat
message would be a lossy second copy of it that then drifts. M1 was accepted
carrying an *"In-tranche scope refinements"* section of exactly this shape.

⚠️ This supersedes, but does not contradict, the earlier plan above. Nothing
about the **engineering** decision changed — ADR 0012 stands unedited.

There is no code in this task and there never was. Delivered 2026-09-07:

- **`docs/scf/milestone-2-rfp-deviations.md` §2 — the artefact that does the
  work.** Was a 22-line pointer; now the full argument. States the reworded
  criterion verbatim, names the weakened claim in those words, proves the
  handler-written header would report `Miss` on every genuine hit (body-hash
  table), shows CloudFront emits the wrong string, gives the precedent for
  amending in place, presents the latency evidence, and names
  `/backfill/status` as the weakest route rather than letting a reader find it.
- **`docs/prices-api-general-overview.md` §12** — AC 3 now carries an in-place
  amendment note, the same treatment AC 2 already has for [[0157]].
- **`docs/prices-api-cache-verification.md`** — reframed from *requests a
  rewording* to *declares one*. Evidence untouched.

What is left: [[0128]] must carry the AC 3 entry. Activated 2026-09-07.

## Acceptance Criteria

- [x] The team has a decision on whether to reword or to build the edge —
      **decided 2026-09-04: reword. No CloudFront, no `X-Cache`.**
      ✅ **Ratified by the team lead the same day**, so what goes to the
      reviewer is a team position rather than one developer's call.
- [x] The reasoning is written down where a future session will find it before
      re-opening the question — **ADR 0012**, accepted, cross-linked from this
      task and from `docs/prices-api-cache-verification.md`.
- [x] 🔴 ~~The reviewer has been asked, in writing, before submission~~ —
      **superseded 2026-09-07 by the operator: no message is sent.** The
      amendment is *declared and justified in the repo*, where the submission
      package carries it, rather than negotiated in advance. Reason: the
      argument is long, evidence-led and already written; a chat message would
      be a lossy second copy of it, and M1 was accepted on exactly this
      discipline of declaring in-tranche refinements in the package itself.
- [x] The justification a reviewer reads is written and is part of the M2
      package — `docs/scf/milestone-2-rfp-deviations.md` §2, rewritten from a
      20-line pointer into the full argument: the reworded criterion verbatim,
      why the header cannot exist, both closed alternatives with their
      measurements, the precedent for amending in place, and the evidence with
      its weakest route named.
- [x] The criteria list itself carries an in-place amendment note against AC 3
      — `docs/prices-api-general-overview.md` §12, matching the note AC 2
      already carries about [[0157]].
- [x] The two artefacts no longer describe a conversation that will not happen
      — `docs/prices-api-cache-verification.md` reframed from *requests a
      rewording* to *declares one*; ADR 0012 unchanged, it was already written
      as a decision.
- [ ] [[0128]]'s AC 3 entry states which observable the criterion was graded
      against, and labels the latency evidence as the weaker claim in those
      words — not blurred. **(deferred to [[0128]])** — 0128 was activated
      2026-09-07 and now carries this as its own numbered acceptance criterion,
      quoting the required sentence. The artefact it applies to,
      `docs/scf/milestone-2-evidence.md`, is 0128's deliverable and does not
      exist yet, so this could not be satisfied here.

## Implementation Notes

**PR #288**, merged 2026-09-07 — 4 files, +238/-44. No code, by design.

| file | change |
| --- | --- |
| `docs/scf/milestone-2-rfp-deviations.md` §2 | 22 → ~146 lines: the reworded criterion verbatim, the weakened claim named, the body-hash proof, CloudFront's wrong string, the amendment precedent, the latency evidence with its weakest route named |
| `docs/prices-api-general-overview.md` §12 | in-place amendment note against AC 3, matching AC 2's note about [[0157]] |
| `docs/prices-api-cache-verification.md` | reframed from *requests a rewording* to *declares one*; measurements untouched |

**Verified before citing**, rather than repeated from this task file: M1's
accepted `milestone-1-form-answers.md` really does carry an *"In-tranche scope
refinements"* section; Tranche 2 AC 2 in the overview really does carry an
in-place amendment note about 0157; AC 6 really does defer its spot-check dates
to the reviewer. All three are load-bearing for the argument in §2.

## Design Decisions

### From Plan

1. **The criterion is amended, not built to.** ADR 0012 stands: no CloudFront,
   no `X-Cache` header. Nothing here reopened it.

### Emerged

2. **🔒 The reviewer is not messaged — operator decision, 2026-09-07.** This
   supersedes the plan this task was written around, which was to raise the
   wording in writing before submission. The amendment is instead *declared and
   justified in the repo*, carried by the submission package. Reason: the
   argument is long and evidence-led and already written; a message would be a
   lossy second copy of it that then drifts out of step with the package.
3. **No new document was created.** The instruction was to write the
   justification down, and the obvious reading was a new file. Rejected: the
   argument already lived in three places, and a fourth copy is precisely the
   drift this project has been bitten by. §2 of the deviations document was
   expanded instead — it is the file that is already *in* the M2 package.
4. **The criteria list itself was annotated.** Not asked for. Added because a
   reviewer reading the criteria in the overview would otherwise meet AC 3
   unqualified, and AC 2 already establishes the pattern.
5. **The two older artefacts were reframed, not left alone.** Both still said
   the rewording *"needs the reviewer's agreement"*. Left as-is they would have
   described a conversation that is not going to happen, and a future session
   would have re-opened it.

## Issues Encountered

- **This task's own framing was stale in three files at once** — its title, its
  Implementation section and two `docs/` artefacts all described the reviewer
  ask. 🔑 A decision recorded in one place while the other copies still state
  the old plan is not recorded; that is the same failure mode already on record
  for a decision left contradicted by a code comment.

## Future Work

None spawned. The single remaining item is deferred to [[0128]], which is
active and carries it as a numbered criterion.

## Notes

- 🗄️ **The alternatives are closed, not merely unchosen.** ADR 0012 records all
  three — a handler-written header (reports the *opposite* of the truth), a
  CloudFront distribution (emits `Hit from cloudfront`, so the literal wording
  is still unmet after 3-5 days), and an `EDGE` endpoint type (CloudFront with
  less control). Reopening any of them needs a new argument, not a re-reading.

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
