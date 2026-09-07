---
id: "0128"
title: "SCF Milestone 2 verification package — evidence doc, form answers, video scenario"
type: DOCS
status: active
related_adr: []
related_tasks: ["0102", "0117", "0120", "0121", "0122", "0123", "0124", "0125", "0126", "0127", "0237", "0248", "0262"]
tags: [layer-docs, priority-high, effort-medium, milestone-M2, scf, submission, evidence]
milestone: 2
links:
  - "../../../docs/scf/milestone-1-evidence.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Mirrors task 0102,
      which produced the Milestone 1 package (`docs/scf/milestone-1-evidence.md`,
      `milestone-1-form-answers.md`, `milestone-1-video-scenario.md`) and was
      accepted. Last task in the M2 sequence — it consumes every other task's
      output.
  - date: 2026-09-07
    status: active
    who: okarcz
    note: >
      📦 **Package drafted — 9 of 10 ACs closed.** Four deliverables:
      `milestone-2-evidence.md` (999 lines, 18-page PDF),
      `milestone-2-form-answers.md`, `milestone-2-video-scenario.md`, and
      queries (10)-(22) appended to `ch-demo-queries.sql`. `build-pdf.sh` is
      now parameterised (`./build-pdf.sh 2`) instead of hardcoded to M1.
      ⏳ **Only "re-run every cited figure close to submission" is open**, by
      design. 🔑 Two contradictions inside the package were found and fixed
      rather than shipped: the load-test report and its README both claimed the
      gateway "caches on the path only", which [[0122]] had falsified (the
      arithmetic survives because the k6 script sends no query string, but the
      precondition now travels with the figure); and `docs/scf/README.md` still
      forbade screenshotting the dashboard as an empty scaffold, which [[0125]]
      replaced the same morning.
  - date: 2026-09-07
    status: active
    who: okarcz
    note: >
      📥 **AC 1 evidence is in — [[0120]] closed the same day, and with it the
      last unmet Tranche 2 criterion.** Cite: **1032 pass, 0 fail, 0 skip**,
      exit 0, on production 2026-09-07 at 10:40 and again at 11:09 UTC,
      reproducible with `npm run conformance:0120`. Reports are gitignored as
      regenerable, so the numbers in 0120 are the citation. ⚠️ **Two things
      this package must state rather than smooth over**: the check count rose
      886 → 1032 because assertions were *added*, and **no production code
      changed** — every failure that disappeared was a defect in the test, not
      a fix to the API. Also fold in [[0230]]'s determinism proof (1,032
      identical verdicts across two runs while 53 details moved), because it is
      what makes the report citable rather than a snapshot.
  - date: 2026-09-07
    status: active
    who: okarcz
    note: >
      Activated alongside [[0262]], which is the last input this package was
      waiting on. 0262 produced the Tranche 2 AC 3 position: the criterion is
      graded against a reworded observable, declared in
      `docs/scf/milestone-2-rfp-deviations.md` rather than negotiated with the
      reviewer in advance. AC 3's entry in this package must name that
      observable and label the latency evidence as the weaker claim.
---

# SCF Milestone 2 verification package

## Summary

Produce the Milestone 2 submission set, following the shape that got Milestone 1
accepted:

- `docs/scf/milestone-2-evidence.md` — per-AC evidence with reproducible queries
- `docs/scf/milestone-2-form-answers.md` — the SCF submission form responses
- `docs/scf/milestone-2-video-scenario.md` — the demo walkthrough script
- a refresh of `docs/scf/ch-demo-queries.sql` for anything M2 adds

## Context

Milestone 1's package worked because of a specific discipline worth repeating:
every claim was tied to a runnable query or a live URL, and **Section 6 —
"What is deliberately not claimed"** listed the gaps honestly, with a
destination for each. That table is what makes the rest of the document
credible, and it is also what created the M2 scope this task now has to close
(three of its rows became [[0124]], [[0125]], [[0126]]).

M2 must do the same for M3 — and M3 is Tranche 3, *"Production Launch &
Validation"*, so the honest gap list is short and mostly known already:
Swagger UI, the onboarding portal, the integration suite in CI, the security
review, the public repo, and the 7-day post-launch report.

## Implementation

- **Evidence document**, one section per Tranche 2 acceptance criterion:

  | AC | Claim | Evidence from |
  |----|-------|---------------|
  | 1 | 7 endpoint groups, correct + schema-valid, 20 assets | [[0120]] |
  | 2 | Load test 100 req/s, p95 <200ms, errors <0.1% | [[0121]] |
  | 3 | Cache hits within TTL | [[0122]] |
  | 4 | VWAP verifiable against raw rows, ≥3 assets | [[0123]] |
  | 5 | `earliest_data_available` ≤ 2022-01-01 | [[0127]] |
  | 6 | `timeframe=all` USDC from ≥ Jan 2022, 1d candles spot-checked | [[0127]] |

  Plus the §9 work bullets that have no numbered AC — full VWAP formula
  ([[0072]] + [[0118]]), outlier detection ([[0072]]), Aquarius as a named
  source ([[0072]]/[[0080]], observed in [[0120]]), input validation
  ([[0119]]) — and the three M1-deferred items ([[0124]], [[0125]],
  [[0126]]).

- **Reproducibility.** Every number gets a query or a command a reviewer can run
  themselves, in the M1 style. Include the live endpoint/access table (API base
  URL — updated for the custom domain if [[0126]] landed — key-gated routes,
  CH access, dashboard read-only role, repo link).

- **"What is deliberately not claimed"** — the M3 scope list above, each row
  with a destination, plus any M2 defect found and deferred rather than fixed.

- **Form answers** — mirror `milestone-1-form-answers.md`, updated for the T2
  deliverable and budget line.

- **Video scenario** — a walkthrough of the public API a reviewer can follow:
  list assets, drill into one, pull its price with the `sources` breakdown,
  pull OHLCV at two granularities, batch a few assets, show the oracle
  cross-reference, show `/backfill/status`, show the dashboard. Keep it to the
  deployed API — M1's scenario deliberately narrated live URLs rather than
  local runs, and that is what made it verifiable.

- **Freshness.** Re-run the citable checks close to submission. Numbers drift:
  the backfill advances, `earliest_data_available` moves, coverage percentages
  change. A stale figure is the easiest avoidable error in this document.

## Acceptance Criteria

- [x] `milestone-2-evidence.md` covers all 6 Tranche 2 ACs, each with
      reproducible evidence — **written 2026-09-07, 999 lines**, §5. Each
      criterion carries measured figures, a runnable command, and its own
      stated limits rather than a footnote.
- [x] 🔴 **DONE — AC 3's entry names the observable the criterion was graded against,
      and labels the latency evidence as the weaker claim in those words** —
      *a header would be the cache asserting itself; latency is behaviour
      consistent with a cache*. Not blurred, not softened. **Inherited from
      [[0262]], which closed on this hand-off** — 0262 settled everything else
      (ADR 0012; the amendment declared in
      `docs/scf/milestone-2-rfp-deviations.md` §2 and noted in place against
      AC 3 in `docs/prices-api-general-overview.md` §12). ⚠️ Fold §2 in **by
      reference**; reproducing its argument makes a fourth copy that drifts.
- [x] Every §9 Tranche 2 work bullet addressed, including those without a
      numbered AC — **§6**: the VWAP formula and threshold, outlier detection,
      Aquarius as a named source, input validation. ⚠️ §6.1 corrects §9's own
      wording: the formula runs in a ClickHouse MV, not a "Current Price
      Updater Lambda".
- [x] The three M1-deferred items ([[0124]], [[0125]], [[0126]]) are shown as
      delivered — **§7**, all three, with the dashboard screenshots embedded
      and both deploy failures during 0126 disclosed.
- [x] "What is deliberately not claimed" section present, with a destination
      for every row — **§8, 18 rows**, each pointing at a task or a tranche.
- [x] Live endpoints + access table current — **§9**. Custom domain
      throughout, `/api-docs-json` marked anonymous, dashboard viewer listed,
      and a note that the retired `execute-api` URLs in the M1 package were
      amended in place rather than left dead.
- [x] `milestone-2-form-answers.md` and `milestone-2-video-scenario.md`
      complete — form answers across all four fields with the deviations
      declared in Field 1 body; video scenario is 8 scenes, 6-7 min, entirely
      against the deployed API, with scene 6 built around stating the missing
      header honestly.
- [x] `ch-demo-queries.sql` refreshed for M2 additions — **queries (10)-(22)**
      appended under a MILESTONE 2 banner. Column names verified against
      `init.sql` rather than assumed (two were wrong on the first pass).
- [ ] ⏳ **All cited figures re-run within days of submission** — the only
      criterion that cannot be closed early by design. Checklist is at the
      foot of `milestone-2-form-answers.md`.
- [x] No claim in the package lacks a task, query, or URL behind it.

## Notes

- 📥 **INHERITED 2026-09-02 from [[0248]]: quote or link
  `docs/prices-api-general-overview.md` §5.7.** 0248 needed a home for the
  RFP-deviation answer on Blend (the RFP names four aggregation markets, we
  ingest three plus Phoenix) and this package did not exist, so the operator
  ruled it a permanent project fact and it landed in the general design doc
  instead. §5.7 has the RFP quote, the venue table, the no-trades /
  oracle-consumer reasoning and the backstop-AMM caveat — **written to be
  lifted, not paraphrased.** The same slot is where [[0237]]'s float-vs-string
  answer belongs. ⚠️ Do not re-derive either: paraphrasing a deviation entry is
  how the two versions drift apart.
- **Do not open this task until its inputs exist.** The M1 package was written
  after the work, from real outputs; drafting it early produces claims that then
  have to be walked back.
- Per the project's convention, the submission PR stays open until the
  underlying work is confirmed in production — the open PR *is* the
  verification-pending signal.
