---
id: "0275"
title: "19 ClickHouse integration tests have never run in CI — cargo test --workspace skips every #[ignore] and no workflow provides a database"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0215", "0172", "0182", "0218", "0114"]
tags: [layer-infra, priority-high, effort-medium, milestone-M3, ci, testing, clickhouse]
milestone: 3
links:
  - "../../../.github/workflows/ci.yml"
  - "../../../packages/enrichment-worker/tests/ch_enrich_it.rs"
history:
  - date: 2026-09-10
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0215]]. Found while closing its "a test asserts TWO pivot
      statements" criterion: the end-to-end coverage already existed and would
      have caught the defect — but it has never run. 0215 shipped a unit-level
      guard instead, which is a workaround for this task, not a replacement.
---

# The ClickHouse integration tests have never run in CI

## Summary

`packages/enrichment-worker/tests/ch_enrich_it.rs` holds **19 integration
tests** against a real ClickHouse. Every one is `#[ignore]`, because they need a
reachable database. CI runs `cargo test --workspace` (`ci.yml:175`), which
**skips `#[ignore]` tests**, and no workflow in `.github/workflows/` defines a
ClickHouse service or passes `--ignored`.

So they run only when someone remembers, by hand, on a laptop:

```bash
docker compose up -d clickhouse
cargo test -p enrichment-worker --test ch_enrich_it -- --ignored
```

The suite output states it plainly on every CI run, and has for months:

```
running 19 tests
test result: ok. 0 passed; 0 failed; 19 ignored; 0 measured
```

## Why it matters

These are not incidental tests. They are the regression guards for defects that
have already reached production and cost weeks:

| test | guards |
|---|---|
| `usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg` | [[0172]] — the $1 USDT peg, a ~7.4x overstatement across 44,657 candles |
| `enrich_fills_close_usd_across_oracle_peg_and_pivot_tiers` | tier precedence, idempotence, no leftover reference table |
| `the_usd_reset_*` (5 tests) | [[0182]] — the reset that destroyed 157 candles at an epoch boundary |
| `coarse_sweep_*` / `coarse_repair_*` (5 tests) | [[0114]] / [[0218]] |

⚠️ **A guard that cannot fail the build is documentation, not a guard.** [[0215]]
asked for a test "so a silently-narrowed pivot set fails the suite"; the test
existed and the suite could not fail, which is why 0215 had to add a unit-level
plan assertion to get an actually-armed check.

## Implementation

- Add a ClickHouse service to the Rust CI job, pinned to the **exact production
  version** (26.3.10.60 as of 2026-09-10 — read it, do not copy this number
  forward), and run the ignored tests with `CLICKHOUSE_URL` pointed at it.
- Expect failures on the first run and budget for them. Nothing has enforced
  these tests for months; some will have rotted against schema and behaviour
  changes. **Triage each one — a test that no longer matches intended behaviour
  must be fixed or deleted with a reason, never `#[ignore]`d back to sleep.**
- Decide whether they gate every PR or run on a schedule. Gating is stronger;
  the runtime cost and the shared-runner impact are the trade-off.
- Once armed, review whether `plan_issues_one_peg_and_two_pivots` ([[0215]])
  stays. It covers the emission where the ITs cover the outcome, so it probably
  earns its place either way — but that should be a decision, not an accident.

## Acceptance Criteria

- [ ] CI runs the `ch_enrich_it` tests against a ClickHouse pinned to the
      version production is on, verified by reading prod rather than assuming.
- [ ] The run is visible in the CI log as a non-zero passed count — a job that
      silently skips them again fails this criterion.
- [ ] Every test that fails on first arming is triaged: fixed, or deleted with
      the reason recorded. None are re-ignored to make the build green.
- [ ] A deliberately broken pivot set (or equivalent induced defect) turns the
      CI job red — verified by inducing, on a branch, not inferred.
- [ ] Any other crate's `#[ignore]`d integration tests are inventoried, so this
      is answered for the workspace and not just `enrichment-worker`.

## Notes

- Local runs must match the prod ClickHouse version; a test trusted against a
  different build proves less than it appears to.
- Do not fold this into [[0215]]. Its defect is fixed and its remaining scope is
  one crate; this is a workspace-wide gap in how tests are enforced.
