---
id: "0017"
title: "Local ClickHouse instance setup and access for prices-api Tranche 1 backfill"
type: FEATURE
status: backlog
related_adr: ["0001", "0003", "0004", "0007"]
related_tasks: ["0015", "0018", "0051", "0052", "0053", "0058", "0026", "0061"]
tags: [layer-infra, priority-high, effort-small, milestone-M1, infra, backfill, clickhouse, hetzner, block-explorer]
milestone: 1
links:
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/S-redesigned-backfill-recommendation.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/archive/0205_FEATURE_backfill-runner-clickhouse-target-flag.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/archive/0206_FEATURE_clickhouse-persist-real-inserts/README.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0045_clickhouse-local-backfill-then-mirror-to-hetzner-via-freeze-rsync-attach.md"
history:
  - date: 2026-05-12
    status: backlog
    who: operator
    note: >
      Spawned from task 0015 closure. ADR 0001 commits Stream 1 to a
      local-CH-sourced backfill on the operator's workstation. This
      task is the operational landing: spin up CH locally, run BE's
      backfill-runner against it, document the access mechanism that
      lets the prices-api Tranche 1 consumer query the local CH.
  - date: 2026-06-19
    status: backlog
    who: claude
    note: >
      Refreshed against post-ADR-0007 architecture. Local store is now
      ClickHouse mirroring the Hetzner `prices.*` schema (NOT a local
      Postgres) — same local-CH→Hetzner shape BE uses. Added the
      archival/historical USD asset price requirement (volume_quote /
      volume_quote_usd / close_usd + oracle_prices; tasks 0058/0026/0061)
      and the mTLS cloud-push path via the 0052 client. Replaced personal
      username with "operator".
---

# Local ClickHouse instance setup and access for prices-api Tranche 1 backfill

## Summary

Stand up a local ClickHouse instance on the operator's workstation that
serves **two roles** for the Stream 1 historical AMM backfill:

1. **Raw input** — populated by running BE's `backfill-runner
   --target=clickhouse` (BE task 0205) for the Soroban-activation-onward
   ledger range (~ledger 48.5M to current tip, ~8.5M ledgers), giving the
   backfill CLI a local `soroban_events` source to query.
2. **Local `prices.*` mirror** — the same `prices` schema that lives on
   the Hetzner CH box, applied locally from our canonical
   `packages/prices-clickhouse/schema/init.sql`. The Stream 1 backfill
   (task 0053) aggregates decoded swaps into this local mirror, then runs
   a one-shot mTLS push to Hetzner `prices.*`. **This replaces the earlier
   "aggregate into a local Postgres" design** — we now mirror Hetzner in a
   local ClickHouse, the same local-CH→Hetzner shape BE uses (BE ADR 0045).

Document the access mechanism that lets the backfill CLI query the local
CH. Tear-down trigger: once Tranche 1 backfill completes and the OHLCV
rows are pushed to Hetzner `prices.*`.

## Context

ADR 0001 commits prices-api Stream 1 to local-CH-sourced backfill; this
task is the infrastructure side. Two architectural shifts since the
2026-05-12 draft change the shape:

- **ADR 0007 (accepted 2026-05-20)** — the live data sink is BE's shared
  Hetzner ClickHouse, **not** prices-owned RDS Postgres. All OHLCV / oracle
  / asset / backfill-progress data lives in the `prices` database on the
  Hetzner box. The backfill therefore stages into a *local CH mirror* of
  `prices.*` and pushes to Hetzner — no Postgres anywhere in the path.
- **ADR 0003/0004** — OHLCV rows are per-source
  (`source ∈ {soroswap, aquarius, phoenix, sdex, …}`) on
  `ReplacingMergeTree(version)`, PK `(timestamp, asset_id, quote_asset_id,
  source)`. Cross-source merge happens at read time.

The `prices.*` schema is already authored and shipped (tasks 0059/0060/0061)
in `packages/prices-clickhouse/schema/` (`init.sql` + `seed.sql` +
`rollups.sql` + `views.sql`), applied locally by the
`packages/prices-clickhouse-init` binary — the exact same artifact that
applies to Hetzner. The cloud push uses the shared mTLS client from task
0052 (`packages/prices-clickhouse`, `aws-mtls` feature) to Caddy:443.

The input side is gated on BE task 0206 (real CH writer) — **shipped**
(archived), as are BE 0205 (the `--target=clickhouse` flag) and BE 0117
(the local backfill benchmark, the storage/throughput proxy signal).

### Archival / historical USD asset price requirement

The `prices.*` OHLCV tables now carry USD columns that the historical
backfill must account for:

- `volume_quote Decimal(38,14)` — native quote-asset volume, restored by
  task 0058. The backfill (0053) **must populate this** per bucket
  (`Σ |quote_amount|`); it is the input to USD enrichment.
- `volume_quote_usd Decimal(38,14) DEFAULT 0` and
  `close_usd Decimal(38,14) DEFAULT 0` (task 0061) — filled by enrichment
  as `oracle_usd × volume_quote` and `oracle_usd × close`.
- `oracle_prices` table — the USD reference series enrichment ASOF-joins
  against (tasks 0024/0026).

For a **historical** backfill this needs **archival USD asset prices**:
USD references for *past* timestamps back across the Soroban range, not
just live oracle ticks. Whether `oracle_prices` actually carries history
that far back — or whether an archival USD price source must be loaded
into the local mirror before enrichment can run — is an **open question**
(see Notes / spawn point). The local mirror must at minimum include the
`oracle_prices` table and the USD columns so the enrichment path exists
end-to-end; sourcing the archival USD series is tracked with 0026.

## Implementation

- Docker-compose CH service mirroring BE's compose definition (CH version,
  port mapping `127.0.0.1:8123`, healthcheck, volume mount).
- Apply **both** schemas to the local instance:
  - BE's `init.sql` (`soroban_events` + friends) so `backfill-runner` has
    its target tables — the raw input side.
  - Our `packages/prices-clickhouse/schema/{init,seed,rollups,views}.sql`
    via `prices-clickhouse-init` so the `prices.*` mirror matches Hetzner
    exactly (USD columns + `oracle_prices` + per-source OHLCV +
    `backfill_progress` seeded `soroban_amm`).
- Run BE's `backfill-runner --target=clickhouse` against the public ledger
  archive for the Soroban range to populate `soroban_events`. Disk estimate:
  BE task 0117 benchmark is the input (~100–150 GB order-of-magnitude for
  the Soroban-only range; the `prices.*` mirror itself is tiny, ~0.45 GB/yr).
- Access mechanism for the backfill CLI: local plaintext HTTP
  (`localhost:8123`) on the workstation — no mTLS for the local hop;
  mTLS is only the Hetzner push (task 0052 client), not local reads.
- Capture actual disk usage, completion time, and any population errors as
  a closing note for ADR 0001's "consequences" section.

## Acceptance Criteria

- [ ] Local CH instance running on the operator's workstation with **BE's
      schema** applied (idempotent `init.sql` from BE task 0204) for the
      `soroban_events` input.
- [ ] Local CH also hosts the **`prices.*` mirror** of the Hetzner schema,
      applied from `packages/prices-clickhouse/schema/*` via
      `prices-clickhouse-init` — same artifact as the Hetzner apply,
      including the USD columns (`volume_quote`, `volume_quote_usd`,
      `close_usd`) and the `oracle_prices` table.
- [ ] BE `backfill-runner --target=clickhouse` completes against the
      Soroban-activation-onward ledger range with no `parts_to_throw_insert`
      errors and zero parser-data loss (verified against BE task 0206's
      coverage contract).
- [ ] Access mechanism documented; the backfill CLI can run a smoke query
      (`SELECT count() FROM soroban_events WHERE signature = 'swap'`)
      against the local CH and write a smoke row into the local
      `prices.price_ohlcv_1m` mirror.
- [ ] Archival USD price availability assessed and recorded: whether
      `oracle_prices` covers the historical range, or an archival USD price
      source is required (hand-off to 0026 if so).
- [ ] Disk usage, run time, and any anomalies captured as a closing note
      appended to ADR 0001 or a `notes/G-backfill-run-log.md`.
- [ ] Tear-down checklist documented (when to nuke the volume, how to do
      it cleanly).

## Notes

- **Local-only / prepare-only constraint**: the whole flow runs against
  local Docker (CH on localhost); the only public network access is the
  read-only `--no-sign-request` ledger fetch by `backfill-runner`. The
  Hetzner push (0053) is a separate, approval-gated step — not part of
  this setup task.
- BE's task 0206 must be merged before this task can run to completion —
  **done** (archived). If a future BE writer change lands, re-verify
  against 0206's coverage contract.
- **Stale references to fix elsewhere** (out of scope here, flag only):
  task 0053 and `docs/database-schema/database-schema-overview.md` still
  describe the backfill as aggregating into a *local Postgres*. Those need
  the same local-CH-mirror correction this task adopts.
- Storage estimate for `soroban_events`: BE has not published a hard number
  for the Soroban-activation-onward window; order-of-magnitude ~100–150 GB,
  verify against BE task 0117 benchmark output.
