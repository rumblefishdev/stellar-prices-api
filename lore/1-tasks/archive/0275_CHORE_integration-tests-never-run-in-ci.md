---
id: "0275"
title: "229 ClickHouse integration tests have never run in CI — cargo test --workspace skips every #[ignore] and no workflow provides a database"
type: CHORE
status: completed
related_adr: []
related_tasks: ["0215", "0172", "0182", "0218", "0114"]
tags: [layer-infra, priority-high, effort-medium, milestone-M3, ci, testing, clickhouse]
milestone: 3
links:
  - "../../../.github/workflows/ci.yml"
  - "../../../packages/enrichment-worker/tests/ch_enrich_it.rs"
  - "../../../tools/scripts/ignored-tests.sh"
  - "../../3-wiki/project/ci-pipeline.md"
history:
  - date: "2026-09-10"
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0215]]. Found while closing its "a test asserts TWO pivot
      statements" criterion: the end-to-end coverage already existed and would
      have caught the defect — but it has never run. 0215 shipped a unit-level
      guard instead, which is a workaround for this task, not a replacement.
  - date: "2026-09-10"
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
  - date: "2026-09-18"
    status: active
    who: akot
    note: >
      Activated. The "19 tests" in the title is stale: a count on 2026-09-18
      found 238 `#[ignore]` across 40 files, most of them needing ClickHouse.
      The workspace inventory (last criterion) is therefore the first step, not
      the last — it decides what the CI job has to run.
  - date: "2026-09-18"
    status: completed
    who: akot
    note: >
      All 229 ClickHouse-class #[ignore] tests (31 targets, 12 crates) run on
      every Rust PR against 26.3.10.60 / UTC, asserted equal to prod. Derived
      inventory + asserted count (tools/scripts/ignored-tests.sh, 18 node:test
      cases seen red first). Four baseline reds fixed, none re-ignored. PR #327
      CI run 35364008161: 229/0/31, 96 s of tests, ~2 min per PR. Induced
      defect on draft PR #328, run 35365350665: cargo test green, IT step red
      on usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg.
      12 commits; 18 design decisions (9 planned, 9 emerged).
---

# The ClickHouse integration tests have never run in CI

## Summary

> **Corrected 2026-09-18.** The "19" below was `ch_enrich_it` alone, counted on
> 2026-09-10. The workspace holds **239** `#[ignore]`d tests, **229** of them
> ClickHouse-class, in 31 targets across 12 crates — see
> [Inventory](#inventory-2026-09-18). The text below is kept as written.

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

- [x] CI runs the ClickHouse tests against a ClickHouse pinned to the version
      production is on, verified by reading prod rather than assuming.
      **CI run [`35364008161`](https://github.com/rumblefishdev/stellar-prices-api/actions/runs/35364008161)
      (PR #327):** `preflight` logged `server version() = 26.3.10.60,
      timezone() = UTC`, then `229 passed, 0 failed over 31 target summaries`,
      with `execution_bound_error_it` passing behind `caddy:2.11.4`. Prod read 2026-09-18 over mTLS:
      `SELECT version(), timezone()` = `26.3.10.60 UTC`; `docker-compose.yml`
      pins `clickhouse/clickhouse-server:26.3.10.60`, and `ignored-tests.sh
      preflight` refuses any other `version()` or a non-UTC server (checked
      against a fake 26.3.10.59 pin: exit 1).
- [x] The run is visible in the CI log as a non-zero passed count — a job that
      silently skips them again fails. `ignored-tests.sh run` is red unless
      `sum(passed) == ` the derived number of ClickHouse-class `#[ignore]`s AND
      one `test result:` line per derived target, with `failed == 0`. Three
      consecutive local runs on 2026-09-18: **229 passed, 0 failed, 31 target
      summaries** each (99.8–101.2 s of test time), the third after dropping
      `prices` and every `it_*` database and re-bootstrapping with
      `prices-clickhouse-init --rollups`. Guard tests (18, `node:test`) seen red
      first; each rule mutation-checked (removing it reddens its own case).
      Same result in CI: 229 / 0 / 31, 96 s of test time.
- [x] Every test that failed on first arming is triaged — none re-ignored.
      The four reds of the 2026-09-18 baseline:
      `rollup_freshness_it` (7/25) and `symbol_queue_it` (1 test, flaky) share
      the `prices` database → the whole step runs `--test-threads=1` (D9);
      `endpoints_it::backfill_status_maps_both_streams` had a fixture
      timestamp that aged past the 7-day stall threshold → seeded
      `now() - INTERVAL 1 DAY` (commit `0db99d9`, no assertion weakened);
      `execution_bound_error_it` needs a reverse proxy → armed behind
      `scripts/ch-proxy-0281.sh` (Caddy 2.11.4) in CI and locally.
- [x] A deliberately broken pivot set turns the CI job red — verified by
      inducing, on a branch. **CI run [`35365350665`](https://github.com/rumblefishdev/stellar-prices-api/actions/runs/35365350665)**
      on throwaway draft PR #328 (closed, branch deleted): `cargo test
      --workspace` **success**, `ClickHouse integration tests` **failure** —
      `ch_enrich_it` 53 passed / 5 failed, among them
      `usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg`; the
      Lambda build was skipped. The
      patch (USDT arm of `resolve_reference_ids` compares against the USDC
      issuer, so USDT never resolves and `pivot_ids()` narrows to `[xlm]`)
      leaves `cargo test --workspace` green (1121 passed) and turns
      `ignored-tests.sh run` red: `usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg`
      fails with "USDT-quoted candle must not be left unpriced at 0 …", plus 4
      sibling ITs; 224 passed / 5 failed — locally and in CI alike.
- [x] Every other crate's `#[ignore]`d integration tests are inventoried —
      see below, and derived on every CI run by `ignored-tests.sh check`.

## Inventory (2026-09-18)

Derived by `tools/scripts/ignored-tests.sh expect` / `targets`, not by hand.

| class | reason prefix | tests | targets | CI |
|---|---|---|---|---|
| CH | `requires ClickHouse` | **229** | 31 in 12 crates | every Rust PR |
| NET | `requires public network` | 5 | `asset-discovery/symbol_rpc_it` (3), `oracle-worker/oracle_it` (1), `supply-worker/supply_net_it` (1) | never |
| PROD | `requires production` | 5 | `enrichment-worker/post_run_0228_it` (2), `post_run_0268_it` (2), `prices-clickhouse/mtls_smoke_it` (1) | never |

NET/PROD are recorded only: no nightly job and no backlog task (Adam,
2026-09-18) — third-party uptime and production state must not gate a PR.
`prices-ledger-processor/tests/incremental_assets_it.rs` has no `#[ignore]` and
already runs in `cargo test --workspace`.

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

## Implementation Notes

- `tools/scripts/ignored-tests.sh` — `check` / `targets` / `expect` /
  `image-tag` / `preflight` / `assert LOG` / `run` (default). Shaped after
  `lambda-assets.sh`: permissive extraction, then validation; refuses an empty
  inventory. The header carries the reasons for the vocabulary, the serial
  flag and the no-two-concurrent-runs rule.
- `tools/scripts/ignored-tests.test.mjs` — 18 `node:test` cases over throwaway
  fixture trees; `npm run ignored-tests:verify-guard`, run in the `typescript`
  job (Node from `.nvmrc`, 22.22.0).
- 239 `#[ignore]` reasons normalised to three byte-identical strings (incl. the
  7 bare ones in `ch_enrich_it.rs`); `supply_it`'s Horizon test moved to
  `supply_net_it.rs`.
- `ci.yml` `rust` job: ClickHouse started right after checkout, `check` before
  the toolchain install; after `cargo test --workspace`: `up --wait
  --wait-timeout 120` + `preflight`, schema via `prices-clickhouse-init
  --rollups`, proxy, `ignored-tests.sh run`, `failure()` log dump, an
  `always()` stop before the Lambda build; `timeout-minutes` on every
  ClickHouse step; `cargo test --workspace` itself runs with
  `CLICKHOUSE_URL=http://127.0.0.1:9`. Filter gains
  `docker-compose.yml` and `scripts/**`. No `continue-on-error`.
- `scripts/ch-proxy-0281.sh`: one Caddyfile with `{$CH_UPSTREAM:localhost:8123}`
  / `{$CH_PROXY_PORT:8124}`, `caddy:2.11.4`, bounded readiness loop that
  needs HTTP 2xx and a body of `1` (a 502 from Caddy is not "up"),
  `caddyfile` subcommand for docker-less runs.
- Docs: IT headers, the 0142 runbook, the prices-clickhouse README, and
  [ci-pipeline](../../3-wiki/project/ci-pipeline.md).

## Issues Encountered

- **The baseline was not 19/19 green.** Four targets were red on a fresh
  server (see the triage criterion); two were isolation, one time rot, one a
  missing proxy. None was a product defect.
- **`rollup_pf_it` will rot around 2027-05-06**: its fixed 2026 buckets leave
  the monthly MV's 400-day window. Green today; a comment at the assertion
  says so. Not changed here.
- **Code review (2026-09-18): 0 critical, 4 warnings, all fixed and proven**
  (commits `c88b2ee`, `eae8b59`, `b9ded2d`) — see decisions 16–18. Of the 7
  info items, IN-01 (a wrong comment about the image pull) and IN-05 (stop
  the containers) were taken; IN-02 (refuse a non-loopback `CLICKHOUSE_URL`),
  IN-03 (five more rules that survive mutation), IN-04 (directory-form
  `tests/foo_it/main.rs` targets), IN-06 (the `last_push_at` shape check now
  accepts fractional seconds) and IN-07 (runbook one-liners, bash 3.2) were
  left as recorded in the review, none reachable today.
- **First CI run green, first try**: Docker Compose v2.38.2 on
  `ubuntu-24.04-arm`; start 10 s, version assertion 1 s, schema 6 s, proxy
  6 s, tests 96 s, stop 4 s — about 2 minutes per Rust PR.
- **A plain YAML `#` in a step name** (`Classify #[ignore]d tests`) would have
  truncated it to `Classify`; quoted.

## Design Decisions

### From Plan

1. **Closed vocabulary (D1)** — three reason prefixes; bare or unknown fails.
2. **One class per target (D2)** — `supply_it` split.
3. **One derived script (D3)** — `ignored-tests.sh`, called identically by CI
   and humans; nothing hand-listed.
4. **The count is asserted (D4)** — passed sum and summary-line count.
5. **Same `rust` job, after `cargo test --workspace` (D5)** — binaries are
   already compiled.
6. **`docker compose up`, not `services:` (D6)** — one pin; `preflight`
   asserts version and UTC.
7. **Caddy for `execution_bound_error_it` (D7)** — armed, not excluded.
8. **No `continue-on-error` (D8)**, not even for one run.
9. **`--test-threads=1` for the whole step (D9)** — Adam, 2026-09-18; ~100 s
   serial vs ~50 s parallel, no per-target exception list.

### Emerged

10. **The induced defect moved from the pivot set to `resolve_reference_ids`.**
    Narrowing `pivot_ids()` reddens two plain unit tests —
    `ch_enrich.rs:4219-4220` (`pivot_ids() == [5, 7]`) and `:3638`
    (`plan_issues_one_peg_and_two_pivots`) — so CI would die at
    `cargo test --workspace` before the IT step ran: a vacuous proof.
    `resolve_reference_ids` only runs against a live client and nothing
    unit-pins it; breaking its USDT arm is the exact 0215 shape (a reference
    that silently fails to resolve). Both halves verified locally.
11. **`plan_issues_one_peg_and_two_pivots` (0215) stays.** It pins which
    statements are issued; the ITs pin the outcome. Complements, not
    duplicates — and decision 10 is itself evidence it earns its place.
12. **`cargo test --workspace --test NAME …`, one invocation.** `-p` and
    `--test` form a cross product in cargo, not pairs, so a `-p/--test` list
    would be a false comfort; `--test seed_it` selects both crates' `seed_it`,
    and the summary-line count proves it. `targets` still prints `-p` per
    target for attribution.
13. **The guard's tests run in the `typescript` job**, not through PR #325's
    infra Nx `test` target, which is not on `develop`. Move them there once it
    lands.
14. **`check` also fails an `#[ignore]` outside `packages/*/tests/*_it.rs`**
    and a CH target sharing its name with any other `_it` target — both would
    otherwise escape the derivation silently.
15. **Only the 0142 runbook changed among the runbooks.** The other
    `docker compose up -d clickhouse` lines (backfill, pool seeding, load test,
    schema quick start) bring ClickHouse up for local work, not for tests, and
    stay correct.
16. **`cargo test --workspace` runs against a closed port** (review WR-02).
    ClickHouse is already up on the tests' default URL by then, so a test
    that lost its `#[ignore]` would pass there and quietly shrink the
    asserted inventory — before 0275 that same mistake turned CI red.
    Proven: an un-ignored `usd_rate_it` fails with `Connection refused`; the
    workspace stays 1121/0.
17. **Every ClickHouse step is time-bounded** (review WR-01): 5 / 10 / 3 / 20
    minutes. The ITs set no request timeout and the proxy allows 7200 s; a
    hung query would otherwise hold the runner for 360 minutes, and a
    cancelled job skips the `failure()` log dump.
18. **The proxy's readiness needs a real answer** (review WR-04): checked
    against a local caddy with a dead upstream — 502, old loop "ready", new
    loop not.

## Future Work

Recorded, not spawned as tasks — both fail loudly in CI when they matter,
which is the point of this task:

- `rollup_pf_it` rots around 2027-05-06 (fixed 2026 buckets vs the monthly
  MV's 400-day window). CI will go red on it then; the assertion carries a
  comment saying why.
- Move the guard's `node:test` run into the infra Nx `test` target once PR
  #325 lands (decision 13).
