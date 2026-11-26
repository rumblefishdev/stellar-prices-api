---
id: "0155"
title: "Decide and declare the API license — the published OpenAPI document emits an empty license object"
type: DOCS
status: active
assignee: akot
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
  - date: 2026-11-24
    status: active
    who: akot
    note: >
      Activated. The decision is made: the repo is public and #419
      (2026-11-13) added an MIT LICENSE file, so what is left is declaring
      it in Cargo and the OpenAPI document. One PR to develop.
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

- [x] License decided and recorded — MIT, the root `LICENSE` Marek added in
      #419 (2026-11-13) on a public repo
- [x] `info.license` in the served document is non-empty and carries `url` or
      `identifier` — `{"name": "MIT", "identifier": "MIT"}` in the document the
      build emits (`npm run openapi:extract`); production serves it from the
      first API deploy after this PR
- [x] `info-license-strict` re-enabled and `npm run openapi:lint` passes

## Implementation Notes

- `Cargo.toml`: `[workspace.package] license = "MIT"`; each of the 23 crates
  has `license.workspace = true`.
- `redocly.yaml`: the `info-license-strict: off` line and its comment are gone,
  so the rule runs at the `recommended-strict` severity. No rule is off now.
- `web/portal/public/openapi.json`: refreshed from the extracted document; the
  only difference is `info.license`.
- `packages/prices-api/tests/openapi.rs`: `spec_is_valid_openapi_3` asserts
  `info.license` (13 tests in the file pass).
- `docs/scf/api-endpoints.md` and `docs/scf/milestone-3-rfp-deviations.md` no
  longer say the license is undecided.

## Design Decisions

### From Plan

1. **MIT, as an SPDX `identifier`**: the license is a standard one, so
   `identifier`, not `url`.

### Emerged

2. **The API's license is taken to be the source's license.** The task asked
   to confirm whether the two differ. Nobody was asked again: #419 put MIT on
   the public repo, the skill on skills.stellar.org is MIT (task 0318), and no
   separate terms page for the API exists.
3. **Nothing declared in the `#[openapi]` attribute.** The plan named
   `info(license(...))` in `openapi/mod.rs`. utoipa-gen 5.5 already copies
   `CARGO_PKG_LICENSE` into both `name` and `identifier`
   (`utoipa-gen/src/openapi/info.rs`), so the Cargo field alone gives the
   object the lint asks for, and the license has one home.
4. **Every crate, not only `prices-api`.** `LICENSE` covers the whole repo;
   one crate carrying the field would read as the others being something else.
5. **The lint rule is not restated in `redocly.yaml`.** `recommended-strict`
   has it at error severity; the file restates only the rules it raises above
   that set or guards against a switch back to `recommended`.

## Issues Encountered

- **Stale build after adding the field.** `cargo run --bin extract_openapi`
  emitted `MIT` right away, while `cargo test --test openapi` still saw
  `{"name": ""}` until the crate's sources were touched: utoipa reads
  `CARGO_PKG_LICENSE` with `std::env::var` at macro expansion, which cargo does
  not track for the test build. Local only; CI builds from clean.
