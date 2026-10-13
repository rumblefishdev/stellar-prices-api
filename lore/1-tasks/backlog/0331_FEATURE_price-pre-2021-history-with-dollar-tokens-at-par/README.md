---
id: "0331"
title: "No USD price before 2021-02 — price that history by counting every dollar token at par ($1)"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0286", "0201", "0172", "0212", "0267", "0294"]
tags: [layer-backend, priority-high, effort-medium, enrichment, clickhouse, data-correctness]
links:
  - "../../../../docs/runbooks/0286-reingest-history.md"
  - "../../../../packages/prices-ingest-core/src/canonical.rs"
  - "../../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../../docs/scf/milestone-3-evidence.md"
history:
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

# No USD price before 2021-02 — price that history by counting every dollar token at par

## Summary

Every USD value we publish is anchored on canonical USDC (`GA5Z…`), priced with a
measured USDC/USD rate. USDC trades on Stellar from 2021-02-01, so nothing earlier has
a USD reference: candles from 2015-11 to 2021-01 carry volumes and no USD price, on
every tier. This task prices that period by counting dollar tokens at $1 before
2021-02, both where they are the quote and as the source of the XLM/USD rate that
everything quoted in XLM is converted through.

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

## Decision (2026-10-13)

1. Before 2021-02-01, **every dollar token** counts as $1. Not a list of issuers.
2. **On-chain only**, chosen over an external XLM/USD series. The trade-off is accepted:
   a bucket stays unpriced when no dollar-token market traded within the pivot window.
3. **History only.** The change lives in the repricing pass run from fishuser-hero. The
   live path and the USDC-only rule for 2021-02 onward stay as they are.

## Safeguards

Anyone can issue a token called `USD`, so "every dollar token" needs these:

1. The XLM/USD reference is **volume-weighted** across all dollar-token markets in the
   bucket, as the USDC reference already is.
2. A market whose implied XLM/USD deviates from the bucket's median by more than a
   threshold is left out. Threshold to be set from the report.
3. **Pre-run report** (`notes/`): every dollar token traded before 2021-02, with
   volume, implied XLM/USD and deviation.
4. A distinct `method` on these candles, so a client can tell an assumed par from a
   measured rate.
5. A spot-check against Bitstamp XLM/USD where Bitstamp has data.

## Open questions

- Which codes make a dollar token: `USD`, `USDT`, `USDC`, others?
- `method` label: the existing `assumed-par`, or a new one?
- The outlier threshold.
- Coverage ceiling: `canonical.rs` ranks quotes USDC > USDT > XLM, so XLM's own daily
  series before 2021 is the XLM/USDT market. It has candles on 21 days of 2018, 63 of
  2019 and 257 of 2020 (API, 2026-10-13). XLM's own series cannot exceed that.

## Plan

1. Pre-run report → `notes/`.
2. Decide codes, threshold and label; record them here.
3. Implement the par rule for buckets before 2021-02 in the repricing pass, with tests.
4. Dry run, then run over 2015-11 → 2021-01 on all tiers. After 0286 §7b (~2026-10-16):
   one pass at a time on the cluster shared with the block explorer.
5. Spot-check against Bitstamp. Then the M3 reply and evidence.

## Acceptance Criteria

- [ ] Pre-run report in `notes/`, read by Marek or Adam
- [ ] Codes, threshold and label decided and recorded here
- [ ] XLM daily candles before 2021-02 priced wherever a dollar-token market traded
      within the window; the remaining days explained
- [ ] Spot-check against Bitstamp XLM/USD within an agreed tolerance
- [ ] Rows from 2021-02-01 on unchanged
