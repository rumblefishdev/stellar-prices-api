---
id: "0255"
title: "Narrowing the CORS gateway response to THROTTLED is deployed and has NO EFFECT — every /v1 4xx still carries the portal's origin"
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
---

# The narrowing is live, correct, and does nothing

## Summary

`api-gateway-stack.ts` customises exactly two gateway responses — `THROTTLED`
and `DEFAULT_5XX` — to carry the portal's CORS headers. Production agrees. And
production still stamps those headers onto 4xx types that are **not** customised.

**The fix is deployed and ineffective.** Choosing a narrower `ResponseType`
does not scope the header, which is what everyone involved assumed it would.

## Evidence — measured 2026-09-02, 09:30Z and re-confirmed 09:59Z

### The control plane says the narrowing is in place

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

### The wire says otherwise

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

### It is not a missing or partial deploy

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

## Why it matters

Not security — the header names our own origin on API Gateway's generic
`{"message":"Forbidden"}`. Nothing leaks.

It matters because **it collides with [[0126]]'s deliverable.** Once `/v1`
answers preflight with `Access-Control-Allow-Origin: *`, its ERROR responses
will still say `https://sorobanscan.rumblefish.dev`. A third-party browser
consumer hitting a 403 or 429 gets a CORS mismatch and reads it as a dead
network rather than as the status it is — which is the exact failure 0194 added
these headers to prevent, aimed at the wrong audience.

Note the preflight case above: `OPTIONS` on `/v1` today returns 403 **with** the
portal's origin attached. So the misleading header is already on the very
response a browser consults first.

## ⚠️ It also breaks the preflight itself, not only error responses

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

## 🔑 The generalisable lesson

**A gateway-response change was verified by reading the CDK template, and the
template was right.** Nothing in the review, the tests or the deploy could have
caught this — only probing the deployed API could, and nothing did until 0126
went looking for a different problem.

This is [[deploy-ships-stale-lambda-assets]]'s rule arriving from a new
direction: there, the file was right and the running artefact was old; here the
config is right and the runtime behaviour disagrees with it. Same
countermeasure — **verify the RESPONSE, not the declaration.**

## Implementation

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

## Acceptance Criteria

- [ ] The behaviour is reproduced (or refuted) in a scratch API, away from prod
- [ ] The mechanism is identified and written down — not inferred
- [ ] `/v1` error responses do not carry the portal's origin, OR it is recorded
      as unavoidable with the reason and the consequence for third-party
      browser consumers stated
- [ ] A probe of the DEPLOYED API pins whichever outcome holds — never a
      template assertion alone
- [ ] [[0126]]'s matching AC references this task rather than restating it
