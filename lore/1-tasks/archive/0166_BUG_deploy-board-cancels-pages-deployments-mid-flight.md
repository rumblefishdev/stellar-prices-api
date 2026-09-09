---
id: "0166"
title: "deploy-board cancels Pages deployments mid-flight — the board silently serves stale data for days"
type: BUG
status: completed
related_adr: []
related_tasks: ["0141", "0169"]
tags: ["priority-medium", "effort-small", "ci", "github-actions", "tooling", "silent-failure"]
links:
  - "../../../.github/workflows/deploy-board.yml"
history:
  - date: 2026-08-07
    status: active
    who: okarcz
    note: >
      Found while debugging why tasks 0156-0164 never appeared on the board
      after PR #174 merged to develop. The live board.json was measured at 142
      tasks (max id 0155, 0124 still listed active) while run #260 had built
      160 tasks, deployed, and logged "Reported success!". Root cause is
      `cancel-in-progress: true` on the `pages` concurrency group. Confirmed
      against the BE repo, which carries a byte-identical workflow and whose
      logs show the full chain explicitly. Fix is our repo only — BE is out of
      scope by explicit instruction.
  - date: 2026-08-10
    status: completed
    who: okarcz
    note: >
      COMPLETED. The headline defect is fixed and proven on real traffic:
      cancel-in-progress: false shipped in eda7df5 (PR #181), and every
      deploy-board run since has succeeded - eda7df5, d37e2ff, bee237c,
      f1504fa, 696f58a, db73107 - against three cancellations and one failure
      in the ~100 minutes before it. d37e2ff and bee237c merged 102 SECONDS
      apart and both deployed, which exceeds the "two rapid merges" AC. Live
      board verified from published bytes: 164 tasks, max id 0168, matching a
      local generator run exactly, with 0156 showing active (it only became
      active at 696f58a).
      ONE AC DEFERRED, NOT MET: the schedule + workflow_dispatch recovery path
      is INERT. GitHub resolves both triggers from the DEFAULT branch (master),
      which still carries the pre-0166 workflow including cancel-in-progress:
      true. Zero event=schedule runs in three days where ~72 were due. The
      irony is that this task flagged "the default branch is master" as
      load-bearing but applied it only to the checkout ref pin. Spawned as
      0169, whose acceptance test counts actual runs rather than asserting the
      YAML - which is exactly what let this slip through here.
      Push-triggered deploys are unaffected (push uses the pushed ref's own
      workflow file), so this is a missing safety net, not an active outage.
---

# `deploy-board` cancels Pages deployments mid-flight

## Summary

`.github/workflows/deploy-board.yml` sets `cancel-in-progress: true` on the
`pages` concurrency group. When merges to `develop` land closer together than a
board deploy takes (~10–30 min), a run in the middle of its **Pages deployment**
is killed. GitHub Pages is left with a wedged in-progress deployment, the next
run's `actions/deploy-pages` polls it until `Timeout reached, aborting!`, and the
site keeps serving the last deployment that completed.

The board then reports **whatever it last successfully published**, with no
signal that it is stale. It sat two days behind before anyone noticed.

## Evidence

### Our repo — successful runs whose content never went live

Task counts pulled from each run's `board.json generated (N tasks)` log line:

| run | time (UTC) | commit | tasks built | outcome |
|---|---|---|---|---|
| #255 | 12:26 | | 141 | ✅ success |
| #256 | 14:05 | `dabdd15` | **142** | ✅ success |
| #259 | 14:27 | `adcc373` | 151 | ✅ success, *"Reported success!"* |
| #260 | 14:39 | `e937aee` | **160** | ✅ success, *"Reported success!"* |
| #261 | 14:52 | | — | ⊘ cancelled |
| #262 | 14:58 | | — | ⊘ cancelled |
| #263 | 16:05 | `064a6ce` | — | ❌ no runner (see below) |

Live site measured 2026-08-07:

```
$ curl https://rumblefishdev.github.io/stellar-prices-api/board.json
count: 142   max id: 0155   active: 0088,0124,0136,0111
```

142 tasks = run **#256**, from 14:05 on 08-06. Independently corroborated by
`0124` still being listed **active** — 0124 was archived at 14:22, so the live
content is definitively pre-14:22. Runs #259 and #260 both built correctly, both
ran `actions/deploy-pages`, both logged `Created deployment for e937aee…` and
`Reported success!` — and neither is live.

#260's deploy job held the concurrency slot from 14:39 (queued) to 15:09:17
(finished), a **30-minute** window. #261 (14:52) and #262 (14:58) both arrived
inside it.

### BE repo — the same workflow, and its logs show the mechanism outright

`rumblefishdev/soroban-block-explorer` carries a byte-identical
`deploy-board.yml` apart from `cache: npm` + `npm ci` (which **it has and we
lack**). Its 2026-08-06 runs:

```
#1067  build → success   deploy → CANCELLED   15:11:28 → 15:14:52
#1068  build → success   deploy → FAILURE     15:16:16 → 15:26:26
```

`#1067`'s deploy was killed mid-deployment at 15:14:52 — the moment `#1068`
started (15:14:17). `#1068` then inherited the wedged state:

```
15:26:18  Current status: deployment_in_progress
15:26:24  Current status: deployment_in_progress
15:26:24  ##[error]Timeout reached, aborting!
```

Exactly 600 s — the `actions/deploy-pages` default `timeout: 600000`.

Consistent with this, `GET /repos/…/pages/deployments/{sha}` returns
`{"status":"deployment_in_progress"}` for **every** sha queried on our repo,
including long-finished ones.

**BE's board is nonetheless current (453 tasks) — not because its config is
better, but because it receives ~5 pushes to `develop` per day, any one of which
repairs it.** Our `develop` is quiet, so the defect became visible here first.
BE is a latent outage waiting for a quiet week; reporting it to its owner is
out of scope for this task (see §Out of scope).

### A second, unrelated failure that compounded it

Run **#263** (the merge of PR #178) is marked `failure` but nothing ran:

```
JOB build  → cancelled   runner_name=""   steps=0   16:05:06 → 16:20:08  (15m02s)
JOB deploy → skipped
```

Zero steps, no runner ever assigned, cancelled at exactly 15 minutes — GitHub's
runner-acquisition timeout. PR #179's CI (`#406`) shows the identical signature
in the same window (`16:49:33 → 17:04:35`, runner `""`, 0 steps). **Not a config
bug and not this task's subject** — a transient GitHub-hosted runner shortage on
2026-08-06 ~16:00–17:05 UTC. It cleared: run #407 (08-07 09:39) acquired a
runner normally.

It matters here only because it removed the last chance to self-heal: #261/#262
had already been discarded, #263 was the newest run, and it failed.

## Why this class of bug is expensive

Cancelled runs render as a **grey ⊘, not a red ✗**, so #261/#262 look benign in
the Actions tab. Combined with #259/#260 reporting *success* while publishing
nothing, every available signal said the board was healthy while it was two days
stale. Same shape as [[0141]] (deploy ships stale lambda assets, every signal
green).

## Implementation Plan

Single file: `.github/workflows/deploy-board.yml`.

### Step 1 — stop cancelling deployments

```yaml
concurrency:
  group: pages
  cancel-in-progress: false
```

This is what GitHub's own Pages starter workflow ships, with the comment:
*"do NOT cancel in-progress runs as we want to allow these production
deployments to complete."*

Concurrency semantics after the change: one **running** slot, one **pending**
slot; a new arrival cancels the older *pending* run, never the running one.
Concurrent merges by two people are safe — each run checks out its own trigger
commit (`github.sha`), and a later `develop` commit is a strict superset of an
earlier one, so a discarded pending run loses no content.

### Step 2 — add a recovery path

```yaml
on:
  push:
    branches: [develop]
    paths:
      - 'lore/**'
      - 'tools/scripts/generate-lore-board.mjs'
      - '.github/workflows/deploy-board.yml'
  schedule:
    - cron: '17 * * * *'
  workflow_dispatch:
```

Step 1 alone does **not** make the board reliable: if the newest run fails,
everything queued behind it was already discarded and nothing retries. The
hourly schedule self-heals within an hour; `workflow_dispatch` allows a manual
re-publish without an empty commit. The `paths` filter cuts contention by not
queueing board deploys for code-only merges.

### Step 3 — ⚠️ pin the checkout ref

```yaml
      - uses: actions/checkout@v4
        with:
          ref: develop
```

**Load-bearing.** This repo's default branch is `master`, and `schedule` /
`workflow_dispatch` runs check out the **default** branch — not the branch in
the `push` filter. Without this, the hourly cron would publish `master`'s lore
over `develop`'s and make the board actively worse than stale, while reporting
success.

### Step 4 — restore `npm ci`

We dropped `cache: npm` + `npm ci` relative to BE. It works today only because
`generate-lore-board.mjs` uses pure node builtins (`node:fs`, `node:path`, zero
deps). The first dependency it gains breaks our board build and not BE's.

## Acceptance Criteria

- [x] `cancel-in-progress: false` on the `pages` group. **Shipped `eda7df5`
      (PR #181).**
- [~] `workflow_dispatch` and `schedule` triggers present; `paths` filter scoped
      to lore + the generator + the workflow itself. **Present in develop's file,
      but `schedule` and `workflow_dispatch` are INERT — GitHub resolves both
      from the DEFAULT branch (`master`), which still carries the pre-fix
      workflow. Zero scheduled runs in three days where ~72 were due. Deferred to
      [[0169]].** The `paths` filter works (it is a `push` trigger, resolved from
      the pushed ref).
- [x] `actions/checkout@v4` pins `ref: develop` so scheduled/dispatched runs do
      not publish `master`.
- [x] `npm ci` + `cache: npm` restored.
- [x] The live board serves the current `develop` tip — verified 2026-08-10:
      `curl …/board.json` returns **164 tasks, max id 0168**, matching a local
      `node tools/scripts/generate-lore-board.mjs` exactly (`board.json generated
      (164 tasks)`). Corroborated by `0156` appearing **active**, which it only
      became at `696f58a`. Verified from the published bytes, not a green check.
- [x] Two rapid merges to `develop` both complete, with the later content live.
      **Verified stronger than specified:** `d37e2ff` (12:23:53Z) and `bee237c`
      (12:25:35Z) merged **102 seconds apart** and both deployed successfully.
      Six consecutive successes since the fix (`eda7df5`, `d37e2ff`, `bee237c`,
      `f1504fa`, `696f58a`, `db73107`) against three cancellations and one
      failure in the ~100 minutes before it.
- [ ] ~~Manual dispatch as an alternative to the second rapid merge~~ — not
      exercisable; see the deferred AC above. Covered by [[0169]].

## Verification

The green check is not the artifact. Verify the published bytes:

```bash
node tools/scripts/generate-lore-board.mjs          # prints: board.json generated (N tasks)
curl -s https://rumblefishdev.github.io/stellar-prices-api/board.json \
  | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const t=JSON.parse(s).tasks||JSON.parse(s);console.log('live:',t.length)})"
```

The two counts must match (allowing for commits landed since).

## Issues Encountered

- **The `schedule` / `workflow_dispatch` half of the fix never took effect.**
  Found on 2026-08-10 while verifying ACs for archive. GitHub resolves both
  triggers from the **default branch** (`master`), not from the branch the
  workflow lives on; `master` still carries the pre-0166 file, including
  `cancel-in-progress: true`. Zero `event: schedule` runs in three days against
  ~72 due. Spawned as [[0169]].

  The task file itself flagged "this repo's DEFAULT branch is master" as
  load-bearing — but applied it only to the `checkout` ref pin, never following
  the implication through to the triggers that same fact governs. **A fix
  written on `develop` and verified by reading the YAML looked complete; only
  counting actual runs exposed it.** That is the same silent-failure shape this
  task exists to document.

- **`push` masks the drift.** Push events use the workflow file from the pushed
  commit, so `develop` merges correctly run the fixed copy. The primary defect
  is genuinely fixed and the board is genuinely live — only the safety net is
  missing, which is precisely the part that produces no signal until it is
  needed.

## Design Decisions

### Emerged

1. **Archived with the recovery path deferred rather than held open.** The
   headline defect — deploys cancelled mid-flight, board silently stale — is
   fixed and proven over six consecutive runs including a 102-second merge pair.
   Holding 0166 open for a distinct root cause (branch drift on the default
   branch) would conflate two bugs; [[0169]] carries the remainder with its own
   acceptance test.
2. **0169's acceptance test counts runs, not YAML.** Written explicitly as
   "a run with `event: schedule` appears within ~2 h", because asserting the
   trigger's presence in the file is exactly what let this slip through here.

## Out of scope

- **The BE repo.** `rumblefishdev/soroban-block-explorer` carries the same
  defect and would benefit from the same three-line change, but it is not ours
  to edit — flagged to its owner instead ([[team-adam-kot-task-ownership]]).
  Read-only inspection only; nothing was modified there.
- **The runner-acquisition failure** (#263 / #406). Transient GitHub-side
  shortage, already cleared, no code change available. Recovered by re-running.
- Board *content* or layout changes — `generate-lore-board.mjs` is correct and
  was verified to build 161 tasks locally.

## Notes

- Scheduled workflows are delayed or dropped by GitHub under load, so the cron
  is a safety net, not the primary path. Push remains the fast path.
- Recovery for the current staleness: `gh run rerun 31118545567` (#263). Its
  `head_sha` is `064a6ce`, still the tip of `develop`, so a re-run publishes the
  fully current board without an empty commit.
