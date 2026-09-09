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

Task 0265 composed a USDC/USD series from two independent outside feeds and
versioned it in this repo at **two grains**:

```
lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1d.csv
lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1h.csv
```

2049 daily rows and 49 176 hourly ones, both 2021-01-25 → 2026-09-04, both with
the same ten columns
`ts,open,high,low,close,n_obs,source,quality,xcheck_spread_bps,xcheck_sources`;
`ts` is the UTC bucket **start** (day start / full hour);
`source ∈ {chainlink, bitstamp}`;
`quality ∈ {measured, measured-disputed, fallback}` (`measured-disputed` is a
daily cross-check verdict and does not occur at hourly grain).

**Both files are loaded, DAILY FIRST and HOURLY SECOND** (Adam, 2026-09-09), and
the order is not cosmetic:

- They collide at **every midnight**: `prices.usd_rate` keys on
  (identity, `timestamp`, `method`) and both grains write the same `method`, so
  a day's 00:00 row is ONE ReplacingMergeTree key. The values differ — the daily
  row carries the **day's** close, the hourly row the **00:00 hour's** — at
  1 980 of the 2 049 shared midnights. The higher `version` wins, i.e. whichever
  grain was loaded last.
- Hourly must win, because `price_usd_series_1h` and `/ohlcv` at `1h` resolve at
  the hour and the 00:00 bucket must carry the 00:00 hour's number.
- The daily surface does not move when it does: `price_usd_series` argMaxes over
  the whole day and lands on the 23:00 row, whose close **is** the daily close
  for all 2049 days (pinned by `enrichment-worker/tests/composed_usdc_csv.rs`).

> **The loader now enforces this order.** Before any write, a shadow load at
> `--grain daily` counts staged rows away from UTC midnight — rows only the
> hourly pass can produce — and refuses if it finds any
> (`LoadError::DailyAfterHourly`). Until this gate existed, the wrong order was
> refused by nothing: the rows are valid, the counts match, only the values are
> wrong, and `price_usd_series_1h` would publish the day close for the 00:00
> hour of all 1 872 covered days. To deliberately re-seed the daily file after
> an hourly load, pass `--allow-daily-after-hourly` **and re-run the hourly
> pass afterwards**, or the midnights stay wrong. The gate reads the table, so
> `--dry-run` (which contacts no server) cannot report it — it fires at the
> start of the real load, before the first INSERT.

Loading the daily file alone is a valid, smaller deliverable — `/ohlcv` treats a
lone daily row as valid for its whole UTC day — but it leaves
`price_usd_series_1h` publishing the import at 00:00 and `1`/`peg` for the other
twenty-three hours of every covered day, and it cannot show the depeg **trough**
(2023-03-11 07:00, `0.8833`) at `granularity=1h` at all.

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

## Preconditions

Four things, and the second is the one a first-time operator hits: **every
INSERT the tool renders names the `quality` column**, so on a cluster that has
not yet had this task's `init.sql` applied, step 3 fails on its first chunk
with `No such column quality` — loudly, with nothing half-written, but with no
hint of why. Check before you build.

0. **The server is in UTC.** Run `SELECT timezone()` and stop unless it says
   `UTC`. Every day and hour boundary in this repo — the candle tables' unzoned
   `toStartOfInterval`, the views' day buckets, the loader's and the tiers'
   ASOF floors — is computed in the SERVER's timezone, so on a non-UTC server
   imported rows land on the wrong day and every gate below misreads. The
   loader refuses to write on its own if this is not `UTC`
   (`ServerNotUtc`), but check first: the refusal comes after the dry run.

1. **The schema and the views from this branch are applied to the cluster.**
   The real tool is `prices-clickhouse-init` — there is no `apply-schema`
   binary. Read the caveat in
   [`0142-rollup-mv-reapply.md`](0142-rollup-mv-reapply.md) ("Re-creating by
   re-applying the file") first, because the binary does **not** apply one
   statement: it unconditionally re-lands **all of `init.sql`** (every
   `CREATE TABLE IF NOT EXISTS` and every `ALTER TABLE … ADD COLUMN IF NOT
EXISTS`, including the one this task adds to `prices.usd_rate`), **seeds
   `backfill_progress`** (idempotent, `NOT IN`-guarded), and **`CREATE OR
REPLACE`s all six views in `views.sql` from your working tree** — which
   needs the `DROP VIEW` grant, so it runs as the container's `default` user on
   the host over the loopback port, never as `prices_writer`/`prices_reader`
   and never from a laptop (that runbook's "Where these commands run").

   Run it from a checkout of **this branch**, so the two widened
   `price_usd_series` grains land with the column:

   ```bash
   # On the Hetzner host; build for the host target and copy up, or build there.
   read -rs CH_PW
   CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_USER=default \
   CLICKHOUSE_PASSWORD="$CH_PW" ./prices-clickhouse-init
   ```

   Applying the widened views **before** the load is harmless and deliberate:
   they read `method = 'external'`, which holds no rows until step 5's promote,
   so until then they publish exactly what they publish today. The promote is
   the single switch.

   Confirm the column landed — this is the check step 3 depends on:

   ```sql
   SELECT count() AS has_quality
   FROM system.columns
   WHERE database = 'prices' AND table = 'usd_rate' AND name = 'quality'
   ```

   Expect `1`. And confirm both view grains carry the widened predicate.

   ⚠️ **Do not substring-match the formatted predicate.** ClickHouse does not
   store the text you submitted: `create_table_query` is re-serialised from the
   AST, and `packages/prices-clickhouse/src/drift.rs` exists precisely because
   of it — _"`INTERVAL 15 MINUTE` becomes `toIntervalMinute(15)`, whitespace
   collapses, identifiers may gain backticks … a naive text compare would
   report drift permanently, which is worse than no check at all."_ Whether the
   printer reproduces `method IN ('oracle', 'external')` byte-for-byte on
   26.3.10.60 is unverified, and `create_table_query` is **empty** for a
   session user lacking `SHOW COLUMNS` on the object (`drift.rs`), which reads
   as "the views did not land" on a cluster that is fine. An earlier draft of
   this runbook made exactly that mistake.

   Two checks instead. The first lists the views and flags a **bare token** the
   AST printer cannot reshape, and it returns a row per view whether or not the
   flag is set, so an empty `create_table_query` shows up as `has_external = 0`
   next to a `ddl_len` of 0 rather than as a missing view:

   ```sql
   SELECT name,
          position(create_table_query, '''external''') > 0 AS has_external,
          length(create_table_query)                     AS ddl_len
   FROM system.tables
   WHERE database = 'prices'
     AND name IN ('price_usd_series', 'price_usd_series_1h')
   ORDER BY name
   ```

   Expect **two rows**, `has_external = 1` on both. `ddl_len = 0` means the
   session user cannot read the DDL — re-run as the container's `default`
   user; it does **not** mean the views are wrong.

   The second cannot lie at all, because it EXECUTES the predicate rather than
   reading its text. It costs nothing and returns 0 until step 5's promote,
   which is the correct answer before then:

   ```sql
   SELECT count() FROM prices.price_usd_series WHERE method = 'external'
   ```

   A view without the widening **fails to parse this** — `method` is projected
   by the widened definition only — so an error here is the unambiguous
   "the old view is still installed". Expect `0` now, and a non-zero count when
   you run it again at step 6. Then `prices-clickhouse-drift` exits 0.

2. **The four mTLS variables and `CH_DATABASE`** are exported (section 1
   below lists them). The tool refuses `--transport hetzner` without
   `CH_DOMAIN`.

3. **`SET param_epoch`** is in your `clickhouse-client` session (section 0
   above). Every verification query below reads `{epoch:UInt32}`; a session
   without it fails on the first one rather than silently checking the wrong
   window.

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

## 2. Dry run (DAILY) — and these figures are a GATE

Sections 2–4 are the **daily** pass. Section 4½ repeats them for the hourly
file; section 5 promotes both at once.

```bash
DATA=lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data
CSV=$DATA/composed_usdc_usd_1d.csv
CSV_1H=$DATA/composed_usdc_usd_1h.csv

cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --dry-run --grain daily "$CSV"
```

⚠️ `--grain` defaults to `daily` and the default is **not** symmetric: every
midnight is also a full hour, so the daily file parses at `--grain hourly` too
and would be loaded as 2049 isolated hours. Name the flag beside the file, every
time, as the commands here do.

It must print exactly:

| Figure                      | Value                             |
| --------------------------- | --------------------------------- |
| grain                       | **daily**                         |
| rows parsed                 | **2049**                          |
| loadable (below epoch)      | **1872**                          |
| skipped (at/above epoch)    | **177**                           |
| quality `fallback`          | **24**                            |
| quality `measured-disputed` | **4**                             |
| quality `measured`          | **1844**                          |
| 2023-03-11 00:00 close      | **0.96812** (chainlink, measured) |

> ⚠️ **A figure that does not match is a STOP, not a note.** These numbers are
> measured against the versioned file and pinned by a CI test
> (`enrichment-worker/tests/composed_usdc_csv.rs`). If the tool and this table
> disagree, either the file is not the artefact this procedure was written for or
> the tool's partition rule has moved — and in both cases what gets loaded is
> unknown. Do not "check the diff and continue".

A dry run writes nothing and opens no write path. It is safe to repeat.

---

## 3. Shadow load (DAILY)

```bash
cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --grain daily "$CSV"
```

`--shadow` is ON by default. The rows land under `method = 'external-candidate'`
and **nothing reads that word** — not the views, not the API, and not task
0268's re-enrichment. Staged rows are inert by construction rather than by
anyone's discipline, which is what makes step 4 a real gate.

The tool logs one `version` (the run's unix time) and stamps every row of the
run with it, so a later run is distinguishable from this one.

---

## 4. The three verification queries (DAILY)

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
SELECT countIf(timestamp != toStartOfDay(timestamp, 'UTC')) AS not_midnight,
       count() AS rows
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate'
```

Expect `not_midnight = 0`, `rows = 1872`. Anything else — stop and do not
promote.

> Two things this expression has to get right, and an earlier draft got one of
> each wrong. **Not `toTime()`**: that function anchors the time-of-day to
> **1970-01-02**, not 01-01, so an expectation written against it halts a
> correct load. And **`toStartOfDay` must name its zone**: `timestamp` is a bare
> `DateTime` and nothing in this repo pins the server's timezone
> (`docker-compose.yml` sets no `TZ`; `ch-prod-01`'s zone is undocumented), so
> an unzoned `toStartOfDay` resolves locally and on a UTC+2 server this gate
> reads `not_midnight = 1872` on a perfectly correct load. `'UTC'` is not
> optional here.

⚠️ **After the hourly pass (section 4½) this query no longer applies as
written** — `external-candidate` then holds 44 918 rows at every full hour, of
which 1 872 are midnights. Run 4b before the hourly load, or use the hourly
form given in 4½.

**4c. The depeg day reads back correctly.** The falsifier for the whole task.

```sql
SELECT toString(usd_rate) AS rate, reference_asset AS source, quality
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate'
  AND timestamp = toDateTime('2023-03-11 00:00:00')
```

Expect `rate = 0.96812`, `source = chainlink`, `quality = measured`. The
column is `Decimal(38, 14)` and ClickHouse prints a Decimal with its trailing
zeros **trimmed**, so `0.96812` — not `0.96812000000000` — is the printed form
of the stored value; compare numerically if in doubt. `0.9681` as quoted in the
task text is a rounding, not the stored value, and does not match either.

---

## 4½. The hourly grain — repeat 2–4 for the second file

Same tool, same refusals, same epoch partition, same staging word. Only
`--grain` and the file change. **Do this after the daily pass has passed 4a–4c,
never before it** — the two grains share every midnight key and the last write
wins (section 0).

**Dry run.** These figures are a GATE in exactly the same sense as section 2's,
and are pinned by the same CI test:

```bash
cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --dry-run --grain hourly "$CSV_1H"
```

| Figure                   | Value                                |
| ------------------------ | ------------------------------------ |
| grain                    | **hourly**                           |
| rows parsed              | **49176**                            |
| loadable (below epoch)   | **44918**                            |
| skipped (at/above epoch) | **4258**                             |
| quality `fallback`       | **597**                              |
| quality `measured`       | **44321**                            |
| source `bitstamp`        | **597**                              |
| source `chainlink`       | **44321**                            |
| loadable span (unix)     | **1611532800 .. 1773234000**         |
| 2023-03-11 00:00 close   | **0.99503491** (chainlink, measured) |
| 2023-03-11 07:00 close   | **0.8833** (chainlink, measured)     |
| 2023-03-11 23:00 close   | **0.96812** (chainlink, measured)    |

Two of those deserve a sentence:

- **No `measured-disputed` line.** That verdict is the composer's DAILY
  cross-check between feeds; it has no hourly analogue, so the four disputed
  days appear here as plain `measured`. Its absence is expected, not a
  truncated file.
- **`2023-03-11 00:00` is 0.99503491, not 0.96812.** That is the shared-midnight
  collision, visible: the daily file's 00:00 row carries the day's close, this
  one carries the 00:00 hour's. The hourly value is the right one for an hourly
  bucket, and loading hourly second is what makes it win.

**Shadow load.** 44 918 rows in chunks of 250 — about 180 requests against a
shared cluster, a few minutes:

```bash
cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --grain hourly "$CSV_1H"
```

**Verify.** The hourly forms of 4a–4c. Note `not_full_hour`, not
`not_midnight` — and `'UTC'` for the same reason as before:

```sql
-- 4a′ span and count
SELECT count() AS rows, min(timestamp) AS first, max(timestamp) AS last,
       max(timestamp) < toDateTime({epoch:UInt32}) AS below_epoch
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate';

-- 4b′ every stamp is a full hour UTC, and the midnights are still all there
SELECT countIf(timestamp != toStartOfHour(timestamp, 'UTC'))  AS not_full_hour,
       countIf(timestamp  = toStartOfDay(timestamp, 'UTC'))   AS midnights,
       count()                                                AS rows
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate';

-- 4c′ the trough hour, which a daily-only load cannot show
SELECT toString(timestamp) AS hour, toString(usd_rate) AS rate,
       reference_asset AS source, quality
FROM prices.usd_rate FINAL
WHERE asset_kind = 'credit' AND asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND contract_address = '' AND method = 'external-candidate'
  AND timestamp IN (toDateTime(1678518000), toDateTime(1678575600))
ORDER BY timestamp
```

Expect: `rows = 44918`, `first = 2021-01-25 00:00:00`, `last = 2026-03-11
13:00:00`, `below_epoch = 1`; `not_full_hour = 0`, `midnights = 1872`; and two
rows reading `0.8833` (07:00) and `0.96812` (23:00), both
`chainlink`/`measured`.

⚠️ `rows = 44918`, **not** 46 790 — the staged set is a UNION, not a sum: the
hourly file's 1 872 midnight rows replaced the daily file's at the shared key.
A count of 46 790 would mean the two grains are writing different `method`
values and the collision is not happening, which breaks everything section 0
says.

---

## 5. Promote

One promote covers both grains: it selects every staged row of this identity
below the epoch, whatever grain wrote it. Run it once, after **both** passes.

```bash
cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
  --transport hetzner --promote "$CSV_1H"
```

> ⚠️ **The promote ADDS a key; it does not move one.** `method` is part of
> `usd_rate`'s `ORDER BY`, so the `external-candidate` rows and the `external`
> rows are **different** ReplacingMergeTree keys and **both survive**. Nothing is
> deleted. The staged rows stay behind, still inert because no read predicate
> names them, and their storage cost is negligible (44 918 narrow rows). Do not
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

Expect **44 918 of each** after both passes (1 872 of each if you loaded the
daily file only). Fewer `external` than `external-candidate` means the promote
did not finish.

---

## 6. Confirm the read path sees the rows

The schema and the views were applied in **Preconditions** (there is no
separate apply step and no `apply-schema` binary); the promote is what made
the rows visible to them. Confirm on the data, not the DDL:

```sql
SELECT toString(close_usd) AS close, method
FROM prices.price_usd_series
WHERE asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND bucket = toDateTime('2023-03-11 00:00:00')
```

Expect `0.96812`, `external`. `1`, `peg` means the promote has not run (step 5) or the views on the cluster predate this branch (Preconditions).

And the hourly grain, which is what section 4½ bought — three different numbers
inside one day, where a daily-only load publishes one:

```sql
SELECT toString(bucket) AS hour, toString(close_usd) AS close, method
FROM prices.price_usd_series_1h
WHERE asset_code = 'USDC'
  AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND bucket IN (toDateTime('2023-03-11 00:00:00'),
                 toDateTime('2023-03-11 07:00:00'),
                 toDateTime('2023-03-11 23:00:00'))
ORDER BY bucket
```

Expect `0.99503491`, `0.8833`, `0.96812`, all `external`. A single value
repeated three times means only the daily file was loaded; `1`/`peg` on the
07:00 and 23:00 hours means the same. (Buckets appear only where a candle
exists for that hour, so an empty result is a gap in `price_ohlcv_1h`, not a
missing rate.)

Both `price_usd_series` grains and `queries_ch::ohlcv_peg_series` read
`oracle` and `external` rows, and where one bucket holds both, `oracle` wins
**across the whole bucket** by an explicit rank — never by timestamp. That is
not a hypothetical: the composed series' last loadable row is `2026-03-11
00:00`, the epoch is `14:00` the same day, so the **1d bucket of 2026-03-11
holds both** and reads `oracle`.

---

## 7. Deploy the API

`Candle` gained two nullable fields (`source`, `quality`) and the response schema
grew with them. Deploy `prices-api` after the Preconditions: the new binary
reads `usd_rate.quality` directly (`queries_ch::ohlcv_peg_series` queries the
**table**, not the views), so a new binary against a cluster without the
column errors on every canonical-USDC `/ohlcv` request. The old binary against
the new column is harmless — it simply does not select it.

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
  "close": "0.96812",
  "method": "external",
  "source": "chainlink",
  "quality": "measured"
}
```

(Trailing zeros trimmed, as in step 4c.) `"close": "1"` with `"method": "peg"`
means the read path is not seeing the rows: re-check step 5's counts (a shadow
load that was never promoted looks exactly like this) and then the
Preconditions.

**The hourly falsifier**, which the daily grain cannot produce:

```bash
curl -s "https://<api-host>/v1/assets/USDC:GA5Z…/ohlcv?granularity=1h&start=2023-03-11T00:00:00Z&end=2023-03-11T23:00:00Z&base_currency=USD" \
  | jq '.data[] | select(.timestamp | test("T(00|07|23):")) | {timestamp, close, method}'
```

Expect `0.99503491` at 00:00, **`0.8833` at 07:00** and `0.96812` at 23:00, all
`external`. The trough is the hour the peg actually broke, and a daily-only
load answers `0.96812` for all three.

**Grains.** `/ohlcv` floors an imported row at the start of its UTC day, so a
LONE daily row serves every bucket of that day at every grain — matching task
0268's external tier, which prices every candle of an imported day from the same
row. `price_usd_series_1h` has no such net: it buckets an imported row exactly
like a poll. With **both** files loaded the two surfaces resolve the same row per
bucket and agree everywhere, which is the state this procedure leaves the
cluster in. From a daily-only load they disagree on 23 of every 24 hours — that
is the reason section 4½ is part of the procedure and not an optional extra.

**Provenance.** `source` and `quality` are `null` — not `""` — on every bucket
whose rate came from a poll or from the peg. An empty string there is a
defect, not a value.

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
