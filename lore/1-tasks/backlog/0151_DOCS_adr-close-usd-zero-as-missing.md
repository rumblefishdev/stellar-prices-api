---
id: "0151"
title: "ADR: close_usd's zero-as-missing sentinel is what makes the whole 0144 bug class expressible"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0144", "0135", "0146", "0147", "0148", "0149", "0138"]
tags:
  ["priority-low", "effort-medium", "clickhouse", "schema", "adr", "data-correctness"]
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 9). Too invasive to retrofit
      during the 0144 fix chain, but it should be written down before the next
      surface is built on the same footing.
---

# ADR — `close_usd` zero-as-missing

## Summary

`close_usd` is `Decimal(38,14) DEFAULT 0` on a **non-nullable** column
(`init.sql:114`). So three distinct facts share one value:

1. **not yet priced** — enrichment has not reached this row (transient)
2. **will never be priced** — exotic quote, no oracle; a permanent floor by
   design (`ch_enrich.rs:31-32`)
3. **genuinely worth nothing**

Every defect in [[0144]] is a different aggregate meeting that value:

| Aggregate | What it does with the sentinel | Task |
|---|---|---|
| `argMax(close_usd, ts)` | returns it, discarding priced rows | [[0146]], [[0135]] |
| `sum(close_usd * volume)` | weights it as a real zero | [[0147]] |
| `WHERE close_usd > 0` | fixes the arithmetic by silently changing the population | [[0147]], [[0135]] |

And `views.sql`'s header already promises consumers **value-or-absent**
semantics classified against `usd_reference` — which is a contract the storage
does not implement. "Partially enriched" is a third state that today
masquerades as a good value.

## What the ADR must decide

- Whether `Nullable(Decimal)` or a **companion status column** is the target
  shape. Nullable makes the class unrepresentable but costs on a hot column;
  a status column is additive but leaves the sentinel in place for anything
  that ignores it.
- Whether state 2 (permanently unpriceable) deserves its own marker distinct
  from state 1 — [[0147]]'s coverage gate needs to distinguish them, and today
  it can only infer the difference from volume share.
- The migration story: `close_usd` is written by enrichment, the sweep, six
  rollup MVs and four pre-roll scripts. Any change touches all of them.
- Whether this is worth doing at all, or whether [[0146]] + [[0147]] leave the
  residual risk low enough to accept the sentinel permanently. **"Accept it,
  with these guardrails" is a legitimate outcome** — the point is that it be
  decided rather than inherited.

## Ordering

Deliberately last. The [[0144]] chain must land first: it establishes what the
guardrails actually cost and whether they are sufficient, which is the evidence
this ADR needs. Writing it earlier would be speculation.

## Acceptance Criteria

- [ ] ADR filed in `lore/2-adrs/` with a decision, not just an analysis.
- [ ] Cross-linked from [[0144]] and from `init.sql`'s column comment.
- [ ] The `views.sql` value-or-absent contract either implemented or the
      header corrected to match reality.
- [ ] If "accept the sentinel" is the outcome, the guardrails that make it
      acceptable are enumerated and each has a test.
