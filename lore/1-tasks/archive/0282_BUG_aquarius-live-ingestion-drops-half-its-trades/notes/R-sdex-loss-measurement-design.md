# R — How to measure SDEX loss, when the evidence self-destructs

**Drafted 2026-09-15.** Design for the outstanding acceptance criterion
*"SDEX's loss is quantified, with an instrument that does not depend on catching
duplicates before the merge."*

## The idea, in one line

Re-derive the truth from the **ledger archive** into a **local** ClickHouse, and
diff it against production.

## Why this instrument, when two others already failed

⛔ **Counting contested buckets in a non-`FINAL` read** — the first attempt, and
the source of the retracted "SDEX is affected, 23 of 595" claim. It fails twice
over: RMT merges collapse the duplicates within seconds so a snapshot sees only
what has not merged yet (two of three samples saw nothing at all), and a **row
count cannot tell a duplicate from a slice** — the contested SDEX rows turned
out to be byte-identical duplicates carrying the same trade, not two halves of a
minute.

⛔ **Comparing against BE** — works for AMM because `default.soroban_events`
holds the raw trades. **BE has no classic-trades table**, so there is no
equivalent for SDEX. This is the whole reason SDEX is still unmeasured while
aquarius is measured to the row.

✅ **Re-derivation.** SDEX trades come from ledger XDR operation results, which
are re-derivable from the public archive — that is exactly how [[0088]] built
3,738,476 pre-Soroban candles. Ground truth is reconstructed independently, so
nothing is racing a merge and duplicates-vs-slices never arises.

## 🔑 It does NOT depend on the live fix — run it in PARALLEL

It measures candles **already written**; nothing is rewriting them. It can start
immediately, alongside the PR #313 work, and it should: **this number gates the
entire repair decision** for 98% of the estate (16.6 M of the 17.0 M live-era
candles are SDEX).

## ✅ The window is uncontaminated — verified 2026-09-15

`prices.backfill_progress`:

| stream | status | newest_data_available |
| --- | --- | --- |
| `sdex_archive` | completed | **2026-07-06 09:35** |
| `soroban_amm` | paused @ 63,352,611 | 2026-07-06 09:35 |

Both end **before** the live era starts (2026-07-16). So every SDEX candle in
the live era was written by the live path alone — there is nothing to
disentangle, and the diff measures the live path and only the live path.
⚠️ Re-check this immediately before the run in case a job is started meanwhile.

## Tooling — all of it already exists

- `sdex-backfill --transport local` syncs **64,000-ledger partitions** from the
  public archive with `--no-sign-request` and indexes into a local Docker
  ClickHouse. Satisfies [[feedback-local-only-no-prod-data]] as written.
- ⚠️ **There is no `--dry-run`** — the tool always writes to a sink. That is
  fine: the local database *is* the measurement artefact.
- Production is read-only via `chq` as `dev_read`
  ([[feedback-user-runs-prod-ch-queries]]).

## Sample windows

The live era (63,494,982 → 64,439,313) spans partitions **992–1006**. Take two,
spread across it:

| partition | ledgers | dates | why |
| --- | --- | --- | --- |
| **993** | 63,552,000–63,615,999 | Jul 19 → Jul 23 | early era, includes a weekend |
| **1000** | 64,000,000–64,063,999 | Aug 17 → Aug 21 | mid era |
| *1004* (optional) | 64,256,000–64,319,999 | Sep 3 → Sep 7 | late era, trend check |

Two partitions ≈ **8 days ≈ 13% of the live era**.

⛔ **Avoid 992** (straddles the era boundary — part of it predates the live path)
and ⛔ **1006** (contains the 2026-09-14 Galexie delivery stall, which would
confound the result with a feed outage).

## Procedure

1. Local ClickHouse pinned to **26.3.10.60** ([[feedback-local-tests-match-prod-version]]); apply the prices schema.
2. `sdex-backfill --transport local --mode combined --start <lo> --end <hi>`.
3. Export local truth grouped by `(minute, base identity, quote identity)`.
4. Export production for the same window in the same shape.
5. Diff.

## 🔑 Confounders — this is what decides whether the number is real

1. 🔴 **Asset surrogate IDs will NOT match.** Local `prices.assets` interns
   assets in its own order, so `asset_id` / `quote_asset_id` differ from
   production for the same asset. **Join on `(asset_code, issuer_address,
   asset_type)`, never on `asset_id`.** This is the easiest way to produce a
   confident, wrong answer.
2. 🔴 **Exclude the enrichment columns.** `close_usd` and `volume_quote_usd` are
   written later by the enrichment worker, not by the ingest path, and will
   differ for reasons unrelated to this defect. Compare **only** `trade_count`,
   `volume_base`, `volume_quote`, `open`, `high`, `low`, `close`.
3. ⚠️ **Pin the code version** to the commit that was live during the window. If
   the extractor changed since, the diff conflates a version difference with the
   defect.
4. ⚠️ **Drop the edge minutes.** The first and last minute of a partition are
   clipped by the range boundary and will always differ.
5. ⚠️ **Re-verify contamination** (above) immediately before running.

## What it produces

- **trades lost** = `sum(truth.trade_count) − sum(prod.trade_count)`, absolute
  and as a percentage
- **damaged bucket count**, and the distribution of the shortfall
- **volume lost**, reported separately — trade counts can agree while volume
  does not
- *optional confirmation leg:* re-replay the same downloaded partition through
  `prices-cli --max-iterations 1`, which reproduces one-ledger-per-run exactly.
  If production matches **that** rather than the truth, the mechanism is
  confirmed for SDEX the same way the last-write-wins ceiling confirmed it for
  aquarius.

## Cost

~1 hour per partition locally, dominated by the 64,000-ledger download. Two
partitions ≈ an afternoon. No AWS spend, no production writes.

## 🔒 Decision rule — write it down BEFORE running

| result | action |
| --- | --- |
| **< 1%** | Record it and do **not** repair SDEX. 944k ledgers of replay is not worth it. |
| **1–5%** | Repair only the busiest pairs, where the damage concentrates. |
| **> 5%** | Full SDEX reprice, scoped as its own task. |

⚠️ **Prior expectation is the low end** — SDEX averages **1.73 trades per
candle** against aquarius's 1.16, and most SDEX pairs trade rarely, so most
buckets should be single-ledger and therefore intact, with damage concentrated
on a handful of busy pairs. **That is a hypothesis, not a result**, and stating
it here is so that a number matching it is not read as confirmation of anything
other than itself.

---

# 📊 RESULTS — partition 993 (2026-07-19 17:01 → 07-23 21:01 UTC)

**Run 2026-09-17.** Local ClickHouse 26.3.10.60 (fresh Docker project
`sdexloss`), `sdex-backfill --transport local --mode combined` from `develop`
at `cc950cc` (extraction code unchanged in substance since 2026-07-14).
Production exported read-only as `dev_read`. Edge minutes dropped as planned.
Pre-run checks: `backfill_progress` newest data 2026-07-06 for both streams
(its `target_ledger` 63,795,749 is only the progress denominator); prod still
holds `_1m` SDEX for the window. Partition 1000 still running at the time of
writing.

## 🔴 SDEX loses ~64% of its trades — worse than Aquarius

| | truth (archive) | production | lost |
| --- | --- | --- | --- |
| trades | **5,528,759** | **1,970,460** | **3,558,299 (64.4%)** |
| candles | 1,459,439 | 1,459,439 | 0 |

**No candle is missing — the candles that exist are undercounted.** Every one
of the 5,639 minutes checked early was short, none over.

## ✅ The instrument is sound — single-trade candles match exactly

Bucket-level, joined on `(minute, asset_type:code:issuer)` both sides:

| truth trades in candle | candles | exact in prod | short | over | trades retained |
| --- | --- | --- | --- | --- | --- |
| 1 | 720,447 | 703,792 | **0** | 0 | 97.7%* |
| 2-3 | 389,985 | 80,954 | 299,289 | 0 | **55.2%** |
| 4-10 | 249,767 | 8,332 | 237,346 | 0 | **30.3%** |
| 11+ | 99,240 | 249 | 98,785 | 0 | **12.0%** |

\* The 16,655 single-trade candles with no prod match **all** touch one of the
3,312 colliding asset ids ([[0139]]), which the prod side excludes — an
exclusion artefact, not loss. Among comparable single-trade candles the match
is **100%**, and **no candle anywhere carries more trades in prod than in
truth**. So both sides count trades identically, and the shortfall is the
0282 mechanism: the busier the candle, the more writes it took, the less
survived — the same shape as Aquarius (41.9% / 15.1%).

## What the damage looks like

Of 1,428,747 comparable candles, **635,420 (44.5%) are damaged**. In those:

| field | still correct |
| --- | --- |
| `close` | **100%** — the last write carries the minute's last trade |
| `high` | 51.4% |
| `low` | 44.9% |
| `open` | **10.5%** |
| `volume_base` | ~⅓ retained (mean 35.5%, median 33.3%) |

Undamaged candles match on all four prices, 100%.

**The loss is spread, not concentrated.** The ten most-damaged pairs carry
only 19% of all lost trades:

| pair | truth | prod | lost |
| --- | --- | --- | --- |
| XLM / USDC (GA5Z…) | 290,132 | 22,766 | **92.2%** |
| GOLD (GBCB…) / XLM | 210,198 | 21,153 | 89.9% |
| SHX (GDST…) / XLM | 135,985 | 13,874 | 89.8% |
| VELO (GDM4…) / XLM | 100,980 | 11,214 | 88.9% |
| GOLD / SHX | 99,391 | 12,193 | 87.7% |

⚠️ **XLM/USDC is the reference market** for USD pricing, so its volume is
understated ~13x in the live era. Its `close` is right, so `close_usd`
derived from it should be unaffected — worth confirming, not assuming.

## Against the decision rule (written 2026-09-15, before the run)

**> 5% → full SDEX reprice, scoped as its own task.** The prior expectation
("the low end") was wrong: most SDEX *candles* are single-trade and intact, but
most SDEX *trades* sit in busy candles, and those lost the most.

⏳ Held until partition 1000 confirms. Note for the scoping: [[0286]] phase 3
already re-ingests the whole chain from the archive, which rebuilds SDEX too —
so the "own task" may be that same run rather than a separate one.

---

# 📊 RESULTS — partition 1000 (2026-08-17 19:41 → 08-21 23:54 UTC) and the verdict

Same method, same code, run finished 13:59 UTC 2026-09-17.

| | truth (archive) | production | lost |
| --- | --- | --- | --- |
| trades | **5,818,584** | **1,850,290** | **3,968,294 (68.2%)** |
| candles | 1,282,341 | 1,282,341 | 0 |

| truth trades in candle | candles | exact | short | over | trades kept | `open` right (damaged) | `close` right (damaged) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 794,704 | 786,962 | 0 | 0 | 99.0%* | — | — |
| 2-3 | 257,888 | 41,734 | 211,734 | 0 | 51.0% | 15.0% | **100%** |
| 4-10 | 141,849 | 5,841 | 129,770 | 0 | 28.3% | 13.7% | **100%** |
| 11+ | 87,900 | 601 | 77,263 | 0 | 12.3% | 3.4% | **100%** |

\* All 28,436 candles without a prod match (every class) touch a colliding
asset id ([[0139]]) — the exclusion artefact again, not loss.

## Both samples together

| sample | truth | prod | lost | truth ÷ prod |
| --- | --- | --- | --- | --- |
| 993 (Jul 19-23) | 5,528,759 | 1,970,460 | 64.4% | 2.81x |
| 1000 (Aug 17-21) | 5,818,584 | 1,850,290 | 68.2% | 3.14x |
| **total** | **11,347,343** | **3,820,750** | **66.3%** | **2.97x** |

Same shape in both, same checks passing in both: no single-trade candle is
short, nothing is over-counted, `close` is always right.

## 🔒 Verdict against the rule written 2026-09-15

**> 5% → full SDEX repair.** Decided 2026-09-17 with the operator: **the repair
is [[0286]] phase 3**, which rebuilds SDEX from the archive in the same run as
everything else. No separate SDEX job.

**Expectation for phase 3's verification** (live-era months, from 2026-07-16):
SDEX `trade_count` rises by roughly **2.8-3.1x**, candle counts unchanged,
`close` unchanged; `open`/`high`/`low`/volume move.

## Kept for re-use

The local Docker project `sdexloss` (volume `sdexloss_clickhouse-data`) holds
the rebuilt truth for both windows, and `.temp/sdex-loss/` the production
exports. Re-running the comparison after phase 3 against the same local truth
is the cheapest verification there is.
