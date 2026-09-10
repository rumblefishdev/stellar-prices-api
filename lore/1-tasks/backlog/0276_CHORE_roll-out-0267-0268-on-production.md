---
id: "0276"
title: "Roll out 0267 and 0268 on production — load the measured USDC/USD history, deploy the API, re-enrich the USDC-quoted candles"
type: CHORE
status: backlog
related_adr: ["0011"]
related_tasks: ["0267", "0268", "0247", "0265", "0266", "0127", "0128", "0141", "0182", "0228"]
tags: [layer-backend, layer-api, priority-high, effort-medium, milestone-M3, clickhouse, deployment, data-correctness, stablecoin]
milestone: 3
links:
  - "../../../docs/runbooks/load-external-usdc-rate.md"
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: 2026-09-10
    status: backlog
    who: claude
    note: >
      Spawned from [[0267]] and [[0268]]. Their code merged to develop on
      2026-09-10 (PR #293 -> e4065b5, PR #300 -> 8090705); every criterion
      they still leave open closes on the production run, which is this task.
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

## Acceptance Criteria

From [[0267]]:

- [ ] `/ohlcv` for `USDC:GA5Z…KZVN` on 2023-03-11 at `1d` returns
      `close = "0.96812"`, `method = external`, `source = chainlink`,
      `quality = measured`
- [ ] At `1h`, 00:00 / 07:00 / 23:00 read `0.99503491` / `0.8833` / `0.96812`
- [ ] No bucket of that series between 2021-01-25 and 2026-03-10 carries `peg`
- [ ] `usd_rate` holds 44 918 `external` rows (and 44 918 staged)

From [[0268]]:

- [ ] `unexplained_dollar = 0` on `price_ohlcv_1h`, `_4h`, `_1d`
- [ ] `native` on 2023-03-11 reads below 0.99 of its USDC-denominated close on
      `_1h` / `_4h` / `_1d`; `post_run_0268_it` passes with the recorded
      `version_before_1w` / `version_before_1M`
- [ ] `price_usd_series` and the stored `close_usd` agree for `native` in deep
      history
- [ ] Runtime and rows touched per table recorded here, as [[0182]] did
- [ ] [[0266]]'s dislocation table re-measured, result recorded there

Close-out:

- [ ] Release note: `method: peg` -> `assumed-par` on quote legs, and the new
      `external` — a breaking wire change (ADR 0011 deviation accepted
      2026-09-10)
- [ ] USDC re-included in [[0127]] / [[0128]]'s spot-check table with 0.96812
- [ ] [[0247]] closed; [[0267]] and [[0268]] archived with their deferred
      criteria pointing here

## Known caveats

- `measured-disputed` does not survive the hourly pass (runbook §8). Expected.
- `post_run_0268_it` builds its client with no user or password. Against a
  password-protected `default` it fails on authentication, not on data.
- Until the campaign ends, USDC with `base_currency=XLM` on the depeg day is
  ~3 % off. Run the campaign straight after the deploy.
- XLM- and USDT-quoted candles keep the $1 assumption through the pivot tier —
  [[0228]], out of scope here.

## Open

- A read-only rollout checker (verification queries with the expected figures
  in code, baselines written to an artefact directory) was proposed on
  2026-09-10 and not decided.
