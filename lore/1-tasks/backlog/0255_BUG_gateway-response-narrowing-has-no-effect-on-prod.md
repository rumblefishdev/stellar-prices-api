---
id: "0255"
title: "The THROTTLED 429 still carries the portal's credentialed origin, and why the 4xx leak stopped is still unexplained"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0126", "0194"]
tags: ["priority-medium", "effort-small", "api-gateway", "cors", "infra", "verification"]
links:
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-09-02
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0126]] after probing prod. [[0194]]'s code review found the
      CORS gateway response was DEFAULT_4XX — API-wide, stamping the portal's
      single origin onto every keyless /v1 403 — and narrowed it to THROTTLED.
      That fix is merged, deployed and CONFIRMED live in the API Gateway
      control plane. It changes nothing on the wire. Filed separately from
      0126 because it is not 0126's CORS work and not 0194's tail: their code
      is correct and shipped. This is API Gateway behaviour we do not
      understand, and it invalidates the verification method both tasks used.
  - date: 2026-09-02
    status: backlog
    who: okarcz
    note: >
      🎉 THE SYMPTOM IS GONE — measured right after [[0126]]'s deploy. All four
      probes that leaked the portal's origin this morning now carry NO CORS
      header at all, credentials and Vary included. Nothing in this task was
      worked on; the fix, if it is one, was a side effect. ⚠️ The MECHANISM is
      still unknown and that is now the whole of this task: what changed
      alongside is that the ApiGateway deploy created a NEW
      AWS::ApiGateway::Deployment and repointed the stage. The plausible
      reading - control-plane gateway responses are not what the STAGE serves
      until a new deployment - would also explain why `cdk diff --strict` saw
      no difference while the wire disagreed. That is a HYPOTHESIS. This task's
      own lesson is that explaining a measurement instead of extending it
      produced two wrong answers already, so it is not being written up as
      resolved. Re-scoped from "fix the leak" to "explain why it stopped, and
      write down what that means for verifying gateway config".
  - date: 2026-09-02
    status: backlog
    who: okarcz
    note: >
      CONFIRMED FROM A REAL BROWSER, which every prior measurement on this task
      was not. Running [[0126]]'s AC-1 browser check from origin
      http://0.0.0.0:8080, a keyless GET /v1/assets/native/price fails with
      "No 'Access-Control-Allow-Origin' header is present on the requested
      resource" - ABSENT, not mismatched. That is first-hand corroboration of
      the curl measurement taken after the deploy, from the only client whose
      opinion the CORS rules describe. 🔑 It also sharpens WHY this matters,
      which the task previously stated as a mismatch: Chrome logs
      `403 (Forbidden)` in the console but script receives only
      `TypeError: Failed to fetch` with NO status, because a response the CORS
      check rejects is never surfaced to the caller. So a browser client cannot
      distinguish a bad API key from a dead API - unchanged by the header
      going absent, and NOT fixed by the leak stopping. The mechanism question
      is untouched and this task stays open on it.
  - date: 2026-09-25
    status: backlog
    who: okarcz
    note: >
      NARROWED on the operator's 2026-09-25 decision, from the 2026-09-24 read-only
      check. Now verified on the wire: /v1 403 and 404 responses carry no CORS
      header, and the OPTIONS preflight answers 204 with
      Access-Control-Allow-Origin `*`. The original symptom (every /v1 4xx
      carrying the portal's origin) is gone. Still open: the THROTTLED 429 path,
      which is still customised with the portal's origin plus credentials
      (infra/src/lib/stacks/api-gateway-stack.ts:1004-1015); writing down the
      mechanism; a repeatable probe of the deployed API; and reconciling
      [[0126]]'s matching AC. Retitled, because the old title described a leak
      that is no longer measured. Original text kept below.
---

# THROTTLED 429 still carries the portal's credentialed origin; the mechanism is unexplained

## Summary (narrowed 2026-09-25)

**Verified 2026-09-24:** keyless `/v1` `403`s and unmapped-route `404`s
(the `UnknownRoute` gateway response, `infra/src/lib/stacks/api-gateway-stack.ts:1034`)
carry **no** CORS header. The `/v1` preflight answers **`204` with
`Access-Control-Allow-Origin: *`**. The symptom this task was filed for is gone.

What is left:

1. **The `THROTTLED` 429 path.** `api-gateway-stack.ts:1004-1015` still
   customises `THROTTLED` (and `DEFAULT_5XX`) API-wide with
   `Access-Control-Allow-Origin: <portalWebOrigin>` +
   `Access-Control-Allow-Credentials: true`. A throttled `/v1` request, including
   a throttled preflight, therefore goes to a third-party origin with the
   portal's credentialed origin, not `*`. That makes `/v1` CORS break
   intermittently under throttle (see "It also breaks the preflight itself" below).
   It has not been probed.
2. **The mechanism.** Why the 4xx leak stopped on the 2026-09-02 deploy is still a
   hypothesis (the gateway responses a stage serves come from its deployment,
   not from the control plane). It has not been confirmed.
3. **A repeatable probe** of the deployed API, not a template assertion.
4. **The [[0126]] link.** 0126 (archived) has a matching AC
   (`lore/1-tasks/archive/0126_FEATURE_api-edge-cors-custom-domain-waf.md:655-664`)
   that already cites this task, but it reads "MEASURED CLEAN" and "has NO
   EFFECT … cannot close until 0255 does" in the same item. When this task
   closes, that AC needs to be reconciled with the outcome.

## Acceptance Criteria

- [x] ~~`/v1` error responses do not carry the portal's origin~~ **for 403/404:**
      verified 2026-09-24. No CORS header on a keyless `/v1` 403 or an
      unmapped-route 404, and the preflight returns 204 with `*`.
- [ ] The `THROTTLED` 429 path is decided: the portal's credentialed origin is
      either removed or scoped off `/v1`, or recorded as unavoidable with the
      reason and the consequence for third-party browser consumers.
- [ ] The mechanism (why the leak stopped on a new stage deployment) is
      identified and written down, reproduced in a scratch API if needed. It must
      not be inferred.
- [ ] A repeatable probe of the DEPLOYED API is committed somewhere it runs
      again, e.g.
      `curl -si -H 'Origin: https://evil.example' https://prices-api.sorobanscan.rumblefish.dev/v1/assets/native/price`
      plus the matching `OPTIONS` preflight. It asserts the absent header on
      403 and `*` on the 204 preflight.
- [ ] [[0126]]'s matching AC references this task's outcome, and its
      self-contradictory wording is reconciled.

## Original scope (before 2026-09-25 narrowing)

> Kept verbatim for the record (headings demoted one level). Original title: *The narrowing is live, correct, and does nothing*. The Summary and Acceptance Criteria above supersede it.

### ✅ UPDATE 2026-09-02 — the leak STOPPED, cause unconfirmed

Measured immediately after [[0126]] deployed (Compute, then ApiGateway):

| request | this morning | after the deploy |
|---|---|---|
| `/v1/assets/native/price`, no key, `Origin: evil` | 403 + `…sorobanscan…` | **403, no CORS header** |
| same, **no `Origin` header** | 403 + `…sorobanscan…` | **403, no CORS header** |
| `/nope`, no `Origin` | 403 + `…sorobanscan…` | **403, no CORS header** |
| `/v1/assets`, no key | 403 + `…sorobanscan…` | **403, no CORS header** |

`Access-Control-Allow-Credentials` and `Vary: Origin` went with them.

⚠️ **Nothing in this task was worked on.** No code here changed; 0126 deployed
CORS preflights and `disableExecuteApiEndpoint`. So this is a side effect, and
**the bug this task was filed for — that a deployed, correct control-plane
change had no effect — is still unexplained.**

🔑 **The task is therefore RE-SCOPED, not closed:** from *fix the leak* to
*explain why it stopped*. The candidate is that the ApiGateway deploy created a
new `AWS::ApiGateway::Deployment` and repointed the stage
(`ApiDeployment…1de519c6` → `…13811535`), i.e. **gateway responses are served
per stage-deployment, and the control plane can be ahead of what the stage
serves.** That would explain why `describe-stacks` and `cdk diff --strict` both
reported agreement while the wire disagreed — they compare against the control
plane, which was correct all along.

⚠️ **That is a hypothesis and is labelled as one deliberately.** Two published
diagnoses on this bug were already wrong, both from explaining a measurement
instead of extending it. Confirming it needs a deliberate test, not a story.

### 🔎 First-hand browser confirmation — 2026-09-02

Every measurement on this task until now was `curl`. [[0126]]'s AC-1 browser run
put a real client on it, from origin `http://0.0.0.0:8080`:

```
Access to fetch at 'https://prices-api.sorobanscan.rumblefish.dev/v1/assets/native/price'
from origin 'http://0.0.0.0:8080' has been blocked by CORS policy:
No 'Access-Control-Allow-Origin' header is present on the requested resource.
GET .../v1/assets/native/price net::ERR_FAILED 403 (Forbidden)
→ script sees: TypeError: Failed to fetch
```

Two things this pins that `curl` could not:

1. **The header is ABSENT, not mismatched.** The browser's own wording
   distinguishes the two, and it says absent — corroborating the post-deploy
   measurement above from the client the CORS rules are written for.
2. **The consequence is worse than "a misleading origin".** Chrome logs
   `403 (Forbidden)`, but **script receives only `TypeError` with no status**,
   because a response failing the CORS check is never surfaced to the caller.
   A browser consumer therefore cannot tell a bad API key from a dead API.

🔑 **The leak stopping did NOT fix this.** "Why it matters" below was written
when the header carried the portal's origin; the header is gone and the
third-party failure mode is identical. Whatever this task concludes has to
address the missing header, not just the wrong one.

### Summary

`api-gateway-stack.ts` customises exactly two gateway responses — `THROTTLED`
and `DEFAULT_5XX` — to carry the portal's CORS headers. Production agrees. And
production still stamps those headers onto 4xx types that are **not** customised.

**The fix is deployed and ineffective.** Choosing a narrower `ResponseType`
does not scope the header, which is what everyone involved assumed it would.

### Evidence — measured 2026-09-02, 09:30Z and re-confirmed 09:59Z

#### The control plane says the narrowing is in place

```
aws apigateway get-gateway-responses --rest-api-id 02mabge71l
  → customised (defaultResponse == false): ["THROTTLED", "DEFAULT_5XX"]
```

And each 4xx type that actually serves these requests is untouched:

| responseType | `responseParameters` | `defaultResponse` |
|---|---|---|
| `DEFAULT_4XX` | `{}` | `true` |
| `ACCESS_DENIED` | `{}` | `true` |
| `INVALID_API_KEY` | `{}` | `true` |
| `MISSING_AUTHENTICATION_TOKEN` | `{}` | `true` |

#### The wire says otherwise

Every one of these carries `Access-Control-Allow-Origin: https://sorobanscan.rumblefish.dev`,
`Access-Control-Allow-Credentials: true` and `Vary: Origin`:

| request | status | errortype |
|---|---|---|
| `GET /v1/assets/native/price`, no key, **no `Origin` header** | 403 | `ForbiddenException` |
| same + `Origin: https://evil.example` | 403 | `ForbiddenException` |
| `GET /nope`, no `Origin` | 403 | `MissingAuthenticationTokenException` |
| `OPTIONS /v1/assets/native/price` (preflight) | 403 | — |

Control: `GET /health` with an `Origin` returns **200 with no CORS headers**, so
this is specific to the gateway's error path and not a blanket header.

#### It is not a missing or partial deploy

Ruled out explicitly, because that was the first (wrong) diagnosis:

- `cdk diff Prices-production-ApiGateway --strict` → the only differences are
  `AWS::CDK::Metadata` and two Output **descriptions** where a mojibake `?`
  becomes `→` / `—`. Nothing functional. The stack is up to date.
- CFN `LastUpdatedTime` `2026-09-01T12:23:31Z`; stage deployment `vsrfht`
  created `2026-09-01T12:23:38Z` — both AFTER PR #268 merged at `12:02Z`.
- `PortalHostingStack` is not deployed at all, so **no CloudFront is in the
  path** — these probes hit API Gateway's regional endpoint directly.

⚠️ **Two wrong diagnoses were published before this one**, both from inference
rather than measurement: "merged but never deployed" (a CEST/UTC timestamp
misread — `git log` prints local, `LastModified` is UTC) and "saved but not
published to the stage". Neither survived contact with `cdk diff` and
`describe-stacks`. A third mechanism guess is not wanted; the next step is an
experiment, not more reasoning.

### Why it matters

Not security — the header names our own origin on API Gateway's generic
`{"message":"Forbidden"}`. Nothing leaks.

⚠️ **PARTLY STALE — corrected 2026-09-02, kept for the reasoning.** The error
responses no longer carry the portal's origin; they carry no CORS header at all.
The collision with [[0126]] described here SURVIVES that change unaltered, for
the reason given in the browser section above.

It matters because **it collides with [[0126]]'s deliverable.** Once `/v1`
answers preflight with `Access-Control-Allow-Origin: *`, its ERROR responses
will still say `https://sorobanscan.rumblefish.dev`. A third-party browser
consumer hitting a 403 or 429 gets a CORS mismatch and reads it as a dead
network rather than as the status it is — which is the exact failure 0194 added
these headers to prevent, aimed at the wrong audience.

Note the preflight case above: `OPTIONS` on `/v1` today returns 403 **with** the
portal's origin attached. So the misleading header is already on the very
response a browser consults first.

### ⚠️ It also breaks the preflight itself, not only error responses

Added 2026-09-02 from the review of PR #277, because neither that PR nor the
first draft of this task had it.

[[0126]]'s new `OPTIONS` methods inherit the stage default throttle
(200 rps / 400 burst). A throttled preflight answers `429` through the
`THROTTLED` gateway response — the one type that IS customised — so it comes
back carrying `Access-Control-Allow-Origin: <portalWebOrigin>` **and**
`Access-Control-Allow-Credentials: true`, against a third party's origin, and
credentials are invalid alongside the `*` that same preflight advertises when
it succeeds.

🔑 **So this is not only "errors read as a dead network". Under throttle the
data API's CORS is intermittently broken** — the same call works, then does
not, with no signal a caller could act on. That moves this from a defect on the
error path to one on the success path's precondition.

Measured today: `OPTIONS /v1/assets/native/price` already returns 403 **with**
the portal's origin attached, before any of 0126's work is deployed.

### 🔑 The generalisable lesson

**A gateway-response change was verified by reading the CDK template, and the
template was right.** Nothing in the review, the tests or the deploy could have
caught this — only probing the deployed API could, and nothing did until 0126
went looking for a different problem.

This is [[deploy-ships-stale-lambda-assets]]'s rule arriving from a new
direction: there, the file was right and the running artefact was old; here the
config is right and the runtime behaviour disagrees with it. Same
countermeasure — **verify the RESPONSE, not the declaration.**

### Implementation

- **Reproduce in isolation first.** A scratch REST API with one method and one
  customised `THROTTLED` gateway response, then probe a 403. This is the whole
  question and it does not need prod. If the header appears there too, it is
  API Gateway behaviour and the fix is a different mechanism, not a different
  `ResponseType`.
- Check whether a customised response's `responseParameters` propagate to
  sibling types, and whether an explicitly customised `DEFAULT_4XX` with EMPTY
  parameters overrides that (an explicit no-op may beat an absent entry).
- If no scoping mechanism exists at the gateway level, the options are: accept
  it and document it; drop the CORS headers from gateway responses entirely and
  let the portal treat an unlabelled 429 as it did before; or move the portal
  to a surface where its errors are separable.
- ⚠️ Whatever ships, **verify by probing the deployed API**, and add that probe
  somewhere it runs again. A template assertion cannot see this class of defect.

### Acceptance Criteria

- [ ] The behaviour is reproduced (or refuted) in a scratch API, away from prod
- [ ] The mechanism is identified and written down — not inferred
- [ ] `/v1` error responses do not carry the portal's origin, OR it is recorded
      as unavoidable with the reason and the consequence for third-party
      browser consumers stated
- [ ] A probe of the DEPLOYED API pins whichever outcome holds — never a
      template assertion alone
- [ ] [[0126]]'s matching AC references this task rather than restating it
