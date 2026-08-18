---
id: "0206"
title: "Validation follow-ups — cursor {sort,order} binding, typed identifier path extractor, test dedup"
type: REFACTOR
status: backlog
related_adr: []
related_tasks: ["0119", "0118", "0120"]
tags: [layer-backend, priority-low, effort-small, api, validation, cleanup]
links:
  - "../../../packages/prices-api/src/common/cursor.rs"
  - "../../../packages/prices-api/src/common/extract.rs"
history:
  - date: 2026-08-18
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0119 future work — the deliberately deferred remainders of
      the input-validation hardening (review findings accepted but not
      applied in PR #217).
---

# Validation follow-ups from 0119

## Summary

Four small, independent leftovers from [[0119]], each recorded there as a
known limitation or accepted review finding.

## Context

PR #217 hardened every input but deliberately deferred the below to keep the
diff reviewable. None is a bug; each is a recorded trap for future work.

## Implementation

- **Cursor `{sort, order}` binding** (`common/cursor.rs`): carry the producing
  sort/order in the token and validate on decode. Closes the recorded
  limitation that switching between two same-typed sorts mid-walk yields a
  wrong page (not an error). Replaces `Cursor::valid_for(bool)` — the bool
  cannot express a third sort kind (e.g. a future date sort).
- **`ValidatedPath<AssetIdentifier>`** (`common/extract.rs`): run
  `AssetIdentifier::parse` inside the extractor. Deletes the four identical
  parse blocks in `assets/handlers.rs` (×3) and `oracles/handlers.rs`, and
  makes the `invalid_id` rejection code correct by construction — today it is
  correct only while every path param happens to be an identifier.
- **Shared negative-test assert helper** (`tests/common/mod.rs`): ~30 tests
  repeat the same 3-line status+code assertion; a
  `assert_400(uri, code) -> Value` helper also asserts
  `Cache-Control: no-store` uniformly (today only one test per file checks it).
- **`parse_time` rejection-block dedup** (`assets/handlers.rs`): the two
  12-line start/end blocks differ by one word; fold when [[0118]] adds the
  `min_volume_usd` range param anyway.

## Acceptance Criteria

- [ ] Cursor tokens carry and validate `{sort, order}`; a sort-switched
      replay is a 400, not a wrong page; in-flight old tokens still decode
- [ ] Path extraction yields a parsed `AssetIdentifier`; the four manual
      parse blocks are gone
- [ ] Negative tests assert the `no-store` header uniformly via the helper
- [ ] No behavior change on any accepted input (integration suite green)
