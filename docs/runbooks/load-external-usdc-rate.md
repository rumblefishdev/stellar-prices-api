# Load the composed USDC/USD history into `prices.usd_rate`

Task 0267. Operator procedure for `load-external-rate`: dry run → shadow load →
three verification queries → promote → deploy → check.

## Why this is its own file and not an appendix in `repair-coarse-usd-values.md`

Both reasons are mechanical. Neither is a matter of taste, and neither should be
"tidied" away by folding this back into the campaign runbook.

1. **That file's epoch literal is counted by a unit test.**
   `the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant`
   (`enrichment-worker/src/ch_enrich.rs`) asserts the oracle epoch appears
   **exactly once** in `repair-coarse-usd-values.md` and that **no second
   ten-digit number starting `177` appears anywhere in it**. Any new section
   there that mentions the epoch turns a green CI red — or, worse, pressures
   someone into removing the epoch from a section that needs it.
2. **"Appendix B" is referenced by name from eight source and test locations**
   (`coarse-repair.rs`, `ch_enrich.rs` ×4, `post_run_0268_it.rs`,
   `ch_enrich_it.rs`, `queries_ch.rs`, `prices-clickhouse/src/lib.rs`).
   Inserting a section before it and renumbering would silently invalidate every
   one of those pointers — they are prose references, so nothing would fail.

This file has the same epoch discipline applied to itself: the literal is typed
**once**, below, and
`the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant` in
`enrichment-worker/src/external_rate.rs` pins that one copy to
`prices_clickhouse::USDC_ORACLE_EPOCH_S`.

---

## 0. What this loads, and why

`prices.usd_rate` has never held a USDC/USD rate below **2026-03-11 14:00 UTC**,
the first instant our own Reflector polling produced one. Everything earlier
therefore fell back to the $1 peg — including **2023-03-11, the day USDC
depegged**, which the API published as a literal `1.0` labelled `peg`.

Task 0265 composed a daily USDC/USD series from two independent outside feeds
and versioned it in this repo:

```
lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1d.csv
```

2049 daily rows, 2021-01-25 → 2026-09-04. Columns
`ts,open,high,low,close,n_obs,source,quality,xcheck_spread_bps,xcheck_sources`;
`ts` is the UTC day **start**; `source ∈ {chainlink, bitstamp}`;
`quality ∈ {measured, measured-disputed, fallback}`.

This procedure loads it as `method = 'external'` rows — measured evidence of the
same standing as a poll, kept a distinct word because task 0247 forbids
publishing an import as `oracle`.

**Identity.** One asset only: `USDC` issued by
`GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN` (Circle's own
issuance). Asset codes are not unique on Stellar — hundreds of issuers publish a
"USDC" — so the tool refuses every other code and issuer in code, not in prose
(the task 0173 gate).

**The epoch, typed once.** Rows at or above
`prices_clickhouse::USDC_ORACLE_EPOCH_S` are **not loaded**: our own polling is
primary from there, and the composed series merely runs past it. The tool
partitions the file rather than refusing it, and reports the skipped count.

```sql
-- prices_clickhouse::USDC_ORACLE_EPOCH_S, 2026-03-11 14:00 UTC.
-- The only hand-typed copy in this runbook; every query below reads the parameter.
SET param_epoch = 1773237600
```

(`clickhouse-client` keeps it for the session; over HTTP pass `?param_epoch=` on
each request.) The tool logs the same value in its plan output — if the two
differ, stop.

---

## 1. Build the tool

`load-external-rate` is behind `required-features = ["aws-mtls"]`, so a plain
`cargo build` does **not** produce it and CI never compiles it.

```bash
cargo build -p enrichment-worker --features aws-mtls --bin load-external-rate
```

mTLS environment, the same four variables `coarse-repair` and `sdex-backfill`
use:

```bash
export CH_DOMAIN=...          # the Caddy host fronting Hetzner ClickHouse
export MTLS_CERT_PATH=...
export MTLS_KEY_PATH=...
export MTLS_CA_PATH=...
export CH_DATABASE=prices
```

---

## 2. Dry run — and these figures are a GATE

```bash
CSV=lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1d.csv

cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --dry-run "$CSV"
```

It must print exactly:

| Figure                      | Value                             |
| --------------------------- | --------------------------------- |
| rows parsed                 | **2049**                          |
| loadable (below epoch)      | **1872**                          |
| skipped (at/above epoch)    | **177**                           |
| quality `fallback`          | **24**                            |
| quality `measured-disputed` | **4**                             |
| quality `measured`          | **1844**                          |
| 2023-03-11 close            | **0.96812** (chainlink, measured) |

> ⚠️ **A figure that does not match is a STOP, not a note.** These numbers are
> measured against the versioned file and pinned by a CI test
> (`enrichment-worker/tests/composed_usdc_csv.rs`). If the tool and this table
> disagree, either the file is not the artefact this procedure was written for or
> the tool's partition rule has moved — and in both cases what gets loaded is
> unknown. Do not "check the diff and continue".

A dry run writes nothing and opens no write path. It is safe to repeat.

---

## 3. Shadow load

```bash
cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner "$CSV"
```

`--shadow` is ON by default. The rows land under `method = 'external-candidate'`
and **nothing reads that word** — not the views, not the API, and not task
0268's re-enrichment. Staged rows are inert by construction rather than by
anyone's discipline, which is what makes step 4 a real gate.

The tool logs one `version` (the run's unix time) and stamps every row of the
run with it, so a later run is distinguishable from this one.

---

## 4. The three verification queries

Run all three. Each fails differently, and none is implied by the others.

**4a. Row count and time span.** The staged set must be the whole loadable
partition and must stop below the epoch.

```sql
SELECT count() AS rows,
       min(timestamp) AS first,
       max(timestamp) AS last,
       max(timestamp) < toDateTime({epoch:UInt32}) AS below_epoch
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate'
```

Expect `rows = 1872`, `first = 2021-01-25 00:00:00`, `last = 2026-03-11
00:00:00`, `below_epoch = 1`.

**4b. Every stamp is midnight UTC.** This is the same check task 0268's
Appendix B precondition 2 runs, and for the same reason: that tier resolves a
candle's rate at the **bucket END** with a strict `rts < bend`, so a row stamped
anywhere later in the day resolves every bucket to the **previous day's** rate —
an off-by-one-day error that produces entirely plausible numbers and fails
nowhere.

```sql
SELECT DISTINCT toString(toTime(timestamp)) AS time_of_day
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate'
```

Expect exactly one row: `1970-01-01 00:00:00` (midnight). Anything else — stop
and do not promote.

**4c. The depeg day reads back correctly.** The falsifier for the whole task.

```sql
SELECT toString(usd_rate) AS rate, reference_asset AS source, quality
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate'
  AND timestamp = toDateTime('2023-03-11 00:00:00')
```

Expect `rate = 0.96812000000000`, `source = chainlink`, `quality = measured`.
The column is `Decimal(38, 14)`, so the full fourteen-place form is correct —
`0.9681` as quoted in the task text is a rounding, not the stored value.

---

## 5. Promote

```bash
cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --promote "$CSV"
```

> ⚠️ **The promote ADDS a key; it does not move one.** `method` is part of
> `usd_rate`'s `ORDER BY`, so the `external-candidate` rows and the `external`
> rows are **different** ReplacingMergeTree keys and **both survive**. Nothing is
> deleted. The staged rows stay behind, still inert because no read predicate
> names them, and their storage cost is negligible (1872 narrow rows). Do not
> "clean them up" — leaving them is what makes the rollback in step 9 free.

Re-running the promote is idempotent only because RMT dedups the identical
promoted key on the higher version. Re-run it if a run is interrupted.

Confirm:

```sql
SELECT method, count() AS rows
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method IN ('external', 'external-candidate')
GROUP BY method
```

Expect **1872 of each**. Fewer `external` than `external-candidate` means the
promote did not finish.

---

## 6. Apply the schema and the views

The rows are useless until the read path can see them.

```bash
# `quality` column on prices.usd_rate (idempotent ALTER), then the widened views.
cargo run -p prices-clickhouse --bin apply-schema   # or your usual schema-apply path
```

Both `price_usd_series` grains and `queries_ch::ohlcv_peg_series` now read
`method IN ('oracle', 'external')`. Where a bucket somehow held both, `oracle`
wins by an explicit rank — not by timestamp.

---

## 7. Deploy the API

`Candle` gained two nullable fields (`source`, `quality`) and the response schema
grew with them. Deploy `prices-api` after the views are applied: an old binary
against new views is harmless (it simply does not select the two columns), but a
new binary against old views cannot find `quality` and will error.

---

## 8. The check that closes the task

```bash
curl -s "https://<api-host>/v1/assets/USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN/ohlcv?granularity=1d&start=2023-03-11T00:00:00Z&end=2023-03-11T00:00:00Z&base_currency=USD" \
  | jq '.data[0] | {timestamp, close, method, source, quality}'
```

Expect:

```json
{
  "timestamp": "2023-03-11T00:00:00Z",
  "close": "0.96812000000000",
  "method": "external",
  "source": "chainlink",
  "quality": "measured"
}
```

`"close": "1.00000000000000"` with `"method": "peg"` means the read path is not
seeing the rows: re-check step 5's counts (a shadow load that was never
promoted looks exactly like this) and then that the views were applied.

Then spot-check a `fallback` and a `measured-disputed` day and confirm `quality`
reports them — those 28 days are the whole reason the column exists.

---

## 9. Rollback

**Nothing is deleted, so there is nothing to restore.**

Revert the read path's preference to oracle-only — the `WHERE method` predicate
in `views.sql` (both grains) and in `queries_ch::ohlcv_peg_series` — and re-apply
the views. The API immediately returns to today's behaviour: the $1 peg,
labelled `peg`, for every pre-epoch bucket. Every loaded row stays on disk,
unread.

That is the entire reason the promote is additive rather than a rewrite: the
rollback is a read-path revert, not a data repair. If instead the rows themselves
are wrong (a bad CSV got past step 2), the promoted rows can be superseded by a
corrected load at a higher `version` — still without a delete.

---

## What this procedure deliberately does NOT do

- **It does not re-enrich candles.** Every USDC-quoted candle's `close_usd` is
  task 0268's campaign — `docs/runbooks/repair-coarse-usd-values.md`
  **Appendix B**, whose precondition 1 counts exactly the rows step 5 produces.
  Run this first; that second.
- **It does not append ongoing rounds.** Task 0267 repairs HISTORY only. Our own
  polling is primary from the epoch on, and no ongoing import is built, planned,
  or filed as future work.
