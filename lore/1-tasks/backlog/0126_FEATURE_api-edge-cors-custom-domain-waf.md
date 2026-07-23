---
id: "0126"
title: "API edge — CORS preflight, custom domain, and a recorded WAF decision"
type: FEATURE
status: backlog
related_adr: ["0008"]
related_tasks: ["0124", "0121", "0128"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M2, api-gateway, cors, dns, security]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "Custom API
      domain, WAF, CORS preflight" row of `milestone-1-evidence.md` Table 4,
      which records all three as deliberately deferred from M1 with the API
      served on the raw execute-api URL.
---

# API edge — CORS, custom domain, WAF decision

## Summary

M1 serves the API on
`https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production` with no
CORS handling, no custom domain, and no WAF — all three deliberately deferred to
Tranche 2 and recorded as such.

Three loosely-related edge concerns, grouped because they touch the same stack
and the same DNS/deploy step.

## Context

**CORS is the functional one.** Overview §4's base URL is
`https://api.prices.stellar.example.com/v1` and the Tranche 3 deliverable is a
browser-based onboarding portal with example queries. Without preflight
handling, **no browser can call this API** — every cross-origin request fails
before it reaches a handler. It is also the item most likely to be discovered by
the first external consumer rather than by us.

**The custom domain is presentational but load-bearing for the spec.** §4 and
the OpenAPI `servers` field both promise a stable hostname; an execute-api URL
changes if the API is recreated, which would break every published client. It
also unblocks [[0124]]'s `servers` value being something worth publishing.

**WAF is a genuine decision, not a default-yes.** The API is public, read-only,
key-gated, over public blockchain data, with no PII (§7) and API Gateway
throttling already in place at two levels (stage: 200 rps / 400 burst; per key:
100/200). AWS WAF adds ~$5-6/mo per web ACL plus per-request cost against a §10
budget of ~$108/mo total. **The deliverable is a recorded decision with
reasoning — not necessarily a deployed WAF.** A defensible "no, because X, and
here is what would change our mind" is a complete outcome.

## Implementation

**CORS**

- Decide the allowed-origin policy. For a public read API with key auth, `*` is
  the conventional and defensible choice — but note that `Access-Control-Allow-
  Origin: *` cannot be combined with credentials, and confirm nothing relies on
  cookies (nothing should; auth is a header key).
- `x-api-key` must be in `Access-Control-Allow-Headers` or every browser call
  fails preflight — the single most common way this ships broken.
- Implement preflight at the **gateway** (`OPTIONS` mock integration per
  resource) rather than per-handler, so an unauthenticated `OPTIONS` never
  invokes Lambda and never consumes quota. Confirm the `OPTIONS` method is
  **not** `apiKeyRequired` — a preflight cannot carry a key.
- Cache preflight responses (`Access-Control-Max-Age`).
- Verify from a real browser, not only curl. `curl -X OPTIONS` succeeding proves
  less than it appears to.

**Custom domain**

- Choose the hostname and confirm who owns the zone (this is a shared
  sub-account alongside BE — coordinate rather than assume). §4's
  `api.prices.stellar.example.com` is a placeholder, not a decision.
- ACM certificate in the right region for the endpoint type, API Gateway custom
  domain + base-path mapping, DNS record.
- Decide the **stage-path story**: mapping the base path to the stage removes
  `/production` from public URLs, which is nicer — and changes every documented
  URL. Update §4, [[0124]]'s `servers`, and the [[0128]] evidence together, and
  keep the execute-api URL working during the transition.
- Note for anyone touching the handler: the stage-prefix behaviour is already
  subtle here (`AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true` is required for `/v1`
  routing). Re-verify routing after the base-path mapping change.

**WAF**

- Write the decision: threat model, what throttling already covers, cost against
  the §10 budget, and the trigger that would reverse it (e.g. sustained abuse,
  a Stellar-side requirement, an incident).
- If yes: managed rule groups only, rate-based rule aligned with the existing
  throttle, and deploy in **count mode first** — a blocking WAF in front of an
  API whose load profile is about to be measured by [[0121]] will corrupt that
  measurement.

## Acceptance Criteria

- [ ] Cross-origin `GET` from a browser page against every data route succeeds,
      preflight included
- [ ] `x-api-key` is in the allowed-headers list; `OPTIONS` requires no API key
      and does not invoke Lambda
- [ ] Allowed-origin policy decided and recorded
- [ ] Custom domain resolves and serves the API over TLS; certificate valid and
      auto-renewing
- [ ] Every documented URL (§4, OpenAPI `servers`, evidence docs) updated
      consistently; routing re-verified after base-path mapping
- [ ] Execute-api URL still functions, or its retirement is announced and dated
- [ ] WAF decision recorded with reasoning, cost, and a reversal trigger —
      deployed in count mode first if the answer is yes
- [ ] All of it expressed in CDK (Tranche 3 AC 7 requires clean-account
      reproducibility)

## Notes

- Sequence with [[0121]]: settle the domain and any WAF **before** the load
  test, or the run measures an edge that is about to change.
- The custom domain also affects the API Gateway cache — verify [[0122]]'s hit
  behaviour still holds through the new path.
