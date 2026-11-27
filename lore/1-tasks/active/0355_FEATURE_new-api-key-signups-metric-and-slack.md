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
`prices-<env>-overview` dashboard, and post each one with its details to the
team's Slack channel, so a new user is seen the day they sign up rather than when someone
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

1. **Discord ID and a profile link, never the username (2026-11-27).** The
   public privacy policy (`web/portal/src/privacy/privacy-policy.md`) lets
   the Discord ID be stored as the key's identifier, part of "Our operational
   record of users"; the username is for display and the session only. So
   the Slack message carries the ID and `https://discord.com/users/<id>`,
   which shows the username in Discord to whoever clicks, and nothing of ours
   stores it. The username in the message is a policy change first.
2. **Counts keys, not people.** A user who revokes and re-issues counts again;
   an adopted key (`created = false`) does not. Our own testing on production
   counts too: of 9 new keys in the 29 days before this task, 7 were test keys
   created and deleted on 2026-11-18 and 2026-11-20.
3. **The team's main channel, not the ops one (daily, 2026-11-27).**
   Sign-ups are news for the whole team, not incidents, so the message goes
   to its own topic, `prices-<env>-signups`, and from there to the team's
   main Slack channel rather than `#stellar-prices-api-bot` (ops alarms).
   Routed to Slack only when `opsAlarms.slack.signupsChannelIdSsmParam` is
   set; the channel is that parameter's value, so moving it is an SSM change,
   not a code change. It reuses the workspace already authorized in AWS
   Chatbot.
4. **No alarm.** A CloudWatch alarm could only say "at least one key in five
   minutes", and Chatbot renders it as a red ALARM card with "Threshold
   Crossed: 1 out of the last 1 datapoints…", an incident in the team's main
   channel. Nothing is added to the dashboard's alarm strip.
5. **No history.** A metric filter counts from its deploy on; the keys issued
   before stay in API Gateway (createdDate).
6. **One message per key, from a small Lambda (2026-11-27).** A subscription
   filter with the metric filter's pattern feeds `prices-<env>-signup-notifier`
   (Node.js 22, one `.mjs` file in `infra/lambda/signup-notifier`, no build:
   the runtime ships the AWS SDK; the other Lambdas are Rust). It reads the
   key from API Gateway (`apigateway:GET` only) and publishes a Chatbot custom
   notification: ":tada: New Prices API key", the profile link and ID, the
   plan (a re-issue carries a paid plan over), portal or API hub (the
   `IssuedBy` tag), first key or re-issue, `/change-plan <id>` with a link to
   the skill, the key id and the dashboard row. About $0.00001 a month at
   today's ~10 keys, less than the $0.10 alarm it replaces. No errors alarm:
   a failure loses one message, the dashboard still counts the key, and the
   error is in the Lambda's log group.

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
- [ ] A new key posts one message with its details to the team's main Slack
      channel; the ops channel stays quiet. (The handler run locally on a
      real key with SNS stubbed gave the expected message, 2026-11-27.)
- [x] The alarm strip is unchanged; `verify-dashboard-synth.mjs` passes
      (Observability synthesized alone: strip 80 own + 10 imported).
- [x] A guard test fails if the log line or the pattern is reworded alone, if
      the hub stops issuing through `issue_for`, if the metric and
      subscription filters stop sharing the pattern, or if the notifier stops
      publishing to the sign-ups topic; `signup-notifier.test.mjs` covers the
      message.
