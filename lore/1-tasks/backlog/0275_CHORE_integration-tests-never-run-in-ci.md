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
  - date: 2026-09-10
    status: backlog
    who: okarcz
    note: >
      📐 **Measured before estimating — the suite is CLEAN and FAST.** Ran all
      19 locally against `docker compose` ClickHouse: **19 passed, 0 failed, in
      0.57 s**. So this task carries **no triage burden** — the original
      "expect failures, budget for them" framing was a guess and it was wrong.
      Also settled: `docker-compose.yml` already pins **26.3.10.60**, byte-equal
      to prod's `version()` read the same day, so the version-parity question
      is answered before the work starts. What remains is genuinely small — a
      service container plus `-- --ignored`.
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
  forward; `docker-compose.yml` already pins exactly this and is the model), and
  run the ignored tests with `CLICKHOUSE_URL` pointed at it.
- ✅ **No triage expected.** All 19 pass today (measured 2026-09-10, 0.57 s).
  Should one fail on first arming anyway, the rule still stands: fix it or
  delete it with a reason, never `#[ignore]` it back to sleep.
- **Gate every PR.** The runtime objection does not survive the measurement —
  0.57 s of tests behind a ~20-30 s container start. A nightly schedule would
  buy nothing and would report failures away from the change that caused them.
  Consider landing `continue-on-error` for one or two runs purely to confirm the
  container wiring, then removing it; do not leave it non-blocking.
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
      (Expected to be vacuous — all 19 pass as of 2026-09-10.)
- [ ] A deliberately broken pivot set (or equivalent induced defect) turns the
      CI job red — verified by inducing, on a branch, not inferred.
- [ ] Any other crate's `#[ignore]`d integration tests are inventoried, so this
      is answered for the workspace and not just `enrichment-worker`.

## ⏱️ Timing against Adam's open PRs

**#293 (task 0268) takes `ch_enrich_it` from 19 to 34 fixtures, and #300 (0267)
adds 8 more** across `prices-api` and `prices-clickhouse` — the dormant suite
roughly doubles when they land. Both PRs instruct the operator to run those
fixtures manually before the production repricing campaign, so today that
instruction is the only thing standing between the campaign and a defect:
everything CI checks on them is SQL-string assertions, not behaviour against a
real database. #293 carries a **bounded reset**, which is the operation that
destroyed 157 candles in [[0182]].

Arming CI does not conflict with those PRs — the existing 19 are green, so a red
build on his branches would mean his code, not inherited rot.

## Notes

- Local runs must match the prod ClickHouse version; a test trusted against a
  different build proves less than it appears to.
- Do not fold this into [[0215]]. Its defect is fixed and its remaining scope is
  one crate; this is a workspace-wide gap in how tests are enforced.
