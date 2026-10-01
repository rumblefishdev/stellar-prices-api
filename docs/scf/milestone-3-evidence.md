---
margin:
  x: 1.5cm
  y: 1.5cm
---

# Stellar Prices API — Milestone 3 Deliverable Evidence

> - **Project:** Stellar Prices API
> - **Team:** Rumble Fish
> - **Status of this document:** ⚠️ **DRAFT — skeleton opened 2026-09-25.** Every
>   figure carries the date it was measured; a section marked _To fill_ has no
>   evidence yet and must not be read as a claim.
>
> This document maps every Tranche 3 acceptance criterion to evidence against the
> deployed production API: live URLs, runnable commands, and the tasks and
> reports behind each figure. **Everything here is reproducible against the
> public deployment.** The API base is `https://prices-api.sorobanscan.rumblefish.dev`,
> a REGIONAL custom domain mapped at the root. Key-gated routes need an
> `x-api-key` header; the reviewer key published with the Milestone 2 package
> (`prices-production-scf-reviewer-key-20260909T120021Z`, public free tier —
> 1 request per second, burst 5, 100,000 requests a month, read-only) is
> unchanged and every command below runs as printed with `API_KEY` set to it.
>
> **Criteria graded against amended wording** are set out in full in
> [`milestone-3-rfp-deviations.md`](milestone-3-rfp-deviations.md), part of
> this submission. The project runs as a second tenant on infrastructure the
> Soroban Block Explorer already operates.

## 1. Executive summary

Milestone 3 — **Production Launch & Validation** — is the last tranche. The
service went public on **2026-09-23 at 09:40 CEST**, when the onboarding portal
at `https://sorobanscan.rumblefish.dev/api/` lost its staging basic auth; the
API itself had been serving on the custom domain since Tranche 2.

State of the nine Tranche 3 acceptance criteria **as of 2026-09-25** (this table
is rewritten on submission day; the rows say what is claimed, not what is
hoped):

| AC  | Criterion (short)                                           | State on 2026-09-25                                                                                                                                                         |
| --- | ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `/backfill/status`: running, fresh push, depth ≤ 2018-01-01 | **Depth met — 2015-11-18.** Liveness graded on amended wording (deviations §1): the archive completed 2026-07-27                                                            |
| 2   | OpenAPI lints clean; Swagger UI deployed                    | Lint is a CI job (`npm run openapi:lint`, Redocly CLI 2.44.0); reference rendered at `…/api/docs` (deviations §4). _To fill: lint output on the day_                        |
| 3   | Portal accessible; self-service key flow works              | Portal public since 09-23; sign-in gated on the Stellar Developers guild since 09-02 (0254). _To fill:_ the recorded walk (0164)                                            |
| 4   | Integration suite passes on CI, link provided               | **Met.** CI starts ClickHouse and runs the integration suite on every Rust change since PR #327 (2026-09-22); run linked in §5                                              |
| 5   | Load test: p95 < 100 ms at 100 req/s, plan named            | **Met 2026-09-18 — p95 49.0 ms, 0 errors in 30,001 requests**, plan `prices-production-loadtest-plan`; scope in deviations §5                                               |
| 6   | Security checklist signed off                               | mTLS-only ClickHouse, secrets in Secrets Manager, inputs validated, no wildcard **actions**; the 23 `Resource: "*"` statements are inventoried in §5                        |
| 7   | Repo public; `cdk deploy` from README in a fresh account    | **Met on the fresh-account runbook** (0297, PR #357): repo PUBLIC, `README.md` → `infra/README.md`; macOS via 0239 (PR #360). Not run in an empty account (§5)              |
| 8   | Dashboard accessible to Stellar (read-only IAM); alarms OK  | Dashboard `prices-production-overview`, 65 alarms, all OK on 10-01. Access **on request to a named reviewer, MFA enforced** (deviations §3)                                 |
| 9   | 7-day post-launch report                                    | **Met 2026-09-30 — uptime 100.000 %, 0 × 5XX in 65,806 requests, gateway p95 145.2 ms**; push cadence and `earliest_data_available` replaced by live ingestion signals (§4) |

The Tranche 3 work items with no numbered criterion are listed in §6, the known
issues this submission declares in §7, and what it deliberately does not claim
in §8.

## 2. Deliverable definition

§9 of the technical design defines Tranche 3 as **Production Launch &
Validation**, weeks 10 to 13. The work it names, and where each stands:

| Work item                                                                 | State                                                                                                                                                                                  |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OpenAPI 3.0 specification covering all endpoints                          | Served from the code at `/api-docs-json` as OpenAPI 3.1.0 (deviations §4); 74 production-shaped examples and a lint gate since 2026-09-24 (task 0306)                                  |
| Self-service onboarding portal: key request, quickstart, example queries  | Public since 2026-09-23 at `…/api/`; quick start reconciled with the spec (0163, 0233); privacy policy (0303); Discord sign-in on the Stellar Developers guild since 2026-09-02 (0254) |
| Integration test suite, automated, runs in CI, all 7 endpoint groups      | 229 ClickHouse-backed tests run in CI since 2026-09-22 (task 0275); each of the seven `/v1` route groups has its own (§5, AC 4)                                                        |
| Load test report: k6, documented plan, results at 100 / 500 / 1000 req/s  | [`prices-api-load-test-100rps.md`](../prices-api-load-test-100rps.md) — all three rates on 2026-09-18 (task 0293)                                                                      |
| Security review checklist: IAM least privilege, no secrets in env, inputs | Audit in task 0194; no wildcard actions, every `Resource: "*"` statement inventoried (§5, AC 6)                                                                                        |
| X-Ray tracing end-to-end                                                  | `TracingConfig.Mode: Active` on api-handler, oracle, enrichment and ledger-processor (§6)                                                                                              |
| CloudWatch dashboards: latency, errors, ingestion lag, CH write latency…  | `prices-production-overview`; 65 `prices-production-*` alarms, all of them on its alarm strip (§6)                                                                                     |
| GitHub repository public with README, architecture docs, deploy steps     | PUBLIC since 2026-09-16; root `README.md` and the fresh-account runbook in `infra/README.md` since 2026-09-25 (PR #357, task 0297; AC 7)                                               |

**The backfill milestone for the tranche** — SDEX history to ~January 2018 —
was overtaken during Tranche 2: the archive walked to genesis and reports
`completed` at **2015-11-18** since 2026-07-27. The reviewer-confirmation list
that expected a running backfill is amended in the design document (2026-09-08)
and in [`milestone-2-rfp-deviations.md`](milestone-2-rfp-deviations.md) §4;
this package carries it as deviations §1. What _is_ running during Tranche 3 is
a **quality re-computation of the history** (task 0286, phase 3, from
2026-09-23): candles rebuilt from price-forming fills in fill order. It changes
values, not coverage, and is declared in §7.

## 3. Architecture

Unchanged in shape from Milestones 1 and 2; the full description is
[`docs/prices-api-general-overview.md`](../prices-api-general-overview.md) §2
and §3. What Tranche 3 added on top of the Milestone 2 platform:

| Layer         | Addition                                                                                                                                                                                            |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Portal (SPA)  | Served under `/api/*` of the explorer's CloudFront distribution from its own S3 bucket; Discord OAuth sign-in, key issuance on the free plan, dashboard, quick start                                |
| API Gateway   | 404 `not_found` in the error envelope for unknown routes (0309); five usage plans, free and four paid tiers, live since 2026-09-24 (0311)                                                           |
| Lambda (axum) | Portal routes whose sources load on the first portal request, never at cold start; a failed load costs that request only (0194, 0311); `as_of` and `price_status` beside every current price (0216) |
| ClickHouse    | Price-forming-fill candle definitions across all tiers (0286 phase 1–2, ADR 0287); scoped `prices_admin` identity for the history re-ingest (explorer 0567)                                         |
| Observability | api-handler error and 5xx alarms (0249) and portal-load-failed alarm (0311); liveness and duration alarms on every scheduled worker (0223, 0256); weekly coverage sweep of swap venues (0100)       |

_To fill: the component table and a data-flow figure if the reviewer packet
needs one; otherwise the pointer above stands._

## 4. Deviations from the approved wording

Each is set out in full, with its measurements and reasoning, in
[`milestone-3-rfp-deviations.md`](milestone-3-rfp-deviations.md). The rows below
are pointers, not summaries.

| #   | Wording says                                                              | We deliver                                                                                                                                                | Kind                       |
| --- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| 1   | AC 1: `sdex.status: "running"`, `last_push_at` fresh                      | the archive completed on 2026-07-27; liveness graded on the ingestion alarms and `realtime_tip_ledger` (carried from M2)                                  | disclosed, delivered early |
| 2   | AC 9: report "SDEX push cadence and `earliest_data_available` trajectory" | both are flat by construction since 2026-07-27; the report carries ingest-queue age and `current_prices` freshness against their alarm thresholds instead | disclosed                  |
| 3   | AC 8: "read-only IAM role" for the Stellar team                           | no standing identity: a read-only IAM user is created for a named reviewer on request, MFA enforced, and removed after the review                         | disclosed                  |
| 4   | AC 2: `openapi-validator` lint, Swagger UI; Work list: OpenAPI 3.0        | an OpenAPI 3.1.0 document, linted by Redocly `recommended-strict`; the reference at `…/api/docs` is the portal's own renderer, in Swagger UI's layout     | disclosed                  |
| 5   | AC 5: p95 < 100 ms at 100 req/s, plan named                               | met as written on a scenario with 98.3 % cache hits; the miss-only row, the 500 / 1000 req/s rows and the client's location travel with it                | disclosed (met as written) |

## 5. Acceptance-criteria evidence

### AC 1 — `GET /backfill/status` shows a running, fresh backfill with depth ≤ 2018-01-01

**Verdict: the depth clause is met by six years — `earliest_data_available` is
2015-11-18. The two liveness clauses are graded on amended wording** (deviations
§1): the archive completed on 2026-07-27, so `sdex.status` is `completed` and
nothing pushes.

_To fill on submission day:_ the live `GET /v1/backfill/status` body, and the
state of the three signals the amendment names — `prices-production-rollup-freshness-*`,
the ledger-processor lag alarm, and `realtime_tip_ledger` against the chain tip.

#### Reproduce it

```bash
curl -s -H "x-api-key: $API_KEY" \
  https://prices-api.sorobanscan.rumblefish.dev/v1/backfill/status | jq .
```

### AC 2 — OpenAPI spec passes lint with no errors; Swagger UI deployed

**Verdict: _to fill_ — the gate exists; the output on the day is not yet
recorded.** The document is generated from the handler code (utoipa) and served
at `/api-docs-json`; since task 0306 it carries a production-shaped example for
every field and a lint that fails on an example that stops validating against
its schema (`no-invalid-schema-examples`, `redocly.yaml`). The rendered
reference is the portal's own page, `https://sorobanscan.rumblefish.dev/api/docs`.
Neither the lint tool nor the reference is the one the wording names; both
substitutions, and the OpenAPI version, are set out in deviations §4.

#### Reproduce it

```bash
npm run openapi:lint          # extracts target/openapi.json from the code and lints it
```

_To fill:_ the lint summary line, the CI job that runs it, and a screenshot of
`/api/docs`.

### AC 3 — Onboarding portal accessible; self-service API key request flow functional

**Verdict: _to fill_ — the flow works on production; the recorded walk is not
in this document yet.** The portal is public at `https://sorobanscan.rumblefish.dev/api/`
since 2026-09-23 09:40 CEST. The flow is Discord OAuth → eligibility check
(guild membership, account age) → a key on the free plan → the dashboard with
the key's plan and usage.

Eligibility is checked against the official **Stellar Developers** guild
(`897514728459468821`) since 2026-09-02 (task 0254; production parameter
`/prices/production/discord-guild-id`, version 2). A member the guild has not
screened yet is refused with its own answer (`pending_rules`). SDF, which owns
the guild, asked in an official statement that the project hold the Discord
developer account and run the integration itself (task 0179, closed
2026-09-28). The team has walked the flow end to end on production several
times (task 0164, closed 2026-09-28).

_To fill:_ the recorded walk on production — a sign-in, a key issued, a `/v1`
call with it, the key revoked — with timestamps (video scene 2 can record it).

### AC 4 — Integration test suite: all tests pass on CI, link provided

**Verdict: met.** Since PR #327 (task 0275, merged 2026-09-22) the CI workflow
starts a ClickHouse instance, applies the schema, starts the mTLS reverse proxy,
and runs the integration suite — the tests that were `#[ignore]`d for lack of a
database — on every change to Rust code, after the unit tests.

The seven endpoint groups the Work list names are the seven `/v1` routes of §9,
counted as the Milestone 2 package counted them
([`milestone-2-evidence.md`](milestone-2-evidence.md) AC 1); the design
document's §4 files the same routes under five headings. Each route has its own
tests in `packages/prices-api/tests/`: `GET /v1/assets` in `list_it.rs`;
`/v1/assets/{id}`, `POST /v1/prices/batch`, `/v1/oracles/{id}` and
`/v1/backfill/status` in `endpoints_it.rs`; `…/price` in `price_it.rs`; and
`…/ohlcv` in `ohlcv_it.rs`.

A run to cite: [`36001067242`](https://github.com/rumblefishdev/stellar-prices-api/actions/runs/36001067242)
(2026-09-24, job _Rust (fmt, clippy, test, lambda build)_, steps _Start
ClickHouse → Apply the ClickHouse schema → Start the ClickHouse reverse proxy →
ClickHouse integration tests_, all `success`).

_To fill on submission day:_ the newest green run on `develop`, and the test
count from its log (229 integration tests at #327).

#### Reproduce it locally

```bash
docker compose up -d clickhouse
tools/scripts/ignored-tests.sh        # exactly what CI runs
```

### AC 5 — Load test: p95 < 100 ms at 100 req/s, the usage plan named

**Verdict: met. p95 = 49.0 ms against a 100 ms bar, 0 errors in 30,001
requests, k6 exit 0.** Measured 2026-09-18 with k6 v2.2.0 from the operator's
laptop in Poland, bracketed by one-minute controls that put the network floor at
a median of ~45 ms ([`prices-api-load-test-100rps.md`](../prices-api-load-test-100rps.md)
§"Evidence run and the ceiling — 2026-09-18", task 0293).

**The plan, as the criterion asks:** `prices-production-loadtest-plan` (id
`i12bsj`), 150 req/s / burst 300 for the 100 req/s rows, raised to 1200 / 2400
for the 500 and 1000 req/s rows and restored afterwards. No ordinary key can
sustain the run — the default plan is 1 req/s and a 100,000 monthly quota.

| rate       | scenario                         | requests | failed                                | k6 med / p95 / p99 (ms) | gateway p95 | cache hits |
| ---------- | -------------------------------- | -------- | ------------------------------------- | ----------------------- | ----------- | ---------- |
| 100 req/s  | **AC scenario**, 17 of 20 assets | 30,001   | 0                                     | 44.7 / **49.0** / 130   | 6 ms        | 98.3 %     |
| 100 req/s  | wide pool, 3,463 assets (misses) | 30,000   | 0                                     | 71.7 / **129.9** / 171  | 45–90 ms    | 0 %        |
| 500 req/s  | wide pool × 4 key variants       | 149,880  | 0                                     | 68.8 / **133.0** / 261  | 87–95 ms    | ~2.6 %     |
| 1000 req/s | wide pool × 8 key variants       | 93,351   | 14,865 (15.9 %), all Lambda throttles | 637 / 1,730 / 2,700     | 1.4–1.5 s   | 0 %        |

**Scope of the claim** (deviations §5). The AC scenario is 98.3 % cache hits and the report
says so; the miss-only row is the honest companion (p95 129.9 ms from Poland,
~45 ms of it network, 45–90 ms as the gateway measures it). 500 req/s held with
p95 moved by 3 ms; the ramp to 1000 req/s found the ceiling in the shared
ClickHouse box between 500 and ~900 req/s, not in Lambda or the gateway. The
overload slowed the explorer's indexer for two minutes (its alarm paged and
recovered; nothing lost) — recorded in the report as collateral.

**A side effect the report does not record.** On the 500 and 1000 req/s ramps,
three bursts of 157, 278 and 255 Lambda cold starts within 1–2 s throttled
Parameter Store and closed the onboarding portal in 243 execution environments
(§7); `/v1` answered throughout. Bursts of 26–68 cold starts per second closed
nothing. The AC-scenario run (09:33–09:44 UTC) had 41 cold starts, no closure
and no Parameter Store throttle, so its 49.0 ms is unaffected. The cause has
been fixed since 2026-09-25 (§7): a `/v1` cold start no longer reads Parameter
Store at all. Counted
2026-09-28 with CloudWatch Logs Insights on `/aws/lambda/prices-production-api-handler`,
per second over each ramp and in total over the AC run's window:

```
stats sum(strcontains(@message, "INIT_START")) as cold_starts,
      sum(strcontains(@message, "portal closed at cold start")) as portal_closed,
      sum(strcontains(@message, "ThrottlingException")) as ssm_throttled
  by bin(1s)
```

#### Reproduce it

```sh
S='avg,min,med,max,p(50),p(90),p(95),p(99)'
k6 run packages/prices-api/loadtest/price_load.js \
  -e BASE_URL=https://prices-api.sorobanscan.rumblefish.dev -e API_KEY="$API_KEY" \
  --summary-trend-stats="$S" --summary-export=loadtest-ac.json; echo "exit=$?"
```

`exit=0` means every threshold held. Raw exports and the observer's logs are in
[`docs/loadtest-results/2026-09-18-*`](../loadtest-results/). The key must be
on a plan that allows 100 req/s.

**Since the measurement:** the history re-ingest (task 0286 phase 3) has been
running on the same box since 2026-09-23; figures re-run during it describe a
loaded box. State the date beside any re-run.

### AC 6 — Security checklist signed off

**Verdict: met — all four items have evidence; the wildcard-IAM item rests on
the inventory below, taken from the templates of 2026-09-25.** The checklist
as the design document states it:

| Item                                                 | Evidence                                                                                                                                                                                                                                                                                                                       | State            |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------- |
| ClickHouse reachable only via mTLS through Caddy:443 | every client is an mTLS identity mapped by certificate CN; the API's identity is `prices_reader` (SELECT on `prices.*` only), workers `prices_writer`                                                                                                                                                                          | met (M1/M2)      |
| mTLS cert + key in Secrets Manager, not env vars     | `prices/production/clickhouse-mtls-prices-{api,ingestion}-production`; Lambdas read them through the Parameters and Secrets extension                                                                                                                                                                                          | met (M1)         |
| All inputs validated                                 | every invalid input answers a 400 in the error envelope (task 0119); unknown routes answer 404 in the same envelope (0309)                                                                                                                                                                                                     | met (M2, M3)     |
| No wildcard IAM                                      | **No wildcard actions anywhere** — no `actions: ['*']`, no `service:*`, no administrative managed policy (the reading the explorer's Milestone 3 checklist also uses). The synthesized production templates hold **23** statements with `Resource: "*"`, every one an action that has no resource ARN: see the inventory below | met, inventoried |

Audit of the portal's attack surface: task 0194 (archived).

**Inventory of the 23 `Resource: "*"` statements** (from the synthesized
templates of 2026-09-25, not from a source grep — CDK adds some itself):

| group                                               | count | who                                                                                                      | why `*`                                                                                    | what bounds it                                                                                                                                                    |
| --------------------------------------------------- | ----- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `xray:PutTraceSegments`, `xray:PutTelemetryRecords` | 13    | every traced Lambda (api-handler, ledger-processor, ten scheduled workers)                               | X-Ray has no resource ARNs for these actions; CDK adds the statement for `tracing: ACTIVE` | write-only trace upload. The ledger-processor carries it twice (an explicit `XRayWrite` next to the automatic one) — a redundancy, not a widening                 |
| `cloudwatch:PutMetricData`                          | 8     | ledger-processor, oracle, enrichment, coarse-sweep, both freshness probes, mtls-notafter, coverage-sweep | `PutMetricData` has no resource-level scoping                                              | `StringEquals cloudwatch:namespace = Prices/<Ingest, Oracle, Enrichment, Backfill, Rollup, Mtls, Coverage>` — each worker can publish only to its own namespace   |
| `cloudwatch:DescribeAlarms`                         | 1     | mtls-notafter probe (the daily stuck-alarm digest, task 0214)                                            | read-only; a prefix ARN would deny at runtime and page the ops channel                     | reads names and states only; the `prices-production-` filter is in the code                                                                                       |
| CloudWatch read set (`Get*`, `List*`, `Describe*`)  | 1 → 0 | the reviewer identity of task 0125                                                                       | these read actions have no resource ARNs                                                   | **removed by task 0295** (PR #355): no standing identity remains in any template; the same nine actions are granted on request, per person, with an MFA condition |

_To fill on submission day:_ re-run the inventory on that day's templates and
state the count (22 once 0295 is deployed).

### AC 7 — GitHub repository public; `cdk deploy` from README works in a fresh AWS account

**Verdict: met on the fresh-account runbook, the way the Soroban Block
Explorer claimed its equivalent criterion. The runbook has not been executed
in an empty AWS account; what the claim rests on, and what was not done, is
below.**

- **Public repository:** `https://github.com/rumblefishdev/stellar-prices-api`,
  `gh repo view` → `visibility: PUBLIC` since 2026-09-16 (re-checked
  2026-09-28).
- **From README:** the root [`README.md`](../../README.md) (new on 2026-09-25,
  PR #357) leads to the runbook in [`infra/README.md`](../../infra/README.md)
  §"Fresh-account deployment": prerequisites → the platform (the Soroban Block
  Explorer's AWS stacks and its Hetzner ClickHouse server, by that project's
  own runbook) → the `prices` tenant (client certificates, CN → user map,
  schema) → secrets and SSM seeds → domain config → `npm run infra:bootstrap`
  → `npm run infra:deploy:production` → post-deploy checks → tear-down
  (task 0297).
- **Manual by design, and named in the runbook:** ordering the Hetzner server
  and its Storage Box; issuing client certificates from the platform CA, whose
  key never touches CDK or CI; creating the three Secrets Manager values and
  seeding three SSM parameters (CDK owns only the names, so a deploy cannot
  overwrite live credentials); registering the Discord application; a Route 53
  hosted zone.

#### What the claim rests on

| Observable                                                                                                                                  | Evidence                                                                                                                                                                                 | Checked    |
| ------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| The CDK app synthesizes with no AWS account or credentials: every SSM read is a deploy-time `valueForStringParameter`, no context lookups   | CI step "Synth production app", [run 36396870253](https://github.com/rumblefishdev/stellar-prices-api/actions/runs/36396870253) (PR #358; its `infra/` tree is identical to `develop`'s) | 2026-09-28 |
| Every name the runbook has an operator create or read matches the code: five `/platform/*` inputs, three secrets, three seeds, five outputs | read against `infra/src/lib/stacks/compute-stack.ts`, `infra/src/lib/mtls.ts`, `infra/Makefile`                                                                                          | 2026-09-28 |
| `make bootstrap` works on a fresh clone                                                                                                     | fixed in PR #357: it names the environment and no longer synthesizes the app, which cannot synth without built assets; exercised up to the credential call                               | 2026-09-25 |
| The build, synth and secrets steps work on macOS, not only on the Linux CI runner                                                           | on macOS 26.6: 12 aarch64 Lambda bootstraps built and verified, `synth-production` with no credentials, step 4's RAM-disk variant on dummy certificates (task 0239, PR #360)             | 2026-09-28 |

**What was not done.** No `cdk bootstrap` or `cdk deploy` has been run in an
empty account. A run needs more than an AWS account: the platform this API is
a tenant of — the explorer's AWS stacks and a hand-ordered Hetzner server with
its Storage Box — would have to be stood up first, at its hardware cost and
lead time. Every production deploy so far has gone to the one account.
Milestone 1 graded the sibling criterion — _"`cdk deploy` from a clean AWS
account produces the full stack with no manual steps"_ — on a synth of the
stacks and deploys to the existing account, with the out-of-band prerequisites
named as a caveat.

### AC 8 — CloudWatch dashboard accessible to the Stellar team (read-only IAM role); all alarms OK

**Verdict: met on the amended wording (deviations §3); alarms OK on
2026-10-01.** The dashboard is `prices-production-overview` in `eu-central-1`;
**65** `prices-production-*` alarms stand behind it (up from 53 at Milestone 2),
including — since 2026-09-22 — error and portal-closed alarms on the
api-handler (the portal-closed one replaced on the afternoon of 2026-09-25
by `portal-load-failed`, task 0311) and liveness/duration alarms on every
scheduled worker. On
2026-10-01 08:23 CEST all 65 read OK
(`aws cloudwatch describe-alarms --alarm-name-prefix prices-production-`).

**Access is granted on request, to a named person, with MFA — not through a
standing role or user.** The account is shared with the Soroban Block Explorer
and is otherwise SSO-only; no external principal is known that a cross-account
role could trust. A reviewer asks by name (first name, surname, e-mail, purpose
and end date); the operator creates a read-only IAM user for that person with
the scoped policy in
[`docs/runbooks/0295-dashboard-access-on-request.md`](../runbooks/0295-dashboard-access-on-request.md)
(nine CloudWatch read actions, denied without MFA), hands over a one-time
password, and deletes the user after the review. This is the model the block
explorer's Milestone 3 package used for its equivalent criterion.

A second path goes through IAM Identity Center: the reviewer gets an Identity
Center user and a `PricesDashboardRead` permission set with the same nine
CloudWatch read actions, assigned for the review window, with no IAM user and
no password set by the operator
([`docs/runbooks/0295-dashboard-access-via-identity-center.md`](../runbooks/0295-dashboard-access-via-identity-center.md),
PR #364).

_To fill on submission day:_ the alarm table.

### AC 9 — 7-day post-launch monitoring report

**Verdict: met, with two of the five named metrics replaced (deviations §2).
Over the 7 days after launch the API returned no 5XX: uptime 100.000 %, error
rate 0.000 % of 65,806 requests, gateway p95 145.2 ms.** Window **2026-09-23
09:40 → 2026-09-30 09:40 CEST**; launch is the moment the explorer's basic auth
came off `/api/*` (07:40:45 UTC) and the portal became public, agreed at the
team's daily on 2026-09-24. Figures pulled 2026-09-30 09:46 CEST. The full
report is [`milestone-3-monitoring-report.md`](milestone-3-monitoring-report.md)
(task 0296).

| AC 9 asks for                             | Reported                                                                                                                                 | Source                                                                        |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| uptime %                                  | **100.000 %**: 100 % minus the mean 5XX rate of the window's 2,016 five-minute intervals (report §2)                                     | API Gateway `Count` and `5XXError`, stage `production`, 1-minute sums         |
| error rate                                | **0.000 %** 5XX (0 of 65,806); 4XX 13.36 % (8,791), client errors, not downtime                                                          | same                                                                          |
| p95 latency                               | **145.2 ms** at the gateway (p50 25.9, p99 795.7); **254.4 ms** on the Lambda path (p50 38.7, p99 896.7)                                 | `Latency`, `IntegrationLatency`; CloudWatch percentiles over the whole window |
| SDEX push cadence (§2)                    | oldest ledger in the ingest queue ≤ 120 s in **all 10,080 minutes** (max 5 s); `current_prices` lag ≤ 900 s in all 672 probes (max 18 s) | `AWS/SQS ApproximateAgeOfOldestMessage`; `Prices/Rollup RollupLagSeconds`     |
| `earliest_data_available` trajectory (§2) | `2015-11-18T03:47:00Z` at export, as on 2026-09-09; `realtime_tip_ledger` 5 ledgers behind the network at export                         | `GET /v1/backfill/status`; Horizon                                            |

**Scope of the claim.** Nothing probes the API from outside (`GET /health` is a
keyless gateway mock), so uptime is derived from the requests the API served,
and an interval without requests counts as up. Requests reached the API in
**274 of the 10,080 minutes**, and **96.9 %** of all requests came from a
teammate's test bursts inside the window: a `curl` loop on 2026-09-24 and k6
runs for the paid plans (task 0311) on 2026-09-25 (report §6). The evidence
that the service was live in every minute is the ingestion signal, which is
measured regardless of traffic. The gateway p95 includes cache hits and 4XX
rejections answered in about a millisecond; the Lambda-path p95 is the figure
to set beside AC 5's miss-only row. The shared ClickHouse box ran the history
re-ingest (task 0286) for the whole window.

**Incidents.** None reached `/v1` as an error. Report §7 lists eight, each
with its cause:

- three oracle out-of-memory ticks during the re-ingest (§7 of this document);
- two partial portal closures under the test bursts, cause fixed 2026-09-25
  14:47 (§7);
- asset-discovery's two liveness alarms missing 2026-09-25 14:49 → 09-29 09:49
  after an Observability deploy from a feature branch, restored from `develop`;
- `zero-invariant-1` in ALARM at the start of the window, OK from 09-24 09:21;
- `coverage-sweep-unclassified` in ALARM since 2026-09-28 08:14 (see AC 8).

#### Reproduce it

Every figure above comes from the raw export committed next to the report, so
it re-runs without AWS access:

```sh
python3 docs/scf/milestone-3-monitoring/report.py
python3 docs/scf/milestone-3-monitoring/report.py --self-test
```

`docs/scf/milestone-3-monitoring/data/` holds the CloudWatch output, 1-minute
series included, and the exact queries that produced it. `export.sh` in the same
directory re-pulls it with read access to the production account; CloudWatch
keeps 1-minute points for 15 days, so a re-pull after ~2026-10-08 returns
coarser data than the committed export.

## 6. Work items without a numbered criterion

| Item                                    | State                                                                                                                                                                                       |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| X-Ray tracing enabled end-to-end        | `TracingConfig.Mode: Active` on `api-handler`, `oracle`, `enrichment`, `ledger-processor`; the gateway stage traces too (client IPs are in X-Ray for 30 days, as the privacy policy states) |
| CloudWatch dashboards                   | `prices-production-overview`: API latency and error rate, ingestion lag, ClickHouse write latency, mTLS NotAfter, backfill progress, worker duration/errors, alarm strip (all 65 alarms)    |
| Security review checklist               | see AC 6                                                                                                                                                                                    |
| README, architecture docs, deploy steps | `README.md`, `docs/prices-api-general-overview.md`, `docs/runbooks/`, `infra/README.md` (the fresh-account runbook, AC 7)                                                                   |

## 7. Known issues declared with this submission

Each row is either fixed and verified by submission, or declared here with the
task that owns it. Written from the task ledger, not from memory.

| Issue                                                                                                                                         | State on 2026-09-25                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Task             |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------- |
| Candles were built from every fill, dust included, in the wrong intra-ledger order; live Aquarius ingestion dropped ~50 % of its trades       | Fix live since 2026-09-22; 09-19/20 measured at exactly zero loss. History re-computation running since 2026-09-23 (stage A, pre-Soroban months)                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | 0282, 0286       |
| `pool_registry` had not learned a pool since 2026-07-06; 42 pools missing                                                                     | Seeded 2026-09-18; live persistence deployed 2026-09-22; alarm live; the first new pool was persisted on 2026-09-30 16:52 CEST (Aquarius `CD2CU3DR…`)                                                                                                                                                                                                                                                                                                                                                                                                                                                              | 0291             |
| A new pool's trades are rejected by an execution environment that started before the pool was persisted (the registry is read once, at start) | Seen 2026-09-30: 3 trades of `CD2CU3DR…` rejected between 17:00 and 18:40 CEST, until that environment was replaced at 19:14. Re-ingest of the range and a fix: _task to open_                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| The portal closed itself in an execution environment when Parameter Store throttled its cold start (account default 40 TPS)                   | **Fixed 2026-09-25 14:47 CEST.** The cold start now reads only the mTLS bundle; the portal's sources load on the first portal request, and a failed load answers that request only — the next retries after a 2 s cooldown. The alarm now watches failed loads (`portal-load-failed`). Before the fix it occurred on 2026-09-18 on the load test's 500/1000 req/s ramps (243 environments, found in the logs on 2026-09-21, before any alarm existed) and on 2026-09-24 and 2026-09-25 (both caught by the alarm); `/v1` was unaffected throughout. Since the fix: 0 closures and 0 failed loads (read 2026-09-28) | 0249, 0194, 0311 |
| The oracle worker runs out of memory while the re-ingest re-emits the asset registry (it reads without `FINAL`)                               | One 5-minute tick lost per ~1.5 h cycle until the backfill writes deltas                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | 0226, 0140       |
| Load-test latency describes a box that is now also running the re-ingest                                                                      | Declared beside the AC 5 figures                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | 0293, 0047       |

## 8. What is deliberately not claimed

Milestone 3 is the last tranche, so this section has nowhere to push things:
each row is closed, declared as a deviation, or handed to post-delivery with a
name on it.

| Item                                                        | Disposition                                                                                                                                                                                                             |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Completion of the history re-computation (0286 phase 3)     | _post-delivery_, operator-run; stage A of four started 2026-09-23; values change, coverage does not                                                                                                                     |
| A run of the runbook in an empty AWS account (AC 7)         | _not claimed_: AC 7 rests on the runbook (§5); a run needs the explorer platform and a Hetzner server first. On a reviewer's request: task 0297 reopened                                                                |
| Uptime measured by an external probe (AC 9)                 | _not claimed_: no canary exists; uptime is derived from served requests, which reached the API in 274 of the window's 10,080 minutes (§5, AC 9)                                                                         |
| A standing read-only identity for the Stellar team (AC 8)   | _declared_: none exists by design; access is created per named reviewer on request, MFA enforced (deviations §3)                                                                                                        |
| Paid usage plans and a dashboard that states the key's plan | _not claimed_: live since 2026-09-24 (task 0311), outside the criteria; the evidence covers the free plan                                                                                                               |
| Content-Security-Policy on the portal                       | _not claimed_: none is set. The portal is served by the explorer's distribution, which deliberately sets none, and loads the explorer's Google Tag Manager container since task 0316; a portal CSP was dropped after it |

## 9. Live endpoints and access

| Resource                 | URL / address                                                                                                                  | Access                                                                                                                                                                                                                                   |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Production API base      | `https://prices-api.sorobanscan.rumblefish.dev`                                                                                | `x-api-key`, reviewer key as in M2                                                                                                                                                                                                       |
| OpenAPI document         | `…/api-docs-json`                                                                                                              | **Anonymous**                                                                                                                                                                                                                            |
| Health probe             | `…/health`                                                                                                                     | Anonymous                                                                                                                                                                                                                                |
| `/v1` route groups       | `…/v1/assets`, `…/v1/assets/{id}`, `…/price`, `…/ohlcv`, `POST …/v1/prices/batch`, `…/v1/oracles/{id}`, `…/v1/backfill/status` | `x-api-key`                                                                                                                                                                                                                              |
| Unknown route            | any other path or method                                                                                                       | 404 `not_found` in the error envelope (since 09-24)                                                                                                                                                                                      |
| Onboarding portal        | `https://sorobanscan.rumblefish.dev/api/`                                                                                      | **Anonymous** since 2026-09-23; Discord sign-in for keys                                                                                                                                                                                 |
| API reference (rendered) | `https://sorobanscan.rumblefish.dev/api/docs`                                                                                  | Anonymous                                                                                                                                                                                                                                |
| Privacy policy           | `https://sorobanscan.rumblefish.dev/api/privacy-policy`                                                                        | Anonymous                                                                                                                                                                                                                                |
| Production ClickHouse    | `ch.sorobanscan.rumblefish.dev`, database `prices`                                                                             | mTLS, client certificate on request                                                                                                                                                                                                      |
| CloudWatch dashboard     | `prices-production-overview`, `eu-central-1`                                                                                   | **On request** to a named reviewer: read-only IAM user, MFA enforced, removed after the review — `docs/runbooks/0295-dashboard-access-on-request.md`; request to the operator by e-mail with name, surname, e-mail, purpose and end date |
| Production alarms        | `prices-production-*`, `eu-central-1`                                                                                          | same identity as the dashboard (`DescribeAlarms`, `DescribeAlarmHistory`)                                                                                                                                                                |
| GitHub repository        | `https://github.com/rumblefishdev/stellar-prices-api`                                                                          | **Public**                                                                                                                                                                                                                               |

_Table — live verification endpoints and the access model for reviewers._

## 10. Repository navigation

| Topic                                               | Path                                                                                  |
| --------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Technical design, §9 criteria and their amendments  | `docs/prices-api-general-overview.md`                                                 |
| **Deviations from the criteria wording**            | `docs/scf/milestone-3-rfp-deviations.md`                                              |
| Load-test report (AC 5)                             | `docs/prices-api-load-test-100rps.md`, `docs/loadtest-results/`                       |
| Route/auth/TTL table and reviewer SQL               | `docs/scf/api-endpoints.md`, `docs/scf/ch-demo-queries.sql`                           |
| CI workflow and the integration-test harness (AC 4) | `.github/workflows/ci.yml`, `tools/scripts/ignored-tests.sh`                          |
| OpenAPI lint (AC 2)                                 | `redocly.yaml`, `npm run openapi:lint`                                                |
| Fresh-account deployment runbook (AC 7)             | `README.md` → `infra/README.md` §"Fresh-account deployment"                           |
| REST API, portal, ClickHouse schema, CDK app        | `packages/prices-api/`, `web/portal/`, `packages/prices-clickhouse/schema/`, `infra/` |
| Operator runbooks and ADRs                          | `docs/runbooks/`, `lore/2-adrs/`                                                      |
| Task ledger for this package                        | `lore/1-tasks/active/0294_DOCS_scf-milestone-3-verification-package.md`               |

_Key ADRs for Milestone 3: **0287** (price-forming fills and fill order),
**0292** (`close_usd` zero-as-missing sentinel and its guardrails)._
