---
id: "0187"
title: "Issue the simplest possible key and show it — AWS is the source of truth, no database"
type: FEATURE
status: active
related_adr: ["0008", "0010"]
related_tasks: ["0183", "0157", "0158", "0160", "0186", "0188", "0190", "0194"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, iam, slice-4]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../archive/0158_FEATURE_discord-key-registry-table.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Fourth slice and the first one a stranger would call the product. Two of
      [[0160]]'s four operations, chosen together because reveal is the same
      lookup as issue. Deliberately built with **no registry table** — [[0158]]'s
      own argument is that API Gateway is the arbiter, and nothing in this slice
      needs a row.
  - date: "2026-08-18"
    status: active
    who: akot
    note: >
      Activated on top of [[0185]] (#218) and [[0186]] (#220), both merged to
      `develop`. The signed session cookie [[0186]] issues is exactly what this
      slice reads to know whose key to issue, so the prerequisite is in place and
      the round-trip is verified locally. Carry [[0186]]'s one open criterion
      forward: the session **through CloudFront** is unverified until [[0205]]'s
      deploy, and these routes sit under the same depth-3 prefix — so a depth-3
      `403` against the deployed gateway belongs to [[0205]], not here.
---

# Issue a key, and show it

## Summary

**Story:** *as a signed-in developer, I press one button, get an API key on
screen, and it works against `/v1/` on the first `curl` — and when I come back
tomorrow the same key is still there.*

This is the slice the epic exists for. After it, self-service is functional for
anyone we let sign in.

## Context

[[0160]] kept issue, reveal, usage and rework together "because they share a
Lambda, an IAM policy and the registry record". They do — but they are four
stories, and bundling them is why nothing was demonstrable until all four
existed. This slice takes the two that answer "where is my key".

**No ClickHouse table.** [[0158]] designed one, and its own "Issue flow" section
explains why it is not needed first: key names are `discord-<userId>-key`, and
**API Gateway, not our store, is the source of truth for whether a key exists**.
The registry buys a hot-path read and a history; neither is required here. It
returns as [[0190]], which has to justify itself.

## Implementation

**Issue** — `POST /api-tokens/api/key`

1. `GetApiKeys(nameQuery = "discord-<userId>-key")`, **page to exhaustion**, then
   **filter to exact name equality in the client**.
2. Nothing found → `CreateApiKey(name = "discord-<userId>-key")` + tag it +
   `CreateUsagePlanKey` onto the free plan.
3. More than one survives the filter → keep the earliest `createdDate`,
   `DeleteApiKey` the rest. This is the reconciler, and it is deterministic:
   both sides of a double-submit read the same list and compute the same winner.

**Reveal** — `GET /api-tokens/api/key` → `GetApiKey(includeValue=true)`. If it
404s, re-enter the issue flow from step 1 and adopt or re-create: a key deleted
by hand in the console otherwise leaves the user with a dead id forever
([[0160]] "Settled 2026-08-07" #4).

**Three things measured on 2026-08-12 that this code must respect** (evidence:
archived `0180/notes/R-apigw-namequery-quota-and-disable.md`):

- `nameQuery` is a **case-sensitive prefix match**, not exact. So the
  client-side exact filter is load-bearing, not defence in depth. Comment it as
  such — "AWS returns prefixes and never promised not to" — so it is not later
  simplified away.
- The prefix hazard is therefore **real**, not hypothetical: Discord snowflakes
  are 17–19 digits, so a shorter id prefixes a longer one, and step 3 would
  delete a stranger's key. The `-key` suffix is what prevents it. Keep it.
- A `nameQuery` result **still paginates** — it comes back with a `position`
  token. Ranking by earliest `createdDate` off page one can pick a winner from a
  partial list and delete a key it never saw.

**Other requirements**

- **`Cache-Control: no-store` on the reveal response, and `cachingEnabled: false`
  on these methods.** Not deferrable to [[0194]]: `deployOptions.cachingEnabled`
  is on in this stack and the gateway cache has no cache-key parameters, so every
  caller collapses onto one entry — a cached reveal hands one user another user's
  key. The CloudFront behaviour must not cache these paths either.
- **IAM, narrow:** `apigateway:POST` on `/apikeys` and
  `/usageplans/{freePlanId}/keys`; `GET` on `/apikeys` (the **collection** — the
  reconciler lists it, and without this every path here fails at runtime with
  `AccessDenied`), on `/apikeys/{id}`; `DELETE` on `/apikeys/{id}`. No wildcards.
  `POST /apikeys` cannot be narrowed further — there is no ARN for "keys this
  function created" — so record that as a consciously accepted limit, mitigated
  by tagging and by attaching only to the self-service plan. [[0194]] audits it.
- **Read the plan id from SSM** ([[0157]]), never hard-coded and never a
  cross-stack reference: `ComputeStack` is a dependency of `ApiGatewayStack` and
  cannot import from it.
- **Never log a key value**, including error paths and X-Ray annotations.
- Handlers in their own module under their own path prefix inside the existing
  `prices-api` router, so the IAM additions are obviously attributable to them.
- Frontend: a button, and the key masked by default with a reveal toggle and a
  copy button. The masking and the copy button are the only two UI niceties that
  earn their place before [[0193]] — one because this renders during
  screen-shares, the other because it is what people actually use.

**Not in this slice:** eligibility ([[0189]]), usage ([[0188]]), rework
([[0191]]), revocation ([[0192]]).

**And the gap that leaves is the reason [[0183]] exists.** Until [[0189]] lands,
this code issues a real key on the real usage plan to anyone who can sign in —
and **this deploy goes to production**, because `envName` is typed `'production'`
and `infra/envs/` holds only `production.json`. There is no dev distribution to
be relaxed on. The only thing standing between a stranger and a production key
for the three slices between here and the gate is `PORTAL_ENABLED=false`, so
treat that flag as part of this slice's correctness, not as ops hygiene.

The same applies to the reconciler above: it calls `DeleteApiKey` against
production keys, with the snowflake prefix hazard live. Exercise it from a local
run against keys this task created, and nothing else — and remember the flag
lives in the Lambda, so it does not protect a laptop holding production
credentials.

## Acceptance Criteria

- [ ] **Ships closed — this is the slice the flag exists for.** With
      `PORTAL_ENABLED=false` ([[0183]]) the issue route is unreachable in
      production, because until [[0189]] lands that flag is the only thing
      between a stranger and a real key. Note the flag does **not** protect a
      local run holding production credentials: those keys are real, and
      [[0194]] cleans them up
- [ ] First press issues a key attached to the free plan; the value is shown
- [ ] That key returns `200` from a `/v1/` route on the first try
- [ ] A second press returns the **same** key, not a new one
- [ ] Signing out and back in still shows the same key
- [ ] Two concurrent first presses converge on one key, and the loser is deleted
- [ ] A key deleted by hand in the console is adopted or re-created on the next
      reveal, not returned as a dead id
- [ ] The reconciler pages `GetApiKeys` to exhaustion before ranking
- [ ] A user id that is a prefix of another user's id cannot see or delete that
      other user's key
- [ ] Reveal is not cached at either layer, verified against the synthesized
      template rather than assumed
- [ ] IAM policy names specific resources; the un-narrowable `POST /apikeys` is
      documented as an accepted limit
- [ ] No key value appears in any log or trace

## Notes

- Epic AC 2 is satisfiable at the end of this slice for anyone with a Discord
  account. AC 2 *as the reviewer means it* needs [[0189]] as well.
- These are control-plane calls, throttled far harder than the data plane and
  metered per account — the same budget our CDK deploys draw on. Backoff belongs
  here; the `GetUsage` in-process cache belongs to [[0188]].
