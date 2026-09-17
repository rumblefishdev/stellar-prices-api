# R — what the ClickHouse box recorded for 2026-09-03 (read 2026-09-17)

Read-only, over `ssh sorban-prod` → `docker exec app-clickhouse-1 clickhouse-client`.
Server 26.3.10.60, up 72 days. All times UTC. Kept verbatim because `system.*_log`
has no TTL today and may get one.

## Server limits — none of them is the ceiling

| setting | value |
|---|---|
| `max_connections` | 4096 (default) |
| `max_concurrent_queries` | 1000 |
| `max_concurrent_select_queries` / `_insert_` / `max_waiting_queries` | 0 (unlimited) |
| `keep_alive_timeout` | 10 s |
| `max_memory_usage` (per query) | 6 GB |

## `system.query_log`, all users, per minute

| minute | QueryStart | QueryFinish | ExceptionBeforeStart | p50 ms (finished) |
|---|---|---|---|---|
| 06:31 | 318 | 318 | 0 | 3 |
| 06:32 | 3,124 | 3,124 | 1 | 8 |
| 06:33 | 5,489 | 5,486 | 0 | 7 |
| 06:34 | 1,447 | 1,449 | **4,921** | 6 |
| 06:35 | 380 | 379 | **6,000** | 3 |
| 06:36 | 291 | 292 | **6,000** | 3 |
| 06:37 | 339 | 339 | **6,000** | 3 |
| 06:38 | 274 | 274 | **5,391** | 3 |
| 06:39 | 277 | 277 | 500 | 2 |
| 06:40 | 305 | 305 | 5 | 3 |
| 06:41–06:57 | ~300/min | ~300/min | 0–4/min | 2–3 |
| 07:00 → | ~290/min | ~290/min | 0 | 2–3 |

Baseline is ~300 queries/min across every tenant of the box.

## Exceptions, 06:28–07:30

| code | n | first | last | text |
|---|---|---|---|---|
| **201** | **28,853** | 06:34:10 | 06:57:54 | ``Quota for user `prices_reader` for 3600s has been exceeded: queries = 10001/10000. Interval will end at 2026-09-03 07:00:00.`` |
| 47 | 2 | 06:30:10 | 06:32:06 | unknown identifier `asset_issuer` (someone's ad-hoc query) |
| 241 | 1 | 06:33:17 | — | one query over its 5.59 GiB memory limit (not ours) |

## What the 10,000 admitted `prices_reader` queries cost (06:00–07:00)

| queries | Σ execution | avg | Σ read_rows | rows / query | Σ read_bytes |
|---|---|---|---|---|---|
| 10,000 | 89.8 s | 9 ms | 175,445,572 | 17,545 | 14.65 GiB |

## Quotas (`system.quotas` + `system.quota_limits`, storage `users_xml`)

| quota | applies to | queries/h | execution_time/h | read_rows/h | errors |
|---|---|---|---|---|---|
| `prices_read` | `prices_reader` | **10,000** | 1000 s | 50 B | unlimited |
| `api_throttle` | **nobody** | 10,000 | 1000 s | 50 B | unlimited |
| `unlimited` | `api_reader`, `dev_shared` | — | — | — | — |
| `prices_write`, `high_write`, `default` | writers, `default` | — | — | — | — |
| `dev_read` | `dev_read` | — | — | 100 B | — |

Intervals are 3600 s, **not randomized** — they align to the clock hour.

## Every code-201 in the log's lifetime (since 2026-05-19)

```sql
SELECT event_date, user, count() AS n, min(event_time) AS first, max(event_time) AS last
FROM system.query_log WHERE exception_code = 201 AND event_date >= '2026-05-19'
GROUP BY event_date, user ORDER BY event_date
```

| date | user | refusals | first | last |
|---|---|---|---|---|
| 2026-06-01 | `api_reader` | 231 | 10:52:23 | 14:59:20 |
| 2026-06-02 | `api_reader` | 39 | 07:25:21 | 08:10:15 |
| 2026-06-10 | `api_reader` | 674 | 11:46:37 | 13:59:56 |
| 2026-06-15 | `api_reader` | 4,193 | 09:12:39 | 10:59:56 |
| 2026-06-16 | `dev_read` | 14 | 10:52:54 | 23:13:46 |
| 2026-06-18 | `dev_read` | 31 | 11:22:36 | 13:59:59 |
| 2026-06-19 | `dev_read` | 2 | 06:56:41 | 06:56:58 |
| 2026-06-23 | `dev_read` | 33 | 07:49:18 | 08:59:29 |
| 2026-06-25 | `dev_read` | 31 | 07:26:20 | 08:59:58 |
| 2026-06-26 | `api_reader` | 169 | 10:57:12 | 10:58:33 |
| 2026-06-29 | `api_reader` | 1,399 | 07:52:12 | 12:50:38 |
| 2026-06-30 | `api_reader` | 3,655 | 08:38:57 | 08:59:59 |
| 2026-07-06 | `dev_read` | 102 | 11:47:04 | 12:52:55 |
| 2026-08-26 | `dev_read` | 9 | 13:45:07 | 13:54:23 |
| **2026-09-03** | **`prices_reader`** | **28,853** | 06:34:10 | 06:57:54 |
| 2026-09-05 | `dev_read` | 1 | 13:59:33 | 13:59:33 |
| 2026-09-11 | `dev_read` | 4 | 12:04:43 | 12:45:25 |
| 2026-09-12 | `dev_read` | 77 | 17:48:25 | 17:59:39 |
| 2026-09-13 | `dev_read` | 449 | 12:30:10 | 12:59:46 |
| 2026-09-16 | `dev_read` | 2 | 16:43:41 | 16:43:42 |

Twenty rows, the complete result. `api_reader` (sbe's API) was refused on **seven
days in June**, 10,360 times in all, and never after 2026-07-01, when sbe moved it
to `unlimited` (`c51ba735`, sbe `lore-0338`). Refusals repeatedly run up to
`:59:5x` — the same top-of-the-hour release seen on 2026-09-03.

This matters beyond this task: sbe's task 0250 and `clickhouse-rbac.md` record
that quotas are **not** enforced on the Caddy `X-ClickHouse-User` path, from a
May smoke test whose counter stayed at 0. `api_reader` and `prices_reader` reach
the box only through Caddy (8123 is bound to 127.0.0.1), so every row of theirs
above is a quota enforced on that path. Why the May test read 0 is not explained
here.

## `prices_reader` headroom, last 14 days

Worst hour outside the test: **909** queries (2026-09-04 13:00). ~11× under the cap.

## Noted in passing — not this task's

`system` logs have no TTL: `text_log` 75.7 GiB, `query_log` 23.7 GiB, `part_log`
17.0 GiB, growing since May/July. sbe owns the box.
