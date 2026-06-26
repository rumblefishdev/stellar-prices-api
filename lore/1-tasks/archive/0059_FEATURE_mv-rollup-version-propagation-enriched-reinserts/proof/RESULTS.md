# Proof results — 0059 rollup-propagation

Executed 2026-06-09 against a throwaway `clickhouse/clickhouse-server:24.8`
(reported version `24.8.14.39`) via `./run.sh`. Reproduce with `./run.sh`.

## Findings (all empirically observed)

| # | Finding | Evidence |
|---|---------|----------|
| 1 | **The draft §3.2 MV does not compile.** `sum(volume_base) AS volume_base` shadows the column; the `vwap` line then does `sum(volume_base)` again → the inner ref resolves to the alias → aggregate-in-aggregate. | `Code: 184 ILLEGAL_AGGREGATION: Aggregate function sum(volume_quote_usd) AS volume_quote_usd is found inside another aggregate function`. Bites again on any re-aggregation query copied from the draft. Fix: `vwap = volume_quote_usd / nullIf(volume_base, 0)` (reference the aliases, don't re-sum). |
| 2 | **Draft under-counts to 1/15 on the live path — no enrichment needed.** 15 per-minute rows inserted as 15 separate blocks → 15 partial `_15m` rows, same sort key → `ReplacingMergeTree` keeps the `max(version)` one (minute 14). | `_1m FINAL` truth `volume_base=150`; `_15m_draft FINAL` `volume_base=10, trade_count=1`. |
| 3 | **Enrichment does not propagate through the draft MV.** Re-inserting minute `00:06` with `version=8` (orig 7 + 1) correctly wins in `_1m`, but its `_15m` partial carries `max(version)=8 < 15`, so it loses to the existing partial. | `_1m FINAL` bucket `volume_quote_usd=500`; `_15m_draft FINAL` stays `volume_quote_usd=0`. |
| 4 | **Re-aggregate from `_1m FINAL` is correct and enriched.** One full-bucket recompute over the deduplicated source. | `open=100, high=115, low=100, close=114.5, volume_base=150, volume_quote=15000, volume_quote_usd=500, vwap=3.333…, trade_count=15`. |
| 5 | **`max(version)` is an insufficient version projection for a ReplacingMergeTree rollup target.** Enriching an *early* minute (7→8) leaves the bucket `max(version)=15` unchanged, so the stale and corrected rollup rows tie on version; dedup then relies on fragile insertion-order tie-breaking. | `max_version` stayed `15` pre/post enrichment; `sum_version` went `120 → 121`. |

## Captured run (abridged)

```
### clickhouse 24.8.14.39
### _1m FINAL (truth):            volume_base=150  volume_quote_usd=0   trade_count=15
### _15m_draft FINAL (draft MV):  volume_base=10   volume_quote_usd=0   trade_count=1   version=15   <- under-count
### re-aggregate from _1m FINAL:  open=100 high=115 low=100 close=114.5
                                  volume_base=150 volume_quote=15000 volume_quote_usd=500
                                  vwap=3.333… trade_count=15 max_version=15 sum_version=121          <- correct
### _15m_draft FINAL after enrich: volume_base=10 volume_quote_usd=0 version=15                       <- no propagation
```

## What this means for the recommendation

- The draft insert-trigger MV is wrong on **three** independent counts (won't
  compile, under-counts, doesn't propagate) → **0051 must not ship it**.
- **Re-aggregate-from-`_1m FINAL` is correct by construction.** Prefer the
  **true Refreshable MV** (atomic target replace — sidesteps finding #5
  entirely). If a scheduled `INSERT … SELECT … FROM _1m FINAL` into a
  `ReplacingMergeTree` is used instead (Option A′), it **must** project a
  strictly-increasing version (e.g. `sum(version)` or a refresh epoch), because
  `max(version)` ties pre/post early-minute enrichment.

## Scope / caveats

- CH `24.8.14` only; a single 2-hop-capable series. The mechanism is
  version-general but the exact behaviour should be re-confirmed on the BE
  Hetzner cluster's CH version.
- A true `REFRESH`-clause Refreshable MV was not exercised (it needs the
  experimental flag on this version); the deterministic `INSERT … SELECT …
  FROM _1m FINAL` stands in for one refresh tick and is what proves finding #4.
- Chain depth > 2 (`_15m → _1h → …`) not yet exercised — each hop reads the
  previous grain `FINAL`, same pattern.
