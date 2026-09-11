---
id: "0098"
title: "Version-gap CI guard — surface stellar-xdr protocol lag before it freezes prod"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0094", "0091"]
tags: ["milestone-M1", "priority-medium", "effort-small", "phase-live", "ci", "resilience"]
links:
  - "../active/0094_FEATURE_proto27-deploy-replay-verify.md — parent (AC #5)"
history:
  - date: 2026-07-16
    status: backlog
    who: okarcz
    note: >
      Spawned from 0094 future work (AC #5). Proto27 (Zipper) froze LIVE
      ingestion because the deployed ledger-processor was on stellar-xdr 26 while
      mainnet advanced to protocol 27 — the decode wall at ledger 63,401,875
      stalled the reconcile loop and was only caught reactively (0091 bumped
      xdr→27, 0094 deployed + verified the crossing). This task makes the lag
      PROACTIVE so the next protocol bump surfaces before it can freeze prod.
---

# Version-gap CI guard — stellar-xdr protocol lag

## Summary

Add a guard that surfaces when our pinned `stellar-xdr` (and `xdr-parser`) lags
the live Stellar mainnet protocol version, so a future protocol bump is caught
*before* it stalls live ingestion at a decode wall — not after (as proto27 was).

## Context

Proto27 froze the live processor: it was running `stellar-xdr 26` while mainnet
crossed to protocol 27, hitting an XDR decode wall at ledger `63,401,875`. The
freeze was diagnosed and fixed reactively across tasks 0091 (xdr→27, exact-pin
`=27.0.0`, PR #104) and 0094 (deploy + crossing verify). Note `xdr-parser` is
deliberately kept on `branch="develop"` while `stellar-xdr` is exact-pinned (see
[[xdr-parser-develop-branch-intentional]]) — the guard must account for that.

## Implementation

- **CI job**: compare our pinned `stellar-xdr` protocol version against the
  current mainnet protocol (Horizon `/` `core_supported_protocol_version` or an
  RPC `getNetwork`/ledger header) and **warn/fail** when ours < network.
- Optionally a **Renovate rule** tracking `stellar-xdr` releases so a new
  protocol version opens a PR automatically.
- Wire a heads-up (existing Slack alarm channel `#stellar-prices-api-bot`) or a
  scheduled check, so lag surfaces even without a dependency PR.
- Document the check + response in `docs/runbooks/deploy-ledger-processor.md`.

## Acceptance Criteria

- [x] Guard exists (CI job and/or Renovate rule) tracking `stellar-xdr` vs the
      live mainnet protocol version. **PR #306** — `verify-xdr-protocol-gap.mjs`,
      wired into `ci.yml` (advisory) and `xdr-protocol-watch.yml` (daily,
      strict). No Renovate rule; see Design Decisions.
- [x] It warns/fails when the pinned protocol lags the network protocol.
      **Two tiers**, and the warning one is the valuable one — see below.
- [x] Behaviour documented in the deploy runbook; `xdr-parser` develop-pin caveat
      noted so the guard doesn't false-alarm on it.
      `docs/runbooks/deploy-ledger-processor.md` gains a
      *Protocol-version lag* section plus a preflight line in step 0.
- [ ] 🔴 **`#stellar-prices-api-bot` is subscribed to the workflow** via the
      Slack GitHub app. One `/github subscribe` in the channel; no secret, no
      webhook, no AWS. Operator's — it is typed in Slack, not in this repo.
      ⛔ **The webhook approach is RETRACTED, 2026-09-11.** The workspace is at
      its installed-app limit, so an incoming webhook cannot be created at all.
      Both other routes are closed too: SNS → AWS Chatbot (the ops-alarm path,
      task 0056) needs AWS credentials this workflow does not have, and Slack
      Workflow Builder needs a paid plan.
- [ ] 🔴 **The workflow reaches `master`.** `schedule` and `workflow_dispatch`
      run **only from the default branch**, which here is `master` — a release
      branch 85 commits behind `develop`. Merging to `develop` alone leaves the
      watch dead while every file reads present. See the runbook's first
      section.
- [ ] Confirmed end to end by a manual `workflow_dispatch` run posting to
      `#stellar-prices-api-bot`. Cheap to check right now and **self-testing
      while it lasts**: we are behind protocol 28, so strict mode fails on
      purpose and a correctly wired webhook posts immediately. That window
      closes when [[0277]] lands.

# 📕 DEPLOY RUNBOOK — arming the watch

Three things must be true before the guard actually guards: the workflow is on
the **default branch**, the `SLACK_WEBHOOK_URL` secret exists, and one manual
run has been seen to post. None is enforced by code.

## 🔴 FIRST — `schedule` only runs from the DEFAULT branch, and ours is `master`

GitHub runs a `schedule` trigger **only from the repository's default branch**,
and only shows the `workflow_dispatch` "Run workflow" button for a workflow that
exists there. This repository's default branch is **`master`**, a release branch
last touched 2026-09-09 and currently **85 commits behind `develop`**.

So merging PR #306 into `develop` gives us the advisory CI job — `pull_request`
runs from the PR's own branch — and **leaves the daily watch dead**. The file
would be present, the task would read closed, and nothing would ever fire.

⚠️ That is this task's own failure mode wearing a different hat, and the third
instance of it in a fortnight: [[0215]]'s Caddyfile that the container was not
reading, [[0141]]'s lambda asset that never shipped, and now a workflow on the
wrong branch. **Verify the state the running system holds, not the file you
merged.**

`deploy-board.yml` is not a counter-example — it triggers on `push`, which runs
from the pushed branch.

## 0. Pre-check [local machine, stellar-prices-api repo]

Run all commands together:

```bash
gh repo view --json defaultBranchRef --jq .defaultBranchRef.name
git fetch origin -q && git log origin/master --oneline -1 -- .github/workflows/xdr-protocol-watch.yml
```

**Checkpoint.** The first prints `master`. The second prints **nothing** until
step 4 is done — that empty output is the dead-watch condition, and re-running
this line is how you confirm step 4 worked.

## 1. Subscribe the channel [Slack, in #stellar-prices-api-bot]

Type this in the channel (once; `/github signin` first if it asks):

```
/github subscribe rumblefishdev/stellar-prices-api workflows:{name:"XDR protocol watch"}
```

**Checkpoint.** The app replies confirming the subscription and lists what the
channel is now subscribed to.

⚠️ **It matches the workflow's `name:` EXACTLY.** Renaming `name: XDR protocol
watch` in the workflow silently unsubscribes the channel — failing workflow,
silent channel. Rename both together or neither.

## 2. Nothing to store [no action]

There is no secret. Left numbered so the step numbers in this runbook match the
commits and the PR discussion that produced them.

⛔ **An incoming webhook was the original plan and it is not available**: the
workspace is at its installed-app limit. SNS → AWS Chatbot, the route every ops
alarm uses, needs AWS credentials this workflow does not have. Workflow Builder
needs a paid plan. The GitHub app was already installed, which is why this is
one slash command instead of any of that.

## 3. Merge PR #306 into `develop` [GitHub]

Normal review and merge. This arms the advisory CI job. It does **not** arm the
watch.

## 4. Get the workflow onto `master` [local machine, stellar-prices-api repo]

The schedule cannot start until the file is on the default branch, and the next
release merge could be weeks away — the Protocol 28 vote is 2026-09-16.

Open a small PR carrying **exactly two files**, `develop` → `master`:

```
.github/workflows/xdr-protocol-watch.yml
tools/scripts/verify-xdr-protocol-gap.mjs
```

⚠️ **Two, not one.** `master` has `.nvmrc` and `package.json` but not the
`xdr:` npm scripts, which is why the workflow invokes
`node tools/scripts/verify-xdr-protocol-gap.mjs --watch` directly instead of
going through `npm run`. Through npm it would fail on `master` with "missing
script" while passing on every branch anyone would think to test it on.

⛔ **Do not merge `develop` into `master` to achieve this.** That is 85
unrelated commits onto a release branch as a side effect of arming a watch.

**Checkpoint.** Re-run the second command from step 0. It must now print a
commit. Until it does, everything below is untestable.

## 5. Prove delivery end to end [GitHub Actions, or local]

```bash
gh workflow run xdr-protocol-watch.yml --repo rumblefishdev/stellar-prices-api --ref master
```

Or: *Actions → XDR protocol watch → Run workflow*.

**Checkpoint — what a good result looks like.** The run **FAILS**, and that is
the pass condition. While our pin is behind protocol 28 the strict mode is
supposed to fail, so a red run plus a Slack message is the guard working. A
green run at this moment would mean the check is not reading what it claims to.

Expect a message in **#stellar-prices-api-bot** from the GitHub app naming the
workflow and its failed conclusion. Click through and the run's **summary**
carries the report itself:

```
## stellar-xdr protocol lag

error: stellar-xdr 27 lags the protocol core already supports (28).
  pinned stellar-xdr 27 | mainnet current 27 | core supports 28
```

⏳ **This self-test expires.** It works because we are currently behind; once
[[0277]] lands the check goes green and proving delivery needs a deliberately
wrong pin. Do it now.

If the run fails but the channel stays quiet, the subscription is the suspect,
not the workflow — re-run `/github subscribe` and check the workflow `name:`
still matches character for character.

## 6. Final test [local machine]

```bash
gh run list --workflow xdr-protocol-watch.yml --repo rumblefishdev/stellar-prices-api --limit 3
```

**Done when** a run is listed, its conclusion is `failure`, and the message
appeared in the channel. Tick the two open criteria above.

## Reverting

`/github unsubscribe rumblefishdev/stellar-prices-api workflows:{name:"XDR protocol watch"}`
in the channel, and delete the workflow file.
The check itself stays useful without either — `npm run xdr:verify-protocol-gap`
is standalone and is already a preflight step in
`docs/runbooks/deploy-ledger-processor.md`.

## Implementation Notes

Shipped on `feat/0098_xdr-protocol-version-gap-ci-guard`, **PR #306**.
450 lines, no new dependency.

**What it reads.** Horizon's root document, one GET, no credentials:

```
current_protocol_version        27   what mainnet runs NOW
core_supported_protocol_version 28   what core is READY to run
```

…against the `stellar-xdr` major in `Cargo.toml`. The mapping the check
encodes is that the crate's MAJOR version tracks the protocol version — 27
decodes protocol 27, 28 decodes 28. That assumption is stated in the script
header so a future reader can test it rather than infer it.

**Three checks, not one:**

| check | fails when |
|---|---|
| BEHIND | `ours < current_protocol_version` — mainnet has moved past us |
| LAGGING | `ours < core_supported_protocol_version` — an upgrade is available |
| lockfile | two `stellar-xdr` majors resolved, or the lock disagrees with the pin |

The third is [[0277]]'s compile break, named precisely instead of surfacing as
a wall of trait errors.

**Measured on the day it was written** — the guard fired its warning tier
correctly against live mainnet, five days before the Protocol 28 vote:

```
notice: stellar-xdr 27 lags the protocol core already supports (28).
  pinned stellar-xdr 27 | mainnet current 27 | core supports 28
```

**All eight paths exercised** before commit, against a stub Horizon on
localhost and stub manifests in a throwaway tree: BEHIND, LAGGING, current,
unreachable-soft, unreachable-strict, two-majors, lock-disagrees-with-pin,
pin-missing. The Slack payload was posted to a local sink and confirmed valid
JSON carrying the report text.

## Design Decisions

### From Plan

1. **Horizon's root document as the source**, as the task specified. It needs
   no credentials, no SDK and no AWS, which is what keeps the check runnable
   from CI, from a laptop and from the deploy runbook's preflight identically.

### Emerged

2. **Two tiers, and `core_supported` is the one that does the work.** The task
   said "warn/fail when ours < network", which reads as one comparison. Horizon
   publishes two numbers, and the useful one is `core_supported_protocol_version`
   because it rises **weeks before the vote**. By the time `current` moves we
   are already in the outage. So LAGGING is the early warning and BEHIND is the
   backstop, not the other way round.

3. **The scheduled workflow is the guard; the CI job is a convenience.** This
   is the decision that matters most and it is not what the task title
   ("version-gap CI guard") implies. Nothing in this repo changed while proto27
   froze us — mainnet moved and our pin stood still — so a `push` /
   `pull_request` check sees no event at all and would not have caught the very
   incident it was spawned from. The clock is the mechanism.

4. **PR CI is advisory; only the scheduled run is strict.** A PR must never go
   red because the Stellar Foundation announced something or Horizon had a bad
   minute. A developer can fix neither, and a gate that red-lights unrelated
   PRs gets switched off within a week — at which point the guard is worse than
   absent, because it looks present. BEHIND still fails on a PR.

5. **An unreachable Horizon FAILS under `--watch`.** A scheduled watch that
   quietly checks nothing has the exact shape of the outage being guarded
   against: every signal green, nothing actually verified. It is only a notice
   on a PR.

6. **The CI job is ungated by `changes` path filters.** Every other job in
   `ci.yml` is gated on `rust`/`typescript` paths. This one cannot be: the
   condition it watches produces no diff on our side at all.

7. **Delivery is the Slack GitHub app — every other route was closed.**
   Worked through in order, and worth recording so it is not re-litigated:

   | route | why not |
   |---|---|
   | SNS → AWS Chatbot (the ops-alarm path, task 0056) | needs AWS credentials in CI; an OIDC role + CDK change + deploy is disproportionate here |
   | Slack incoming webhook | needs a Slack app, and **the workspace is at its installed-app limit** |
   | Slack Workflow Builder | needs a paid plan |
   | **Slack GitHub app** | **already installed.** One `/github subscribe`, no secret, no AWS |

   The cost is that the app posts a run and a link rather than our text, so the
   report is written to `$GITHUB_STEP_SUMMARY` and is the first thing visible
   after the click.

   🔴 **The subscription matches the workflow `name:` exactly**, so renaming it
   silently unsubscribes the channel: a failing workflow and a quiet channel,
   which is this task's own failure mode. Named in the workflow, the runbook
   and here, because nothing enforces it.

8. **No Renovate rule** (the task offered it as optional). There is no Renovate
   config in this repo, so adding one is repo-wide dependency automation rather
   than a rule — a much larger change than this task. It would also solve a
   different problem: Renovate fires when a **crate version publishes**, while
   the thing that hurt us is **mainnet voting**. The daily watch covers both,
   because `core_supported` moves either way.

9. **The two-tier decision was put to the operator, and Slack was chosen.** The
   alternatives were GitHub's own notification (zero config) or additionally
   opening a GitHub issue. Issues were ruled out on evidence: `gh issue list`
   returns empty — the repo has never used them, so an issue would be a dead
   letter.

## Issues Encountered

- **YAML ate the channel name.** `- name: Post the lag to #stellar-prices-api-bot`
  parses as a comment from the `#` onward, silently truncating the step name to
  "Post the lag to". Caught by parsing the workflow with PyYAML and printing the
  step names rather than by reading it. Quoted the string. Worth remembering:
  any `#` in an unquoted YAML scalar is a comment, and a channel name is the
  most likely place to hit it.

- **Neither `develop` nor `master` has branch protection**, so the advisory CI
  job is not a required status check and cannot block a merge. That is recorded
  rather than changed — turning on branch protection is a repo-policy decision,
  not this task's, and it would not help the proto27 case anyway (no PR was
  involved).

## Known Limit — it reads the REPO, not the deployed binary

Stated in the runbook and the script header rather than left implicit, because
it is the same trap as [[0141]]: **merging a fix is not shipping one.** 0091
merged the proto27 bump on 2026-07-14 and production stayed frozen until 0094
deployed the binary days later. A green protocol check means the source is
correct; only a deploy establishes that the running Lambda is.

The deployed half is covered from the other side by
`prices-production-rollup-freshness-1m`, which measures **data** rather than MV
exit status and whose description already names "upstream ingestion has
halted". Between the two, both halves are watched — neither alone is enough.
That pairing is now written down in the runbook so the next reader does not
have to rediscover which check covers which half.
