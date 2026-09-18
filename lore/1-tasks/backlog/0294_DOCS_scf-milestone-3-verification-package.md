---
id: "0294"
title: "SCF Milestone 3 verification package — evidence doc, form answers, video scenario, deviations"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0102", "0128", "0293", "0260", "0275", "0164", "0249", "0179", "0233", "0239", "0194", "0047"]
tags: [layer-docs, priority-high, effort-medium, milestone-M3, scf, submission, evidence]
milestone: 3
links:
  - "../../../docs/scf/milestone-2-evidence.md"
  - "../../../docs/scf/milestone-2-rfp-deviations.md"
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/prices-api-load-test-100rps.md"
history:
  - date: 2026-09-18
    status: backlog
    who: stkrolikiewicz
    note: >
      Created once the first Tranche 3 criterion had its evidence in the repo
      (AC 5, load test — [[0293]]). M1 and M2 each had a package task
      ([[0102]], [[0128]]); M3 had none, and a walk of the nine criteria on
      2026-09-16 found several with no owner at all. This task owns the
      package and keeps the per-criterion ledger; it does not own closing
      every gap.
  - date: 2026-09-18
    status: backlog
    who: stkrolikiewicz
    note: >
      Corrected within the hour, from the task files rather than from my own
      two-day-old ledger: [[0275]] was activated today and now covers the whole
      workspace (238 ignored tests, not 207, and owned), and [[0239]] is about
      macOS prerequisites for a local deploy — it does NOT own the fresh-account
      `cdk deploy` of AC 7, which has no task.
---

# SCF Milestone 3 verification package

## Summary

Produce the Milestone 3 submission set, in the shape that got Milestones 1 and 2
accepted:

- `docs/scf/milestone-3-evidence.md` — per-criterion evidence, each claim tied
  to a runnable query, a command or a live URL
- `docs/scf/milestone-3-rfp-deviations.md` — every place where what is delivered
  differs from the letter of §9, declared before a reviewer finds it
- `docs/scf/milestone-3-form-answers.md` — the SCF submission form responses
- `docs/scf/milestone-3-video-scenario.md` — the demo walkthrough script
- a refresh of `docs/scf/ch-demo-queries.sql` for anything M3 adds

## Context

Tranche 3 is _"Production Launch & Validation"_
(`docs/prices-api-general-overview.md` §9): nine numbered acceptance criteria
and a **Work** list that is a separate set — two Work items have no numbered
criterion at all, and a ledger built only from the criteria would not show them.

What made the earlier packages credible is worth repeating: every number had a
query behind it, and a section titled **"What is deliberately not claimed"**
listed the gaps with a destination for each. M3 is the last tranche, so that
section has nowhere to push things — each row is either closed, declared as a
deviation, or handed to post-delivery with a name on it.

### Where each criterion stands — 2026-09-18

| AC | Criterion (short) | Evidence / owner | State |
|----|-------------------|------------------|-------|
| 1 | `/backfill/status`: running, fresh push, `earliest_data_available` ≤ 2018-01-01 | amended 2026-09-08 in §9 — the archive completed 2026-07-27; depth met (2015-11-18), liveness graded on freshness alarms. Declared in M2 deviations §4 | carry the deviation forward; re-run the figures |
| 2 | OpenAPI lints clean; Swagger UI deployed | [[0233]] (spec vs portal docs) | polish, not a blocker — confirm the lint output and the URL |
| 3 | Portal accessible; self-service key flow works | [[0164]] (end-to-end proof on production), [[0249]] (api-handler error alarm; portal closes itself at cold start), [[0179]] (SDF consent for the Discord guild — check whether still needed) | **open** |
| 4 | Integration suite passes on CI, link provided | [[0275]] — **active since 2026-09-18 (Adam)**, re-scoped the same day to the whole workspace: 238 `#[ignore]` across 40 files, most needing ClickHouse, the 63 API endpoint tests among them; its inventory decides what the CI job runs | **open, owned** |
| 5 | Load test: p95 <100 ms at 100 req/s, plan named | [[0293]], [[0260]]; `docs/prices-api-load-test-100rps.md` §"Evidence run and the ceiling" | **met 2026-09-18** — AC scenario p95 49.0 ms; miss-only row and the 500/1000 req/s rows beside it |
| 6 | Security checklist: no wildcard IAM, mTLS only, secrets not in env, inputs validated | [[0194]] (audit, archived); 10 `resources: ['*']` statements in `infra/src/lib/stacks/*.ts` need naming and a reason each | needs the table |
| 7 | Repo public; `cdk deploy` from README works in a fresh account | repo is PUBLIC (checked 2026-09-16). The fresh-account deploy has never been rehearsed and **no task owns it** — [[0239]] is adjacent only (two undocumented macOS prerequisites for a *local* deploy) | **half met, the other half unowned** |
| 8 | Dashboard accessible to Stellar via a read-only IAM role; all alarms OK | the role **does not exist in `infra/`; no task** | **open, unowned** |
| 9 | 7-day post-launch report: uptime, error rate, p95, push cadence, `earliest_data_available` | **no task**; needs an agreed definition of "launch" | **open, unowned** |

Work items without a numbered criterion: **X-Ray tracing end-to-end** — met
(`TracingConfig.Mode: Active` on api-handler, oracle, enrichment,
ledger-processor; `tracingEnabled` on the stage, checked 2026-09-16);
**CloudWatch dashboards** — exist, to be listed panel by panel.

### Deviations already known

- **AC 1** — two clauses unsatisfiable since the archive finished early
  (amended in §9, declared for M2).
- **AC 5** — the criterion is met on the scenario it names (98.3 % cache hits).
  The honest companion numbers travel with it: miss-only p95 129.9 ms measured
  from Poland, 45–90 ms at API Gateway; production today is ~4 % cache hits at
  0–1,100 requests a day. The 1000 req/s row is a ceiling, not a pass: the
  shared ClickHouse box saturates between 500 and ~900 req/s.
- **"7 endpoint groups"** (Work list and the conformance wording) vs **five** in
  §4 and in the OpenAPI tags — reconcile before submission; the criterion will
  be read against that number.

## Implementation

- **Evidence document**, one section per criterion, in the M2 layout: the claim,
  the observable it is graded against, the query / command / URL, the figure and
  the date it was re-run. Carry the table above forward and replace each "State"
  with its evidence.
- **Deviations document** from the list above, in the M2 format. A deviation
  declared by us reads as rigour; the same fact found by a reviewer reads as a
  defect.
- **"What is deliberately not claimed"** — for M3 this is the post-delivery list:
  full historical backfill, anything in AC 4 / 8 / 9 not closed by submission,
  the unexplained ~4 % slow mode between Lambda and ClickHouse
  ([[0293]]), the shared-box ceiling ([[0047]]).
- **Live endpoints and access table** — API base URL, key-gated routes, Swagger
  UI, portal, dashboard (with the read-only role once it exists), repo.
- **Form answers and video scenario** — mirror the M2 files; the scenario stays
  on the deployed API and the public portal, as before.
- **Freshness.** Re-run every cited figure close to submission. A month-long
  backfill starts 2026-09-21 on the shared box — latency and load figures taken
  during it describe a different box from the 2026-09-18 load-test numbers.
- **Unowned gaps.** AC 8 (read-only role), AC 9 (7-day report) and the
  fresh-account half of AC 7 have no task. Creating them is the first step
  here — or recording that a criterion will be declared as a deviation instead.

## Acceptance Criteria

- [ ] `milestone-3-evidence.md` covers all 9 Tranche 3 criteria, each with the
      observable it is graded against and a reproducible source
- [ ] Every §9 Tranche 3 Work bullet is addressed, including the two without a
      numbered criterion
- [ ] `milestone-3-rfp-deviations.md` declares every known deviation, including
      AC 1, the AC 5 companion numbers and the endpoint-group count
- [ ] Every criterion that is open on 2026-09-18 is closed, declared as a
      deviation, or listed under "deliberately not claimed" with an owner —
      none is silently absent
- [ ] "What is deliberately not claimed" section present, each row with a
      destination
- [ ] Live endpoints + access table current, including how the Stellar team
      reaches the dashboard
- [ ] `milestone-3-form-answers.md` and `milestone-3-video-scenario.md` written
- [ ] All cited figures re-run within days of submission, with the date beside
      each
- [ ] No claim in the package lacks a task, query or URL behind it

## Notes

- The per-criterion table above is a snapshot. Task statuses move; re-derive it
  from lore before relying on it.
- M2's package took 13 related tasks to assemble. Start the evidence document
  early and fill it as criteria close, rather than writing it at the end.
