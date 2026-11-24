---
id: "0354"
title: "Retire the old API host prices-api.sorobanscan.rumblefish.dev — move the cache check and k6 test to the new host first"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0340"]
tags: [layer-infra, priority-low, effort-small]
links:
  - "../active/0340_FEATURE_prices-api-on-prices-api-sorobanscan-com.md"
history:
  - date: 2026-11-24
    status: backlog
    who: okarcz
    note: >
      Spawned from 0340 future work (its Step 5, the old host's fate), on the
      author's suggestion in the 2026-11-24 board triage.
---

# Retire the old API host prices-api.sorobanscan.rumblefish.dev

## Summary

Since [[0340]] the API is served at `prices-api.sorobanscan.com`. The old host
`prices-api.sorobanscan.rumblefish.dev` still serves API-key clients. Decide
between a `308` redirect and retiring it, and do it.

## Context

0340 Step 5 left two checks on the old host on purpose: the cache verification
and the k6 load test measure API Gateway directly, and the new host puts
Cloudflare in front. They need another target before the old host goes. API
Gateway cannot redirect by itself.

## Implementation

- Move the cache verification and the k6 load test to a target that does not
  depend on the old host (the new host, or the regional `execute-api` target).
- Decide `308` or retirement; check who still calls the old host first
  (API Gateway access logs by host).
- Do it, then update the docs that still name the old host (dated evidence
  stays as submitted).

## Acceptance Criteria

- [ ] Cache verification and k6 run without the old host
- [ ] Decision recorded (`308` or retire) with the remaining traffic measured
- [ ] Old host redirected or retired; docs updated
