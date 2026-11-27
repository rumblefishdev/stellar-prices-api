---
id: "0355"
title: "New API keys: a metric, a dashboard row and a Slack message per sign-up"
type: FEATURE
status: active
related_adr: ["0010"]
related_tasks: ["0187", "0348", "0249"]
tags: [layer-infra, priority-medium, effort-small]
history:
  - date: 2026-11-26
    status: active
    who: stkrolikiewicz
    note: >
      Asked for while answering an SCF survey question: who signed up and on
      which plan took a manual walk over API Gateway keys and Discord profiles.
---

# New API keys: a metric, a dashboard row and a Slack message per sign-up

## Summary

Count the keys the portal and the API hub issue, show the count on the
`prices-<env>-overview` dashboard, and post to a Slack channel when one is
issued, so a new user is seen the day they sign up rather than when someone
goes looking.

## Context

On 2026-11-26 answering "how many users, on which plans" meant listing API
Gateway keys, reading per-key usage per plan, and opening each
`discord-<id>-key`'s Discord profile by hand. 11 accounts, 8 of them ours.
Nothing told us when the 3 outside ones arrived.

Every key is issued by `keys::issue_for`, which logs `portal issued an API
key` with a boolean `created`. The API hub (0348) issues through the same
function, so that one line covers both ways in; the hub's own `hub issued an
API key` line is not counted, or each hub key would count twice (checked on
production logs: every hub key had both lines). A metric filter on the
api-handler log group turns it into a metric, the same way the
portal-load-failed signal (0249, 0311) works.

## Decisions

1. **No Discord username anywhere new.** The public privacy policy
   (`web/portal/src/privacy/privacy-policy.md`) says the username is used for
   display and the session only; the Discord ID may be stored with the key.
   The Slack message says "a key was issued", not who. Who, when needed: the
   `change-plan` runbook by Discord ID. Changing that is a policy change first.
2. **Counts keys, not people.** A user who revokes and re-issues counts again;
   an adopted key (`created = false`) does not. Our own testing on production
   counts too: of 9 new keys in the 29 days before this task, 7 were test keys
   created and deleted on 2026-11-18 and 2026-11-20.
3. **The team's main channel, not the ops one (daily, 2026-11-27).**
   Sign-ups are news for the whole team, not incidents, so the alarm goes to
   its own topic, `prices-<env>-signups`, and from there to the team's main
   Slack channel rather than `#stellar-prices-api-bot` (ops alarms). Routed
   to Slack only when `opsAlarms.slack.signupsChannelIdSsmParam` is set; the
   channel is that parameter's value, so moving it is an SSM change, not a
   code change. It reuses the workspace already authorized in AWS Chatbot.
4. **Kept out of the alarm strip.** The strip on the dashboard is the ops
   health board; a sign-up turning it red for five minutes would read as an
   incident. `verify-dashboard-synth.mjs` counts it out by name.
5. **No history.** A metric filter counts from its deploy on; the keys issued
   before stay in API Gateway (createdDate).
6. **A custom message, not the alarm card (2026-11-27).** The alarm has no
   action: Chatbot's stock card is a red ALARM with "Threshold Crossed: 1 out
   of the last 1 datapoints…", an incident in the team's main channel. An
   EventBridge rule on the alarm's move to ALARM publishes a Chatbot custom
   notification to the sign-ups topic instead: ":tada: New Prices API key",
   one line that someone got a key, a link to the dashboard row, and when to
   use the `change-plan` skill (who it is, or a plan change) with links to
   API Gateway's keys, where the Discord ID is in the key's name, and to the
   skill. Plain words for the whole team, no ID in the message. No Lambda.
   The alarm only feeds the rule; several keys within minutes still post
   once.

## Before deploying Observability

1. Invite the AWS app to the team's main channel (`/invite @Amazon Q` or
   `@AWS`, whichever the workspace shows; it is already in
   `#stellar-prices-api-bot`).
2. Put that channel's ID in SSM, shared account, eu-central-1 (done
   2026-11-27, `C0AN59N4J8M`):
   `aws ssm put-parameter --name /prices/production/slack-signups-channel-id --type String --value <channel-id>`
3. Deploy Observability. Then issue a test key (or wait for a real one) and
   check the message lands.

## Acceptance

- [x] `PortalNewApiKeys` (namespace `Prices/ApiHandler`) counts new keys from
      the portal and the hub once each: pattern checked with
      `aws logs test-metric-filter` on real production lines.
- [ ] The overview dashboard has a sign-ups row: per day over 90 days, and the
      30-day total.
- [ ] A new key posts the custom sign-up message to the team's main Slack
      channel; the ops channel stays quiet.
- [x] The alarm is not in the alarm strip; `verify-dashboard-synth.mjs` passes
      (Observability synthesized alone: strip 80 own + 10 imported, unchanged).
- [x] A guard test fails if the log line or the filter is reworded alone, if
      the hub stops issuing through `issue_for`, or if the message moves back
      to the alarm's own action or off the sign-ups topic.
