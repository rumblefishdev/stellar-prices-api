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
