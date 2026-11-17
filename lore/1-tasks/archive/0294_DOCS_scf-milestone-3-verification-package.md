---
id: "0294"
title: "SCF Milestone 3 verification package — evidence doc, form answers, video scenario, deviations"
type: DOCS
status: completed
related_adr: []
related_tasks: ["0102", "0128", "0293", "0260", "0275", "0164", "0249", "0179", "0233", "0239", "0194", "0047", "0295", "0296", "0297", "0311", "0101", "0139", "0274"]
tags: [layer-docs, priority-high, effort-medium, milestone-M3, scf, submission, evidence]
milestone: 3
links:
  - "../../../docs/scf/milestone-3-evidence.md"
  - "../../../docs/scf/milestone-3-rfp-deviations.md"
  - "../../../docs/scf/milestone-2-evidence.md"
  - "../../../docs/scf/milestone-2-rfp-deviations.md"
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/prices-api-load-test-100rps.md"
history:
  - date: 2026-10-06
    status: completed
    who: stkrolikiewicz
    note: >
      Submitted: Aga sent the SCF Deliverable Verification form on 2026-10-06
      with the shortened answers of #392. The package —
      milestone-3-evidence.md (510 lines, 16-page PDF), four declared
      deviations, the form answers, the video scenario and a 7:39 film —
      merged in #354 (2026-10-05 17:27 UTC) and reached master in #391 three
      minutes later; every repo link the PDF and the form make answered 200
      on master. The PDF, the film and their folder are on Google Drive with
      link sharing. The form's G- and C-address fields do not apply: the API
      owns no Stellar account and no contract. Closing decisions and the gaps
      found on the way out are under Design Decisions and Issues Encountered.
  - date: 2026-09-29
    status: active
    who: claude
    note: >
      AC 8 holds again: production has no IAM user (list-users empty after
      the Observability deploy of 09:49 from origin/develop @ 8302e00d). It
      did not hold 09-25 14:48 → 09-29 09:49: the 14:48 deploy from
      [[0311]]'s branch (#351, without #349 and #355) recreated the 0125
      viewer and deleted the two asset-discovery liveness alarms (details in
      [[0295]] and [[0256]]). 65 alarms again; the one not OK is
      coverage-sweep-unclassified (ALARM since 09-28 08:14, the first probe
      run found the sda aggregator; allow-listed in #358, merged 09-28 11:02;
      whether the deployed probe carries it was not checked). Access
      runbooks: explicit Deny without MFA (#359, merged) and an IAM Identity
      Center path (#364, open, not walked). Before submission, re-read any
      sentence in the package that says "no IAM user" with a date between
      09-25 and 09-29.
  - date: 2026-09-28
    status: active
    who: stkrolikiewicz
    note: >
      Measured with Logs Insights over the api-handler log: [[0293]]'s load
      test of 2026-09-18 closed the portal 243 times, all in three bursts on
      the 500/1000 req/s ramps (157, 278 and 255 cold starts within 1–2 s);
      bursts of 26–68 cold starts per second closed nothing. The AC 5 run had
      41 cold starts and no closure or SSM throttle, so 49.0 ms stands. The
      side effect is in neither 0293 nor docs/loadtest-results — added to the
      known-issues row and the AC 5 bullet. Not a reason to re-run. The cause
      is fixed: [[0311]] moved the portal's source reads off the cold start,
      live since 2026-09-25 14:47 CEST (PR #351, merged 09-28); 0 closures
      and 0 failed loads since.
  - date: 2026-09-25
    status: active
    who: stkrolikiewicz
    note: >
      Activated. The package files did not exist a week after the task was
      filed; the launch is now dated (09-23 09:40) and AC 5, AC 4 (CI runs the
      ClickHouse suite since #327) and AC 7's first half (repo PUBLIC) have
      evidence in the repo, so the skeleton can be written from facts rather
      than placeholders. Branch docs/0294_scf-milestone-3-verification-package:
      milestone-3-evidence.md in the M2 layout, milestone-3-rfp-deviations.md,
      and stubs for the form answers and the video scenario. (The file move
      went out one commit earlier with the status still `backlog` — a script
      assertion tripped on a repeated phrase; corrected here.)
  - date: 2026-09-25
    status: backlog
    who: stkrolikiewicz
    note: >
      Launch date for row 9 recorded: 2026-09-23 09:40 CEST (07:40:45 UTC),
      the moment the explorer's basic auth came off /api/* and the portal
      became public — agreed at the 2026-09-24 daily, carried in [[0296]]
      with the window 09-23 09:40 → 09-30 09:40. Deltas since the 09-18 table
      (the table itself is kept as that day's snapshot): row 4 — [[0275]]
      merged 2026-09-22 (#327), CI now runs the ClickHouse integration
      suite; row 3 — portal public since 09-23, [[0249]]'s alarms live since
      09-22 and fired for real on 09-24; row 6 — `prices_admin` (explorer
      0567) is a scoped admin identity, no new wildcard. The "Freshness"
      bullet corrected: the re-ingest started 09-23, not 09-21. A running
      list of known issues to declare is opened below.
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
| 7 | Repo public; `cdk deploy` from README works in a fresh account | repo is PUBLIC (checked 2026-09-16). The fresh-account deploy has never been rehearsed — [[0297]]; [[0239]] is adjacent only (two undocumented macOS prerequisites for a *local* deploy) | **half met; the other half owned, not started** |
| 8 | Dashboard accessible to Stellar via a read-only IAM role; all alarms OK | the role does not exist in `infra/` — [[0295]]; needs the Stellar team's AWS account id or their preference for a shared link | **open, owned, not started** |
| 9 | 7-day post-launch report: uptime, error rate, p95, push cadence, `earliest_data_available` | [[0296]]; blocked on an agreed definition of "launch"; two of its five metrics are obsolete since the archive completed | **open, owned, blocked on a decision** |

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
  Beside the 500/1000 rows, declare a side effect 0293 did not record: their
  ramps closed the portal in 243 execution environments (198 of a fleet
  capped at 700 in the 1000 run) — see the portal row under Known issues.
  `/v1` answered throughout. The AC-scenario run (09:33–09:44 UTC) had 41
  cold starts, 0 closures and 0 SSM throttles, so the 49.0 ms is untouched.
  No re-run for this. The fix is live since 2026-09-25 14:47 ([[0311]]), so
  the next ramp is its verification (0 SSM reads at a `/v1` cold start);
  run it after the launch window (09-30 09:40), and state that the box is
  then running [[0286]]'s re-ingest.
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
- **Freshness.** Re-run every cited figure close to submission. [[0286]]'s
  phase-3 re-ingest runs on the shared box from 2026-09-23 16:10 CEST (stage A;
  ~22–28 days in total) — latency and load figures taken during it describe a
  different box from the 2026-09-18 load-test numbers.
- **Launch, for row 9 and the evidence narrative:** 2026-09-23 09:40 CEST, the
  explorer's basic auth off `/api/*` ([[0305]], explorer 0574). The 7-day
  window and its incident log live in [[0296]].

### Known issues to declare — running list (opened 2026-09-25)

Each row is either fixed-and-verified by submission or declared as a known
issue with its task. Kept here so the package is not written from memory.

| issue | state | task |
|---|---|---|
| Candles built from every fill, dust included, in the wrong intra-ledger order; live Aquarius ingestion dropped ~50 % of its trades | fix live since 2026-09-22 (phase 1), 09-19/20 measured at exactly zero loss; history re-ingest in progress from 2026-09-23 (stage A 16/99 months on 09-24) | [[0282]], [[0286]] |
| `pool_registry` had not learned a pool since 2026-07-06; 42 pools missing | seeded 2026-09-18, live persistence deployed 2026-09-22, alarm live; the "first new pool" production check still open | [[0291]] |
| Portal closes itself in an execution environment when Parameter Store throttles its cold start (account default 40 TPS; each cold start reads 3 parameters, even one triggered by `/v1`); happened 2026-09-18 on [[0293]]'s 500/1000 req/s ramps (243 closures from bursts of 157, 278 and 255 cold starts in 1–2 s; 26–68 per second closed nothing — Logs Insights, read 2026-09-28), 2026-09-24 under a teammate's request burst (69 cold starts, 7 closures) and 2026-09-25 under a teammate's k6 run for 0311 (concurrency 200) | **fixed 2026-09-25 14:47 CEST** by [[0311]] (PR #351, merged 09-28): the cold start reads only the mTLS bundle; the portal's sources load on the first portal request, a failed load answers that request only and the next retries after a 2 s cooldown; alarm `portal-load-failed` replaced `portal-closed`. 0 closures and 0 failed loads since (Logs Insights, read 2026-09-28). 09-24 and 09-25 were caught by the old alarm ([[0249]]); 09-18 predates it, was found in the logs on 09-21 and never reached 0293's report | [[0249]], 0194, [[0311]] |
| Oracle OOMs while the re-ingest re-emits the asset registry (reads without `FINAL`) | one 5-minute tick lost per ~1.5 h cycle until the backfill writes deltas | [[0226]], [[0140]] |
| Load-test latency describes a box that is now also running the re-ingest | declared beside the 0293 figures | [[0293]], [[0047]] |
- **The three gaps that had no owner now have tasks**: [[0295]] (AC 8,
  read-only dashboard access), [[0296]] (AC 9, 7-day report) and [[0297]]
  (AC 7, fresh-account deploy). Each may end as a declared deviation rather
  than a closed criterion — that outcome belongs in this package too.

## Acceptance Criteria

- [x] `milestone-3-evidence.md` covers all 9 Tranche 3 criteria, each with the
      observable it is graded against and a reproducible source — **§5**, each
      with a command, a live URL or a CI run, and the date it was read.
- [x] Every §9 Tranche 3 Work bullet is addressed, including the two without a
      numbered criterion — **§6**: X-Ray tracing end to end and the CloudWatch
      dashboard.
- [x] `milestone-3-rfp-deviations.md` declares every known deviation, including
      AC 1, the AC 5 companion numbers and the endpoint-group count — **four**:
      AC 1 (the backfill finished early), AC 9 (two metrics flat since), AC 2
      (Redocly and the portal's renderer in place of `openapi-validator` and
      Swagger UI), AC 5 (scope and companion numbers). The endpoint-group count
      turned out not to be a deviation (Design Decision 7).
- [x] Every criterion that is open on 2026-09-18 is closed, declared as a
      deviation, or listed under "deliberately not claimed" with an owner —
      none is silently absent. AC 3, 4, 8 and 9 closed; AC 7 met on the
      fresh-account runbook, not run in an empty account ([[0297]] reopens on a
      reviewer's request, §8); AC 6's 22 wildcard-resource statements each
      named with what limits them.
- [x] "What is deliberately not claimed" section present, each row with a
      destination — as **§7 Known issues** and **§8 Limitations**, each row
      naming its task (Design Decision 6).
- [x] Live endpoints + access table current, including how the Stellar team
      reaches the dashboard — **§9**: read-only access per named reviewer
      through IAM Identity Center ([[0295]]'s runbook).
- [x] `milestone-3-form-answers.md` and `milestone-3-video-scenario.md` written
      — the form went out with the shortened answers of #392.
- [x] All cited figures re-run within days of submission, with the date beside
      each — AC 2, 3, 4, 6 and 8 on 2026-10-05, AC 9's window 09-23 → 09-30.
      **Two exceptions, both dated in the package:** AC 5 (2026-09-18, no
      re-run by the decision of 09-28) and AC 1 (2026-10-01, Design Decision 3).
- [x] No claim in the package lacks a task, query or URL behind it

## Implementation Notes

- `docs/scf/`: `milestone-3-evidence.md` (510 lines, §1–§10),
  `milestone-3-rfp-deviations.md`, `milestone-3-form-answers.md`,
  `milestone-3-video-scenario.md`, `screenshots/m3-ac2-api-reference.png`,
  and `repo-links.lua`, which `build-pdf.sh` now runs. Side edits: the
  load-test report, the 0295 dashboard-access runbook, the monitoring report
  and the general overview.
- PRs: #354 (the package), #391 (release to master), #392 (shortened answers).
  The AC 2 lint and the AC 4 test counts come from the master run of the
  earlier release #381 (37298469682).
- Google Drive, link-shared: the evidence PDF, the film (15 clips stitched,
  7:39) and the folder holding both.

## Issues Encountered

- **Dead links in the PDF.** Typst renders a relative Markdown link as an
  in-document reference, so every link to a repo document went nowhere.
  Reported on 2026-10-05; fixed by `repo-links.lua` (Design Decision 4).
- **Two sentences written before the release reached Drive**: AC 7's
  "on `master` from the next release" and the access table's "request by
  e-mail". Fixed (b943f811, a5816d8e); each PDF went up as a new version under
  the same Drive link, checked by its size each time.
- **The film.** QuickTime's variable-frame-rate clips froze their picture, and
  one clip's narration ran 12 s late; the stitch padded the frames and cut
  12 s of silence to resync. The ffmpeg scripts were not kept in the repo.
- **Not in the package:** [[0101]]'s AMM history holes (Soroswap
  2026-07-06 → 07-11, Phoenix ~2 % short; blocked) are named in neither the
  Milestone 2 nor the Milestone 3 known issues. Found at closing.

## Design Decisions

### From Plan

1. **The M2 layout and the same four documents**: evidence per criterion with
   its observable, source and date; deviations in their own document; form
   answers and video scenario mirrored from M2.
2. **Launch at 2026-09-23 09:40 CEST** for the AC 9 window and the narrative
   ([[0296]]).

### Emerged

3. **AC 1 stays at its 2026-10-01 reading** (decided 2026-10-05). A re-read
   that day found that the explorer's Galexie export stalled from 2026-10-01
   19:08 to 10-02 09:07 CEST: no ledger reached the pipeline for 14 h,
   `ledger-processor-no-invocations` and `rollup-freshness-1m` fired within
   25 minutes, and `ledger-processor-lag` stayed in ALARM 09:17–13:27 while
   the backlog drained. The [[0139]] window then held the writers
   14:08–14:57. Both postdate the reading; neither is in the package.
4. **Repo links in the PDF point at `blob/master`** (`repo-links.lua`): a PDF
   has no repository beside it. `build-pdf.sh` stops if the filter no longer
   rewrites.
5. **No published address for dashboard access** (decided 2026-10-05): the
   evidence says "on request", the submitted Field 4 is "None." and the access
   line sits in AC 8.
6. **"What is deliberately not claimed" became §7 Known issues and §8
   Limitations.** M3 is the last tranche, so no row can point at a later
   milestone; each names its task instead.
7. **The endpoint-group count is not a deviation**: the evidence counts seven
   `/v1` routes as the M2 package did, and the design document's §4 files the
   same routes under five headings (evidence §5, AC 4).
8. **Answers shortened before submission** (#392): about 3,700 characters
   instead of 8,800, every figure and date unchanged, detail left to the PDF.
9. **`ch-demo-queries.sql` not refreshed**: M3's evidence runs on the API,
   CloudWatch and CI, and adds no ClickHouse query.
10. **#391 released #373 ([[0274]], `price_basis`) to master before it was
    deployed** — declared in #391's description. The package does not
    describe that field.

## Future Work

None spawned. Post-delivery work stays with the tasks the evidence names in §7
and §8.

## Notes

- The per-criterion table above is a snapshot. Task statuses move; re-derive it
  from lore before relying on it.
- M2's package took 13 related tasks to assemble. Start the evidence document
  early and fill it as criteria close, rather than writing it at the end.
