---
id: "0064"
title: "ClickHouse-backed cursor for the Prices Ledger Processor"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0038", "0094", "0091"]
tags: [layer-indexing, priority-high, effort-small, lambda, clickhouse, cursor]
links:
  - "../active/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
history:
  - date: 2026-06-24
    status: backlog
    who: oski
    note: "Spawned from 0038 future work (spec Part D.1)."
  - date: 2026-06-24
    status: backlog
    who: claude
    note: "Added PR #34 review context for finding #3 (cold-start rewind + bootstrap; interim INITIAL_CURSOR SSM seed shipped)."
  - date: 2026-07-14
    status: active
    who: okarcz
    note: >
      Promoted backlog → active, priority-medium → HIGH. Diagnosed live as the
      cause of the still-frozen frontier after the proto27 deploy (0094): the
      /tmp StubFileCursor resets to INITIAL_CURSOR (63,352,611, the backfill
      floor) on every Lambda execution-environment recycle, so the reconcile
      loop oscillates floor→~63,372k→floor and never reaches the wall/tip. proto27
      masked it until now. IMPLEMENTED the durable CH cursor + wired into main.rs;
      all 4 integration tests pass against local CH 26.3.10.60 (= prod). Blocks
      0094 AC #2 (live advances past 63,401,875). Branch feat/0064_ch-backed-cursor.
---

# ClickHouse-backed cursor for the Prices Ledger Processor

## Summary

Replace the Lambda's `StubFileCursor` (a `/tmp` file, lost on cold start)
with a durable cursor read from / written to ClickHouse, so the
doorbell-cursor reconcile loop resumes correctly across container churn.

## Context

Task 0038 ships with `StubFileCursor` as a placeholder. The production
cursor design is the open question in `G-local-prototype-spec.md` Part D.1.
BE's cursor is `max(sequence) FROM default.ledgers`; we only persist
pricing-relevant ledgers, so `max(...) FROM prices.price_ohlcv_1m` undercounts.

## Review findings (PR #34 review, 2026-06-24)

Finding #3 (durable cursor) was confirmed in the PR #34 review, with two
concrete failure modes this task removes:

- **Cold-start rewind / reprocessing.** `/tmp` is per-container ephemeral. On
  every container recycle the cursor is lost and re-seeded from the *static*
  `INITIAL_CURSOR`, so the loop rewinds to a fixed ledger and re-walks the
  whole `INITIAL_CURSOR..tip` span. Idempotent (RMT), but the redundant S3
  fetch + decode + write is paid on every cold start; if the seed is far
  behind it can blow the Lambda timeout and livelock the doorbell.
- **Bootstrap.** Without a seed the loop errors on `cursor.read()` and DLQs
  every doorbell. Interim mitigation already shipped in PR #34: `main.rs`
  seeds from `INITIAL_CURSOR`, wired in CDK from the prices-owned SSM param
  `/prices/{env}/ledger-processor/initial-cursor` (`compute-stack.ts`). This
  task supersedes that stop-gap with the durable CH cursor and should retire
  the static seed (or keep it only as a genuine first-run bootstrap).

## Implementation

- Lean: own single-row `prices.processed_ledgers` (ReplacingMergeTree,
  updated last per run — D.1 option 1).
- Implement `Cursor` over `prices-clickhouse` (mTLS client); wire into
  `main.rs` in place of `StubFileCursor`.
- Decide seed-on-empty behaviour (env `INITIAL_CURSOR` vs first-S3-probe).

## Acceptance Criteria

- [x] Cursor table added to the schema — `prices.ingest_cursor` (chose this over
      `processed_ledgers`; single row per consumer `id`, RMT(updated_at)).
- [x] CH `Cursor` impl (`cursor::ClickHouseCursor`); reconcile resumes from CH
      across cold starts. Proven by `cursor_ch_it::cursor_survives_a_new_client_instance`
      (a fresh client reads the persisted value, does not rewind).
- [x] Idempotent: re-run from the persisted cursor is a no-op past the tip
      (existing `reconcile_e2e::idempotent_on_re_run_from_same_cursor` + gap-stop;
      cursor persistence doesn't change candle idempotency — RMT by version).
- [ ] **Deploy** the new processor to prod (approval-gated) — the freeze only
      lifts once the xdr-27 **and** CH-cursor build is running. **Requires the
      `prices.ingest_cursor` table applied to prod CH first** (init.sql, out-of-band).

## Implementation Notes

Built on branch `feat/0064_ch-backed-cursor`:

- **Schema** — `prices.ingest_cursor (id String, ledger UInt64, updated_at
  DateTime64(3))`, `ReplacingMergeTree(ledger) ORDER BY id`, in `init.sql`
  (`updated_at` is informational only). `init_sql_parses_into_statements`
  bumped 28 → 29.
- **`cursor::ClickHouseCursor`** — `read()` = `SELECT ledger … FINAL WHERE id=?`
  via `fetch_optional`; 0 rows → `CursorError::Empty` (seed signal), query error
  → `Read` (never seeded on). `write()` = `INSERT … now64(3)`.
- **`main.rs`** — swapped `StubFileCursor` → `ClickHouseCursor`, built from the
  sink's shared mTLS client (`sink.client().clone()`); dropped `CURSOR_FILE` /
  `/tmp`. Seed-on-empty from `INITIAL_CURSOR` retained (genuine first-run only).
- **Tests** — `tests/cursor_ch_it.rs`, 5 cases (incl. `a_lower_write_never_rewinds`
  + Empty-vs-Read), all green vs local CH 26.3.10.60 (= prod), parallel-safe +
  re-run-safe. `cargo check --workspace`, clippy, fmt all clean.

## Design Decisions

### Emerged

1. **`ReplacingMergeTree(ledger)` — monotonic-forward, not `updated_at`-as-version.**
   *(Reversed after the PR #108 code review — the first cut used
   `ReplacingMergeTree(updated_at)`.)* Keying the RMT version on `ledger` means
   FINAL always keeps the HIGHEST ledger for an `id`, so a stray lower write can
   never rewind the cursor, and there is no same-millisecond version tie (which
   `now64(3)` could hit and resolve arbitrarily → a backward read). The cost: a
   deliberate operator rewind needs an explicit `DELETE`/`TRUNCATE`, not just a
   lower `INSERT` — an acceptable, safer trade (accidental rewind was the bug).
2. **Seed only on `CursorError::Empty`, never on `Read`.** *(PR #108 review.)*
   `read()` distinguishes 0-rows (`Empty`) from a failed query (`Read`); `main`
   seeds on `Empty` only. A transient CH read failure at cold start therefore
   fails Init loudly instead of clobbering a healthy cursor with the floor seed
   (which — combined with the old `updated_at` version — would have re-frozen the
   frontier). Backstopped at the storage layer by decision 1.
3. **Share the sink's mTLS client** (`sink.client().clone()`) instead of opening
   a second connection — no extra Secrets-extension fetch / handshake at cold
   start. Required exposing `ClickHouseSink::client()` and promoting `clickhouse`
   from dev- to a normal dependency (already compiled transitively — free).
4. **Kept `INITIAL_CURSOR` as the genuine first-run bootstrap**, not retired. On
   an empty table it seeds once; thereafter the CH value is authoritative. A
   missing table errors at Init (fail-loud, matching the pool_registry contract).
5. **`id = "ledger-processor"`** constant — the table is multi-consumer by
   design, so future consumers get their own row without schema change.
