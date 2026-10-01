# Milestone 3 — 7-day post-launch monitoring report

Tranche 3 AC 9: _"7-day post-launch monitoring report: uptime %, error rate, p95
latency, SDEX push cadence and `earliest_data_available` trajectory."_ Task 0296.

> **Status: figures pulled 2026-09-30 09:46 CEST**, six minutes after the
> window closed, with `export.sh`; tables in §6 printed by `report.py`. The
> raw export, 1-minute series included, is in
> `docs/scf/milestone-3-monitoring/data/`.

## 1. Window

**2026-09-23 09:40 → 2026-09-30 09:40 CEST** (07:40 → 07:40 UTC), 10,080
minutes. Launch is the moment the onboarding portal became public: the
explorer's basic auth came off `/api/*` at 2026-09-23 07:40:45 UTC. Agreed at
the team's daily on 2026-09-24 (task 0296 history).

## 2. Uptime

Nothing probes the API from outside: `GET /health` is a keyless API Gateway
mock that reaches neither Lambda nor ClickHouse. Uptime is therefore derived
from the requests the API actually served.

**Definition.** Source: API Gateway stage metrics `Count` and `5XXError` for
`prices-production-api`, stage `production`, 60-second sums.

1. Split the window into 2,016 intervals of 5 minutes.
2. For each interval, the error rate is 5XX responses divided by requests. An
   interval with no requests has an error rate of 0 %.
3. **Uptime = 100 % − the mean error rate of the 2,016 intervals.**

This is the construction AWS uses for its own service SLAs, Amazon API
Gateway's among them. _Check the current SLA text before quoting it in the
package._

What counts as a failure: a 5XX from the stage. That covers Lambda errors and
timeouts (502/504) and ClickHouse failures surfaced by the handler. 4XX
responses (missing or invalid key, 429 from usage-plan throttling, 404 for
unknown routes) are client errors and are reported separately, not as
downtime.

**Read it together with the traffic.** An interval without requests counts as
up because nothing failed in it, not because anything was checked. Traffic after launch is thin: 274 of the window's 10,080 minutes had any request, and 96.9 % of all requests came from a teammate's tests (§5). §6 therefore also
reports the minutes with at least one request and the minutes with at least
one 5XX, so the uptime figure is never read alone.

## 3. Error rate and latency

- **Error rate:** 5XX ÷ requests over the window, with the count beside it.
  4XX rate reported next to it, marked as client errors.
- **Latency, gateway (`Latency`):** from the request reaching API Gateway to
  the response leaving it: p50, p95, p99 over the whole window, and p95/p99
  per day. It includes cache hits (answered in the gateway) and requests the
  gateway rejects with 4XX (answered in about a millisecond), and both pull
  the percentiles down.
- **Latency, backend (`IntegrationLatency`):** only the requests that reached
  the api-handler Lambda, i.e. cache misses with a valid key. This is the
  number to compare with the load test's miss-only row (0293).
- Percentiles are computed by CloudWatch over the whole period (one datapoint
  per window or per day), not averaged from per-minute percentiles.
- The box under ClickHouse runs task 0286's history re-ingest for the whole
  window (stage A from 2026-09-23 16:10 CEST). Latency here describes that
  loaded box.

## 4. Push cadence and `earliest_data_available`: replaced by live signals

The SDEX archive finished on 2026-07-27, so nothing pushes any more and
`earliest_data_available` stays at 2015-11-18. `Prices/Backfill` has no
datapoint in the last two weeks (`list-metrics`, 2026-09-29). Same root as the
AC 1 amendment; declared in `milestone-3-rfp-deviations.md` §2.
Reported instead:

| AC 9 wording                         | Reported instead                                                                                                                       | Source                                                                                                                                  |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| SDEX push cadence                    | live ingestion keeps up: share of minutes the oldest ledger notification waited ≤ 120 s, and the maximum                               | `AWS/SQS ApproximateAgeOfOldestMessage`, `prices-ingest-production`, 60 s (threshold of alarm `prices-production-ledger-processor-lag`) |
|                                      | prices are recomputed: share of 15-minute probes with `current_prices` lag ≤ 900 s, and the maximum                                    | `Prices/Rollup RollupLagSeconds`, `Table=current_prices` (threshold of `prices-production-current-prices-freshness`)                    |
| `earliest_data_available` trajectory | the value at export time, next to `2015-11-18T03:47:00Z` re-verified on 2026-09-09 for the Milestone 2 evidence (flat by construction) | `GET /v1/backfill/status`                                                                                                               |
|                                      | `realtime_tip_ledger` against the network's latest ledger at export time                                                               | `GET /v1/backfill/status`, Horizon `/ledgers?order=desc&limit=1`                                                                        |

## 5. What the window contains

- **The load tests are outside it.** 0293's runs were on 2026-09-17/18.
- **Teammate tests are inside it and are counted.** A `curl` loop on
  `GET /v1/assets` on 2026-09-24 (15:12–15:13 and 15:24–15:25, 5,922
  requests) and k6 runs for task 0311 on 2026-09-25 (seven bursts between
  11:39 and 15:21, 57,817 requests). Together they are 96.9 % of the window's
  65,806 requests, and produced 4XX and portal effects but no 5XX. Outside the
  bursts the API served 2,067 requests in seven days, 947 of them 4XX.
- **Incidents:** §7, each with its cause.

## 6. Figures

**In one paragraph.** Over the 7 days after launch the API returned no 5XX:
uptime 100.000 % by §2, error rate 0.000 % of 65,806 requests. Gateway latency
p95 was 145.2 ms, and 254.4 ms for the requests that reached the Lambda. Live
ingestion kept up in every one of the 10,080 minutes (oldest ledger waited at
most 5 s against a 120 s threshold), and `current_prices` never lagged more
than 18 s (threshold 900 s). Read these with the traffic: requests reached the
API in 274 minutes, and 96.9 % of the requests were a teammate's tests (§5).

### Window

| figure                                    | value                                         |
| ----------------------------------------- | --------------------------------------------- |
| uptime (§2, 5-minute intervals)           | 100.000 %                                     |
| minutes with at least one 5XX             | 0                                             |
| minutes with any request                  | 274 of 10,080                                 |
| requests                                  | 65,806                                        |
| 5XX rate                                  | 0.000 % (0)                                   |
| 4XX rate (client errors, not downtime)    | 13.36 % (8,791)                               |
| latency p50, gateway (ms)                 | 25.9                                          |
| latency p95, gateway (ms)                 | 145.2                                         |
| latency p99, gateway (ms)                 | 795.7                                         |
| integration latency p50, Lambda path (ms) | 38.7                                          |
| integration latency p95, Lambda path (ms) | 254.4                                         |
| integration latency p99, Lambda path (ms) | 896.7                                         |
| cache hit ratio                           | 42.8 %                                        |
| api-handler errors                        | 0                                             |
| api-handler throttles                     | 0                                             |
| ledger-processor errors                   | 0                                             |
| portal closed at cold start               | 11                                            |
| portal source loads failed                | no data                                       |
| ingest lag ≤ 120 s                        | 100.00 % of 10,080 minutes with data; max 5 s |
| current_prices lag ≤ 900 s                | 100.00 % of 672 probes; max 18 s              |

### Per day (09:40 → 09:40 CEST)

| day from    | requests | 5XX | 4XX   | p95 (ms) | p99 (ms) |
| ----------- | -------- | --- | ----- | -------- | -------- |
| 09-23 09:40 | 680      | 0   | 181   | 221.6    | 899.6    |
| 09-24 09:40 | 6,229    | 0   | 1,812 | 11.5     | 1,565.3  |
| 09-25 09:40 | 58,165   | 0   | 6,243 | 149.1    | 793.5    |
| 09-26 09:40 | 100      | 0   | 95    | 1.1      | 612.9    |
| 09-27 09:40 | 171      | 0   | 171   | 0.0      | 0.0      |
| 09-28 09:40 | 95       | 0   | 87    | 19.7     | 720.4    |
| 09-29 09:40 | 366      | 0   | 202   | 610.6    | 729.7    |

### Point samples at export (2026-09-30T07:46:24Z)

- `sdex.earliest_data_available`: 2015-11-18T03:47:00Z
- `realtime_tip_ledger`: 64693715 against the network's 64693720 (5 ledgers behind)

### Test traffic inside the window

| when (CEST)               | requests | 4XX   | source                                         |
| ------------------------- | -------- | ----- | ---------------------------------------------- |
| 09-24 15:12–15:13         | 1,780    | 288   | `curl` loop on `GET /v1/assets`                |
| 09-24 15:24–15:25         | 4,142    | 1,326 | same loop; closed the portal in 7 environments |
| 09-25 11:39–11:41         | 8,049    | 2,422 | k6 for 0311                                    |
| 09-25 11:47–11:50         | 12,121   | 3,728 | k6 for 0311                                    |
| 09-25 11:54–12:18         | 34,257   | 3     | k6 for 0311 (concurrency 200)                  |
| 09-25 12:22–12:25         | 1,192    | 0     | k6 for 0311                                    |
| 09-25 14:53, 14:59, 15:21 | 2,198    | 77    | k6 for 0311                                    |

Bursts are minutes with ≥ 100 requests, merged across gaps of up to 3 minutes,
from the 1-minute `Count` series in `data/minute.json`.

### Reproduce

```bash
export AWS_PROFILE=soroban-admin
export API_KEY=<reviewer key published in milestone-2-evidence.md>
sh docs/scf/milestone-3-monitoring/export.sh
python3 docs/scf/milestone-3-monitoring/report.py
```

`export.sh` writes the raw CloudWatch output to
`docs/scf/milestone-3-monitoring/data/`, including the exact queries it ran.
The 1-minute series stay reproducible after CloudWatch rolls them up.
`report.py` prints this section and the alarm transitions for §7.
`python3 report.py --self-test` checks the uptime arithmetic.

## 7. Incidents in the window

No incident reached `/v1` as an error. Each has its cause; the alarm
transitions they produced follow the table.

| when (CEST)                | what                                                                                                                                                          | `/v1`                                                       | cause / task                                                                                                                        |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| window start → 09-24 09:21 | `zero-invariant-1` in ALARM, carried over from before the window                                                                                              | none                                                        | returned to OK 09-24 09:21                                                                                                          |
| 09-24 15:17                | oracle `Runtime.OutOfMemory` (256/256 MB); retried a minute later, completed                                                                                  | none                                                        | the re-ingest rewrites the whole asset registry after every month and the oracle reads it without `FINAL` — tasks 0226, 0140        |
| 09-24 15:24 → ~16:00       | portal closed in 7 of 69 execution environments after the second `curl` burst (69 simultaneous cold starts throttled Parameter Store)                         | none, 0 × 5XX                                               | cold-start reads of three parameters; fixed 09-25 14:47 by task 0311                                                                |
| 09-24 20:09                | oracle out-of-memory, as 15:17                                                                                                                                | none                                                        | as 15:17                                                                                                                            |
| 09-25 12:15 → 12:37        | portal closed in ≥ 4 environments during the k6 run (576 cold starts in 10 minutes)                                                                           | none                                                        | same mechanism; fixed at 14:47 the same day                                                                                         |
| 09-25 14:49 → 09-29 09:49  | asset-discovery's `-no-invocations` and `-duration-near-timeout` alarms absent, deleted by an Observability deploy from task 0311's branch that predated them | none; a monitoring gap, the worker's `-errors` alarm stayed | restored by a deploy from `develop` (task 0256)                                                                                     |
| 09-25 19:14                | oracle out-of-memory, as 09-24 15:17                                                                                                                          | none                                                        | as 09-24 15:17                                                                                                                      |
| 09-28 08:14 →              | `coverage-sweep-unclassified` ALARM: the weekly probe found one unclassified swap emitter (the `sda` aggregator, one event)                                   | none                                                        | task 0100; allow-listed in PR #358. OK since 09-30 17:41, after the window: the alarm now clears on the first clean run (task 0323) |

### Alarm transitions in the window (prices-production-\*)

| when             | alarm                                 | transition              |
| ---------------- | ------------------------------------- | ----------------------- |
| 09-24 09:21 CEST | zero-invariant-1                      | ALARM to OK             |
| 09-24 13:48 CEST | coverage-sweep-probe-errors           | INSUFFICIENT_DATA to OK |
| 09-24 15:18 CEST | oracle-errors                         | OK to ALARM             |
| 09-24 15:23 CEST | oracle-errors                         | ALARM to OK             |
| 09-24 15:24 CEST | api-handler-portal-closed             | OK to ALARM             |
| 09-24 15:39 CEST | api-handler-portal-closed             | ALARM to OK             |
| 09-24 20:09 CEST | oracle-errors                         | OK to ALARM             |
| 09-24 20:13 CEST | oracle-errors                         | ALARM to OK             |
| 09-25 09:20 CEST | asset-discovery-duration-near-timeout | INSUFFICIENT_DATA to OK |
| 09-25 09:20 CEST | asset-discovery-no-invocations        | INSUFFICIENT_DATA to OK |
| 09-25 10:14 CEST | coverage-sweep-unclassified           | INSUFFICIENT_DATA to OK |
| 09-25 12:16 CEST | api-handler-portal-closed             | OK to ALARM             |
| 09-25 12:18 CEST | asset-discovery-no-invocations        | OK to ALARM             |
| 09-25 12:21 CEST | asset-discovery-no-invocations        | ALARM to OK             |
| 09-25 12:37 CEST | api-handler-portal-closed             | ALARM to OK             |
| 09-25 14:49 CEST | api-handler-portal-load-failed        | INSUFFICIENT_DATA to OK |
| 09-25 19:14 CEST | oracle-errors                         | OK to ALARM             |
| 09-25 19:18 CEST | oracle-errors                         | ALARM to OK             |
| 09-28 08:14 CEST | coverage-sweep-unclassified           | OK to ALARM             |
| 09-29 09:49 CEST | asset-discovery-duration-near-timeout | INSUFFICIENT_DATA to OK |
| 09-29 09:49 CEST | asset-discovery-no-invocations        | INSUFFICIENT_DATA to OK |
