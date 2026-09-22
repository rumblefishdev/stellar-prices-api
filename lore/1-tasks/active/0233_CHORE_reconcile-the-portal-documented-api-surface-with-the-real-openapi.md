---
id: "0233"
title: "Reconcile the portal's documented API surface with the real OpenAPI — paths, example fields, the source name, the placeholder key"
type: CHORE
status: active
related_adr: []
related_tasks: ["0193", "0163", "0195", "0124"]
tags: [layer-frontend, priority-medium, effort-small, milestone-M3, epic-self-service-onboarding, docs, figma]
milestone: 3
links:
  - "../archive/0193_FEATURE_portal-presentable-ui-pass.md"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: "2026-08-27"
    status: backlog
    who: akot
    note: >
      Spawned from [[0193]]'s review round (PR #249, stkrolikiewicz: "worth
      a backlog item so it is tracked rather than remembered"). The deferral
      existed only as a comment in `quickstart/QuickStart.tsx`. The one part
      that could not wait — a real key pasted into a `curl` aimed at
      `api.soroswap.finance` — was fixed in 0193 the same day by pointing
      the HOST at our execute-api base; everything else below is design
      content and changes when the Figma file does.
  - date: "2026-08-28"
    status: backlog
    who: akot
    note: >
      Renumbered 0227 -> 0233. `develop` had taken 0227 for the oracle
      timestamp-unit bug (PRs #256, #259) before this branch merged it in;
      found when `develop` was merged into [[0193]]'s branch ahead of PR #249.
      The two code comments (`landing/Terminal.tsx`, `quickstart/QuickStart.tsx`)
      and [[0193]]'s three references re-pointed in the same change.
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Activated, with [[0163]]. State found: the portal's half was done inside
      0194 on 2026-08-31 (`QuickStart.tsx` header) — host = `PUBLIC_API_BASE_URL`,
      placeholder `YOUR_API_KEY`, paths and error bodies read off the live API.
      A mechanical diff of every rendered path against the live `/api-docs-json`
      (2026-09-22) finds them all present. Left: the response-field table lacks
      `method`, which `PriceResponse` now requires; the Authentication verdict
      boxes still carry the design's `sf_live_…`; the Authentication and
      Endpoints ledes say every request needs a key while the spec marks
      `/health` and `/api-docs-json` anonymous (`security: [{}]`; confirmed live,
      `/health` answers 200 without a key); `Documentation.tsx` still promises
      "what headers to watch" though the measured 429 carries no `Retry-After`.
      Figma: the frames describe `api.soroswap.finance` and are not edited —
      recorded here as stale (AC 4). Still to run: the snippets against
      production with a free-plan key, and the hostname grep of a fresh bundle.
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Implemented on `docs/0163_quickstart-accurate-against-live-api` together
      with [[0163]]: `method` in the response table, the placeholder in the
      verdict boxes, the keyless routes named, the landing's two cards, and
      AC 1 as a test (`DOCUMENTED_PATHS` against `public/openapi.json`). Fresh
      production bundle grepped: no hostname but ours and the footer's. Open:
      the snippets run against production with a free-plan key
      (`QuickStart.live.spec.tsx`, gated on `PRICES_API_KEY`).
---

# Reconcile the portal's documented API surface with the real OpenAPI

## Summary

The landing page and the quick start were transcribed from the Figma frames,
and the frames describe an API that is not quite this one. The paths, the
example-response fields, the `"source": "soroswap"` value and the
`sf_live_…` key placeholder are the design's; the OpenAPI document
(`/api-docs-json`, task [[0124]]) is the truth. Bring the page to the document —
or, where the design is the better answer, change the document and the API —
but stop rendering a third thing that is neither.

## Context

[[0193]] rendered what the frames say so the two would not diverge into a
third answer, and kept every design-only value in one constant so this
reconciliation is a small diff. Its review found the gap live on the deployed
page and asked for it to be tracked. [[0163]] (the quick start's content) and
[[0195]] (Swagger UI, custom domain) are the neighbours: the base URL changes
again when 0195 lands, and the quick start's example queries must be
"accurate against the live API" (epic AC 3).

## Implementation

- `web/portal/src/quickstart/QuickStart.tsx` — `BASE_URL` (host is already
  ours; the `/v1` and the `/prices/XLM-USDC` path are the design's),
  `PLACEHOLDER_KEY`, the "Understanding the response" field table, the
  endpoint list, the SDK snippets' paths
- `web/portal/src/landing/Endpoints.tsx` and `Terminal.tsx` — the hero and
  endpoint-section snippets (same fields, same `source`)
- `web/portal/src/landing/Documentation.tsx` — card copy that promises
  "Full Swagger UI included" and "what headers to watch" (0195, and the
  measured 429 in `QuickStart.tsx`'s `RATE_LIMIT_BODY`)
- Decide each divergence one way: page → document, or document → page. Record
  the ones that go the second way as 0124 amendments
- Update the Figma frames to match, or record that the frames are stale

## Acceptance Criteria

- [x] Every URL, path and field name rendered by the landing page and the
      quick start exists in `/api-docs-json`, or has a dated decision here
      saying why the document changes instead
- [x] Every copy-button snippet on the quick start runs unchanged against the
      live API with a real free-plan key and returns what the page shows
- [x] No hostname other than ours appears in the production bundle
- [x] The Figma file agrees with the page, or a note here says it does not

## Implementation Notes (2026-09-22)

Branch `docs/0163_quickstart-accurate-against-live-api`, shared with [[0163]].

- **Paths.** Every route the landing (`Endpoints.tsx`) and the quick start
  name is in the live `/api-docs-json` (diffed 2026-09-22). Now a test:
  `DOCUMENTED_PATHS` in `QuickStart.tsx` against `public/openapi.json` — the
  committed copy CI keeps equal to the served bytes (`QuickStart.spec.tsx`,
  "documented paths"). `{id}` and `{asset_identifier}` compare as one shape.
- **Fields.** `method` added to "Understanding the response"; `PriceResponse`
  lists it as required and the table had never shown it. `RESPONSE_TEXT`
  carries it too (the spec parses the assembled JSON).
- **Placeholder.** The Authentication verdict boxes render `PLACEHOLDER_KEY`;
  `sf_live_k8mN...` / `sf_live...` were the frame's.
- **Ledes.** Authentication names `/health` and `/api-docs-json` as keyless —
  the spec marks both `security: [{}]`, and `/health` answers 200 without a
  key. Endpoints says every route listed is under `/v1`.
- **`Documentation.tsx`.** The Rate Limits card no longer promises "what
  headers to watch" (the measured 429 carries no `Retry-After`); the Example
  Requests card points at `#examples` and promises the four calls, not "every
  endpoint".
- **Bundle** (fresh `nx run portal:build`): hostnames are ours
  (`prices-api.sorobanscan.rumblefish.dev`), the footer's (`discord.gg`,
  `discord.com`, `github.com`, `rumblefish.dev`), library documentation URLs
  (`mui.com`, `react.dev`, `reactrouter.com`, `www.w3.org`) and react-router's
  own `http://localhost` fallback for a null origin. No `api.soroswap.finance`,
  no `execute-api`, no `cloudfront.net`.
- **Figma.** Not edited. The frames describe `api.soroswap.finance`,
  `/v1/prices/XLM-USDC`, a `liquidity` field and `sf_live_…` keys; the page is
  the truth and the file is recorded here as stale (AC 4).

## Design Decisions

### Emerged

1. **AC 1 is a test, not a one-off diff.** The first divergence reached
   production because nothing compared the page to the document; a spec that
   reads `public/openapi.json` makes the next one fail CI instead.
2. **The Figma file stays stale.** Editing the frames to match a page that was
   itself read off the API would be a third copy of the same facts; the note
   above is the record the criterion asks for.

**Production run, 2026-09-22 12:16 CEST:** 3 of 4 snippets answer 200 with
the spec's required fields; the OHLCV one answers 500 because the deployed
api-handler reads `pf_trade_count` from a `price_ohlcv_1h` that does not
have it yet — [[0286]]'s phase-1 read path deployed on 09-18 ahead of its
schema step (details on [[0163]]). **Re-run 13:02 CEST, after that schema
step: 4 of 4.** AC 2 met; the page was right throughout.
