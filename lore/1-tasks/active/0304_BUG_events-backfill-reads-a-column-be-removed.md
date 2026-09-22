---
id: "0304"
title: "events-backfill reads default.soroban_events.transaction_id, which BE removed on 2026-09-17 — discover-pools and the whole AMM read are dead on production"
type: BUG
status: active
related_adr: ["0287"]
related_tasks: ["0290", "0286", "0291"]
tags: [layer-backend, priority-high, effort-small, clickhouse, events-backfill, amm, data-correctness]
links:
  - "../../../packages/events-backfill/src/discover.rs"
  - "../../../packages/events-backfill/src/source.rs"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "../../../docs/runbooks/0286-reingest-history.md"
history:
  - date: "2026-09-22"
    status: active
    who: okarcz
    note: >
      Found while running [[0290]]'s `--discover-pools` seed after merging
      PR #324: the tool failed pre-flight with `Code: 47 UNKNOWN_IDENTIFIER`.
      `default.soroban_events` lost its `transaction_id` column when BE
      altered the table on 2026-09-17 11:49:23; ClickHouse itself suggests
      `transaction_index`. Blocks 0290 AC 2 and the AMM side of [[0286]]
      phase 3. Live ingestion is NOT affected.
---

# events-backfill reads a column BE removed

## Summary

`default.soroban_events` no longer has `transaction_id`. Every
`events-backfill` read that names it fails outright:

```
Code: 47. DB::Exception: Unknown expression identifier `transaction_id`
… Maybe you meant: ['transaction_index']. (UNKNOWN_IDENTIFIER)
```

BE altered the table on **2026-09-17 11:49:23** (`system.tables.
metadata_modification_time`), which is the same day
`docs/runbooks/seed-pool-registry.md` records a successful `--discover-pools`
run — so the break has been invisible for five days.

The live columns are:

```
contract_id Int64, ledger_sequence Int64, transaction_index UInt32,
operation_index UInt16, event_index UInt32, application_order Int16,
event_type Int16, signature LowCardinality(Nullable(String)),
topics_xdr String, data_xdr String
```

## Blast radius

| component | reads the table? | state |
| --- | --- | --- |
| `prices-ledger-processor` (live) | **no** — parses ledger XDR directly | unaffected |
| `events-backfill --discover-pools` | `discover.rs:69`, `ORDER BY … transaction_id …` | **dead** |
| `events-backfill` AMM read | `source.rs:170–193`, three uses incl. `LEFT JOIN default.transactions ON t.id = e.transaction_id` | **dead** |

So live candles keep flowing and nothing is being written wrongly — this is a
tooling outage, not a data defect. But it blocks:

- **[[0290]] AC 2** — the 99 historical SushiSwap pools cannot be seeded into
  `prices.pool_registry`, so the live processor can only learn pools created
  from now on.
- **[[0286]] phase 3, the AMM months** — `source.rs` is the read the re-ingest
  uses for every month at or after Soroban activation.

## The fix is a simplification, not a port

`application_order` — the transaction's apply position, which ADR 0287's D1
needs for fill order — is now **a column on the event row**, and equals
`transaction_index` in every sampled row. The `LEFT JOIN` to
`default.transactions` exists for no other reason, so it goes away entirely.
Three things on [[0286]] lose their purpose with it:

- the join fan-out the S1 review had to collapse to one row per id;
- the WARN-once fallback to `(transaction_id, event_index)` ordering when the
  join misses;
- phase 3's "record the fallback share after the run" deliverable — there is no
  fallback left to have a share.

Grouping moves from the `transaction_id` string to
`(ledger_sequence, transaction_index)`.

## Implementation

- `discover.rs`: `ORDER BY ledger_sequence, transaction_index, event_index`.
- `source.rs`: drop the `default.transactions` join and its `found` flag, read
  `application_order` from `e`, and order by
  `e.ledger_sequence, e.application_order, e.transaction_index, e.event_index`.
- Wherever the Rust side groups events by `transaction_id`, key on
  `(ledger_sequence, transaction_index)` instead.
- Update the SQL-shape unit tests, which assert the old text.
- ⚠️ Decide what an event whose `application_order` is **negative** means now.
  `run.rs:734` currently pins *"a negative application_order cannot be a
  position; degrade, do not wrap"* against the joined value; the same guard has
  to survive on the direct column, and it is `Int16`, not unsigned.
- Update `seed-pool-registry.md` and `0286-reingest-history.md` where they
  describe the join and the fallback share.

## Acceptance Criteria

- [ ] `events-backfill --discover-pools --start 60000000 --end <tip> --dry-run`
      completes against production and reports a `sushiswap` entry in
      `per_venue`.
- [ ] The AMM read returns rows for a known range, and fill order matches what
      the pre-change join produced — pinned by a test on the SQL shape and by a
      spot check against a ledger with more than one transaction.
- [ ] A negative `application_order` still degrades rather than wrapping, with
      a test.
- [ ] No `transaction_id` reference to `default.soroban_events` remains in the
      repo.
- [ ] The two runbooks no longer describe a join that does not exist, and
      0286's "fallback share" deliverable is recorded as void.
