---
id: "0003"
title: "Update amm-trades-schema.md §7 and §11.1 with empirical topic findings"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0001"]
tags: [priority-medium, effort-small, schema-validation]
links:
  - "../active/0001_RESEARCH_dump-amm-swap-events/notes/S-amm-trades-schema-§11-1-resolved.md"
  - "../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: 2026-05-07
    status: backlog
    who: okarcz
    note: "Spawned from 0001 future work."
---

# Update amm-trades-schema.md §7 and §11.1 with empirical findings

## Summary

Replace the hypothetical wording in §7 step 3 ("if any AMM uses a
different symbol the indexer's per-venue mapping handles it") and the
open question §11.1 with the empirical finding from task 0001:
**at least two distinct topic symbols are in use**
(`Symbol("swap")` and `Symbol("trade")`), with different decoder shapes,
so the per-venue mapping is mandatory.

## Context

Lore 0001 / `S-amm-trades-schema-§11-1-resolved.md` documents:
- `swap` event shape: `topics.len()=3`, data is Vec[Address×3, U128×2]
- `trade` event shape: `topics.len()=4`, data is Vec[I128×3]
- Same DDL (§4) covers both; only the decoder differs.

## Implementation

- Update §7 step 3 wording.
- Replace §11.1 with the resolved finding + link to lore 0001.
- Optionally add a small "Per-venue decoder reference" subsection
  documenting the two observed shapes (table form).
- Do NOT change the DDL; it is correct as-is.

## Acceptance Criteria

- [ ] §7 step 3 reflects empirical finding (no longer hypothetical)
- [ ] §11.1 marked resolved with reference to lore 0001 archive path
- [ ] No DDL changes (verify with `git diff`)
