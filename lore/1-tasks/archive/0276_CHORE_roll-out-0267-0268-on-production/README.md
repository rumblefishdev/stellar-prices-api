---
id: "0276"
title: "Roll out 0267 and 0268 on production — load the measured USDC/USD history, deploy the API, re-enrich the USDC-quoted candles"
type: CHORE
status: completed
related_adr: ["0011"]
related_tasks: ["0267", "0268", "0247", "0265", "0266", "0127", "0128", "0141", "0182", "0228", "0278", "0279"]
tags: [layer-backend, layer-api, priority-high, effort-medium, milestone-M3, clickhouse, deployment, data-correctness, stablecoin]
milestone: 3
links:
  - "../../../../docs/runbooks/load-external-usdc-rate.md"
  - "../../../../docs/runbooks/repair-coarse-usd-values.md"
  - "notes/R-production-run-2026-09-11.md"
history:
  - date: "2026-09-10"
    status: backlog
    who: akot
    note: >
      Spawned from [[0267]] and [[0268]]. Their code merged to develop on
      2026-09-10 (PR #293 -> e4065b5, PR #300 -> 8090705); every criterion
      they still leave open closes on the production run, which is this task.
  - date: "2026-09-10"
    status: active
    who: akot
    note: "Activated; taken by akot to run the rollout."
  - date: "2026-09-11"
    status: active
    who: akot
    note: >
      Ran on production 07:47-08:37 UTC: schema and views, daily + hourly
      load (1872 / 44918), promote, Compute deploy, re-enrichment of 9.94 M
      USDC-quoted candles in ~24 min; post_run_0268_it 2/2. Host steps over
      mTLS as dev_shared instead of SSH. Converted to a directory; the run
      record is notes/R-production-run-2026-09-11.md. Spawned [[0278]]
      (dust prints) and [[0279]] (release the FREEZE snapshots on 09-18).
  - date: "2026-09-11"
    status: completed
    who: akot
    note: >
      Completed. All 13 criteria ticked (the release note dropped by Adam's
      decision, 0266 re-measured in the note rather than in 0266). Code on
      the branch: post_run_0268_it gets an mTLS client. Runbooks corrected,
      SCF addendum written, 0247 archived, 0267/0268 deferred criteria closed.
      Follow-ups: [[0278]], [[0279]]; the EventBridge stack and 0215's code
      are still undeployed by choice.
---

# Roll out 0267 and 0268 on production

## Summary

The code that serves USDC's measured USD history ([[0267]]) and re-prices the
USDC-quoted candles from it ([[0268]]) is on `develop`. Production has not
changed: `usd_rate` holds no imported row, the API binary is the old one, and
the USDC-quoted candles before 2026-03-11 still carry `close_usd = close × $1`.
This task runs the two runbooks in their fixed order and records what the run
measured.

## Context

- Procedures: `docs/runbooks/load-external-usdc-rate.md` (0267) and Appendix B
  of `docs/runbooks/repair-coarse-usd-values.md` (0268). Both were corrected on
  2026-09-10 (e6e9ec8): `--transport hetzner` on the campaign, a
  `repair_0268_` FREEZE over 202101–202603, and `timezone()` together with
  `serverTimezone()`.
- The rewritten `/ohlcv` query has executed against ClickHouse 26.3.10.60:
  `views_it` 13/13, `ch_enrich_it` 35/35, `ohlcv_it` 35/36 on 833aae7. The one
  miss is `CREATE USER` on a `users_xml`-only sandbox, not the query.
- The order is fixed. Schema and views, daily load, hourly load, promote,
  Lambda build and API deploy, then the campaign. The API must be live before
  the campaign or every re-priced candle reads `oracle`. The hourly file must be
  loaded before `price_ohlcv_1h` is reset, or the reset freezes the day close
  into every hour (the tool refuses it: `ResetRequiresHourlyRates`).

## Implementation Plan

1. **Host — schema and views.** `SELECT timezone(), serverTimezone()` both
   `UTC`. `prices-clickhouse-drift`, `prices-clickhouse-init` as `default` over
   the loopback, `prices-clickhouse-drift` again. The Preconditions checks.
2. **Load, daily then hourly.** The dry-run figures are gates
   (2049 / 1872 / 177, then 49176 / 44918 / 4258). Shadow load, then checks
   4a–4c and 4a′–4c′.
3. **Promote**, then the read-path checks of runbook §6.
4. **Build every Lambda, then deploy.** `make deploy-production-compute`
   packages whatever sits in `target/lambda/` ([[0141]]), so build all assets
   listed by `tools/scripts/lambda-assets.sh` first. `make diff-production`,
   deploy, then runbook §8's curls (they need `x-api-key`).
5. **Campaign.** Appendix B preconditions 0–7, live baseline, FREEZE, per-table
   dry run, real run, after-checks, `post_run_0268_it`.
6. **Close out** — this task's criteria, then [[0267]], [[0268]], [[0247]].

## Status

**Run on 2026-09-11, 07:47–08:37 UTC. Every production step done and verified.**
The full record — commands, figures, timings, checks — is
[notes/R-production-run-2026-09-11.md](notes/R-production-run-2026-09-11.md).
Open after this task: the `repair_0268_` FREEZE snapshots are kept until
2026-09-18 and released by [[0279]]; the dust-print finding is [[0278]].

## Acceptance Criteria

From [[0267]]:

- [x] `/ohlcv` for `USDC:GA5Z…KZVN` on 2023-03-11 at `1d` returns
      `close = "0.96812"`, `method = external`, `source = chainlink`,
      `quality = measured`
- [x] At `1h`, 00:00 / 07:00 / 23:00 read `0.99503491` / `0.8833` / `0.96812`
- [x] No bucket of that series between 2021-01-25 and 2026-03-10 carries `peg`
      (1w walk: 266 `external`, 1 `oracle` — the epoch week, 0 `peg`)
- [x] `usd_rate` holds 44 918 `external` rows (and 44 918 staged)

From [[0268]]:

- [x] `unexplained_dollar = 0` on `price_ohlcv_1h`, `_4h`, `_1d` — **real**
      count 0; the runbook's raw query returns 14 041 / 7 556 / 2 855, every
      one of them `close = 0` dust or below `Decimal(38, 14)` resolution
      (breakdown in the note, query corrected in Appendix B)
- [x] `native` on 2023-03-11 reads below 0.99 of its USDC-denominated close on
      `_1h` / `_4h` / `_1d` (0.8833–0.99503 / 0.8833–0.96812 / 0.96812);
      `post_run_0268_it` passes (2/2) with `version_before_1w = 45340987001`,
      `version_before_1M = 45657106002`
- [x] `price_usd_series` and the stored `close_usd` agree for `native` in deep
      history (within 0.05 % on four sampled days; the view weights every leg)
- [x] Runtime and rows touched per table recorded (the note): 9.94 M rows in
      ~24 min — `_1d` 654 616 in 2 min 07 s, `_4h` 2 701 406 in 6 min 10 s,
      `_1h` 6 420 215 in 12 min 55 s, `_1w` 120 961, `_1M` 38 724
- [x] [[0266]]'s dislocation table re-measured — recorded in the note, not in
      0266 (Adam, 2026-09-11: leave 0266 as it is). 2023-03-11 XLM 0.746 →
      0.723, everything else unchanged: the cause is not the USDC rate but a
      stroop-quantised close, see [[0278]]

Close-out:

- [x] ~~Release note~~ — dropped (Adam, 2026-09-11); the wire change stands as
      recorded in ADR 0011's accepted deviation
- [x] USDC re-included in the spot-check with 0.96812 — as a dated addendum to
      the submitted SCF milestone-2 documents (evidence AC 6, deviations §3),
      not a rewrite of them
- [x] [[0247]] closed; [[0267]] and [[0268]]'s deferred criteria ticked with a
      pointer here

## Design Decisions

### From Plan

1. **Fixed order** schema → daily → hourly → promote → deploy → campaign, as
   the runbooks require.
2. **Only the Compute stack deployed** (`deploy-production-compute`), per
   runbook §7.

### Emerged

3. **Host steps over mTLS instead of SSH.** Adam's new certs map to
   `dev_shared` / `dev_read`, which hold DDL, `ALTER FREEZE` and `SYSTEM` on
   `*.*`. Schema init ran through a one-off runner calling the init binary's
   own library functions (the binary has no mTLS transport); FREEZE ran as SQL
   and was verified from `alter_partition_verbose_result`, not `ls shadow/`.
   Only a snapshot *restore* would still need the host.
4. **0215 left out.** Lambdas built from f385f37, not develop HEAD, so the
   campaign code was exactly what the ITs had run; 0215 ships with its own
   rollout.
5. **EventBridge stack not deployed** although `diff-production` showed its 9
   Lambdas changed: enrichment and coarse-sweep only touch post-epoch data,
   where the oracle prices USDC.
6. **`--start-month 202101`**, not `202001`: no rate exists before 2021-01-25,
   and it keeps every written month inside the FREEZE range.
7. **`--pivot-window-s 604800` / `2678400` on `_1w` / `_1M`** — the tool
   refused the default; the runbook did not mention it.
8. **`post_run_0268_it` given an mTLS client** (`CH_DOMAIN` + `--features
   aws-mtls`), committed on the task branch, so the after-check could reach prod.
9. **Snapshots kept 7 days** (until 2026-09-18) rather than released at close,
   then released partition by partition (`ALTER TABLE … UNFREEZE`) over mTLS — [[0279]].

## Issues Encountered

- **The campaign dry run is not a gate for reset mode.** Its `zeros` column is
  `CANDIDATE_PRED ∪ reset candidates` (72.7 M on `_1h`), so it cannot show the
  reset population. Counted separately with the tool's own predicate; Appendix B
  now carries the query.
- **`unexplained_dollar` over-counts** `close = 0` dust and rows whose repriced
  value truncates back to `close` at 14 decimals. Corrected query in Appendix B.
- **Runbook scale was 15× low** (654 291 "across all granularities" is the
  `_1d` figure alone); the run itself was ~24 minutes, not hours.
- **The unscoped view check hits `dev_read`'s 3.73 GiB memory limit.**
- **The production deploy is blocked for the agent** by the auto-mode
  classifier; Adam ran it with `!`.
- **Found outside scope:** stroop-quantised dust fills and an order key without
  a transaction index set daily closes (XLM 2023-03-11 at 1/17) and spread
  through the pivot tier — [[0278]]. Also: `dev_shared` / `dev_read` hold
  `DROP` on `*.*` on the shared production cluster; worth raising with the team.

## Known caveats

- `measured-disputed` does not survive the hourly pass (runbook §8). Expected.
- ~~`post_run_0268_it` builds its client with no user or password.~~ Resolved:
  with `CH_DOMAIN` set and `--features aws-mtls` it connects over mTLS.
- Until the campaign ends, USDC with `base_currency=XLM` on the depeg day is
  ~3 % off. Run the campaign straight after the deploy.
- XLM- and USDT-quoted candles keep the $1 assumption through the pivot tier —
  [[0228]], out of scope here.

## Open

- A read-only rollout checker (verification queries with the expected figures
  in code, baselines written to an artefact directory) was proposed on
  2026-09-10 and not decided.
