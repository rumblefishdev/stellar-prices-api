---
id: "0298"
title: "The load-test API key was rotated on 2026-09-21 — the runbook registry and the loadtest README still name the old one"
type: CHORE
status: active
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
