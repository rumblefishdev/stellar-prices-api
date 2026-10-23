---
id: "0336"
title: "coverage-sweep-unclassified stays in ALARM: new instances of the securities-financing ledger wasm page one by one — allow-list the family as not a venue"
type: BUG
status: active
assignee: akot
related_adr: []
related_tasks: ["0100", "0323", "0334"]
tags: [layer-infra, observability, coverage, priority-high, effort-small]
links:
  - "../../../packages/coverage-sweep-probe/allowlist.toml"
  - "../../../packages/coverage-sweep-probe/src/allowlist.rs"
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
history:
  - date: "2026-10-23"
    status: active
    who: akot
    note: "Task created from the daily stuck-alarm digest: coverage-sweep-unclassified in ALARM for 11d 4h"
---

# New instances of the securities-financing ledger wasm keep the sweep in ALARM

## Summary

`prices-production-coverage-sweep-unclassified` has been in ALARM since
2026-10-12 07:18 CEST. [[0334]] fixed the residual it was raised for, yet the
manual run after its deploy still found 4 contracts. All four run wasm
`f3dd3edf…`, the securities-financing ledger that [[0334]] allow-listed by
one contract id (`CDU47AIU…`). The family deploys new instances every few
days, so a `[[contract]]` entry per instance would page again every Monday.

## Context

Runs from `/aws/lambda/prices-production-coverage-sweep-probe`
(`coverage sweep complete`):

| Run (UTC) | unclassified | events | Note |
| --- | --- | --- | --- |
| 2026-10-12 05:17 | 3 | 404 | Comet `CASYAKK3…` (393), `CDU47AIU…`, `CD35HSTR…` ([[0334]]) |
| 2026-10-19 05:17 | 7 | 595 | before PR #405 merged |
| 2026-10-19 09:03 | 4 | 15 | manual, after the 0334 deploy; Comet and both 0334 entries gone |

The four left on 2026-10-19 09:03, all wasm
`f3dd3edfd637a9654054181a9c47391ddafa723697b982698b3c9662a9df974e`,
`trade_opened`/`trade_closed` with shape `sym|u64|bytes -> map`:

- `CAKQ6DWL…` 8 events, ledgers 64,908,423–64,941,821
- `CCLN72NM…` 4 events, 64,958,834–64,962,719
- `CBZYHS2D…` 2 events, 64,944,905–64,959,086
- `CB4B4ELL…` 1 event, 64,978,560

Since [[0323]] the alarm holds the latest run's state, so it stays in ALARM
until a run finds none: the 2026-10-26 run, or a manual invoke after deploy.

## Design

[[0100]] D4 forbids a `[[wasm]]` entry without `until`, because a permanent
family-wide ignore would hide the next pool of a real venue. That reason does
not hold for a family whose code is not a market. Add an explicit exception:

- `[[wasm]]` takes `not_a_venue = true` in place of `until`. Both together,
  or neither, fails validation.
- `f3dd3edf…` moves from `CDU47AIU…`'s `[[contract]]` entry to one such
  `[[wasm]]` entry.
- Runbook §3 outcome (c) says when to use it: the wasm itself rules out a
  market, not the size of the instances seen so far.

## Acceptance Criteria

- [ ] Validator: `not_a_venue` without `until` accepted; with `until`, or
      `until` missing without it, refused (unit tests)
- [ ] The five known instances match `wasm:f3dd3edf…` (unit test)
- [ ] Runbook §3 describes `not_a_venue`
- [ ] EventBridge deployed; one manual invoke logs `coverage sweep complete`
      with `unclassified=0`
- [ ] `coverage-sweep-unclassified` back to OK

## Notes

- The order-book wasm `4d6abb0d…` is allow-listed by two contract ids
  (`CBOSXSEZ…`, `CD35HSTR…` from [[0334]]). It is the same pattern and a
  candidate for `not_a_venue` if a third instance appears.
- The Comet `[[wasm]]` entry `8abc2891…` (`until = "0300"`) is stale: 0300
  shipped. Not removed here.
