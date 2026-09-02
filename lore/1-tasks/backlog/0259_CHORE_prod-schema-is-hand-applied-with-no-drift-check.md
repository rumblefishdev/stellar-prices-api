---
id: "0259"
title: "Production schema is hand-applied with nothing verifying it matches init.sql"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0210", "0178", "0168"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M2, clickhouse, tooling]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/src/bin/prices-clickhouse-drift.rs"
history:
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0210]]'s deploy. Confirming that 0168's and 0178's view
      changes were live took a dozen ad-hoc queries and marker-hunting through
      `SHOW CREATE VIEW` output, because nothing records or checks what has
      been applied.
---

# Nothing knows whether prod's schema matches the repo

## Summary

`grep -r 'prices-clickhouse-init\|init.sql' .github/ Makefile infra/src` returns
nothing: no CI step, no CDK step applies schema. Production DDL is run by hand
from a ClickHouse client, and there is no record of what was applied when.

## Why this surfaced

[[0210]] added a table, so its runbook had to answer "is the schema current?"
before deploying the reader. Answering it meant:

- checking `system.columns` for 0178's `current_prices.method`,
- pulling `SHOW CREATE VIEW` for four views and grepping the normalised SQL for
  0168's `if(max(peg_rate) > 0, 'oracle', 'peg')`,
- separating commits that changed SQL from commits that changed only comments,
  because four of the five pending ones turned out to be comment-only.

Everything was in fact current. The problem is that establishing it was
archaeology, and a *negative* answer would have been found the same way — by
accident, during someone else's deploy.

## What already exists

`packages/prices-clickhouse/src/bin/prices-clickhouse-drift.rs` — a drift binary
nobody runs on a schedule. This may be less "build a tool" than "run the tool".

## Implementation

- Run the existing drift check against production and see what it reports.
- Give it somewhere to run: a scheduled Lambda beside the other probes
  (`*-freshness-probe` are the model), or a CI job against a restored snapshot.
- Alarm on drift rather than reporting it into a log nobody tails.
- Consider recording applied migrations in a table, so "what is on prod" is a
  query rather than an investigation.

## Acceptance Criteria

- [ ] Schema drift between `init.sql` and production is detected automatically
- [ ] The check runs on a schedule and alarms, rather than being a manual step
- [ ] A deploy that needs a new table can answer "is prod current?" in one
      query instead of a dozen
