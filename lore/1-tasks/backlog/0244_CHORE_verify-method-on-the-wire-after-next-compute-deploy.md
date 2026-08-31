---
id: "0244"
title: "Verify `method` reaches the API response after the next Compute deploy — 0178's last AC, blocked only on someone else's rollout"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0178", "0194", "0235"]
tags:
  [
    "priority-medium",
    "effort-small",
    "read-surface",
    "verification",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/dto.rs"
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0178]] to carry its one unmet AC rather than close the task
      on a half-met criterion. The `method` field is merged (PR #272) and the
      ClickHouse side is deployed and verified on prod, but the api-handler
      Lambda still runs the pre-merge build, so the field is absent from the
      JSON. Deploying it was deliberately NOT done - see below.
---

# 0178's last AC: `method` is not on the wire yet

## Summary

`prices.current_prices.method` is live on prod and correct (`oracle_rows = 1`,
canonical USDC tagged `oracle`). The API DTO change shipped in PR #272. But the
running api-handler is the pre-merge binary, so `GET /price` returns the row
**without** the `method` key — measured 2026-08-31.

Nothing is broken: the old handler pins explicit column lists and never selects
it. This is purely "the new build has not been rolled out".

## Why 0178 did not just deploy it

🚫 **`develop` carries merged-but-undeployed infra work belonging to another
dev**, so no deploy from this task could have been ours alone.

`make -C infra diff-production` on 2026-08-31 showed, on top of our api-handler
asset:

- **ApiGateway stack** — `destroy` on the custom domain `ApiApiDomain`, the ACM
  `ApiCertificate`, both Route53 records and the `ApiCustomDomain` output; the
  `/api/{proxy+}` → `/api/api/{proxy+}` move. That is **[[0235]]**
  (`9d39207`, a `refactor!`).
- **PortalHosting** — CloudFront function and cache behaviours rewritten, portal
  bundle replaced. [[0194]] (`7e73dc5`) + 0235.
- **Compute** — a `LedgerProcessorFunction` code change sourced from a
  **2026-08-28** `target/lambda/prices-ledger-processor/bootstrap` that this
  task never rebuilt, an IAM removal (`PortalTagApiKeysOnCreate` off
  `ApiHandlerRole`), and api-handler env changes (`API_BASE_URL` repointed off
  the custom domain, `PORTAL_WEB_ORIGIN` removed).

⚠️ **The lesson, and it generalises:** on a shared `develop`, even a
single-stack `deploy-production-<stack>` ships EVERY merged change in that
stack, not just yours. `deploy-production-compute` is not "just my Lambda". Read
`diff-production` for removals before every deploy — cf.
[[team-adam-kot-task-ownership]].

## What to do

Nothing, until the owner of 0194/0235 deploys. Their Compute deploy carries our
api-handler for free. Then:

```bash
curl -s -H "x-api-key: $PRICES_API_KEY" \
  "<api-base>/v1/assets/USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN/price" \
  | jq '{price_usd, volume_24h_usd, method}'
```

⚠️ The API base URL is itself changing under 0235 — do not assume the
execute-api host used on 2026-08-31 still applies.

## Acceptance Criteria

- [ ] `GET /price` for canonical USDC returns `"method": "oracle"`.
- [ ] A `traded` asset (native XLM) returns `"method": "traded"`.
- [ ] [[0178]]'s AC 2 is then ticked in the archive copy, with the date.
