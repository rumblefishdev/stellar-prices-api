---
id: "0102"
title: "SCF Milestone 1 deliverable-verification package (form answers + evidence PDF + video scenario)"
type: DOCS
status: active
related_adr: []
related_tasks: ["0012", "0026", "0038", "0040", "0053", "0061", "0088", "0089"]
tags: ["milestone-M1", "scf", "effort-medium", "priority-high"]
links:
  - "https://github.com/rumblefishdev/soroban-block-explorer/tree/master/docs/scf"
history:
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      Docs refreshed against live prod status. TWO REAL DEFECTS fixed, both of
      which a reviewer could have caught: (1) AC 6's evidence query asked
      `price_ohlcv_1m` for ~6 months of history, but 1m is a TRANSIENT 7-day
      feeder (cleanup-worker RETENTION: 1m=7d, 15m=30d, oracle_prices=13mo;
      1h/4h/1d/1w/1M retained FOREVER) — as written it would return ~days and
      read as a FAILURE. Repointed to `price_ohlcv_1d`, per source, in both the
      evidence doc and ch-demo-queries.sql. (2) The docs claimed coarse
      granularities "are derived by a ClickHouse materialised-view chain" — the
      six mv_ohlcv_* MVs are DROPPED on prod (0090; they overwrote pre-rolled
      history in replace mode) and coarse is filled by explicit pre-roll instead.
      Verified today: `system.tables` has zero mv_ohlcv* rows. Corrected in the
      evidence exec summary + §6, the form answers, and the video scenario —
      where the operator was scripted to narrate "the six rollup materialised
      views" while `SHOW TABLES` would visibly NOT list them, on camera.
      Strengthened by today's 0097: AC 6 depth is now ~880 days for the AMM
      sources (Soroban activation) vs the ~180-day bar. Disclosed in §6 (per the
      package's own "claim only what is demonstrated" rule): the AMM live-era
      residuals — soroswap 9-day hole 2026-07-06 -> 07-15 and phoenix ~2% light
      over the same window — both pending 0101.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      Renumbered 0099 -> 0102 to resolve an ID collision. This task claimed 0099
      on an unmerged branch (PR #118); task 0099 was independently created and
      MERGED to develop meanwhile (Phoenix variable-length swap fix — live
      deploy). Renumbering this one is the cheaper fix: its ID lived in a single
      frontmatter line, whereas 0099's is referenced from 0101, 0097's archive,
      and merged commits. Nothing external depends on this number — the SCF docs
      reference "milestone 1", not the lore ID. The branch name
      (docs/0099_scf-milestone-1-verification-package) and this PR's commit
      scopes stay stale; cosmetic only.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: "Task created — produce the SCF M1 verification package for stellar-prices-api, mirroring the BE repo's docs/scf structure."
---

# SCF Milestone 1 deliverable-verification package

## Summary

Produce the Stellar Community Fund **Deliverable Verification** package that
proves Milestone 1 of `stellar-prices-api` is complete: copy-paste form
answers, a full written evidence document (rendered to PDF), and a recording
scenario for the ~5–7 minute demo video the operator will record.

## Context

The sibling BE repo (`soroban-block-explorer`) already shipped two SCF
submissions using a documented structure under `docs/scf/`. That structure is
the template: form answers per form field, an evidence companion that maps
every acceptance criterion to concrete on-mainnet proof, a video scenario, and
a reproducible `pandoc + typst` PDF build script.

This task mirrors that structure into `stellar-prices-api/docs/scf/`, scoped to
**our** Milestone 1 deliverable and acceptance criteria.

Where our delivered system differs from the originally approved M1 scope (most
notably the datastore: PostgreSQL/RDS → ClickHouse on Hetzner), the evidence
document must disclose the change honestly and carry the rationale — the same
way BE's `milestone-1-evidence.md` §4 does.

## Implementation Plan

### Step 1: Establish the M1 deliverable + AC baseline

Locate the approved Deliverable 1 text and acceptance criteria; quote verbatim.
Enumerate every `milestone-M1` task and its status.

### Step 2: Author `docs/scf/`

Port from the BE repo:

- `build-pdf.sh`, `header.typ`, `full-width-tables.lua`, `README.md`
  (build toolchain — copied, adjusted for our filenames).

Author new, scoped to our M1:

- `milestone-1-form-answers.md` — exact text per SCF form field.
- `milestone-1-evidence.md` — the source of truth; AC → evidence map,
  architecture, scope-refinement rationale, live query outputs.
- `milestone-1-video-scenario.md` — scene-by-scene recording script, 5–7 min.

### Step 3: Rationalise scope refinements

For each deviation from the approved scope, document: what changed, why, what
did *not* change, and the ADR that records it.

### Step 4: Render + verify

`./build-pdf.sh` → `milestone-1-evidence.pdf`; resolve `<TODO:>` markers for
screenshots/query outputs the operator must capture.

## Acceptance Criteria

- [x] `docs/scf/` exists mirroring the BE repo's structure. — README,
      form-answers, evidence, video scenario, `ch-demo-queries.sql`,
      `architecture.mmd`, build toolchain, `screenshots/`.
- [x] `milestone-1-form-answers.md` covers all four SCF form fields.
- [x] `milestone-1-evidence.md` maps every M1 acceptance criterion to concrete
      evidence. — All 6 ACs from design-doc §9 have a section; resource names,
      code refs, and ADR links are in place. **Live SQL output + screenshots
      remain `<TODO:>` (10 markers)** — operator-gated, see Notes.
- [x] Every scope refinement vs. the approved deliverable is disclosed with a
      rationale and an ADR reference. — §4 covers four refinements:
      RDS→ClickHouse (ADR 0007), Fargate→CLI backfill (ADR 0005/0001),
      staged-push→direct-write (ADR 0009), per-endpoint→single axum Lambda
      (ADR 0008). The approved (RDS-era) AC text is quoted side by side with
      the delivered text.
- [x] `milestone-1-video-scenario.md` is a scene-by-scene script targeting
      5–7 minutes. — 8 scenes, ~6:00 budgeted.
- [ ] `build-pdf.sh` renders `milestone-1-evidence.pdf` successfully.
      — **deferred:** pandoc + typst are not installed on this machine. Script
      syntax-checked and its tool/version guards verified to fire correctly;
      install notes rewritten for Linux (BE's were macOS/Homebrew-only).
- [x] No personal names, no Polish, no secrets (API keys / certs) in any
      committed artifact. — grep-verified.

## Implementation Notes

Ported unchanged from BE `docs/scf/`: `header.typ`, `full-width-tables.lua`.
Ported and adapted: `build-pdf.sh` (Linux install notes + a pandoc ≥ 3.1
version guard, since `--pdf-engine=typst` silently needs it), `README.md`.

Authored fresh for our scope: `milestone-1-evidence.md`,
`milestone-1-form-answers.md`, `milestone-1-video-scenario.md`,
`ch-demo-queries.sql` (11 read-only prod queries), `architecture.mmd`.

## Issues Encountered

- **The approved M1 text is not on disk.** `docs/prices-api-general-overview.md`
  §9 has been rewritten twice since the SCF baseline. The as-approved
  (RDS/Fargate/heartbeat) text survives only in git at commit `6798293`
  (2026-05-04). §4 of the evidence doc quotes it against the delivered text so
  a reviewer can audit each difference rather than trust a summary.

- **Nearly shipped a disproven number.** Drafted §4.1 citing task 0046's
  "14.8× compression, ~0.45 GB/yr" as our own measurement. Both were wrong for
  this purpose: 14.8× was measured on BE's `soroban_events` (a differently
  shaped table) as a proxy, and task 0060 later *measured* our real schema at
  **~3.7 KB/ledger, ≈2.6× compression, a few GB/yr — ~48× the estimate**.
  Corrected in all three documents to disclose the miss explicitly. Note that
  `docs/prices-api-hetzner-storage-estimate.md` still carries the superseded
  ~0.48 GB/yr projection and does not reference 0060 — see Future Work.

- **Public repo default branch is `master`, 448 commits behind `develop`**, and
  is missing `lore/2-adrs/`, `packages/`, and `infra/`. Every SCF source link
  must target `develop`, and the default branch must be switched (or merged)
  before submission or reviewers land on an empty tree. Flagged as a blocker at
  the top of the form-answers checklist.

- **The CloudWatch dashboard is a scaffold** (`prices-production-overview` holds
  a single "widgets land in task 0056" text widget). BE's M1 evidence leans on a
  dashboard screenshot; we cannot. The seven alarms are real and fire-tested
  (`c7c1bb1`), so the evidence rests on those, and both the README ground rules
  and the video script explicitly forbid screenshotting the dashboard.

- **The in-app `X-API-Key` gate is disarmed in production** (no `API_KEYS` env
  var; the gate no-ops when empty). API Gateway's key requirement is the only
  live layer. Documented as defence-in-depth rather than claimed as a second
  layer.

- **§9's "Work" prose still describes the push model ADR 0009 retired.**
  Disclosed inline in §4.3 rather than quietly fixed, since §9 is quoted
  verbatim as the deliverable definition. No AC depends on it.

## Design Decisions

### From Plan

1. **Mirror BE's four-artifact structure** (form answers / evidence+PDF / video
   scenario / build script) rather than invent one.

### Emerged

2. **Quote the *current* §9 as the deliverable definition, not the approved
   one.** BE does the same (their §7.4 already said ClickHouse). The approved
   text is quoted in §4 instead, where the rationale sits. Alternative — leading
   with the approved text — would bury the delivered system under a spec we
   deliberately moved off.

3. **Added a "What is deliberately not claimed" section (§6).** Not in BE's M1.
   Our M1 has more genuinely-open edges (scaffold dashboard, backfill still
   running, 0095 rollup-MV issue, six unverified API routes), and enumerating
   them is cheaper than having a reviewer find one.

4. **Claim only `/v1/backfill/status`, not the full API surface.** Six other
   routes are deployed but have no dated live-response record. Tranche 1 only
   requires the status endpoint; claiming more than we demo is the fastest way
   to lose a reviewer's trust.

5. **Disclosed the compression miss rather than dropping the number.** Silently
   removing it would have been defensible, but the estimate-vs-measurement gap
   is real engineering history and volunteering it is stronger than omitting it.

6. **`ch-demo-queries.sql` is strictly read-only** and run by the operator, per
   the standing convention that Claude does not query prod CH directly.

## Future Work

Spawn as backlog tasks (per `/lore-framework-tasks`) once confirmed with the
operator:

- **Correct `docs/prices-api-hetzner-storage-estimate.md`** — it still carries
  the ~0.48 GB/yr projection superseded by task 0060's measured ~3.7 KB/ledger
  (a few GB/yr), and does not reference 0060. Add a supersession note.
- **Re-sweep design-doc §9 "Work" prose** to drop the ADR-0009-retired
  local-stage-then-push model (disclosed in evidence §4.3).
- **Repo hygiene for public review** — switch the GitHub default branch to
  `develop` (or merge `develop` → `master`), and add a root `README.md`; the
  repo currently has none, so the GitHub landing page is bare.

## Notes

- Placeholders the operator fills at submission time (`<DRIVE_LINK>`,
  `<VIDEO_URL>`, `<API_KEY>`) stay as angle-bracket markers.
- Live query outputs against prod CH must be run by the operator and pasted
  back (see the "user runs prod CH queries" convention). `ch-demo-queries.sql`
  is the operator's script; it is read-only by construction.
- **Task stays `active`**: the package is authored but not yet submittable.
  Remaining operator-gated work is the 10 `<TODO:>` markers (live query output
  + 3 screenshots), the PDF render (needs pandoc + typst installed), the
  `architecture.png` render, and the default-branch fix.
