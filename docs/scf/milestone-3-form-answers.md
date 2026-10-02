# SCF Milestone 3 — Form Answers (stellar-prices-api)

> ⚠️ **DRAFT — first full draft 2026-10-02.** Mirrors the Milestone 2 form
> ([`milestone-2-form-answers.md`](milestone-2-form-answers.md)). Every figure is
> taken from [`milestone-3-evidence.md`](milestone-3-evidence.md); none may
> outrun it. Anything in `<TO FILL: …>` waits for the release run on `master`,
> the recording, or the asset-id migration (task 0139); anything in
> `<ANGLE_BRACKETS>` is a link to place on submission day.
>
> Copy the text inside each blockquote into the matching field of the Stellar
> Community Fund **Deliverable Verification** form.

---

## Field 1 — Tranche Deliverables

> **Deliverable 3 — Production Launch & Validation** (as originally approved).
>
> Milestone 3 opens the Stellar Prices API to developers. The service went
> public on **2026-09-23 at 09:40 CEST**, when the staging password came off
> the onboarding portal; the API itself had been serving on its custom domain
> since Milestone 2. Everything below is the production deployment.
>
> What is live and verifiable today:
>
> 1. **A self-service onboarding portal.** At
>    `https://sorobanscan.rumblefish.dev/prices-api/` (at `/api/` until
>    2026-10-02; every old link answers 301 to the same page). Sign-in is
>    through Discord, and eligibility is membership in the official Stellar
>    Developers server. A key is issued at once on the free plan, one request a
>    second and a hundred thousand a month, and the dashboard shows it with its
>    plan, limits and usage. The quick start matches the published contract,
>    and the portal carries its own privacy policy. `<TO FILL: the recorded
walk — sign-in, key, a /v1 call, revocation — with its timestamps>`
> 2. **A published contract.** The API serves its own OpenAPI 3.1.0 document
>    at `/api-docs-json`, generated from the handler code, with 74
>    production-shaped examples. CI lints it on every Rust change with
>    Redocly's strict ruleset, and the portal renders it as an API reference.
>    `<TO FILL: lint summary line from the release run>`
> 3. **Integration tests in CI against a real database.** Since 2026-09-22,
>    CI starts ClickHouse, applies the schema, starts the mTLS proxy and runs
>    the integration suite against it, covering each of the seven `/v1`
>    routes. `<TO FILL: test count and the link to the first green run on
master after the release>`
> 4. **Load-tested at the approved target.** 100 requests per second for five
>    minutes: **p95 49.0 ms** against a 100 ms bar, **0 errors in 30,001
>    requests**, on the purpose-built plan the criterion asks us to name. The
>    report also carries the miss-only row and the 500 and 1000 req/s rows.
> 5. **A security review.** ClickHouse is reachable only over mutual TLS; the
>    client certificates live in Secrets Manager, not in environment variables;
>    every invalid input gets a 400. No IAM statement grants every action or a
>    whole service, and every statement with a wildcard resource is listed
>    with what limits it. `<TO FILL: wildcard-resource count from the release
templates>`
> 6. **Observability.** X-Ray tracing is active on the API handler, the ledger
>    processor, the oracle and the enrichment worker, and on the gateway stage.
>    The `prices-production-overview` dashboard covers API latency and errors,
>    ingestion lag, ClickHouse write latency, the workers and an alarm strip with
>    every `prices-production-*` alarm. `<TO FILL: alarm count (83 on
2026-10-02, was 65) and the "all OK" date>`
> 7. **Seven days of post-launch monitoring.** From 2026-09-23 09:40 to
>    2026-09-30 09:40 CEST: **uptime 100.000 %, 0 × 5XX in 65,806 requests,
>    gateway p95 145.2 ms.** The report recomputes from a raw CloudWatch export
>    kept in the repository, so a reviewer can rerun it without AWS access.
> 8. **A public repository.** Public since 2026-09-16, with a root README
>    leading to the architecture documents, the operator runbooks and a
>    fresh-account deployment runbook.
>
> The tranche's backfill target, SDEX history back to January 2018, was
> reached early, in Milestone 2: the archive completed on 2026-07-27 and
> reaches back to **2015-11-18**.
>
> **Declared deviations.** Five places where the delivery departs from the
> wording, each with its reason in the deviations document: the backfill that
> finished early (so liveness is graded on the ingestion signals), the two
> monitoring-report metrics it made flat, dashboard access given to a named
> reviewer instead of a standing role, Redocly and the portal's own renderer
> in place of `openapi-validator` and Swagger UI, and the scope of the
> load-test figure.
>
> **Known issues and limitations are listed, each with the task that owns it:**
> the history recomputation that finishes after delivery, the asset-id
> collisions fixed on 2026-10-02 (task 0139), with their older months still
> being re-ingested, a slow path we measured but did not instrument, and the
> ceiling of the database server we share with the Soroban Block Explorer.
>
> **Full evidence — acceptance-criteria mapping with runnable commands, the
> deviation rationale, known issues and limitations:**
> `<EVIDENCE_PDF_URL>`

---

## Field 2 — Deliverable Verification - Video

> `<VIDEO_URL>`
>
> About seven minutes, all on production: (1) scope; (2) the portal, end to
> end — sign-in, a key, a `/v1` call with it, revocation; (3) the OpenAPI
> document, the rendered reference and a 404 in the error envelope; (4) the
> integration tests in CI and the load test; (5) the security checklist;
> (6) operations — backfill status against Horizon, the dashboard and its
> alarms, the monitoring report; (7) the public repository and the
> fresh-account runbook; (8) the deviations and limitations.

---

## Field 3 — Additional Deliverable Verification

> **Evidence package (Google Drive):** `<EVIDENCE_FOLDER_URL>`
>
> - Evidence PDF: `<EVIDENCE_PDF_URL>`
> - Demo video: `<VIDEO_URL>`
>
> The nine acceptance criteria, in order. Deviations are set out once, in
> `.../blob/master/docs/scf/milestone-3-rfp-deviations.md`, and referred to
> here by number.
>
> 1. **Backfill status: depth met by two years.** `earliest_data_available`
>    is 2015-11-18 against a 2018-01-01 bar; liveness is graded on the
>    ingestion alarms and `realtime_tip_ledger` (deviation 1).
>    `GET https://prices-api.sorobanscan.rumblefish.dev/v1/backfill/status`
> 2. **OpenAPI lints clean; reference deployed.** `<TO FILL: lint summary
line from the release run>` Reference:
>    `https://sorobanscan.rumblefish.dev/prices-api/docs` (deviation 4).
>    Reproduce: `npm run openapi:lint`
> 3. **Portal accessible; self-service key flow works.**
>    `https://sorobanscan.rumblefish.dev/prices-api/`, anonymous since
>    2026-09-23; Discord sign-in for keys. `<TO FILL: the recorded walk with
its timestamps>`
> 4. **Integration suite passes on CI.** `<TO FILL: link to the first green
run on master after the release, and its test count>` Reproduce locally:
>    `tools/scripts/ignored-tests.sh`
> 5. **Load test: met.** p95 49.0 ms at 100 req/s for five minutes, 0 errors
>    in 30,001 requests, plan `prices-production-loadtest-plan`; scope in
>    deviation 5. Report:
>    `.../blob/master/docs/prices-api-load-test-100rps.md`
> 6. **Security checklist: met.** mTLS-only ClickHouse, secrets in Secrets
>    Manager, validated inputs, no `Action: "*"` or `service:*`; every
>    `Resource: "*"` statement inventoried in evidence §5, AC 6.
> 7. **Repository public; deployable from the README: met on the
>    fresh-account runbook.** `https://github.com/rumblefishdev/stellar-prices-api`,
>    `README.md` → `infra/README.md` §"Fresh-account deployment". Not run in an
>    empty AWS account; the claim rests on the runbook, a credential-free synth
>    in CI and a name-by-name check of the runbook against the code.
> 8. **Dashboard and alarms: met on the amended wording.**
>    `prices-production-overview`, `eu-central-1`; `<TO FILL: all N alarms
OK on DATE>`. Read-only access for a named reviewer on request
>    (deviation 3; see Field 4).
> 9. **7-day post-launch report: met.** Uptime 100.000 %, 0 × 5XX in 65,806
>    requests, gateway p95 145.2 ms; two named metrics replaced by live
>    signals (deviation 2). Reproduce offline:
>    `python3 docs/scf/milestone-3-monitoring/report.py`
>
> **Live & anonymous:**
>
> - OpenAPI document: `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`
> - Rendered API reference: `https://sorobanscan.rumblefish.dev/prices-api/docs`
> - Health probe: `https://prices-api.sorobanscan.rumblefish.dev/health`
>
> **Key-gated** (`x-api-key`; the Milestone 2 reviewer key works, or issue
> one in the portal): the seven `/v1` route groups listed in evidence §9.

---

## Field 4 — Support Needed

> — No support is needed to complete or verify this tranche: nothing in it
> depends on the Stellar side. SDF settled the Discord integration by asking
> the project to run it itself.
>
> To open the CloudWatch dashboard and the alarms, send your name, surname,
> e-mail, purpose and the end date of access to `<ACCESS_REQUEST_EMAIL>`; a
> read-only IAM Identity Center user is issued for the review window.

---

## Pre-submission checklist

- [ ] Every figure in Fields 1 and 3 appears, with the same value and date, in
      the evidence package
- [ ] Every deviation the package declares is mentioned in Field 3 by number
- [ ] Every `<TO FILL: …>` and `<ANGLE_BRACKET>` placeholder replaced
- [ ] The reviewer key still works and is on the free plan
- [ ] `develop` released to `master`: the repository's default branch shows
      the root `README.md` and the fresh-account runbook (AC 7), and every
      `.../blob/master/...` link above resolves. On 2026-09-28 `master` still
      had neither
- [ ] The first CI run on `master` after the release is green in every job,
      `XDR protocol lag` included (fixed by task 0325, PR #383)
- [ ] The 0139 window is closed: alarms back to OK, the alarm count updated
      wherever "65" appears (evidence, video script, Field 1 item 6)
- [ ] Portal links use `/prices-api/`, and an old `/api/` link still answers
      `301` on submission day
- [ ] The video was recorded against the public deployment, no secrets on
      screen (see the scenario's ⚠️ section)
- [ ] `<ACCESS_REQUEST_EMAIL>` is a shared address, not a person's
