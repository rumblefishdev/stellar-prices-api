---
id: "0256"
title: "asset-discovery's ledger scan has never run on production — the worker re-seeds hourly and scans nothing"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0210", "0054", "0218"]
tags: [layer-backend, priority-high, effort-small, milestone-M2, ingest, defect]
milestone: 2
links:
  - "../../../packages/asset-discovery/src/main.rs"
history:
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Found while deploying [[0210]]'s symbol stage. Reading the worker's
      CloudWatch logs to confirm the symbol stage showed that every run since
      at least 07:17 UTC ends in the same WARN and `scanned: 0`.
---

# The ledger scan is dead code in production

## Summary

`prices.discovery_state` is **empty** on production, and every `asset-discovery`
run logs:

```
WARN  no discovery_state cursor and INITIAL_DISCOVERY_LEDGER unset —
      seeding only, skipping ledger scan
```

The worker's own stats confirm it: `scanned: 0`, `to_ledger: 0`,
`pools_total: 0`, on every run. The scan half of this Lambda has never executed.

## Context

The scan starts from `load_cursor()`, falling back to the operator-set
`INITIAL_DISCOVERY_LEDGER` when there is no cursor. Neither exists, so the
branch is skipped — by design, loudly, but nobody was reading the log.

The 52 soroban assets and the ~207k registry evidently arrive by another path
(most likely `prices-ledger-processor`), which is why the gap went unnoticed:
the registry looks healthy.

## Implementation

- Decide whether the scan is still wanted at all. If `ledger-processor` already
  covers asset discovery, this stage may be redundant and the honest fix is to
  delete it rather than start it.
- If wanted: set `INITIAL_DISCOVERY_LEDGER` in `infra/envs/production.json`,
  pick a sensible starting ledger, and confirm `discovery_state` advances.
- Either way the WARN must stop being a permanent steady state — a worker whose
  main stage is skipped every hour should alarm, not log.

## Acceptance Criteria

- [ ] A recorded decision on whether the ledger scan is still needed
- [ ] If kept: `discovery_state` has a cursor and it advances between runs
- [ ] If dropped: the scan path and its config are removed, not left dormant
- [ ] The permanent WARN is gone — either the scan runs, or the code does not
      pretend it might
