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

## Implementation Notes

- `packages/prices-api/src/x402.rs`: settings, the gate, the three wrappers, the sandbox. The
  crate `stellar-x402` is pinned to `ff5933c2` of its `develop`.
- The keyed handlers gained one thing each: a response extension (`x402::Nothing`) on a `200`
  with nothing in it. It is never sent, so a key holder's answer is the same bytes.
- Tests: `tests/x402.rs` (9, no ClickHouse, a facilitator mocked on loopback) and
  `tests/x402_it.rs` (5, ClickHouse: the `200`, and each empty answer as an unsettled `404`).
- Gateway: `/x402/{proxy+}` with `GET` and `POST`, keyless, caching off, 10 requests a second
  with a burst of 20 per verb. `envs/production.json` gains `x402` and the handler's timeout
  goes from 15 s to 29 s.
- Run locally against the testnet facilitator on 2026-11-27: the terms carry
  `areFeesSponsored`, the route's template and the request's asset. No paid request was made:
  that needs ClickHouse behind the handler, so it is the evidence run after the deploy.

## Design Decisions

### From Plan

1. **Wrappers over the keyed handlers**, behind the crate's payment layer, with the gate asked
   on the first paid request (`PaymentGate::lazy`).
2. **No data, no charge**: `404 unpriced` for an unpriced asset, `404 no_data` for an empty
   `ohlcv` or a batch that found nothing.
3. **The sandbox applies exactly when the network is testnet**, before any query, and only to
   an asset named as the list writes it.
4. **The handler's timeout is 29 s**, for every route: a settlement takes about 6 s.

### Emerged

5. **The routes are added after `auth::apply`, one by one, and `AppConfig` has no new field.**
   The plan put the settings in `AppConfig` (46 literals to edit in the tests) and an exemption
   in `auth::is_exempt`. Adding the routes outside the key gate needs neither. `Router::merge`
   was the first attempt and was measured to replace the fallback with one the key gate does
   not wrap: an unknown path answered `404` where it answers `401` today.
6. **A response extension marks an empty answer.** The keyed handlers return a built
   `Response`, and splitting `get_ohlcv` into a typed core would have touched 270 lines that
   key holders depend on.
7. **A paid answer is `Cache-Control: no-store`.** The keyed answer says `public, max-age=10`,
   and the crate would only have added `private` to it.
8. **One greedy gateway resource, not three.** The plan had three resources. One
   `/x402/{proxy+}` gives the prefix one limit per verb, as the design asks, and a later paid
   route needs no gateway change. An unknown path under it reaches the Lambda, throttled.
9. **The paid routes are not in the OpenAPI document**, and `verify-openapi-routes.mjs` skips
   the prefix on the gateway side only. A buyer reads the terms from the `402` and the catalog.
10. **No facilitator API key setting.** Our facilitator asks for one on pubnet only.
11. **A wrong setting leaves the API running without paid routes**, logged as an error, where
    a panic at cold start would take the keyed routes down with it.
12. **`prices-production-api-5xx` counts the paid routes too.** While the facilitator cannot be
    read, each request to a paid route is a `5xx`, with or without a payment: five in five
    minutes page. The alarm's text says where to look.

## Future Work

- Pubnet (stage 2): a receiving account on pubnet and who holds it, the facilitator's API key,
  the sandbox lifted, one deadline across verify, the handler and settle, and the paid routes
  in the OpenAPI document. It waits for the facilitator to serve pubnet (stellar-x402 0126).
