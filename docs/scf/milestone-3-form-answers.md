# SCF Milestone 3 — Form Answers (stellar-prices-api)

> Mirrors the Milestone 2 form
> ([`milestone-2-form-answers.md`](milestone-2-form-answers.md)), shortened
> before submission. Every figure is taken from
> [`milestone-3-evidence.md`](milestone-3-evidence.md); none may outrun it.
>
> Copy the text inside each blockquote into the matching field of the Stellar
> Community Fund **Deliverable Verification** form.

---

## Field 1 — Tranche Deliverables

> Deliverable 3 — Production Launch & Validation (as originally approved).
>
> The Stellar Prices API went public on 2026-09-23 at 09:40 CEST. Everything
> below is the production deployment:
>
> 1. Self-service onboarding portal: https://sorobanscan.rumblefish.dev/prices-api/
>    — Discord sign-in for members of the Stellar Developers server, a
>    free-plan key issued at once, usage on the dashboard. The full key flow was
>    recorded on production on 2026-10-05.
> 2. Published contract: an OpenAPI 3.1.0 document generated from the handler
>    code, linted clean in CI with Redocly and rendered in the portal.
> 3. Integration tests in CI against a real ClickHouse: 1,407 unit and 306
>    integration tests, 0 failed, on the release run on master (2026-10-05).
> 4. Load test: 100 req/s for five minutes, p95 49.0 ms against a 100 ms bar,
>    0 errors in 30,001 requests.
> 5. Security: ClickHouse over mutual TLS only, secrets in Secrets Manager,
>    every input validated; the 22 IAM statements with a wildcard resource are
>    each listed with what limits them.
> 6. Observability: X-Ray tracing end to end, the prices-production-overview
>    dashboard, all 83 alarms OK on 2026-10-05.
> 7. Seven days of post-launch monitoring (2026-09-23 to 09-30): uptime
>    100.000 %, 0 × 5XX in 65,806 requests, gateway p95 145.2 ms.
> 8. A public repository with a fresh-account deployment runbook.
>
> SDEX history reaches back to 2015-11-18, past the January 2018 target. Four
> deviations from the wording are declared with their reasons; known issues and
> limitations are listed, each with the task that owns it.
>
> Full evidence: https://drive.google.com/file/d/1AktIqEMocAW_DtaHiISdZTfptMu58Xg2/view?usp=sharing

---

## Field 2 — Deliverable Verification - Video

> https://drive.google.com/file/d/1KTFv9A58sU1FLtnqa4xBYYE8oaCI3DgW/view?usp=sharing
>
> About eight minutes, all on production: the portal and the key flow, the
> OpenAPI reference, CI and the load test, security, operations (backfill,
> dashboard, alarms, monitoring report), the repository and the deviations.

---

## Field 3 — Additional Deliverable Verification

> Evidence package: https://drive.google.com/drive/folders/1XnHTsg1hI7e7XR9z5G1MZ2zuyjVmO2K0?usp=sharing
>
> Deviations: https://github.com/rumblefishdev/stellar-prices-api/blob/master/docs/scf/milestone-3-rfp-deviations.md
>
> 1. Backfill status — depth met: earliest_data_available is 2015-11-18 against
>    2018-01-01; liveness graded on the ingestion signals (deviation 1).
>    GET https://prices-api.sorobanscan.rumblefish.dev/v1/backfill/status
> 2. OpenAPI lints clean, reference deployed — met on the release run on master
>    (2026-10-05): https://sorobanscan.rumblefish.dev/prices-api/docs
>    (deviation 3)
> 3. Portal and self-service keys — met, recorded on 2026-10-05:
>    https://sorobanscan.rumblefish.dev/prices-api/
> 4. Integration suite on CI — met:
>    https://github.com/rumblefishdev/stellar-prices-api/actions/runs/37298469682
> 5. Load test — met: p95 49.0 ms at 100 req/s (deviation 4)
> 6. Security checklist — met: evidence §5, AC 6
> 7. Public repository, deployable from the README — met on the fresh-account
>    runbook, not run in an empty AWS account:
>    https://github.com/rumblefishdev/stellar-prices-api
> 8. Dashboard and alarms — met: all 83 alarms OK on 2026-10-05; read-only
>    access for a named reviewer on request
> 9. 7-day post-launch report — met: uptime 100.000 %, 0 × 5XX in 65,806
>    requests (deviation 2)
>
> No key needed: https://prices-api.sorobanscan.rumblefish.dev/api-docs-json and
> https://prices-api.sorobanscan.rumblefish.dev/health. The seven /v1 route
> groups need an x-api-key: the Milestone 2 reviewer key works, or issue one in
> the portal.

---

## Field 4 — Support Needed

> None.

---

## Pre-submission checklist

Checked 2026-10-06.

- [x] Every figure in Fields 1 and 3 appears, with the same value and date, in
      the evidence package
- [x] Every deviation the package declares is mentioned in Field 3 by number
      → deviations 1–4, all four the deviations document declares
- [x] Every `<TO FILL: …>` and `<ANGLE_BRACKET>` placeholder replaced
- [x] The reviewer key still works and is on the free plan
      → `prices-production-scf-reviewer-key-20260909T120021Z`: enabled, on
      `pricing-api-free-production`
- [x] `develop` released to `master`: the repository's default branch shows
      the root `README.md` and the fresh-account runbook (AC 7), and every
      `.../blob/master/...` link above resolves
      → release #381 (2026-10-05), the package in #391
- [x] The first CI run on `master` after the release is green in every job,
      `XDR protocol lag` included (fixed by task 0325, PR #383)
      → run 37298469682, all four jobs green
- [x] The 0139 window is closed: alarms back to OK, the alarm count updated
      wherever "65" appears (evidence, video script, Field 1 item 6)
      → 83 of 83 `prices-production-*` alarms OK; no alarm count of 65 left
- [x] Portal links use `/prices-api/`, and an old `/api/` link still answers
      `301` on submission day
      → `/api/` and `/api/dashboard` answer `301` to `/prices-api/…`
- [x] The video was recorded against the public deployment, no secrets on
      screen (see the scenario's ⚠️ section)
