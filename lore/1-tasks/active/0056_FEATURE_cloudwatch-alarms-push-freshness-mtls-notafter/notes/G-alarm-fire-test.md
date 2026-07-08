---
prefix: G
title: Alarm fire-test artifacts (freshness + mTLS NotAfter)
status: developing
spawned_from: "0056"
---

# G — Alarm fire-test artifacts

Deploy-gated acceptance evidence for task 0056: each ops alarm, forced to
breach against real production data (or a controlled state), delivered a
notification to the ops channel `#stellar-prices-api-bot` via SNS → AWS
Chatbot → Slack. Timestamps are UTC.

Environment: `production` · region `eu-central-1` · account `750702271865` ·
ops topic `arn:aws:sns:eu-central-1:750702271865:prices-production-ops-alarms` ·
Slack channel `#stellar-prices-api-bot` (`C0BFWLMFQ9G`, workspace `T83HLEDJN`).

All alarms carry both `addAlarmAction` + `addOkAction` → the ops topic, so
each fire-test shows a breach **and** a recovery.

---

## 1. SDEX push-freshness — `prices-production-sdex-push-freshness` (Tranche-1 AC #5)

**Method (authentic, real-metric path).** Rather than wait 7 days for a real
skipped push, the operator-tunable threshold was temporarily lowered below the
live metric value, so the alarm breached on the **actual** published
`Prices/Backfill PushAgeSeconds` datapoint (not a forced `set-alarm-state`).
At test time both streams were `running` + pushed, with `last_push_at` frozen
~16 h prior (age ~58,211 s and climbing in lockstep with wall-clock), so the
metric was genuinely flowing.

- Temp change: `config.opsAlarms.sdexPushFreshnessSeconds` `604800 → 3600`
  (uncommitted working-tree edit), `make deploy-production-observability`.
- Metric: `Prices/Backfill` / `PushAgeSeconds`, dim `Stream=sdex_archive`,
  `Environment=production`.

| Transition | Datapoint (age_s @ time) | Threshold | Alarm state | Slack post (UTC) |
|---|---|---|---|---|
| 🚨 OK → ALARM | `58211.0 @ 2026-07-08 11:39` | `3600` | ALARM | **2026-07-08 12:09:28** |
| ✅ ALARM → OK | `58211.0 @ 2026-07-08 11:45` | `604800` (restored) | OK | 2026-07-08 ~12:1x |

Slack breach text (verbatim): _"Threshold Crossed: 1 out of the last 1
datapoints [58211.0 (08/07/26 11:39:00)] was greater than the threshold
(3600.0)…"_ with the alarm description _"sdex_archive.last_push_at has aged
past the Tranche-1 freshness threshold…"_.

- Threshold restored to `604800` + redeployed; `git status` clean afterwards
  (no residual config drift). Alarm returned to OK and posted the recovery.
- **Result: PASS.** Real-metric breach + recovery both delivered to Slack.

> SNS message IDs: delivery confirmed visually in Slack (Chatbot does not
> surface the raw SNS `MessageId` in the channel card). If a raw `MessageId`
> is required for the AC, capture it from an `--protocol email`/SQS test
> subscription on the ops topic, or from CloudTrail `Publish` events.

---

## 2. mTLS NotAfter — `prices-production-mtls-notafter-30d`

**Status: PENDING.** Method (per task Step 4): issue a short-lived (~25-day)
test cert into a **dev-only** secret, run `mtls-notafter-probe`, confirm
`MinDaysToNotAfter` drops below the 30-day threshold and the alarm trips to
Slack, then restore the canonical cert. (Delivery-only alternative:
`set-alarm-state` on the alarm to prove the SNS → Slack path, as validated for
the ledger-processor alarm during the Slack routing smoke test.)

| Transition | Metric value | Threshold | Alarm state | Slack post (UTC) |
|---|---|---|---|---|
| 🚨 OK → ALARM | _(tbd)_ | `30` | | |
| ✅ ALARM → OK | _(tbd)_ | `30` | | |

- **Result: _(tbd)_**

---

## Related smoke test (Slack routing, not a fire-test)

Before the freshness test, the SNS → Chatbot → Slack path itself was proven by
forcing `prices-production-ledger-processor-errors` ALARM→OK via
`set-alarm-state`: both a 🚨 breach and ✅ recovery landed in
`#stellar-prices-api-bot` (2026-07-08). This validated the routing + the
`addOkAction` wiring independently of any real metric.
