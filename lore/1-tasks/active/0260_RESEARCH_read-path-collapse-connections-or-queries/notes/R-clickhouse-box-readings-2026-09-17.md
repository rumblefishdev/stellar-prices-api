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

`api_reader` (sbe's API): 2026-06-01, 06-02, 06-10, 06-15 (4,193), 06-26, 06-29,
06-30 (3,655) — then never again: sbe moved it to `unlimited` on 2026-07-01
(`c51ba735`, sbe `lore-0338`). `dev_read`: a dozen small days. `prices_reader`:
**2026-09-03 only — 28,853.**

## `prices_reader` headroom, last 14 days

Worst hour outside the test: **909** queries (2026-09-04 13:00). ~11× under the cap.

## Noted in passing — not this task's

`system` logs have no TTL: `text_log` 75.7 GiB, `query_log` 23.7 GiB, `part_log`
17.0 GiB, growing since May/July. sbe owns the box.
