---
id: "0026"
title: "volume_quote_usd enrichment Lambda — implement the Phase 1 spec from task 0024"
type: FEATURE
status: active
related_adr: ["0003", "0004", "0007"]
related_tasks: ["0024", "0012", "0022", "0023", "0038", "0058", "0059"]
tags: [layer-indexing, priority-medium, effort-medium, lambda, ohlcv, enrichment, oracle, phase-2, clickhouse]
links:
  - "../../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md"
  - "../../archive/0024_FEATURE_volume-quote-usd-enrichment/README.md"
  - "../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-05-13
    status: blocked
    who: claude
    note: >
      Spawned from 0024 as Phase 2 (implementation). Blocked on
      task 0012 — needs RDS bootstrap, `price_ohlcv` schema, and
      Oracle Fetcher Lambda to be deployed before the enrichment
      Lambda can be built and integration-tested.
  - date: 2026-06-08
    status: active
    who: oski
    note: >
      Activated with the same scope reduction applied to task 0038:
      local-only Rust crate + a written design document for the BE
      cross-team meeting. No AWS deploy, no CDK apply, no live CH
      writes, no EventBridge rule registration. The deliverable is
      a runnable local binary the operator can demonstrate against
      fixture data plus a G-note that lets BE react to the schema
      and merge-semantics choices before any infra commitment.

      **Critical architectural caveat.** The 0024 design spec was
      written 2026-05-13 against the original RDS-Postgres data
      plane. ADR 0007 (accepted 2026-05-20) supersedes that to
      Hetzner ClickHouse. The PG-flavoured SQL in §2 of the
      0024 G-note (`WITH ... FOR UPDATE SKIP LOCKED`,
      `UPDATE ... FROM`, row-lock-bounded batches) does not
      translate directly: CH has no row locks, `ALTER TABLE ...
      UPDATE` is asynchronous, and the idiomatic enrichment
      pattern is INSERT-with-newer-version into a
      `ReplacingMergeTree`, deduped on next merge by the
      ORDER BY key. The local prototype + spec G-note translate
      the algorithm to CH semantics; the PG→CH translation is
      one of the BE-meeting agenda items.

      Out-of-scope for this activation: any AWS deploy,
      EventBridge / Scheduler rule, IAM grants, SSM consumption,
      CDK stack apply, or live CH writes — see the forthcoming
      G-note under `notes/G-local-prototype-spec.md` for the full
      Part C cross-team contract.
  - date: 2026-06-09
    status: blocked
    who: oski
    note: >
      Re-blocked after landing the local prototype + production CH
      Form-B enrichment path (commit 75d00d0). The reduced
      local-only scope is delivered: runnable crate, fixture path,
      production INSERT…SELECT ASOF-JOIN path wired, schema
      requirement (ReplacingMergeTree + restored `volume_quote`)
      documented, follow-ups 0058/0059 spawned. The remaining
      acceptance criteria are integration-only and cannot be met
      without live infra: blocked on 0012 (live ClickHouse endpoint
      + Oracle Fetcher writing `oracle_prices`) and 0051
      (`price_ohlcv_1m` + MV rollup-chain DDL deployed). Also gated
      on 0058 (writers must populate `volume_quote`) and a BE
      cross-team review of the schema/merge-semantics choices in the
      G-note before any infra commitment. The production path
      compiles and passes the prototype unit suite but has NOT been
      run against a live ClickHouse.
  - date: 2026-06-26
    status: active
    who: oski
    note: >
      **Unblocked — every dependency from the 2026-06-09 re-block has
      resolved.** 0012 (live CH endpoint + Oracle Fetcher) ✅ completed;
      0051 (`price_ohlcv_1m` + MV rollup-chain DDL) ✅ completed and live on
      ch-prod-01; 0059 (rollup version propagation under enriched re-inserts)
      ✅ completed/merged (PR #61). The `volume_quote` data dependency
      (tracked as 0058) is satisfied: the shared
      `prices_ingest_core::OhlcvWriter` (writer.rs:122/210) now populates
      `volume_quote` for BOTH writer paths (sdex-backfill + the 0038
      ledger-processor), with `volume_quote_usd` left at DEFAULT 0 for this
      enrichment to fill. Live CH is reachable (0063 tenant + 0052 mTLS, per
      memory). Moving blocked → active to drive the remaining
      integration-only ACs. **Constraints carried in:** stay local-first /
      prepare-not-deploy (no AWS deploy, EventBridge rule, or live prod
      writes without explicit approval); the BE cross-team review of the
      Form-B merge semantics in `notes/G-local-prototype-spec.md` is still
      an open agenda item, not a code blocker. (Body "Context" / "blocked
      until 0012" prose is historical — 0012 is archived/completed.)
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      **Re-integrated the orphaned `enrichment-worker` crate into the Cargo
      workspace and drove the locally-achievable integration ACs green.**
      The crate had been dropped from the root `Cargo.toml` `members` list by
      a later merge (the `members` block was rewritten when 0039/0054's crates
      landed), leaving it out of `cargo metadata`, `cargo check --workspace`,
      and `cargo test --workspace` — i.e. silently un-built and un-tested by CI
      since the prototype commit (d13d3dc). Re-added the member; the crate
      compiles in-workspace against the current `prices-clickhouse` and the
      full workspace check stays clean. Fixed one `collapsible_if` clippy nit
      in `ch_enrich.rs` (run path; behaviour unchanged). Verified: 24 unit +
      2 e2e pass; **all 3 `#[ignore]`d live-CH integration tests
      (`tests/ch_enrich_it.rs`) pass against a local ClickHouse pinned to the
      prod version 26.3.10.60** — exercising the oracle / stablecoin-peg /
      XLM-pivot tiers, idempotency (2nd pass = no change), oracle-miss stays
      `close_usd = 0`, budget-exhaustion-defers-not-pegs, and the snapshot
      watermark bound, plus the run-scoped pivot-ref table cleanup. CI keeps
      these `#[ignore]`d (no CH on the runner) so they compile but don't run
      there. **Remaining ACs are deploy/infra-gated** (CDK Lambda + EventBridge
      Scheduler rule + IAM, CloudWatch metric publish + dashboard, live-prod
      backfill credibility check) and stay out of scope under the carried-in
      prepare-not-deploy constraint; the BE Form-B review is still open. Task
      stays `active`.
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      **Option 1 — CDK + packaging (prepare-only).** Wired the enrichment
      Lambda into the CDK app and matched the sibling-worker packaging, closing
      the EventBridge/IAM ACs at the code+synth level (no deploy). Infra:
      `EnrichmentRule` (`rate(1 hour)`) + worker `Function` + IAM role + error
      alarm + log group in `eventbridge-stack.ts` via the shared
      `createWorkerLambda`, mirroring oracle/cleanup/supply; `enrichment` added
      to the schedule config type + validation + `production.json`. `cdk synth`
      produces the full resource set (verified: rule rate(1 hour) → function;
      env carries the mTLS contract CH_DOMAIN/MTLS_SECRET_NAME + CLICKHOUSE_*).
      Crate: gated the Lambda bin behind a `lambda` feature (required-features,
      lean default build) like the siblings, and rewrote the entrypoint to build
      the mTLS client via `prices_clickhouse::mtls::client_from_lambda_env`
      instead of a plain CLICKHOUSE_URL client (the prior fixture-prototype
      Lambda mode is dropped; the prototype lives on as `enrichment-cli`). Added
      `ChEnrichmentPass::with_client`; url-based `new()` kept for the integration
      tests. CI: added `-p enrichment-worker` to the cargo-lambda build matrix.
      Verified locally: default + `--features lambda` build clean, 24 unit + 2
      e2e pass, clippy/fmt clean, `cargo lambda build` produces the bootstrap,
      infra lint/build/typecheck + `cdk synth` green. **Still deferred:** the
      custom `EnrichmentRowsRemainingAtVolumeZero` metric + CloudWatch publish +
      dashboard (Option 2 / task 0056), the one-shot historical mode (Option 2),
      and the actual deploy + live backfill credibility check (Option 3 — lifts
      prepare-not-deploy). Task stays `active`.
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      **Option 2 — CloudWatch metrics + one-shot mode (prepare-only).** Closed
      the metric-emission half of the telemetry ACs and added the historical
      drain mode. Metrics: new `metrics.rs` publishes the four spec §5 metrics
      (`EnrichmentRowsEnriched`, `EnrichmentOracleMiss`,
      `EnrichmentRowsRemainingAtVolumeZero`, `EnrichmentBatchDurationMs`) via
      `aws_sdk_cloudwatch` under the `Prices/Enrichment` namespace; the
      stats→metric mapping is a pure unit-tested function and the publish is
      `lambda`-gated + best-effort (never fails the pass). `ChPassStats` gained
      `oracle_misses` (remaining after the oracle tier) + `duration_ms`. One-shot:
      `MAX_BATCHES=0` ⇒ unbounded drain in a single invocation (both tier loops
      use `effective_max_batches()`), for clearing a large post-backfill backlog
      (spec §4); covered by a new integration test (`one_shot_drains_full_backlog`).
      Infra: granted `cloudwatch:PutMetricData` to the enrichment role (scoped to
      the `Prices/Enrichment` namespace) and authored the
      `EnrichmentRowsRemainingAtVolumeZero` backlog alarm in observability-stack
      (scaffold threshold; task 0056 tunes + owns the dashboard widgets).
      Verified: 25 unit + 2 e2e + 4 live-CH integration tests green; clippy/fmt
      clean (default + lambda); `cargo lambda build` bootstrap builds; infra
      lint/build/typecheck + `cdk synth` green (PutMetricData grant + alarm
      confirmed in the templates; namespace consistent across worker/IAM/alarm).
      **Remaining (Option 3, deploy-gated):** actual `cdk deploy`, live dashboard
      visibility, and the post-backfill credibility check (≥3 XLM-quoted assets).
      Task stays `active`.
  - date: 2026-07-02
    status: active
    who: claude
    note: >
      **Resolved the two enrichment metric items deferred here from the 0056
      code review (findings #5 + #7).** Both are worker-side + a one-line alarm
      rewire; no deploy.

      *Finding #5 — recency-bounded backlog (excise the stall alarm's idle-env
      false-fire).* `ChEnrichConfig` gains `recent_window_s` (env
      `ENRICH_RECENT_WINDOW_S`, default 7200s). `count_remaining_at_volume_zero`
      now returns `(total, recent)` from a **single** `FINAL` scan
      (`count()` + `countIf(timestamp >= now() - ?)`), `now()` evaluated
      server-side in CH (clock-skew-immune, matching the 0056 freshness-probe
      design). `ChPassStats` gains `rows_remaining_recent`; `metrics.rs`
      publishes `EnrichmentRowsRemainingRecent`. The observability-stack stall
      alarm's backlog term switched `EnrichmentRowsRemainingAtVolumeZero` →
      `EnrichmentRowsRemainingRecent`. Because an *idle* env produces no fresh
      candles, the recency count reads 0 there → the permanent deep-history
      exotic-quote floor no longer trips the alarm (the residual 0056 flagged is
      closed). The full `EnrichmentRowsRemainingAtVolumeZero` metric is still
      published for dashboard/forensic value — just no longer alarmed on.

      *Finding #7 — `EnrichmentBatchDurationMs` mislabeled.* Renamed to
      `EnrichmentPassDurationMs` (it is whole-pass wall-clock: all batches + the
      `FINAL` count scans, not one batch) and added a derived
      `EnrichmentAvgBatchDurationMs = duration_ms / batches`, emitted only when
      `batches > 0`, so operators size batch/timeout headroom off a true
      per-batch figure. No alarm/dashboard consumed the old name (comments only),
      so the rename is safe.

      Verified: 26 unit (+2 metrics tests) + 2 e2e + **5 live-CH ITs** (+1 new
      `recency_bounded_backlog_excludes_deep_history_floor`, asserting total=2 /
      recent=1) green vs prod-pinned CH 26.3.10.60; clippy/fmt clean (default +
      lambda); `cargo check --workspace` green. Infra: `tsc -b` + eslint +
      prettier clean; `cdk synth` of the Observability stack confirms the alarm
      renders on `EnrichmentRowsRemainingRecent` with the SNS action wired.
      Task stays `active` (deploy-gated ACs unchanged).
---

# `volume_quote_usd` enrichment Lambda — implementation

## Summary

Implement the EventBridge cron Lambda specified in task 0024's
[Phase 1 design G-note](../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md).
Phase 1 produced a complete spec; this Phase 2 task lands the
running code + CDK + integration test.

## Context

Task 0024 (archived) split into a design Phase 1 (the G-note) and
this implementation Phase 2. The split was driven by the fact
that 0024's implementation can't usefully exist before task 0012
provides:

- RDS PostgreSQL with `price_ohlcv`, `oracle_prices`, and `assets`
  tables.
- The Oracle Fetcher Lambda writing `oracle_prices` rows.
- The SDEX backfill writing `price_ohlcv` rows that need
  enrichment.

When 0012 lands, 0026 unblocks and can be promoted.

## Implementation

Follow the design spec §1–§6 verbatim. Concretely:

- **Code**: new Rust crate (e.g. `crates/enrichment-worker`) in
  whatever workspace structure task 0012 sets up.
- **CDK**: Lambda function + EventBridge rule + IAM role +
  CloudWatch metric filters. Reuse the patterns 0012 establishes
  for the other Lambdas.
- **Tests**: an integration test that seeds `price_ohlcv` +
  `oracle_prices` and asserts the UPDATE result row-by-row.
- **Telemetry**: emit the four metrics enumerated in spec §5.
- **Alarms**: CloudWatch alarm on
  `EnrichmentRowsRemainingAtVolumeZero` per spec §5.

The historical (post-backfill) one-shot pass per spec §4 lands
either as a separate Lambda or as an invocation mode of the same
Lambda — implementer's choice; document the choice in this task's
notes when made.

## Acceptance Criteria

Carried over from task 0024's design spec §7:

- [x] EventBridge cron Lambda exists with the schema in §2 wired up.
      — CDK authored in `eventbridge-stack.ts` (`EnrichmentRule` rate(1 hour)
      → worker Function), `cdk synth` verified; live deploy still pending.
- [x] CDK + IAM matches §1.1 / §1.2.
      — `createWorkerLambda` (IAM role + mTLS-secret read + SSM read + error
      alarm + log group), arm64/PROVIDED_AL2023; synth-verified. Deploy pending.
- [x] Re-running on already-enriched rows produces zero changes
      (idempotency test). — `ch_enrich_it.rs` vs prod-pinned CH 26.3.10.60.
- [x] Rows with missing oracle stay at `volume_quote_usd = 0`,
      `EnrichmentOracleMiss` metric increments. — "no reference stays 0"
      verified by `ch_enrich_it.rs`; the `EnrichmentOracleMiss` metric is now
      emitted (Option 2: `metrics.rs` maps `ChPassStats.oracle_misses` →
      CloudWatch, alarm wired). Live increment observable only post-deploy.
- [ ] After full SDEX backfill + a one-shot historical enrichment
      pass, `current_prices.volume_24h_usd` for at least 3
      XLM-quoted assets reflects SDEX-sourced volume (>0 and
      credible against Horizon's historical aggregates).
      — one-shot mode now exists (`MAX_BATCHES=0`, Option 2); the live
      credibility check is still deploy-gated (Option 3).
- [ ] CloudWatch metrics from spec §5 are emitted and visible in
      the dashboard. — **emit half done** (Option 2 + the 2026-07-02 metric
      items: `EnrichmentRowsEnriched` / `EnrichmentOracleMiss` /
      `EnrichmentRowsRemainingAtVolumeZero` / `EnrichmentRowsRemainingRecent` /
      `EnrichmentPassDurationMs` / `EnrichmentAvgBatchDurationMs` published via
      `aws_sdk_cloudwatch` under `Prices/Enrichment`; the progress-based stall
      alarm now gates on `EnrichmentRowsRemainingRecent`, synth-verified).
      Dashboard widgets + live visibility remain deploy-gated (task 0056 owns
      the dashboard; observability-stack is still a scaffold).

## Future Work

Spawned from the production implementation (see G-note Decision Log,
2026-06-09):

- **0058** — populate the restored `volume_quote` column in the OHLCV
  writers (prices-ledger-processor 0038 + sdex-backfill + soroban-amm
  backfill). Enrichment reads this column directly; writers must fill it.
- **0059** — MV rollup-chain version propagation under enriched `_1m`
  re-inserts (task 0051 dependency). 0026 enriches `_1m` only.

## Decision Log

### 2026-07-02 — `recent_window_s` default = 4h, must be ≥ the stall alarm's sustain window

**Context.** The stall alarm (observability-stack) fires on
`EnrichmentRowsEnriched = 0 AND EnrichmentRowsRemainingRecent > 0`
sustained across 3 consecutive hourly datapoints
(`evaluationPeriods = datapointsToAlarm = 3`, 1h period → 3h sustain).
`EnrichmentRowsRemainingRecent` counts volume-zero candles whose
`timestamp` is within `recent_window_s` of the CH server clock.

**Bug found in PR #74 review (finding #1).** The window shipped at 2h,
*shorter* than the 3h sustain. A genuinely stuck **fresh** candle stays
inside a 2h window for only ~2 hourly datapoints, so it can never
accumulate the 3 consecutive breaches the alarm needs. Result: a real
enrichment stall in a **low-cadence env** (fresh candles arriving less
often than the window) would never page — a silent-outage regression vs.
the old full-backlog metric, which stayed >0 continuously.

**Root cause.** Two competing failure modes on the same knob:
- *Window too long* → a fresh **exotic** candle (no oracle/peg reference,
  never enrichable) could hold the alarm — a false page.
- *Window too short* → a genuine stall in a sparse env never reaches 3
  datapoints — a missed page (the bug above).

The original PR chose "short" to bound the exotic false-page, but that
requires `enriched = 0` for 3 straight hours *as well*, which effectively
never happens in a live env still producing enrichable candles — so the
exotic risk it was buying was far narrower than the missed-stall risk it
introduced.

**Decision.** Set `recent_window_s` default to **4h (14 400s)**, with the
invariant **`recent_window_s` ≥ the alarm's sustain window**. A fresh
stuck candle now survives all 3 datapoints, so real stalls page again.
The idle-env guarantee (finding #5) is preserved unchanged: the permanent
deep-history exotic-quote floor is *years* old and stays far outside any
few-hour window, so an idle env still reports `recent = 0`.

**Invariant enforcement.** The tie between window and sustain is
documented at all three sites (`ChEnrichConfig::recent_window_s` doc,
`main.rs` env default, and the alarm comment in observability-stack). If
`datapointsToAlarm`/`evaluationPeriods` change, `ENRICH_RECENT_WINDOW_S`
must be raised to match. Left as prose + magic-number defaults for now;
promote to a shared constant if the alarm ever moves off the 3h sustain.

**Deferred (airtight alternative, not taken).** The fully robust fix is to
scope `recent` to candles that *have* a usable reference (oracle/peg) but
are still `volume_quote_usd = 0` — i.e. "should have enriched, didn't".
That removes fresh-exotic candles from the count entirely, decoupling the
window from the exotic floor at any size. It needs an `EXISTS`/join in the
count query (more than a one-liner); file it as follow-up only if a
fresh-exotic false page is ever observed in practice.

### 2026-07-02 — `EnrichmentRowsRemainingRecent` is a steady-state-only signal (finding #2, accepted)

**Context.** `count_remaining_at_volume_zero` derives `recent` from a single
scan that mixes two clocks: the population **ceiling** is the pass-start
`watermark` (frozen, so concurrent inserts can't inflate the count), the
recency **floor** is `now()` evaluated when the scan runs (pass end). The
counted interval is `[now() − recent_window_s, watermark]`.

**Behaviour found in PR #74 review (finding #2).** The two ends only agree
when the pass is short. In a **one-shot drain** whose duration exceeds
`recent_window_s`, `now()` advances past the frozen `watermark`, so the
interval empties and `recent` collapses to **0** regardless of the real fresh
backlog. Example: drain starts 12:00 (watermark freezes at 12:00), runs 6h,
scan at 18:00 → floor 14:00 > ceiling 12:00 → `recent = 0`.

**Why it's harmless.** The stall alarm gates on the *scheduled hourly* pass
(`max_batches = 20`, finishes in seconds → `now() ≈ watermark`), never a
one-shot. And the alarm term is `enriched < 1 AND recent > 0`; a draining
one-shot has `enriched ≫ 0`, so a wrong `recent` can neither false-page nor
mask a real page. `rows_remaining_at_volume_zero` (`total`) is bounded only by
`watermark`, so it stays correct throughout a one-shot drain.

**Decision — accept, document only.** The `now()` anchor is deliberate: it is
what gives the finding-#5 idle-env guarantee (an idle env has nothing near the
wall clock → `recent = 0`). Anchoring the floor to `watermark` would fix
one-shot but re-break finding #5 (an idle env's `watermark` sits *on* the
floor, so it would read >0 again). The two can't be reconciled on one window
over one query, so `recent` is documented as **steady-state-only**: during a
one-shot drain, watch `EnrichmentRowsRemainingAtVolumeZero`, not
`EnrichmentRowsRemainingRecent`. Documented at the `ChPassStats.rows_remaining_recent`
field.

**Deferred (not taken).** If a long one-shot's misleading `recent = 0` ever
bites an operator, skip emitting `EnrichmentRowsRemainingRecent` when
`one_shot == true` (thread the flag into `ChPassStats` → `pass_metrics`). A few
lines; not worth it until observed.

## Notes

- This task is `blocked` until 0012 lands. When unblocked, move
  via `/lore-framework-tasks` from `blocked/` to `active/`.
- A separate v2 two-hop enrichment task for exotic-quote pairs
  (no direct USD oracle) may be spawned later — see 0024 design
  §3.1. Not part of this Phase 2.
