---
id: "0219"
title: "The peg statement writes exactly 1,236 rows on every batch — the same rows, re-selected and re-versioned forever"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0215", "0111", "0212", "0182"]
tags: ["priority-medium", "effort-small", "enrichment", "clickhouse", "data-correctness"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Spawned from 0215's post-fix verification. Pre-existing and unrelated to
      the Caddy fix — the same constant appears in the 7-day baseline taken
      before it. Recorded as an observation with a falsifiable next step, not a
      diagnosis.
---

# The peg statement's written_rows is a constant, and constants are suspicious

## Summary

`peg_sql` writes **exactly 1,236 rows on every single batch**. Eight consecutive
batches on 2026-08-21 (15:19:31 → 15:35:09), all `1236`. The pre-fix baseline
shows the same figure averaged: 54,414 written over 44 runs = **1,236.7**.

Genuinely new candidates would vary batch to batch — the XLM pivot's `10000` is
constant for a known reason (it is `LIMIT`-bound by `batch_size`), but 1,236 is
not any configured limit.

**The likely reading:** the same ~1,236 rows are re-selected and re-written every
batch, each write incrementing `version` on a `ReplacingMergeTree` without
changing the outcome. If so it is pure waste — write amplification and merge
pressure on a shared cluster — and it means the peg leg has been making **no
forward progress at all**, which the run counts alone cannot show.

## Why it plausibly loops

`enrich_batch`-family statements use the widened candidate filter
(`volume_quote_usd = 0 OR close_usd = 0`) with `volume_quote_usd` written
**once** (`if(volume_quote_usd > 0, …)`, `ch_enrich.rs:789`). A row that ends a
pass with `close_usd` set but `volume_quote_usd` still `0` therefore stays
eligible forever: it matches the candidate filter on the `volume_quote_usd = 0`
arm, gets re-written, and its `volume_quote_usd` is deliberately not touched.

⚠️ **This is a hypothesis with an obvious alternative** — 1,236 could be a real
steady-state arrival rate of newly-ingested peg-quoted candles. Do not act on the
loop reading until the identity check below is run.

## First step — cheap and decisive

Compare the row *identities* across two consecutive batches. If they are the same
rows, it is a loop; if disjoint, it is arrival rate:

```sql
SELECT timestamp, asset_id, quote_asset_id, source, version
FROM prices.price_ohlcv_1m FINAL
WHERE quote_asset_id IN (3) AND close_usd > 0
ORDER BY version DESC
LIMIT 20;
```

A cluster of rows carrying a `version` far above the ingest baseline is the
signature. `run_peg_pivot_tier`'s no-progress guard
(`ch_enrich.rs:942`) only breaks when `count_candidates` stops falling overall —
the XLM pivot's 10,000/batch keeps it falling, so a stalled peg leg is masked.

## Acceptance Criteria

- [ ] Established by measurement whether the 1,236 rows are the same rows each
      batch or newly-arrived ones.
- [ ] If a loop: the rows are made ineligible once enriched, or excluded from the
      candidate filter, and `written_rows` is shown to vary afterwards.
- [ ] `version` inflation on the affected rows is quantified before/after.
- [ ] A per-leg no-progress signal exists, so one stalled leg is not masked by
      another leg's progress.

## Out of scope

- The `volume_quote_usd` write-once semantics themselves, which are deliberate
  and depeg-aware ([[0182]]) — this task asks whether the *candidate filter*
  should still re-admit those rows, not whether the column should be rewritten.
