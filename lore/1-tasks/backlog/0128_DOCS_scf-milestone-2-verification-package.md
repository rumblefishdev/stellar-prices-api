---
id: "0128"
title: "SCF Milestone 2 verification package — evidence doc, form answers, video scenario"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0102", "0117", "0120", "0121", "0122", "0123", "0124", "0125", "0126", "0127", "0237", "0248"]
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

- [ ] `milestone-2-evidence.md` covers all 6 Tranche 2 ACs, each with
      reproducible evidence
- [ ] Every §9 Tranche 2 work bullet addressed, including those without a
      numbered AC
- [ ] The three M1-deferred items ([[0124]], [[0125]], [[0126]]) are shown as
      delivered, or their non-delivery is stated in the not-claimed section
- [ ] "What is deliberately not claimed" section present, with a destination
      for every row
- [ ] Live endpoints + access table current (custom domain, dashboard role, API
      key request path)
- [ ] `milestone-2-form-answers.md` and `milestone-2-video-scenario.md` complete
- [ ] `ch-demo-queries.sql` refreshed for M2 additions
- [ ] All cited figures re-run within days of submission
- [ ] No claim in the package lacks a task, query, or URL behind it

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
