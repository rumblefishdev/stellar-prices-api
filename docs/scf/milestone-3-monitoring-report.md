# Milestone 3 — 7-day post-launch monitoring report

Tranche 3 AC 9: _"7-day post-launch monitoring report: uptime %, error rate, p95
latency, SDEX push cadence and `earliest_data_available` trajectory."_ Task 0296.

> **Status: definitions and queries ready; figures pending.** The window
> closes 2026-09-30 09:40 CEST. §6 is filled from `export.sh` and `report.py`
> right after that, while CloudWatch still keeps the 1-minute datapoints of the
> first day (until ~2026-10-08).

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
up because nothing failed in it, not because anything was checked. Traffic
after launch is thin: in the first six days of the window, 233 of ~8,700
minutes had any request at all (partial export, 2026-09-29). §6 therefore also
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
- **Two teammate tests are inside it and are counted**: a `curl` loop on
  2026-09-24 15:23 (~4,100 requests in 2 minutes) and a k6 run for 0311 on
  2026-09-25 11:56–12:25. Both produced 4XX and portal effects, no 5XX. §6
  lists the per-day request counts, so their share is visible.
- **Incidents:** the running log in task 0296, copied to §7 when the window
  closes.

## 6. Figures

_To be filled after 2026-09-30 09:40 CEST._

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

_Copied from task 0296's running log when the window closes, next to the alarm
transitions `report.py` prints._
