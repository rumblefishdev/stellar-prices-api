---
title: "The Soroswap gap is the 0096 bug alone — the 07-11 resumption is a replay cursor position, not a second mechanism"
type: research
status: mature
spawned_from: notes/R-soroswap-five-day-gap-measured-on-prod.md
spawns: []
tags: ["soroswap", "amm", "ingestion", "data-correctness", "measurement", "prod", "proto27"]
links:
  - "../README.md"
  - "R-soroswap-five-day-gap-measured-on-prod.md"
  - "../../../../../packages/soroswap-extractor/src/lib.rs"
history:
  - date: "2026-09-14"
    status: mature
    who: okarcz
    note: >
      Settles the 07-11 vs 07-15 contradiction the task opened with, and the
      trading-lull question, both from production reads as dev_read. The
      resolution is that neither date is a bug boundary: 07-15 15:57Z is when
      the 0096 extractor fix deployed, and 07-11 21:00 is where the post-proto27
      catch-up replay happened to be standing at that moment. One mechanism, and
      the dark range is 5.4 days rather than 9.
---

# The Soroswap gap is the 0096 bug alone

## Conclusion

Three things are settled, all from production measurement:

1. **Not a trading lull.** Soroswap swaps exist in `default.soroban_events` on
   every day of the hole while candles are zero.
2. **One mechanism, not two.** The 0096 `topic[0]` bug accounts for the whole
   gap. [[amm-live-pool-registry-preload-gap]] is **not** implicated.
3. **The dark range is `2026-07-06 09:35` → `2026-07-11 21:00`**, about
   5.4 days — not the 9 days the task premise assumed. The refill is shorter.

The 07-11 date that looked like a contradiction is not a bug boundary at all.
It is **where the post-proto27 catch-up replay was standing when the extractor
fix deployed.**

## Evidence

### 1. The venue was trading throughout

Swap events for our 221 registered Soroswap pools, per day, against candles in
`price_ohlcv_1d`:

| day | raw swaps | soroswap candles |
| --- | --- | --- |
| 2026-07-06 | 426 | 6 (backfill output, last at ledger 63,352,574) |
| 2026-07-07 | 387 | **0** |
| 2026-07-08 | 336 | **0** |
| 2026-07-09 | 3,919 | **0** |
| 2026-07-10 | 268 | **0** |
| 2026-07-11 | 442 | 10 |
| 2026-07-12 | 260 | 22 |

Query shape (the `contract_id IN (…)` subquery is what keeps it off a
full-table scan — `soroban_events` is `ORDER BY (contract_id, ledger_sequence, …)`):

```sql
SELECT toDate(l.closed_at) AS d,
       countIf(JSONExtractString(e.topics_xdr, 2, 'value') = 'swap') AS swaps
FROM default.soroban_events e
INNER JOIN (
  SELECT sequence, min(closed_at) AS closed_at
  FROM default.ledgers WHERE sequence BETWEEN 63330000 AND 63600000 GROUP BY sequence
) l ON l.sequence = e.ledger_sequence
WHERE e.ledger_sequence BETWEEN 63330000 AND 63600000
  AND e.contract_id IN (
    SELECT id FROM default.soroban_contracts FINAL
    WHERE contract_id IN (SELECT contract_id FROM prices.pool_registry FINAL WHERE venue = 'soroswap')
  )
GROUP BY d ORDER BY d
```

**Input present, output absent → ingestion dropped them.** The lull explanation
is dead.

### 2. The envelope never changes, so the extractor cannot explain a 07-11 recovery

`topic[0]` across the whole window, 07-04 → 07-16, every swap, every day:

```
string  SoroswapPair
```

No shape change at 07-11. A binary that reads the action from `topic[0]` would
have dropped 07-11 → 07-14 exactly as it dropped 07-07 → 07-10. Yet those
candles exist. So **the 07-11 → 07-14 candles were not written by the live path
at the time those ledgers closed.**

### 3. Nothing was deployed on 07-11

`git log` over the window has no commit between **07-08 15:48** (`2ff832b`) and
**07-14 17:06** (`e17ed03`, the xdr-27 bump). Whatever changed on 07-11 21:00
was not ours and was not a release.

### 4. The two deploys that bracket it

| moment | event |
| --- | --- |
| 2026-07-08 12:31 | live frontier freezes at ledger ~63,384,067 — proto27 ([[proto27-xdr26-live-freeze]]) |
| 2026-07-14 ~21:00 | xdr-27 build deployed; processor begins catching up |
| **2026-07-15 15:57Z** | **0096 Soroswap extractor fix deployed** (PR #112, `2c53ee4`) |

From `price_ohlcv_1h`, the first Soroswap candle after the gap is
**2026-07-11 21:00**, ledger **63,434,026**.

The arithmetic closes it. The replay ran 63,384,068 → ~63,434,000 in the ~19 h
between the xdr-27 deploy and the extractor deploy: **~2,630 ledgers/hour, about
3.6× real time** — the shape of a catch-up, not of live tailing. From 15:57Z
onward every ledger the replay touched went through the *fixed* extractor, so
Soroswap candles start at whatever ledger the replay had reached. That ledger is
63,434,026, and it closed on 07-11 21:00.

🔑 **63,433,850 is a cursor position, not a market event and not a second bug.**
[[0271]] read it as evidence of a distinct mechanism because it predates the
known fix; it postdates nothing — it is downstream of it.

## What this changes for the reprice

- **Soroswap refill range: `63,352,612` → `63,434,025`**, i.e. up to the first
  ledger the replay repriced correctly. Repricing past that rewrites rows the
  fixed extractor already wrote correctly, for no gain.
  ⚠️ The upper bound still has to be **minute-aligned** per the task's
  §Cross-invocation minute boundary — 63,434,026 closed mid-minute, so the run
  bound must be snapped to the minute edge, not set to this ledger.
- **Phoenix is untouched by any of this.** Its range is still
  `[63352612, deploy_ledger]` with `deploy_ledger` at 2026-07-17 11:57:52, and
  it still needs DELETE-first in `1m` and coarse for the version-tie reason.
- **No live-path fix is owed before refilling.** The mechanism was 0096 and it
  is fixed and deployed. There is no recurrence-at-next-restart hazard, which is
  what the registry hypothesis would have implied.

## Why Phoenix and Aquarius were unaffected

Nothing exotic: their extractors read the action from the topic slot their
venues actually use. Soroswap is the only venue whose envelope puts a
`String` constant in `topic[0]` and the action in `topic[1]`. The same property
is why `signature` is `NULL` on essentially every Soroswap event — BE derives
`signature` from `topic[0]` and only when it is a `Symbol` (see
[[soroban-events-gotchas]] #3). A venue that is invisible to `signature` is
worth treating as a permanent special case, not a one-off.

## Two corrections to carry forward

### ⚠️ `intDiv(version, 1000)` is only a ledger in single-member buckets

The task calls this a "method worth reusing". It holds in `price_ohlcv_1m`,
where `version = ledger_seq * 1000 + op_index`. It does **not** hold in the
coarse tables: the rollup MVs aggregate with `sum(version)`
([[rollup-mvs-replace-mode-wipe]]), so a bucket built from many minutes carries
a sum, not a ledger. It is visible in the same query that produced the table
above — `aquarius` on 2026-07-10 reports a "ledger" of **380,487,849**, which is
no ledger that has ever existed.

It gave a true answer for the July Soroswap boundaries only because those daily
buckets happen to have one contributing candle each. **Sanity-check the
magnitude against the real ledger height before believing any value it returns.**

### ⚠️ `prices.unresolved_pools` cannot witness a live drop

Every row in it has `source = 'backfill'` (138 pools, 7,887 swaps, `last_ledger`
topping out at 63,352,576 — the handoff floor). The live path never writes
there. So the table that exists precisely to record "a swap we could not
classify" is blind to the process that produced this gap. Worth a backlog task
before the sweep is claimed to be trustworthy: a sweep over candle absence finds
holes, but nothing yet records the drops themselves.
