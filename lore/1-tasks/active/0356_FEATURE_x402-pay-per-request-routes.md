---
id: "0356"
title: "Sell three data routes per request through x402 (testnet sandbox)"
type: FEATURE
status: active
assignee: akot
related_adr: []
related_tasks: []
tags: [layer-backend, priority-medium, effort-medium, api, x402, infra]
links:
  - "https://github.com/rumblefishdev/stellar-x402/blob/develop/lore/1-tasks/active/0041_FEATURE_example-integrations.md"
  - "https://github.com/rumblefishdev/stellar-x402/tree/develop/packages/sdk/rust"
history:
  - date: "2026-11-27"
    status: active
    who: akot
    note: >
      Created and started. One of the two example integrations of stellar-x402's task 0041:
      a caller with no API key pays USDC for one request. Stage 1 takes testnet USDC for a
      narrowed scope; pubnet is a later task, by settings.
---

# Sell three data routes per request through x402 (testnet sandbox)

## Summary

A caller with no account pays USDC for one request and gets the answer: `price`, `ohlcv` and
`prices/batch` get paid twins under `/x402/v1/`, settled by the stellar-x402 facilitator and
listed in its catalog. The keyed routes, API keys, plans and quotas do not change.

## Context

- stellar-x402 task 0041 (design and plan in its `notes/`): the RFP's two example integrations
  are this API and SorobanScan. The seller side is the Rust crate `stellar-x402`
  (`packages/sdk/rust` there), pinned by `rev`.
- This API has pubnet data only. While payments are in testnet USDC, which is free, the paid
  routes sell a sandbox: an asset allow-list, `ohlcv` for `1h` and `24h` only, a batch of at
  most 5.
- A paid route must not charge for an answer with nothing in it, must not be served from the
  gateway's cache, and a facilitator outage must not touch the keyed routes.

## Implementation Plan

- `packages/prices-api/src/x402/`: settings from the environment, the payment gate (asked on
  the first paid request, never at cold start), the three routes as wrappers over the keyed
  handlers, the sandbox scope.
- Gateway: three keyless methods with caching off and a throttle; the Lambda's settings; its
  timeout from 15 s to 29 s, since a settlement takes about 6 s.

## Acceptance Criteria

- [ ] Without `X402_PAY_TO` the paid paths do not exist
- [ ] An unpaid request to a paid route answers 402 with the terms and the catalog listing,
      with no API key, and the keyed routes still require theirs
- [ ] A paid request is answered with `PAYMENT-RESPONSE`; an answer with nothing in it, an
      off-scope request and any 4xx are not charged
- [ ] On testnet the sandbox scope holds for every way to name an asset
- [ ] The gateway serves the three methods without a key, uncached and throttled; no keyed
      method changes
