# Local reproduction — all five findings

Self-contained, local-only, ~2 minutes. Pinned to the prod ClickHouse version
(**26.3.10.60**) per [[feedback-local-tests-match-prod-version]] — the results
below were produced on that pin on 2026-08-04.

Every SELECT under test is copied **verbatim** from the shipped schema
(`views.sql`, `current.sql`, `rollups.sql`); the only edit is widening the MVs'
`now()`-relative window predicates so fixed seed timestamps are in range.

```bash
docker run -d --name ch-0144-repro -p 18123:8123 \
  -e CLICKHOUSE_SKIP_USER_SETUP=1 --ulimit nofile=262144:262144 \
  clickhouse/clickhouse-server:26.3.10.60

for f in 01_schema 02_seed 03_tests 04_sweep_durability; do
  docker cp "$f.sql" ch-0144-repro:/tmp/ && \
  docker exec ch-0144-repro clickhouse-client --multiquery --queries-file "/tmp/$f.sql"
done

docker rm -f ch-0144-repro
```

`04_sweep_durability.sql` truncates `price_ohlcv_1h` and re-seeds it, so run it
last (or re-run `02_seed.sql` before repeating `03_tests.sql`).

## What each script asserts

| Script | Finding | Assertion |
|---|---|---|
| `03_tests.sql` TEST A | 3ii root cause | `argMax(close_usd, t.timestamp)` returns **0** for an hour whose last sub-bucket is unpriced; `argMaxIf(…, close_usd > 0)` returns 0.171 |
| `03_tests.sql` TEST B | 3i | `price_usd_series_1h` publishes the 0.764-unit dust print (1.3085) as the whole 13:00 bucket, vs 0.17002 in the fully-enriched control — **7.7×**, BE's number. `priced_volume_share` = 0.000018 |
| `03_tests.sql` TEST C | 1 | native XLM's `price_usd` = **0** unguarded, 0.421 guarded |
| `03_tests.sql` TEST D | 2a | one candle → **2 joined rows**, and both natural identities publish 1.05 — one of them never traded it |
| `04_sweep_durability.sql` | 3ii consequence | the [[0114]] sweep's repair (version 401) is overwritten by the next MV re-append (version 402) and the bucket **vanishes from the view** |

## The one result that constrains the fix

TEST B also measures the naive fix. Weighting over **all** rows, filter removed:

```
bucket                 unfiltered_wavg   rows_total  rows_priced  priced_volume_share
2026-08-04 12:00:00    0.170020          2           2            1
2026-08-04 13:00:00    0.000023          2           1            0.000018
```

`0.000023` against a true ~0.170 — dropping the `close_usd > 0` filter is
**worse than keeping it**, because an unpriced row enters the weighted mean as a
zero numerator against a full-weight denominator. That is exactly why the filter
was written. Neither "filter" nor "no filter" is correct; the population has to
be gated on coverage, which is what `priced_volume_share` measures.
