# Milestone 3 — deviations from the criteria wording, and why

> ⚠️ **DRAFT — opened 2026-09-25.**

This document is part of the Milestone 3 submission. It sets out each place
where the delivered system departs from the literal wording of a Tranche 3
acceptance criterion (technical design §9), with the measurement and the
reasoning behind it. The evidence package
([`milestone-3-evidence.md`](milestone-3-evidence.md) §4) points here.

## 1. AC 1 asks a reviewer to confirm the backfill is still running

Carried unchanged from [`milestone-2-rfp-deviations.md`](milestone-2-rfp-deviations.md)
§4, where it is set out in full.

The criterion reads _"`GET /backfill/status` shows `sdex.status: "running"`,
`sdex.last_push_at` within the Tranche 3 push-cadence window, and
`sdex.earliest_data_available` ≤ 2018-01-01"_. The archive completed during
Tranche 2, on 2026-07-27, and reaches back to **2015-11-18**, two years beyond
the depth clause. The liveness half is
graded on the signals that are live after a backfill: the rollup-freshness
alarms, the ledger-processor lag alarm, and `realtime_tip_ledger` tracking the
chain tip. The design document carries the amendment since 2026-09-08.

### Status

Delivered early. The signals and the `GET /v1/backfill/status` body, read on
2026-10-01, are in evidence AC 1.

## 2. AC 9 names two metrics that are flat by construction

### The deviation

AC 9 asks the 7-day post-launch report for _"uptime %, error rate, p95 latency,
SDEX push cadence and `earliest_data_available` trajectory"_. The last two
describe a backfill that is still running. The archive completed on 2026-07-27,
and `earliest_data_available` has been 2015-11-18 every day since.

### What the report carries instead

Signals that answer the same question, _is the data current?_, each against the
threshold of the alarm that watches it:

| AC 9 wording                         | Reported instead                                                                                                                                                                                                                          | Measured                            |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| SDEX push cadence                    | live ingestion keeps up: age of the oldest ledger notification waiting in the ingest queue (`AWS/SQS ApproximateAgeOfOldestMessage`, `prices-ingest-production`) against 120 s, the threshold of `prices-production-ledger-processor-lag` | every minute of the window          |
|                                      | prices are recomputed: `current_prices` lag behind the 1-minute source (`Prices/Rollup RollupLagSeconds`) against 900 s, the threshold of `prices-production-current-prices-freshness`                                                    | every 15-minute probe of the window |
| `earliest_data_available` trajectory | the value at export time, next to `2015-11-18T03:47:00Z` as re-verified on 2026-09-09 for Milestone 2                                                                                                                                     | once, at export                     |
|                                      | `realtime_tip_ledger` against the network's latest ledger (Horizon)                                                                                                                                                                       | once, at export                     |

The other rollup tiers (15m to 1M) are not charted; any transition of their
freshness alarms in the window is in the report's alarm history.

### Status

Disclosed.

## 3. AC 8: no standing "read-only IAM role" — access on request, per person, with MFA

### The wording

_"CloudWatch dashboard accessible to Stellar team (read-only IAM role); all
alarms OK."_

### The deviation

No standing role or user exists for the Stellar team in `infra/`, by decision
of the operator on 2026-09-25. Access is granted on request: a named reviewer —
first name, surname, e-mail, purpose and end date — gets an IAM Identity Center
user with the `PricesDashboardRead` permission set, assigned for the review
window, with MFA at sign-in
([`docs/runbooks/0295-dashboard-access-via-identity-center.md`](../runbooks/0295-dashboard-access-via-identity-center.md)).

### Why

- **There is no external principal to trust.** A cross-account role needs the
  reviewer's AWS account id, and none has been named.
- **The account is shared** with the Soroban Block Explorer and is otherwise
  SSO-only. A standing credential nobody asked for would go against least
  privilege (AC 6); the block explorer's Milestone 3 offers access "on request"
  for its equivalent criterion.

### What a reviewer gets

The dashboard, its metrics and the alarm states — the nine CloudWatch read
actions of the permission set — and nothing that can read a log group, a trace, a
secret or a Lambda's configuration. The read actions cannot be scoped per
dashboard, so the explorer's dashboard in the same account is visible too.

### Status

Disclosed.

## 4. AC 2: Redocly lints the document, and the reference is the portal's own renderer

### The wording

_"OpenAPI spec passes `openapi-validator` lint with no errors; Swagger UI
deployed."_ The Work list adds _"OpenAPI 3.0 specification covering all
endpoints"_.

### The deviation

- **The document is OpenAPI 3.1.0, not 3.0.** utoipa generates it from the
  handler code, and the API serves it unchanged at `GET /api-docs-json`.
- **The lint is Redocly CLI 2.44.0, not `openapi-validator`.** It runs the
  `recommended-strict` ruleset, which promotes the recommended set's warnings
  to errors, plus `operation-4xx-response` and `no-invalid-schema-examples` at
  error severity ([`redocly.yaml`](../../redocly.yaml)). CI runs it in the
  Rust job, on the bytes the API serves (`npm run openapi:lint`). One rule is
  off: `info-license-strict`, because the API's licence is not decided yet
  (task 0155). The two anonymous routes, `/health` and `/api-docs-json`, are
  exempt from `operation-4xx-response` by name in `.redocly.lint-ignore.yaml`.
- **The reference is not Swagger UI.** `https://sorobanscan.rumblefish.dev/api/docs`
  is the portal's own renderer of the live document, in Swagger UI's layout:
  tags, collapsible operations, parameters, responses, and the schemas at the
  foot. It has no "Try it out".

### Why

- **We read `openapi-validator` as "an OpenAPI linter".** The criterion is the
  wording of our own design document, and the project never adopted a tool by
  that name. The npm package that owns the name, IBM's
  `ibm-openapi-validator`, was run on the same document in task 0124 (recorded
  2026-08-06) and reported 13 errors. Eight were real and are fixed: ledger
  fields bounded to `uint32`, the `limit` range published, a one-line
  `summary` for `/health`, and a safe-integer ceiling on `trade_count`. Four
  of the five left are OpenAPI 3.1 constructs checked against 3.0 rules
  (`type: "null"` in an optional field's `oneOf`, and `$ref` beside a
  `description`), and one is IBM's snake_case house style applied to the path
  `/api-docs-json`. Reaching zero would mean switching those rules off, or
  converting the document to 3.0 before linting, and then the linted document
  would no longer be the one the API serves. At warning level the tool also
  asks for IBM's own error-body shape (`trace`, `errors[]`), which would change
  the error envelope on every endpoint.
- **3.1, because utoipa 5 emits nothing else.** Reaching 3.0 would mean
  downgrading the generator or rewriting the served document after the fact;
  current tooling, Swagger UI included, reads 3.1 natively.
- **Swagger UI was tried and taken out.** `swagger-ui-react` ran in the portal
  for a day (task 0195): its stylesheet assumes a white page, its layout does
  not adapt, and with "Try it out" off it renders every parameter as a disabled
  form that reads as a broken control.

### What a reviewer can check

- The lint as CI runs it: `npm run openapi:lint`, step _Lint OpenAPI document_
  of the Rust job, green on 2026-09-30 in run
  [`36731759202`](https://github.com/rumblefishdev/stellar-prices-api/actions/runs/36731759202).
- The document itself, at `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`.
  It needs no key and is served with `Access-Control-Allow-Origin: *`, so a
  reviewer can load it into a Swagger UI of their own.
- The IBM result: `npm run openapi:extract`, then
  `npx ibm-openapi-validator target/openapi.json --errors-only`.

### Status

Disclosed. _To fill on submission day:_ the Redocly summary line, and the IBM
validator's error count on that day's document (five in task 0124).

## 5. AC 5: met on the scenario it names, and what travels with that number

### The wording

_"Load test report: p95 <100ms at 100 req/s confirmed — same caveat as Tranche
2 AC 2: a purpose-built usage plan is required, and the report names it."_ The
Work list adds _"results at 100/s, 500/s, 1000/s"_.

### Met as written

p95 **49.0 ms** at 100 req/s for five minutes, 0 errors in 30,001 requests, on
the plan the report names, `prices-production-loadtest-plan` (evidence AC 5;
measured 2026-09-18, task 0293). The points below set the scope of that number.

### The scope

- **The scenario is the cache's best case.** It cycles through 20 assets, so
  98.3 % of its requests were answered from the API Gateway cache. Production
  traffic looks the other way: about 4 % hits over 2026-09-04 → 09-16, at
  0–1,100 requests a day, because a 10-second TTL rarely sees the same key
  twice at that volume. The 7-day report's 42.8 % covers a window in which
  63,739 of the 65,806 requests were the test bursts it lists.
- **The miss-only row is over the bar at the client and under it at the
  gateway:** p95 129.9 ms from the operator's laptop in Poland, about 45 ms of
  it network, and 45–90 ms per minute as API Gateway measures it.
- **The client was that laptop, not a machine in `eu-central-1`.** The
  in-region run was dropped by decision on 2026-09-18 (task 0293). One-minute
  controls before and after each run measure the network floor instead, and
  every row carries the gateway-side p95.
- **500 and 1000 req/s are informational, not bars** (the team's decision of
  2026-09-18). 500 req/s held with 0 errors. At 1000 req/s 15.9 % of requests
  failed, all of them Lambda throttles. The limit is the ClickHouse server shared
  with the Soroban Block Explorer, between 500 and ~900 req/s.
- **The server's load has changed since.** The history recomputation (task 0286) has
  run on it since 2026-09-23, after these figures; its effect on latency is not
  measured.

### Status

Disclosed. Figures and their sources: evidence AC 5 and
[`prices-api-load-test-100rps.md`](../prices-api-load-test-100rps.md).
