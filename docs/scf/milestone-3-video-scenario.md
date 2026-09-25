# Milestone 3 — Deliverable Verification Video Script

> ⚠️ **DRAFT — skeleton opened 2026-09-25.** Same shape as
> [`milestone-2-video-scenario.md`](milestone-2-video-scenario.md): everything
> on screen is the public deployment, and the ⚠️ section is read before
> recording.

## Before recording

### Windows to have open, in scene order

_To fill._ Candidates: the portal (`https://sorobanscan.rumblefish.dev/api/`),
the rendered API reference (`…/api/docs`), a terminal with `API_KEY` set to the
reviewer key, the CI run for AC 4, the load-test report, the CloudWatch
dashboard (read-only viewer), the GitHub repository (public).

### ⚠️ Secrets — read this before you hit record

- The only key on screen is the published reviewer key (free plan, read-only).
- No mTLS material, no Secrets Manager console, no Discord OAuth secret, no
  ClickHouse credentials, no AWS account id beyond what CloudWatch URLs show.
- Sign in to the portal with a throwaway Discord account, not a personal one.

### Values to have ready

_To fill on the day:_ the AC 5 figures (p95 49.0 ms, plan `i12bsj`), the CI
run id, the alarm count and state, the launch date and window.

### What NOT to show

- Anything behind basic auth or in the explorer's admin surfaces.
- The re-ingest campaign machine.

## Scene 1 — Intro and scope (~0:25)

_To fill._ What Tranche 3 is; the launch date; the nine criteria in one sentence.

## Scene 2 — The portal is public, a key is issued (~1:00)

_To fill._ Sign in, eligibility, key issued, the dashboard showing the key's
plan and usage, a `/v1` call with it (AC 3).

## Scene 3 — The contract and the reference (~0:40)

_To fill._ `/api-docs-json` fetched, `npm run openapi:lint` output, `/api/docs`
rendered with real examples (AC 2).

## Scene 4 — Tests and load (~0:45)

_To fill._ The CI run with the ClickHouse integration step green (AC 4); the
load-test table and `exit=0` (AC 5).

## Scene 5 — Operations (~0:45)

_To fill._ The dashboard and alarm strip, all OK (AC 8); the backfill status
body and the ingestion signals that replaced the push cadence (AC 1, AC 9).

## Scene 6 — Repository and deploy (~0:30)

_To fill._ The public repository, README deploy section, and the AC 7 outcome
as decided (rehearsed or declared).

## Scene 7 — Deviations and what is not claimed (~0:25)

_To fill._ One sentence per deviation, pointing at the document.
