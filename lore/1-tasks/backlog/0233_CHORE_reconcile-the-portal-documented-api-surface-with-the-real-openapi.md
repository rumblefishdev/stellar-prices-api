---
id: "0233"
title: "Reconcile the portal's documented API surface with the real OpenAPI — paths, example fields, the source name, the placeholder key"
type: CHORE
status: backlog
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

- [ ] Every URL, path and field name rendered by the landing page and the
      quick start exists in `/api-docs-json`, or has a dated decision here
      saying why the document changes instead
- [ ] Every copy-button snippet on the quick start runs unchanged against the
      live API with a real free-plan key and returns what the page shows
- [ ] No hostname other than ours appears in the production bundle
- [ ] The Figma file agrees with the page, or a note here says it does not
