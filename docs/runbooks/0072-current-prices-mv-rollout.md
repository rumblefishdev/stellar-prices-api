# Runbook — rolling out the task-0072 `current_prices` columns

Ships the four previously-DEFAULT columns (`sources`, `price_xlm`,
`change_24h_pct`, `change_7d_pct`) plus the §5.5 median-outlier filter on
`vwap_24h` to production.

The code merged in **PR #150** (2026-07-24) and was deliberately **not deployed**:
the PR deferred the prod apply and the refresh-cost measurement until task
**0114**'s coarse repair had finished, so ~30M re-inserted rows of merge pressure
could not be mistaken for MV cost. 0114 is complete and archived, so that hold is
released.

Three things land, in this order:

| #   | Change                                                          | Where             |
| --- | --------------------------------------------------------------- | ----------------- |
| 1   | `mv_current_prices` DROP + re-CREATE (writes 10 columns, not 6) | ch-prod-01        |
| 2   | `current_price_usd` view forwards the new columns               | ch-prod-01        |
| 3   | `api-handler` Lambda serves them instead of stubs               | AWS Compute stack |

**Ordering is deliberate.** Steps 1–2 are safe in either order relative to step 3
— the columns already exist on the `current_prices` _table_ (at their DEFAULTs),
so neither side errors on the other's absence. Doing the data first means the
API's first post-deploy response already carries real values rather than a
window of zeros.

## Preconditions

- [ ] `develop` contains PR #150 (`47ad4e1`) **and** the `current_price_usd`
      forwarding commit. Deploying the API from a tree without both re-stubs the
      response.
- [ ] Task 0114's coarse repair is finished (it is — archived `ee97db2`).
- [ ] Know whether the **cleanup rule** is currently disabled for the 0088
      backfill. It does not interact with this MV (which reads `price_ohlcv_1m`
      and `_1h`), but `change_7d_pct` reads `_1h` _because_ `_1m`'s 7-day
      retention floor is unreliable while cleanup is off — see the note in
      `current.sql`.
- [ ] The 0088 backfill's write load is understood; step 1's probe measures the
      MV against whatever the cluster is doing at the time.

## Step 0 — capture the rollback artifact (MANDATORY)

A refreshable MV's definition is fixed at create time, so this is a `DROP VIEW` +
re-`CREATE`. **Once the DROP runs, the old definition exists nowhere but here.**

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query \
   \"SHOW CREATE TABLE prices.mv_current_prices FORMAT TSVRaw\"" \
  > /tmp/0072-rollback-mv_current_prices.sql

wc -l /tmp/0072-rollback-mv_current_prices.sql   # must be non-empty
```

Do the same for the view, whose `CREATE OR REPLACE` overwrites in place:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query \
   \"SHOW CREATE TABLE prices.current_price_usd FORMAT TSVRaw\"" \
  > /tmp/0072-rollback-current_price_usd.sql
```

### Make the artifacts replay-able (do it now, not at incident time)

`SHOW CREATE` emits a bare `CREATE …` statement. **Neither file can be fed back
as captured** — replaying either while the object exists fails with
`Code: 57 TABLE_ALREADY_EXISTS`. Fix both here, while nothing is on fire:

```bash
# View: SHOW CREATE emits `CREATE VIEW …`; it must REPLACE, since the rollback
# target already exists (the OR REPLACE in step 4 overwrote it in place).
sed -i '1s/^CREATE VIEW /CREATE OR REPLACE VIEW /' \
  /tmp/0072-rollback-current_price_usd.sql

# MV: a refreshable MV has no OR REPLACE form, so the DROP must precede it —
# the same reason current.sql itself is DROP + re-CREATE.
sed -i '1i DROP VIEW IF EXISTS prices.mv_current_prices;' \
  /tmp/0072-rollback-mv_current_prices.sql
```

Verify both before proceeding — this is the whole point of the step:

```bash
head -1 /tmp/0072-rollback-current_price_usd.sql   # CREATE OR REPLACE VIEW …
head -1 /tmp/0072-rollback-mv_current_prices.sql   # DROP VIEW IF EXISTS …
grep -c "arrayReduce('median'" /tmp/0072-rollback-mv_current_prices.sql  # expect 0
```

That last check is the sanity gate: **0** means the captured MV is the v1
definition, which is what a rollback needs. A non-zero count means the MV on
prod is already the 0072 one and this rollback artifact would restore nothing —
stop and re-plan before anything mutates.

## Step 1 — cost probe (READ-ONLY, run before anything mutates)

The new SELECT does materially more work per refresh than the v1 one: a
two-level aggregation, an array pipeline for the median filter, a JSON build, and
an extra `price_ohlcv_1h FINAL` scan over 7 days for `change_7d_pct`. It runs
**every 60 seconds** on the shared cluster, so measure before committing.

`FORMAT Null` computes everything and discards the output, so this touches no
data:

```bash
# Derived from current.sql itself — never hand-copy the SELECT, it will drift.
{ sed -n '/^WITH/,$p' packages/prices-clickhouse/schema/current.sql \
    | sed 's/;[[:space:]]*$//'; echo 'FORMAT Null'; } > /tmp/mv_probe.sql

ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery \
     --time --memory-usage' < /tmp/mv_probe.sql
```

Then read the real numbers back out of the query log:

```sql
SELECT
    query_duration_ms,
    formatReadableSize(memory_usage)        AS mem,
    formatReadableQuantity(read_rows)       AS rows_read,
    formatReadableSize(read_bytes)          AS bytes_read
FROM system.query_log
WHERE type = 'QueryFinish'
  AND query LIKE '%arrayReduce(\'median\'%'
  AND event_time > now() - INTERVAL 15 MINUTE
ORDER BY event_time DESC
LIMIT 3;
```

**Accept / abort:**

- `query_duration_ms` **< ~15s** → comfortable inside the 60s refresh. Proceed.
- **15–40s** → proceed, but treat the refresh cadence as a follow-up: a refresh
  that overruns its interval is skipped, not queued, so the column just goes
  stale rather than piling up. Record the number.
- **> 40s**, or memory in the GB range → **stop**. Do not apply. The fix is to
  widen the refresh interval (`REFRESH EVERY 5 MINUTE`) or to split the JSON
  build into a companion MV — both are `current.sql` edits, and both want their
  own task rather than an improvised change during a rollout.

Compare against the v1 MV's own cost for context:

```sql
SELECT view, last_success_duration_ms, read_rows, written_rows
FROM system.view_refreshes
WHERE view = 'mv_current_prices';
```

## Step 2 — apply the MV

Applies `DROP VIEW` + re-`CREATE` as one file. Order inside the file is
load-bearing (a CREATE-then-DROP would leave no view at all); a unit test pins it.

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' \
  < packages/prices-clickhouse/schema/current.sql
```

**Exposure during the gap is staleness, not an outage** — `current_prices` is a
real table and keeps serving its last-written rows until the first refresh lands.
No backfill or migration is needed: the MV recomputes every row on each refresh,
so the first one populates the new columns for every asset.

## Step 3 — verify the MV

Force one refresh rather than waiting out the minute:

```sql
SYSTEM REFRESH VIEW prices.mv_current_prices;

-- Watch it land. `exception` MUST be empty.
SELECT view, status, last_success_duration_ms, written_rows, exception
FROM system.view_refreshes
WHERE view = 'mv_current_prices'
FORMAT Vertical;

-- The re-created MV must be the 10-column one. (Do NOT check system.columns for
-- `current_prices` — the TABLE has had all ten columns since 0039; only four of
-- them went unwritten. That check passes before and after, proving nothing.)
SELECT
    countIf(name IN ('sources','price_xlm','change_24h_pct','change_7d_pct')) AS new_cols
FROM system.columns
WHERE database = 'prices' AND table = 'mv_current_prices';   -- expect 4
```

If that returns 0, the MV in place is still the v1 definition — the apply did not
land. Confirm directly with `SHOW CREATE TABLE prices.mv_current_prices` and look
for `arrayReduce('median'` in the body.

Then confirm the columns are actually populated, not just present:

```sql
SELECT
    count()                                     AS assets,
    countIf(sources != '' AND sources != '{}')  AS with_sources,
    countIf(price_xlm > 0)                      AS with_price_xlm,
    countIf(change_24h_pct != 0)                AS with_change_24h,
    countIf(change_7d_pct  != 0)                AS with_change_7d,
    countIf(vwap_24h > 0)                       AS with_vwap
FROM prices.current_prices FINAL;
```

`with_sources` at or near zero means the MV ran but found no priced rows in the
trailing 24h — check enrichment, not this MV. A handful of multi-source assets is
normal; most assets trade on one venue.

Spot-check the shape on a real multi-source asset:

```sql
SELECT asset_id, toString(price_usd) AS price_usd, toString(vwap_24h) AS vwap,
       toString(volume_24h_usd) AS vol_24h, sources
FROM prices.current_prices FINAL
WHERE sources != '' AND length(sources) > 60
ORDER BY volume_24h_usd DESC
LIMIT 5
FORMAT Vertical;
```

Expected: `sources` is a JSON object keyed by venue, each with string-serialised
`price` / `volume_24h` (strings preserve `Decimal(38,14)` precision, per §3.3).
`volume_24h_usd` is a total across **all** sources and may exceed the sum of the
volumes shown — outlier-excluded venues are absent from the JSON but still count
toward the traded total. That asymmetry is intentional, not a bug.

## Step 4 — apply the read-surface view

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' \
  < packages/prices-clickhouse/schema/views.sql
```

`views.sql` is idempotent; the other five views are `CREATE … IF NOT EXISTS` and
no-op. `current_price_usd` is `CREATE OR REPLACE` **specifically because**
`IF NOT EXISTS` does not redefine a view that already exists — against a target
already holding the v1 shape it silently no-ops and the new columns never land.
Verified locally: the `IF NOT EXISTS` form left the view at 6 columns; the
`OR REPLACE` form took it to 13.

## Step 5 — verify the read surface

```sql
SELECT count() FROM system.columns
WHERE database = 'prices' AND table = 'current_price_usd';   -- 13

SELECT asset_kind, asset_code, toString(price_usd) AS price_usd,
       toString(price_xlm) AS price_xlm, toString(vwap_24h) AS vwap, sources
FROM prices.current_price_usd
WHERE asset_kind = 'native'
FORMAT Vertical;
```

XLM's own `price_xlm` should be `1` — but **do not treat anything else as an
abort**. The MV divides `price_usd` by the `xlm_usd` scalar, and the two are not
computed identically: `price_usd` is `argMax(close_usd, timestamp)` with **no**
`close_usd > 0` filter (`current.sql:203`), while `xlm_usd` filters
`close_usd > 0` (`current.sql:113`). So if XLM's newest `price_ohlcv_1m` candle
is un-enriched — enrichment is a separate, lagging pass — `price_usd` reads 0
and `price_xlm` is 0, not 1. A tie at the max timestamp can likewise let the two
aggregations pick different rows and land slightly off 1.

A `price_xlm` of 0 or ≈1-but-not-exactly-1 for XLM therefore diagnoses
**enrichment lag, not a broken view**. Confirm before reacting:

```sql
-- Is XLM's own tip enriched? A zero close_usd at the newest timestamp is the cause.
SELECT timestamp, toString(close), toString(close_usd)
FROM prices.price_ohlcv_1m FINAL
WHERE asset_id = (SELECT asset_id FROM prices.assets FINAL
                  WHERE asset_code = 'XLM' AND issuer_address = '' AND contract_address = '')
ORDER BY timestamp DESC LIMIT 5;
```

New columns are **appended**, so the first six keep their positions and no
consumer is re-ordered. Note this is not the same as being unaffected: the view
now returns **13 columns where it returned 6**, which breaks any consumer
decoding positionally off `SELECT *` (fixed-arity tuple, row struct,
`INSERT … SELECT *`). That is what the heads-up below is for.

This view is BE's in-cluster read path (their 0199 contract — named views, no
HTTP). Tell them two things, not one: the columns are live, **and** the view
went from 6 to 13 columns, so any `SELECT *` on their side should be pinned to
an explicit column list. Point them at the sentinel table in `views.sql`'s JOIN
interop contract — `sources` is `''` (not valid JSON) on a row the MV has never
rewritten, and `0` means "unavailable" on every new numeric column.

## Step 6 — deploy the API

The `api-handler` Lambda lives in the **Compute** stack.

```bash
cd infra
make diff-production          # read-only; review before deploying
make deploy-production-compute
```

Two things to know before running this:

- **It also heals the 0132 CFN drift.** The 0132 egress fix was shipped by a
  surgical `aws lambda update-function-code` precisely to avoid deploying the
  then-unrolled 0072 read-API, which left CloudFormation believing the live
  processor still ran the old asset. This deploy reconciles that — the code it
  ships is the same code already running, so expect no behaviour change there.
- **It deploys every Lambda in the stack from the current tree**, not just the
  API. Confirm `develop` holds nothing else unrolled before running it.

## Step 7 — verify the endpoint

```bash
curl -sS -H "x-api-key: $PRICES_API_KEY" \
  "https://<api-host>/production/v1/assets/native/price" | jq .
```

`sources` must be a populated JSON **object** (not `{}`), and `price_xlm` /
`change_24h_pct` must be non-`"0"` strings for an asset with 24h data. A `{}`
here with populated CH columns means the handler is still the stubbed build —
re-check step 6 shipped.

## Rollback

Independent per layer; nothing here is one-way.

Both CH layers replay the step-0 artifacts, which is why step 0 rewrites them
into runnable form at capture time. **As emitted by `SHOW CREATE` neither one
replays** — both fail `Code: 57 TABLE_ALREADY_EXISTS`. If step 0's `sed` fixups
were skipped, apply them now, before running either command below.

```bash
# View — atomic, no window.
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' \
  < /tmp/0072-rollback-current_price_usd.sql

# MV — the artifact leads with its own DROP (see step 0).
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' \
  < /tmp/0072-rollback-mv_current_prices.sql
```

- **API** — redeploy the Compute stack from the previous commit. The stub build
  reads the same columns and simply ignores them.
- **View** — replays the captured definition. Consumers of the first six columns
  are unaffected either way; anything that started reading the seven new ones
  loses them, so revert the API first if both are coming back.
- **MV** — replays the captured definition. The v1 MV writes only 6
  of the 10 columns, so the other four **freeze at their last written values**
  rather than reverting to DEFAULT: `current_prices` is a ReplacingMergeTree and
  the v1 MV's INSERT does not touch them. If stale non-zero columns would be
  worse than empty ones, clear them explicitly after reverting:

  ```sql
  ALTER TABLE prices.current_prices
    UPDATE sources = '', price_xlm = 0, change_24h_pct = 0, change_7d_pct = 0
    WHERE 1;
  ```

  That is a mutation — check `system.mutations` for completion, and note that
  cleanup on this cluster deletes via `ALTER … DELETE` mutations too, so do not
  confuse the two if both are in flight.
