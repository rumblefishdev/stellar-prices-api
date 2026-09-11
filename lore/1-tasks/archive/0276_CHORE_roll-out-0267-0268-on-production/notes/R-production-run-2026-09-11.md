---
title: "The 0267 + 0268 production run of 2026-09-11 — every command, figure and check"
type: research
status: mature
tags: [production, runbook, usd-rate, re-enrichment, measurements]
links:
  - "../../../../../docs/runbooks/load-external-usdc-rate.md"
  - "../../../../../docs/runbooks/repair-coarse-usd-values.md"
  - "../../../../../docs/ohlcv-outlier-prints-analysis.md"
history:
  - date: "2026-09-11"
    status: mature
    who: akot
    note: "Recorded during and right after the run."
    spawned_from: ["../README.md"]
    spawns: []
---

# The 0267 + 0268 production run of 2026-09-11

All times UTC. Cluster `ch.sorobanscan.rumblefish.dev`, ClickHouse 26.3.10.60,
`timezone()` / `serverTimezone()` = `UTC` / `UTC`. Lambdas built from f385f37
(develop at 8090705 plus lore commits); 0215 (#305) deliberately not included.

## Identities used

| Step | Identity | How |
|---|---|---|
| schema, views, FREEZE | `dev_shared` | Adam's `~/.certs/adamkot-write` over mTLS (grants: CREATE/DROP/ALTER/SYSTEM on `*.*`) |
| loads, promote, campaign | `prices_writer` | `~/prices-mtls/` over mTLS |
| every check | `dev_read` | `~/.certs/adamkot-read` over mTLS |
| deploy | AWS SSO Admin, 750702271865 | run by Adam (`! make -C infra deploy-production-compute`) — the auto-mode classifier blocks production deploys from the agent |

No SSH, no `default` password.

## Step 0 — read-only prep

- Load dry runs matched the gates exactly: daily 2049 / 1872 / 177 (fallback 24,
  disputed 4, measured 1844, 2023-03-11 = 0.96812); hourly 49176 / 44918 / 4258
  (0.99503491 / 0.8833 / 0.96812; span 1611532800 .. 1773234000).
- Schema delta init.sql vs prod: one missing column (`usd_rate.quality`). Views
  compared semantically (`formatQuerySingleLine`): prod carried the 4fcdcb6
  definitions; HEAD differed only in `price_usd_series` and `_1h`.
- Appendix B preconditions: oracle rows before epoch 0; cleanup rule
  `DISABLED`; `COARSE_SWEEP_LOOKBACK_MONTHS=2`; USDC `asset_id = 3`.
- `make diff-production`: code-only changes in Compute (api-handler,
  ledger-processor) and EventBridge (9 Lambdas), one SSM description in Secrets;
  no removals, no IAM. Only Compute was deployed.
- API before: USDC 2023-03-11 `1d` = `"1"` / `peg`; `1h` 00/07/23 all `1` / `peg`.

## Step 1 — schema and views (07:47:52, 2 s)

One-off runner calling `apply_init_sql`, `apply_seed`, `apply_sql(VIEWS_SQL)`
through `mtls::client_with_mtls_from_paths` — the init binary's exact sequence,
over mTLS. After: `has_quality = 1`; both views `has_external = 1`
(`ddl_len` 2895 / 2899); all six views equal HEAD; drift 0 / 6 MVs.

The runbook's unscoped `count() FROM price_usd_series WHERE method='external'`
exceeds `dev_read`'s 3.73 GiB query limit; scoped to canonical USDC it returns
0 (all `peg`) as expected.

## Step 2 — loads

| Pass | Start | Duration | Statements | Version | Checks |
|---|---|---|---|---|---|
| daily | 07:49:51 | 1 s | 8 | 1789112991 | 4a 1872, 2021-01-25 → 2026-03-11 00:00, below epoch; 4b not_midnight 0; 4c 0.96812 chainlink measured |
| hourly | 07:50:13 | 17 s | 180 | 1789113013 | 4a′ 44918, last 2026-03-11 13:00; 4b′ not_full_hour 0, midnights 1872; 4c′ 0.8833 / 0.96812 |

All 44 918 staged rows carry the hourly version. The four `measured-disputed`
days before they were overwritten: 2022-11-09 (1.00090629), 2022-11-23
(0.99987218), 2023-01-19 (1.00000848), 2023-03-13 (0.99887418).

## Step 3 — promote (07:52:20)

Version 1789113140. 44 918 `external` + 44 918 `external-candidate`.
§6: daily 0.96812 `external`; hourly 0.99503491 / 0.8833 / 0.96812 `external`;
the 2026-03-11 1d bucket reads `oracle` (0.99992768714102).

## Step 4 — deploy (07:54:38 → 07:54:54)

`Prices-production-Compute` UPDATE_COMPLETE in 22 s, stage cache flushed.
§8 and precondition 6:

| Check | Result |
|---|---|
| USDC 1d 2023-03-11 | `0.96812` external chainlink measured |
| USDC 1h 00 / 07 / 23 | `0.99503491` / `0.8833` / `0.96812`, external |
| fallback day 2021-02-01 | `0.99998` external bitstamp fallback |
| poll day 2026-06-01 | `oracle`, `source` and `quality` `null` |
| `native` 2023-03-11 1d and all 24 hours | `assumed-par`, never `peg` |
| USDC 1w 2021-01-25 → 2026-03-09 | 266 external, 1 oracle, 0 peg |

api-handler and ledger-processor: 0 errors since the deploy; DLQ empty.

## Step 5 — campaign

Baseline at 07:58:39: `version_before_1w = 45340987001`,
`version_before_1M = 45657106002`; `native`/USDC implied rate on 2023-03-11 =
1.0 on `_1h` / `_4h` / `_1d`.

FREEZE 07:59:58 → 08:00:45 as `dev_shared`: 315 partitions (5 tables ×
202101–202603), 807 parts, every partition's frozen-part count equal to its
active parts (from `alter_partition_verbose_result=1`), none pre-existing.

Reset candidates, counted with the tool's own `reset_pending_pred` (the dry run
cannot show them — its "zeros" column is the union with ordinary zeros):

| Table | Candidates | Start | End | Duration | rows_reset | rows_enriched | gap (close = 0 dust) |
|---|---|---|---|---|---|---|---|
| `_1d` | 654 616 | 08:00:53 | 08:03:00 | 2 min 07 s | 654 616 | 654 466 | 150 |
| `_4h` | 2 701 406 | 08:12:33 | 08:18:43 | 6 min 10 s | 2 701 406 | 2 701 107 | 299 |
| `_1h` | 6 420 215 | 08:18:43 | 08:31:38 | 12 min 55 s | 6 420 215 | 6 419 770 | 445 |
| `_1w` | 120 961 | 08:34:00 | 08:35:30 | 1 min 30 s | 120 961 | 120 886 | 75 |
| `_1M` | 38 724 | 08:35:30 | 08:36:53 | 1 min 23 s | 38 724 | 38 684 | 40 |

9.94 M rows in about 24 minutes. Flags: `--start-month 202101`,
`--reset-quote-asset-id 3 --reset-not-before 0 --reset-require-external-rate
--skip-snapshot --snapshots-verified`; `_1w` / `_1M` `--end-month 202602` and
`--pivot-window-s 604800` / `2678400` (the tool refuses a window shorter than
the bucket; the first `_1w` attempt at 08:31:38 exited before connecting).

The gap is `close = 0` dust: such a row matches `close_usd = close` at 0 = 0,
is re-opened, gets its `volume_quote_usd` recomputed, and keeps `close_usd = 0`,
so it stays a "candidate". No row with `close > 0` was left at zero on any table.

### After

| Check | `_1h` | `_4h` | `_1d` |
|---|---|---|---|
| `unexplained_dollar`, runbook form | 14 041 | 7 556 | 2 855 |
| — of which `close = 0` dust | 433 | 289 | 146 |
| — tier expression `CAST(r.usd * p.close AS Decimal(38, 14))` equals `close` | 13 458 | 7 186 | 2 674 |
| — tier expression truncates to 0 (`close = 1e-14`) | 150 | 81 | 35 |
| **— real** | **0** | **0** | **0** |
| `native`/USDC implied rate 2023-03-11 | 0.8833 – 0.99503 | 0.8833 – 0.96812 | 0.96812 |

`post_run_0268_it` (over mTLS, with the recorded versions): 2 passed.
`_1w` / `_1M`: no `close > 0` row at zero. `price_usd_series` vs stored
`close_usd` for `native`, four deep-history days: within 0.05 % (the view
weights every quote leg). Lambdas still 0 errors after the campaign.

## Spot check against independent sources

USDC matches task 0265's composed CSV to the digit on every sampled pre-epoch
day; 2026-08-01 (post-epoch) reads 1.00040 `oracle` against the CSV's
independent 0.99981. XLM within ±0.5 % of Bitstamp / Binance on 7 of 8 sampled
days. The exception, 2023-03-11 (−28 %), is a stroop-quantised pool fill (1/17)
as the day's close — present before this run and documented in
`docs/ohlcv-outlier-prints-analysis.md`; decision in 0278.

## 0266's table, re-measured (API ÷ Binance)

| Date | XLM before → after | yBTC before → after |
|---|---|---|
| 2023-03-11 | 0.746 → 0.723 | 0.744 → 0.744 |
| 2023-03-12 | 1.017 → 1.008 | 1.008 → 1.008 |
| 2023-03-15 | 0.838 → 0.838 | 0.852 → 0.852 |
| 2023-03-18 | 1.002 → 1.002 | 0.434 → 0.434 |

The dislocation is not the USDC rate, so this run could not remove it: the
shared factor is the XLM/USDC daily close (1/17 on 03-11, 4/57 on 03-15),
which is both XLM's price and the pivot reference for XLM-quoted yBTC. On 03-11
XLM moved 3 % further down because the measured USDC rate was correctly applied
to that close. See 0278.
