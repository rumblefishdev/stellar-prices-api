---
id: "0155"
title: "Decide and declare the API license — the published OpenAPI document emits an empty license object"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0124", "0128"]
tags: [layer-docs, priority-low, effort-small, openapi, licensing]
links:
  - "../../../packages/prices-api/src/openapi/mod.rs"
  - "../../../redocly.yaml"
history:
  - date: 2026-08-04
    status: backlog
    who: akot
    note: >
      Spawned from 0124 future work. The spec is now public at
      /api-docs-json, so `info.license` is public too — and it is
      currently `{"name": ""}`. Not fixed in 0124 because picking a
      license is a business decision, not a lint fix.
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Renumbered 0144 → 0155 (PR #169 review, okarcz). The ID collided:
      this task was spawned on the 0124 branch while the unmerged PR #168
      branch had already claimed 0144 for the BE-0199 USD read-surface
      defects. That task is cited by 0145–0151 and 0154 plus the BE-facing
      reply and the phase plan, so it keeps the number and this one moves.
      0152 was reserved for this task by 0153's renumber note, but has
      since been taken by the self-service onboarding portal (#172); 0153
      and 0154 are also claimed, so 0155 is the next free ID. Nothing about
      the work here changed.
---

# Declare the API license in the OpenAPI document

## Summary

`utoipa` fills `info.license` from `CARGO_PKG_LICENSE`, which is unset for every
crate in this workspace, so the published document carries `{"name": ""}`. Now
that the document is served publicly (task 0124), that empty object is visible
to every reader and to every client generator.

## Context

There is no `LICENSE` file, no `license` field in any `Cargo.toml`, and the only
license string in the repo is `"MIT"` in the root `package.json` — which is
`private: true` and reads as scaffolding boilerplate rather than a deliberate
statement. Asserting MIT in a public API description on that basis would be
inventing a legal position, so 0124 left it alone and turned off the Redocly
`info-license-strict` rule with a pointer here.

## Implementation

- Confirm the intended license for the API (and whether it differs from the
  source license) with whoever owns that call.
- Declare it in the `#[openapi(info(license(...)))]` attribute in
  `packages/prices-api/src/openapi/mod.rs` — `identifier` for an SPDX id, or
  `url` for a proprietary/custom terms page.
- Add the matching `license` field to the workspace `Cargo.toml` and a `LICENSE`
  file if the decision calls for one.
- Re-enable `info-license-strict` in `redocly.yaml` and delete the comment
  pointing at this task.

## Acceptance Criteria

- [ ] License decided and recorded
- [ ] `info.license` in the served document is non-empty and carries `url` or
      `identifier`
- [ ] `info-license-strict` re-enabled and `npm run openapi:lint` passes
