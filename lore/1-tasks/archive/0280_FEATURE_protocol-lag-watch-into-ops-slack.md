---
id: "0280"
title: "Route the protocol-lag watch into #stellar-prices-api-bot through the existing SNS → Chatbot path"
type: FEATURE
status: canceled
related_adr: []
related_tasks: ["0098", "0056", "0277"]
tags: [layer-infra, priority-low, effort-medium, observability, cloudwatch, ci]
links:
  - "../active/0098_FEATURE_xdr-protocol-version-gap-ci-guard.md"
history:
  - date: "2026-11-26"
    status: canceled
    who: okarcz
    reason: wont-do
    note: >
      Closed as won't-do by the operator. 0098's GitHub-issue delivery works,
      needs no secrets and closes itself; moving the alert to Slack changes
      only where it lands, at medium effort. See Outcome.
  - date: 2026-09-11
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0098]]. Its guard delivers through a GitHub issue because
      every Slack route was closed to us; this is the route that would put it
      where the ops alarms already land.
      ⚠️ Renumbered TWICE the same day, 0278 → 0279 → 0280. 0278 was taken by
      a teammate's dust-print task pushed at 11:59 and this was filed at 12:15
      from a REMEMBERED "next free ID"; the repair to 0279 then collided with a
      freeze-snapshot task that arrived in the same pull. Third and fourth ID
      collisions in this project. **Derive the number from the tree, right
      before writing the file** — `find lore/1-tasks lore/2-adrs -maxdepth 2
      -name '0*' | grep -oE '[0-9]{4}' | sort -n | tail -1` — never from a note,
      a memory, or an earlier reading in the same session.
---

# Protocol-lag watch into the ops Slack channel

## Summary

[[0098]]'s daily protocol-lag watch reports through a **GitHub issue**, because
all four Slack routes were closed. This task puts it where every other ops
alarm already arrives: `#stellar-prices-api-bot`, via the **SNS → AWS Chatbot**
path from task 0056.

## Context

Measured 2026-09-11 while building 0098:

| route | why not |
|---|---|
| Slack incoming webhook | the workspace is at its **installed-app limit** |
| Slack GitHub app | in the workspace, **not installed on the GitHub org**; needs an organisation owner |
| Slack Workflow Builder | needs a paid plan |
| SNS → AWS Chatbot | ✅ already installed and working — but needs AWS credentials the workflow does not hold |

Only the last is a matter of engineering rather than someone else's
permission, which is why it is the one worth a task.

⚠️ The issue-based delivery 0098 ships is **not a placeholder**. It needs no
credentials, no app and nobody's approval, and it auto-closes. This task is an
upgrade of the channel, not a repair.

## Implementation

Two shapes; pick one deliberately.

1. **OIDC role for GitHub Actions.** Give the workflow a role that may
   `sns:Publish` to the ops-alarms topic. Smallest change, keeps the check
   where it is — but puts AWS credentials into CI, which this repository has
   deliberately avoided (it holds **no** secrets today).
2. **A scheduled Lambda** beside `rollup-freshness-probe` and
   `mtls-notafter-probe`, publishing a `CloudWatch` metric with an alarm.
   Matches the established pattern exactly and needs no CI credentials.
   ⭐ Also sidesteps the default-branch constraint that forced 0098's workflow
   onto `master`, and could measure the **deployed** binary's protocol version
   rather than the repository's pin — closing 0098's stated limit.

**Option 2 is the recommendation**, for the last point most of all: 0098 can
only see the repo, and [[0141]] is the standing proof that a merged bump is not
a shipped one.

## Acceptance Criteria

- [ ] A protocol lag raises an alarm that reaches `#stellar-prices-api-bot`
      through the existing SNS → Chatbot topic.
- [ ] Verified by inducing, not inferred — the alarm is seen to fire.
- [ ] 0098's GitHub-issue delivery is either retired or deliberately kept, and
      the choice is recorded.
- [ ] If option 2: the metric reflects the **deployed** binary, not the
      repository pin, and 0098's "reads the repo, not the binary" limit is
      struck.

## Outcome (2026-11-26): closed as won't-do

- **The current alert is enough.** `xdr-protocol-watch.yml` (from 0098) runs
  daily from `master`, opens one tracking issue when the pinned protocol lags
  mainnet, and GitHub emails on the failed run. It needs no credentials and no
  approval, and the issue closes itself once the lag is gone.
- **What this task would have added** is the same warning in
  `#stellar-prices-api-bot`, through a new scheduled Lambda, a CloudWatch
  metric and alarm, a CDK change and a deploy. That is a change of channel, not
  a fix, and not worth the effort at low priority.
- **Given up knowingly:** option 2 could have read the protocol version of the
  **deployed** binary rather than the repository pin. 0098's "reads the repo,
  not the binary" limit stays. No follow-up task is spawned for it.
- Acceptance criteria above are left unchecked: none were attempted.
