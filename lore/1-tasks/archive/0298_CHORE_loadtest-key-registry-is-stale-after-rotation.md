---
id: "0298"
title: "The load-test API key was rotated on 2026-09-21 — the runbook registry and the loadtest README still name the old one"
type: CHORE
status: completed
assignee: stkrolikiewicz
related_adr: []
related_tasks: ["0121", "0126", "0293"]
tags: [layer-docs, priority-low, effort-small, api-keys, loadtest, runbook]
links:
  - "../../../docs/runbooks/manual-api-key-tier.md"
  - "../../../packages/prices-api/loadtest/README.md"
history:
  - date: 2026-09-21
    status: backlog
    who: stkrolikiewicz
    note: >
      Rotation done and verified by read-back the same morning; only the
      documents trail it. Kept out of [[0294]] on purpose — key housekeeping
      does not belong in the Milestone 3 evidence package.
  - date: 2026-09-21
    status: active
    who: stkrolikiewicz
    note: >
      Activated. The AWS half is already closed: old key `lxrwlyhjm7` deleted
      (get-api-key returns NotFoundException), only `gc22sbmwa2` is on `i12bsj`.
      Three document edits remain.
  - date: 2026-09-21
    status: completed
    who: stkrolikiewicz
    note: >
      5 of 5 criteria met. 4 files edited (registry row, loadtest README, k6
      usage comment, one clause in the load-test report); no code paths
      changed, no tests added or modified. Old key deleted and read back.
---

# Load-test key registry is stale after the 2026-09-21 rotation

## Summary

The internal load-test key on usage plan `i12bsj` was rotated on 2026-09-21.
The runbook's "Issued manual keys" registry and the loadtest README still carry
the old key's name and id. The registry's own rule is "keep this current, one
row per key" — bring it back in line and finish the rotation.

## Context

Routine hygiene rotation: the old key's value had been displayed in a local
terminal session. A search of both repos' full git history (all refs, reflog,
stashes) found it in **0 commits**; in the working tree it lives only in the
gitignored `.env.local`.

State in AWS, read back 2026-09-21 09:16 CEST:

| Key | Name | State |
| --- | --- | --- |
| `gc22sbmwa2` | `prices-production-loadtest-key-20260921T070812Z` | enabled, on `i12bsj`, returns 200 through `prices-api.sorobanscan.rumblefish.dev` |
| `lxrwlyhjm7` | `prices-production-loadtest-key-20260819T114230Z` | **disabled**, still attached to `i12bsj`, returns 403 |

Plan limits unchanged: 150 req/s, burst 300, 1,000,000/month.

## Implementation

- `docs/runbooks/manual-api-key-tier.md` registry (`## Issued manual keys`):
  replace the `loadtest` row's key name, key id and issued date with the new
  key's. While the old key still exists the registry rule asks for **two**
  rows; once it is deleted, one.
- `packages/prices-api/loadtest/README.md:46`: name the new key.
- `docs/prices-api-load-test-100rps.md:61` **stays as is** — it records the key
  the 2026-09-18 run actually used. At most a footnote that the key has since
  been rotated.
- Delete the old key (runbook step 7) — irreversible, so a human runs it:
  `aws apigateway delete-api-key --api-key lxrwlyhjm7`.
- `packages/prices-api/loadtest/price_load.js:9`: the usage comment still shows
  `BASE_URL=https://<api>/<stage>`, the shape task 0126 retired. The custom
  domain has an empty base path — no `/<stage>`. A stale local `.env.local`
  built from that comment returned 403 for every request during this rotation
  and looked like a bad key.

## Acceptance Criteria

- [x] Registry row(s) for `loadtest` match `aws apigateway get-usage-plan-keys --usage-plan-id i12bsj`
- [x] loadtest README names the current key
- [x] Old key `lxrwlyhjm7` deleted — 2026-09-21, read back: `get-api-key` returns `NotFoundException`
- [x] `price_load.js` usage comment shows the custom-domain form of `BASE_URL`
- [x] The load-test report still names the key its runs used (one clause added: rotated out, successor in the registry)

## Implementation Notes

- `docs/runbooks/manual-api-key-tier.md` — the `loadtest` registry row now
  carries `prices-production-loadtest-key-20260921T070812Z` / `gc22sbmwa2`,
  issued 2026-09-21. One row, because the old key no longer exists.
- `packages/prices-api/loadtest/README.md` — names the new key and the rotation.
- `packages/prices-api/loadtest/price_load.js` — usage comment shows the
  custom-domain form of `BASE_URL` and says why the old form reads as a bad key.
- `docs/prices-api-load-test-100rps.md` — the API key row keeps the old name
  (it is what the runs used) and gains one clause pointing at the successor.

## Issues Encountered

- **A stale `BASE_URL` looked like a failed rotation.** The first read-back
  probes of the new key returned `403` for five minutes. The cause was not
  propagation: the local `.env.local` still pointed at the `execute-api` host
  that task 0126 disabled, which answers `403 Forbidden` to everything —
  including a nonexistent route, which is the tell (a live REST API answers
  that with `Missing Authentication Token`). Through the custom domain the new
  key returned `200`, the old one `403`.
- **The pre-push hook cannot pass on stock macOS.** `tools/scripts/verify-lambda-bootstraps.sh:42`
  (task 0141) uses `[[ -v "$var" ]]`, a bash 4.2 builtin test; macOS ships
  bash 3.2, so 7 of the guard's tests fail with `conditional binary operator
  expected`. Unrelated to this diff. Same class as the `mapfile` problem in
  [[0239]]. This branch was pushed once with `--no-verify`, approved by the
  task owner; CI runs the same checks on Linux.

## Design Decisions

### From Plan

1. **The report keeps the old key name.** It records what the runs used.

### Emerged

2. **One clause added to the report row after all.** The row links to the
   registry "for the key", and after this change the registry no longer lists
   that key — a reader following the link would find a different name with no
   explanation.
3. **One registry row, not two.** The plan allowed for two rows while the old
   key existed; it was deleted before the edit, so the registry's own rule
   ("delete a row when its key is deleted") applies.
