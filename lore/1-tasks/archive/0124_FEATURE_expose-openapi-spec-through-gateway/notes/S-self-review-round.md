---
title: "Self-review round (post-#169) and the mutation checks"
type: synthesis
status: mature
spawns: []
tags: [code-review, ci-gates, mutation-testing, caching]
links: []
history:
  - date: 2026-08-06
    status: mature
    who: akot
    note: "Split out of the 0124 README at archive time, per its Future Work plan."
---

## Self-review round (post-#169)

A multi-agent review over the branch after the #169 fixes landed: four finder
angles, an independent verifier per finding, 33 candidates → 10 reported, 2
refuted. Adam asked for all of it to be applied.

### The finding that matters most

**Three of the four fixes made for okarcz's review were incomplete in the same
way the originals were** — a check that reads as covering something it does not.
The review caught a class of defect; the fixes reproduced the class one layer
down. That is the lesson of this task, not the individual bugs:

1. **The CI paths-filter entry was necessary but not sufficient.** Adding
   `api-gateway-stack.ts` makes the `rust` job *run* on infra-only PRs. Nothing
   in it would have *failed* for the `servers` half: `extract-openapi.sh` reads
   `apiBaseUrl` from `infra/envs/production.json` and exports `API_BASE_URL`
   itself, so it never observes `ComputeStack` putting that variable on the
   Lambda. Rename it to `API_BASE_URI` in an unrelated refactor and synth, lint
   and the route gate all pass while production serves a document with **no
   `servers` block at all** — the CI copy still has one, fabricated from the JSON
   file. Closed by `verify-openapi-servers.mjs`, which compares the synthesized
   Compute template against the extracted document and re-asserts the
   stage-prefix invariant against the deployed `StageName`, so deleting the
   `types.ts` validation cannot silently remove that guarantee either.

2. **Aligning the two guards' method sets removed coverage instead of adding
   it.** Dropping `head`/`options` from both sides made them agree at the cost of
   leaving a documented HEAD checked by *neither* guard in *either* direction —
   reopening, for two verbs, the exact defect this task exists to close. The
   exclusion is now one-sided: OPTIONS skipped on the gateway side only, HEAD
   compared normally, a documented OPTIONS refused outright. okarcz's nit is
   still satisfied — the guards agree — but by raising the weaker side.

3. **`fullPath()` still truncated silently.** The rewrite threw at two exits, but
   truncation happened earlier: any `ParentId` that was not `{Ref}` became
   `null`, and `null` is the walk's "reached the root" signal. Today only
   `Fn::GetAtt … RootResourceId` lands there, so it works — but an imported
   `RestApi` or a cross-stack split ([[0126]]) emits `Fn::ImportValue`, and the
   check would report `/status` for `/v1/backfill/status`.

4. (Not a repeat, but the same shape.) The ledger test matched a `_ledger` name
   **suffix**, so its own promise held only for that name shape. Replaced with a
   type-derived rule over the schemas reachable from the response `$ref`, plus a
   `contains`-based name rule over the whole document.

### Product-level findings

5. **The "no 4xx to document" premise was contradicted by this branch's own
   stack comment.** Both anonymous routes sit under the stage-wide `/*` `*`
   throttle, so API Gateway can 429 either; `/api-docs-json` can also 5xx,
   being a Lambda proxy. Both are now documented, without a body (API Gateway
   produces them and its shape is not `ErrorEnvelope`). The lint-ignore file is
   empty and stays in the tree carrying the reason.

6. **A 3600 s cache with no invalidation.** "Byte-identical for the life of a
   deployment" is true and beside the point — the caches outlive the deployment.
   See the superseded Design Decision #3.

7. **`{}` served as `200 OK`.** `to_json().unwrap_or_else(|_| "{}")` turned a
   serialization failure into a syntactically valid empty document, with no log
   and no metric, then cached it. Before this branch the route was unroutable, so
   nobody would have seen it; now it is the public partner contract. Now
   `.expect`s, matching `extract_openapi`.

8. **`/api-docs-json` had no limits of its own.** Every other Lambda-backed
   route is key-gated and therefore carries the usage plan's per-key rate and
   daily quota; this one carried neither, and throttling is evaluated *before*
   the cache, so an anonymous loop draws down the bucket shared with paying
   partners. Also: the whole `methodSettings` block sat inside
   `if (cacheEnabled)`, so with the cache off the route lost its TTL entry too —
   the configuration where an unbounded keyless route costs the most.

   **This one goes beyond what #169 accepted** ("the residual is small"; okarcz
   asked only that the cost profile be written down), so it is isolated in
   `479548c` and the PR comment says to `git revert` that single commit if he
   would rather keep the reviewed posture. **Open — awaiting his call.**

9. README named the wrong Redocly ruleset — the same `recommended` /
   `recommended-strict` slip the lint gate exists to prevent.

### Partially addressed

10. **`apiBaseUrl` hardcodes the current execute-api REST API id.** The new
    `servers` guard binds the advertised URL to the synthesized deployment and
    to the deployed `StageName`, so it is no longer unguarded — but it cannot
    detect a REST API *id* change, because both sides derive from the same config
    value. Catching that needs a post-deploy check against the live gateway,
    which is out of scope here.

Refuted during verification, recorded so they are not re-raised: the duplicated
`resourcePath` literal beside the TTL, and the `API_KEY_HEADER` const.


## Mutation checks


| Mutation | Before | After |
| --- | --- | --- |
| `API_BASE_URL` → `API_BASE_URI` in the template | not checked at all | exit 1, points at `compute-stack.ts` |
| `servers` drifts from the handler's config | not checked at all | exit 1, prints both sides |
| documented `head /v1/assets/{id}` | green | exit 1, unroutable |
| documented `options` | green | exit 1, own message |
| `ParentId: {Fn::ImportValue}` | `/assets`, phantom drift | exit 1, names the resource |
| gateway-side `OPTIONS` method | passes | passes ([[0126]] unblocked) |
| `tip_seq: u64` with no `maximum` | green, count still 5 | fails, names the field |
| one ledger literal → `4_294_967_296` | green (the assert was a tautology) | fails, names the field |
