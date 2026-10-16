---
id: "0331"
title: "No USD price before 2021-02 — price that history from an imported hourly XLM/USD rate"
type: FEATURE
status: active
assignee: akot
related_adr: []
related_tasks: ["0286", "0201", "0172", "0212", "0267", "0294"]
tags: [layer-backend, priority-high, effort-medium, enrichment, clickhouse, data-correctness]
links:
  - "../../../../docs/runbooks/0286-reingest-history.md"
  - "../../../../packages/prices-ingest-core/src/canonical.rs"
  - "../../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../../docs/scf/milestone-3-evidence.md"
history:
  - date: "2026-10-16"
    status: active
    who: akot
    note: >
      History run done (Plan step 4). Backup repair_0331_bak_* of
      201802–202101, source = backup on six tiers. coarse-repair
      --pre-usdc-xlm-rate over seven tiers wrote exactly what each dry
      run listed, with reset 0 everywhere (1h 794,672; 1m 4,971,301; 1d
      114,117). No insert into 202102 or later. 97 % of what stays
      unpriced is quoted in other assets with no two-hop path. The
      Bitstamp tolerance is still open.
  - date: "2026-10-15"
    status: active
    who: akot
    note: >
      Decisions amended (5–11): a third-party hourly XLM/USD series replaces
      par and the on-chain reference; dollar-token quotes are converted
      through their own XLM market; the API builds native XLM's pre-2021 USD
      series from the rate at request time. Implemented in PR #403 (PR #401
      closed). Measured read-only on prod before deploy: native XLM 1d
      priced on 1081/1081 days, close vs Binance median 0.43 %; 794,672 of
      814,269 in-scope 1h rows priced, 0 set back to 0.
  - date: "2026-10-14"
    status: active
    who: akot
    note: >
      Reassigned from stkrolikiewicz to Adam, who takes the task over
      (PR #401 included).
  - date: "2026-10-13"
    status: active
    who: stkrolikiewicz
    note: >
      Pre-run report in notes/: 11 dollar tokens traded against XLM before
      2021-02; no usable reference before 2018-08; par agrees with Binance to
      0.3–1.9 % median by year; an off-peg USDT sets the reference on 19 days,
      which the median filter cannot stop. Implementation in draft PR #401
      (coarse-repair --par-*), integration test green on CH 26.3.10.60.
  - date: "2026-10-13"
    status: active
    who: stkrolikiewicz
    note: >
      Activated to implement the pre-2021 par mode in coarse-repair on a
      branch, for Adam to review and run after 0286 §7b (~2026-10-16).
  - date: "2026-10-13"
    status: backlog
    who: stkrolikiewicz
    note: >
      Opened after the M3 reviewer found XLM daily USD prices null over
      2018–2025 and asked for priced candles from 2018 and 2022. 2021-02
      onward is repaired by 0286 §7b-2/§7a. Before 2021-02 no row has a USD
      reference. Decided with Marek: before 2021-02 every dollar token counts
      as $1, on-chain, with the safeguards below. Pre-run report goes to notes/.
---

# No USD price before 2021-02 — price that history from an imported hourly XLM/USD rate

## Summary

Every USD value we publish is anchored on canonical USDC (`GA5Z…`), priced with a
measured USDC/USD rate. USDC trades on Stellar from 2021-02-01, so nothing earlier has
a USD reference: candles from 2015-11 to 2021-01 carry volumes and no USD price, on
every tier. This task prices that period from an imported, measured hourly XLM/USD
rate: candles quoted in XLM directly, candles quoted in a dollar token through that
token's own XLM market, and native XLM's own USD series in the API. The first plan,
dollar tokens at $1 (Decision 2026-10-13, PR #401), was replaced on 2026-10-14
(Decisions 5–11).

## Context

- **The gap.** The 0286 runbook calls these rows `before_reference` ("Nothing can price
  them"). [[0201]] measured the first XLM/USDC candle at 2021-02-01 21:00. Adam puts
  them at ~1.8 % of rows (2026-10-13, not measured here). They were unpriced before the
  0286 re-ingest too.
- **Why there is a USDC-only rule.** Stellar USDT (`GCQTGZ…`) was valued at $1 from
  2018-05 to 2026-08. It lost its peg in June 2022 and is worth ~$0.13 now, so its
  quoted candles came out ~7.4× too high ([[0172]], [[0212]]). `current.sql` and
  `views.sql` since fence the USD reference to USDC by name.
- **The depeg is one dated event.** The pivot prices Stellar USDT at monthly closes of
  0.977–1.005 from 2021-02 to 2022-05, then 0.363 in 2022-06. A par rule that ends at
  2021-02-01 never reaches it.
- **Trigger.** M3 review feedback (via Marek, 2026-10-13): XLM daily USD null over
  2018–2025; asked for priced 2018 and 2022 candles. 2022 is repaired; 2018 is this
  task.

## Decision (amended 2026-10-14 and 2026-10-15)

Decided by Adam. Supersedes Decisions 1, 2 and 4 and Safeguards 1, 2 and 4 below;
Decision 3 is widened by Decision 9.

5. **A third-party hourly XLM/USD series replaces par and the on-chain reference.**
   25,551 hourly rows, 2018-02-16 23:00 → 2021-01-31 23:00 UTC, in `prices.usd_rate`
   as `('native','XLM','','')`, staged `external-candidate` with
   `reference_asset = 'third-party'`. Code reads only `method = 'external'` rows of
   that source, so nothing changes until the rows are promoted. The source is not
   named in project documents. It is not Bitstamp, so Safeguard 5 stays an
   independent check. PR #401 (par) is closed.
6. **Dollar-token quotes are measured, not assumed.** A candle quoted in a dollar
   token (`USD`, `USDT`, `USDC` of any issuer, classic, except canonical USDC) is
   converted through that token's own XLM market within `--pivot-window-s` (24 h),
   then through the rate. A token that lost its peg is priced at what it traded for,
   so the write side has no exclusion list. No market in the window: the row stays
   unpriced.
7. **Wide tiers roll up from 1h.** 4h, 1d, 1w and 1M take the sum of their priced
   hours' USD volume and the last priced hour's `close_usd / close`, as `preroll.sql`
   does. `price_ohlcv_1h` is repaired first.
8. **Label: the existing `external`.** No new word, so Safeguard 4 is dropped. On a
   quote leg the API labels every priced non-USDC row before 2021-02-01 `external`.
   That holds while `coarse-repair --pre-usdc-xlm-rate` is the only writer of such
   rows; on 2026-10-15 production had none from any other source.
9. **Scope: history plus native XLM's series in the API.** In USD mode
   `GET /assets/native/ohlcv` builds pre-2021 buckets from the promoted rate at
   request time: `open`/`close` are the first and last hourly rate, `high`/`low` the
   extremes of the hourly averages, volume is XLM traded against dollar tokens minus
   the four off-peg ones (`OFF_PEG_DOLLAR_TOKENS`), and `method = external`,
   `derived = true`, `source = third-party`. It goes live when the rate is promoted,
   independently of the `coarse-repair` run. This changes the `/ohlcv` contract and
   `openapi.json`. Nothing from 2021-02-01 on changes.
10. **XLM's own pre-2021 candles stay priced** through their XLM/USDT legs, as the
    M3 reviewer asked.
11. **The 1m/15m native series reads its partitions outside the table key**, because
    a bucket appears wherever XLM traded and most XLM markets have XLM as the quote.
    Accepted in review on 2026-10-15: at most 1.15 M rows and 0.53 s per request on
    production, and pre-2021 partitions no longer grow.

## Decision (2026-10-13, superseded in part)

1. Before 2021-02-01, **every dollar token** counts as $1. Not a list of issuers.
2. **On-chain only**, chosen over an external XLM/USD series. The trade-off is accepted:
   a bucket stays unpriced when no dollar-token market traded within the pivot window.
3. **History only.** The change lives in the repricing pass run from fishuser-hero. The
   live path and the USDC-only rule for 2021-02 onward stay as they are.
4. **Amended the same day after the pre-run report (Stanisław):** every dollar token
   except four off-peg ones, excluded by identity (`--par-exclude`, its default):
   USDT `GC2PJ6J3`, USD `GD3LHBHY`, USDT `GC3YRTJQ`, USD `GA5O7NTU`. USDT `GC2PJ6J3`
   holds most of the volume on 19 days and is then the median itself, which no
   threshold can fix. With the four out, 1 day of 2018–2020 is more than 10 % off
   Binance, against ~20 with all of them. Threshold `--par-max-dev 0.25`.
   **Accepted by Marek the same day.**

## Safeguards

Anyone can issue a token called `USD`, so "every dollar token" needs these:

1. ~~The XLM/USD reference is volume-weighted across all dollar-token markets in the
   bucket.~~ Superseded by Decision 5: the rate is imported.
2. ~~A market whose implied XLM/USD deviates from the bucket's median by more than a
   threshold is left out.~~ Superseded by Decision 5.
3. **Pre-run report** (`notes/`): every dollar token traded before 2021-02, with
   volume, implied XLM/USD and deviation. Done.
4. ~~A distinct `method` on these candles.~~ Dropped by Decision 8.
5. A spot-check against Bitstamp XLM/USD where Bitstamp has data.

## Open questions

- ~~Which codes make a dollar token~~: `USD`, `USDT` and `USDC`, classic, any issuer
  (`DOLLAR_TOKEN_PRED` in `prices-clickhouse`).
- ~~`method` label~~: the existing `external`, Decision 8.
- ~~The outlier threshold~~ and ~~per-token exclusion~~: no longer apply (Decision 5).
  The four off-peg tokens are still left out of native XLM's volume (Decision 9).
- ~~Coverage~~: priced history starts where the rate does, 2018-02-16 23:00. Before
  that nothing is priced (task 0333).
- ~~Coverage ceiling~~: native XLM's series is built from the rate (Decision 9), so it
  is no longer bound by the 21, 63 and 257 days of its XLM/USDT candles.
- The tolerance for the Bitstamp spot-check (Safeguard 5), to agree with Marek.

## Plan

1. Pre-run report → `notes/` (done 2026-10-13, data in `data/`).
2. Decide codes, source and label; record them here (done, Decisions 5–11).
3. Implement `coarse-repair --pre-usdc-xlm-rate` and native XLM's series, with tests
   (PR #403).
4. Deploy per PR #403: API, promotion of the rate, then after the 0286 drain and A6 a
   dry run and a real run over 2018-02 → 2021-01, tier by tier (1h first). One pass at
   a time on the cluster shared with the block explorer.
5. Spot-check against Bitstamp. Then the M3 reply and evidence.

Steps 1–4 are done. The API went out and the rate was promoted on 2026-10-15
(version 1791990002, 25,551 `external` rows). The history run was 2026-10-16.

## Rollout of the history run (2026-10-16)

**Order and preconditions.** The 0286 1m drain finished on 2026-10-15 at
14:27 UTC, and 0286's after-checks (A6) passed on 2026-10-16, both recorded
on 0286.

**Backup.** Partitions 201802–202101 of 15m, 1h, 4h, 1d, 1w and 1M were
copied into `prices.repair_0331_bak_<tier>` as `dev_shared`, with
`ATTACH PARTITION … FROM`. Each table holds 36 partitions, with source
equal to backup on every tier: 2,302,390 / 1,043,707 / 454,131 / 143,771 /
36,674 / 13,563 rows. Rollback is
`REPLACE PARTITION <p> FROM prices.repair_0331_bak_<tier>`.

**Run.** `coarse-repair --pre-usdc-xlm-rate --start-month 201802
--end-month 202101`, from Adam's laptop as `prices_writer`, 08:34–~09:05
UTC. Each tier had a dry run first, and a gate stopped the chain on any
`reset` > 0 or any month ≥ 202102. Tier order: 1h → 15m → 1m (`--skip-snapshot`)
→ 4h → 1d → 1w → 1M (`--skip-snapshot --snapshots-verified`).

| Tier | Written | Left unpriced | Reset |
| --- | ---: | ---: | ---: |
| 1h | 794,672 | 3,351 | 0 |
| 15m | 1,655,500 | 8,545 | 0 |
| 1m | 4,971,301 | 25,553 | 0 |
| 4h | 357,495 | 1,284 | 0 |
| 1d | 114,117 | 383 | 0 |
| 1w | 27,770 | 55 | 0 |
| 1M | 9,303 | 16 | 0 |

- Every tier wrote exactly what its dry run listed.
- On 1h, 3,005 of the unpriced rows are 201802 hours before the rate starts
  (task 0333). The other 346 are spread over 2018–2021.
- `system.part_log` shows new parts only in partitions 201802–202101, with
  row counts equal to the table above, and none in 202102–202508.

**What is still unpriced, and why.** Measured on 1d rows with volume and a
price, 2018-01-01 → 2021-01-31, after the run.

| Year | Rows priced |
| --- | ---: |
| 2018 | 74.8 % |
| 2019 | 77.7 % |
| 2020 | 81.9 % |
| 2021-01 | 81.8 % |

Of the 30,054 rows still unpriced:

| Cause | Rows |
| --- | ---: |
| Quoted in an asset other than XLM or a dollar token (330 quotes, 275 bases) | 29,328 |
| XLM-quoted before the rate (task 0333) | 579 |
| Dollar-token quote with no XLM market within 24 h | 147 |

- **No two-hop pricing.** The largest group needs a second step, quote →
  XLM → USD, which the pipeline does not have on any date. The same pairs
  are unpriced after 2021 too.
- **The top quotes have XLM markets.** Those quotes are XCN, HKDT, CMS,
  NGNT, LTC, ETH, BTC, XRP and CNY. Most of them have an XLM market on
  450–1,070 of the 1,080 days.

**XLM-quoted rows, implied XLM/USD.** Median per year 0.229 / 0.083 / 0.073
(2018 / 2019 / 2020). The 2020 minimum is 0.031, in March 2020.

**Bitstamp, after the run.** The OHLC endpoint returned data for one of
five dates tried; the other four returned 404. That date was 2020-11-24,
where XLM/USDT 1d `close_usd` is 0.19589 and Bitstamp's close is 0.19589.
Agreement over the whole range was measured on the rate before deploy (AC
below).

## Acceptance Criteria

- [x] Pre-run report in `notes/`, read by Marek or Adam
- [x] Codes, source and label decided and recorded here (Decisions 5–11)
- [x] Native XLM's daily USD candles priced on every day the rate covers
      (2018-02-16 → 2021-01-31). Candles quoted in XLM priced wherever the rate
      covers them; candles quoted in a dollar token also need the token's XLM market
      within 24 h. The remaining rows explained. Measured on prod before deploy
      (2026-10-15): 1081 of 1081 days; 794,672 of 814,269 in-scope 1h rows, the rest
      before the rate (task 0333) or 358 dollar-quoted rows with no token market
      ✅ 2026-10-16. The native series has been live since 2026-10-15 on 1081 of
      1081 days. The history run priced 794,672 1h rows, as measured. The rows
      left are explained in "Rollout of the history run".
- [ ] Spot-check against Bitstamp XLM/USD within an agreed tolerance (measured before
      deploy: median 0.30 %, p95 1.76 %, without 2020-05-28 → 07-15 where Bitstamp
      printed a flat stale price). The tolerance is still to agree with Marek.
- [x] Rows from 2021-02-01 on unchanged. ✅ 2026-10-16: the run listed no month
      ≥ 202102, and `system.part_log` shows no insert into 202102–202508.
