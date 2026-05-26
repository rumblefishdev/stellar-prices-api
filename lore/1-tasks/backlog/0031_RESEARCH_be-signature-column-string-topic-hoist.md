---
id: '0031'
title: 'Evaluate BE-side `signature` hoist for String-typed topic[0] (Soroswap, Phoenix)'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0018', '0017']
tags:
  [
    layer-research,
    priority-medium,
    effort-small,
    cross-repo,
    be-feedback,
    clickhouse,
    signature-column,
    filter-perf,
  ]
links:
  - '../active/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/R-be-storage-format.md'
  - '../active/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md'
  - '../../../../soroban-block-explorer/crates/db-clickhouse/src/persist/stage.rs'
history:
  - date: 2026-05-15
    status: backlog
    who: claude
    note: 'Spawned from 0018 Appendix B item 2.'
---

# BE signature column: String-typed topic[0] hoist

## Summary

`soroban_events.signature` is NULL for every Soroswap event
(topic[0] is `String("SoroswapPair")` / etc.) and every Phoenix
event (topic[0] is `String("swap")`). The undercount blocks the
obvious `WHERE signature = 'swap'` filter for two of the three
target AMMs.

Task 0018 §3.6 + §1.6 work around this with per-AMM
`JSONExtractString(topics_xdr, '$[0].value')` predicates + pool
whitelists. The workaround works but loses CH's
`LowCardinality(Nullable(String))` index acceleration on the
hoisted column. Quantifying the perf cost (and proposing a BE-side
fix if it bites) is this task.

## Context

`crates/db-clickhouse/src/persist/stage.rs::extract_event_signature`
currently:

```rust
if first.get("type").and_then(Value::as_str)? != "sym" {
    return None;
}
```

Three plausible BE-side fixes:

1. **Hoist String-typed topic[0] too** — single-line code change;
   may collide with Symbol semantics (need to confirm no protocol
   uses both topic[0] kinds with different meanings for the same
   string value).
2. **Add a second column** `LowCardinality(Nullable(String)) protocol`
   sourced from `(topic[0] type, topic[0] value, topic[1] value)`
   triple — e.g. `"SoroswapPair:swap"`, `"phoenix:swap"`,
   `"trade"` (Symbol-only events keep current value).
3. **Status quo + document workaround in BE README** —
   acceptable if the perf gap on `JSONExtract`-based filters is
   tolerable at the BE pilot scale.

## Implementation

1. Run a perf microbench on the local CH pilot once 0017 lands:
   compare `WHERE signature = 'trade'` vs
   `WHERE JSONExtractString(topics_xdr, '$[0].value') = '<x>'`
   for similar-cardinality predicate selectivity.
2. Based on results, write up an inbox message / Linear ticket to
   BE recommending option 1, 2, or status-quo.
3. Update task 0017's smoke query plan if option 1/2 is chosen.

## Acceptance Criteria

- [ ] Measured perf gap recorded.
- [ ] Recommendation surfaced to BE.
- [ ] Task 0017's smoke query plan updated if needed.

## Notes

Gated on task 0017's local CH being queryable (so we can actually
microbench).
