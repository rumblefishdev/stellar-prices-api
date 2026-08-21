---
id: "0191"
title: "Replace my key — rework, capped at once per quota period"
type: FEATURE
status: active
related_adr: ["0010"]
related_tasks: ["0183", "0157", "0160", "0180", "0187", "0189", "0190", "0192", "0193"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, slice-8]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/notes/R-apigw-namequery-quota-and-disable.md"
history:
  - date: "2026-08-13"
    status: backlog
    who: akot
    note: >
      Eighth slice, the last of [[0160]]'s four operations, and the new home of
      [[0180]] item 7 (the quota rollover instant). Placed after the dashboard
      because a user who cannot see their key does not need to replace it.
  - date: "2026-08-21"
    status: active
    who: akot
    note: >
      Activated on `develop` with [[0188]] and [[0189]] both merged (#227,
      #230), so the reconciler, the read-only `/key` route, the usage cache
      and `eligibility::decide` are all in place to build on. Step 0's DAY
      rollover measurement runs alongside the implementation; the MONTH
      confirmation cannot happen before 1 September 2026 and is recorded as
      an open limit, not assumed.

---

# Rework — a new key, once a period

## Summary

**Story:** *as a developer who has lost or leaked a key, I can generate a new
one — but not often enough to use it as a free quota reset.*

An atomic swap: the old key is deleted and a new one issued in the same
operation, so the user is never without a working key. The cap blocks the *next*
rework, not the replacement.

## Step 0 — measure the rollover (was [[0180]] item 7)

**The rule we render is ours, not AWS's.** AWS documents neither the quota reset
instant nor its timezone; the only statement anywhere is an example caption,
*"creates a usage plan that resets at the beginning of the month"*, and `offset`
is a **request count**, not a time shift. This is ADR 0010's correction #2 and it
is still open.

A `DAY`-period scratch plan is a proxy for the instant and the timezone, and the
scratch resources from the abandoned run are **still standing** for it: REST API
`9utcrbmoc6`, usage plan `ox7pv0`, key `2ke0ixjy7h`, plus
`item7-quota-rollover.sh` and its partial log in the archived
`0180_RESEARCH_.../measurement/`. Drain the key, poll across a UTC midnight,
record when `429` becomes `200`.

**One honest limit, stated up front: this cannot be fully settled before
1 September 2026**, the next real `MONTH` rollover. The `DAY` proxy is strong
evidence, not proof — and it is enough, because the criterion is to stop
presenting "00:00 UTC on the 1st" as AWS-documented, not to prove AWS's
implementation. Correct [[0157]]'s and this task's wording either way, and run
the `MONTH` confirmation on 1 September if the epic is still open.

**Careful with the control plane while doing it:** `UpdateUsagePlan` is throttled
to **1 request per 20 seconds per account, non-adjustable**, and the whole
control plane shares a **10 rps / burst 40** budget with our deploys. A careless
loop here slows CI for everyone.

## Implementation

- `POST /api-tokens/api/key/rework`. Delete the old key, `CreateApiKey` +
  `CreateUsagePlanKey` for the new one, return the new value.
- **The cap:** allowed only when the current key's creation instant falls
  **before** the current quota period start (1st of the month, 00:00 UTC). Since
  a rework deletes and re-creates, the surviving key's `createdDate` is that
  instant — no stored timestamp is needed unless [[0190]] is built, in which case
  `coalesce(last_rotated_at, created_at)` is the same value from the table.
  Worked example from the 2026-08-07 meeting: reworked on 3 August → next rework
  available 1 September.
- **Why the fallback to creation time is not defensive.** Gating on a rotation
  timestamp alone leaves it null for every fresh key: a user could take a key on
  1 August, spend the whole quota, and rework on 2 August into a clean counter,
  because quota is scoped to `(usagePlanId, apiKeyId)`. Any key acquired inside
  the current period was created inside it, so it can never be reworked inside
  it. One key per period, one quota.
- **Refusal is `409`, not `429`** — `429` implies "retry shortly" when the wait
  can be weeks. Body is the existing `ErrorEnvelope`
  (`packages/prices-api/src/common/errors.rs`) with a new canonical code and
  `next_eligible_at` in `details`; the envelope already has the slot.
- **Requires a fresh eligibility proof** — membership only, never the account age
  ([[0189]] table). An account old enough once is old enough forever.
- **Frontend:** the action opens a modal stating plainly that the current key is
  deleted and stops working immediately, so anything using it breaks on confirm.
  Confirm stays **disabled until the user types `delete-key`**, and is disabled
  again on submit so a double-click cannot fire two reworks. The refusal path
  renders `next_eligible_at` — for a key reworked on 3 August, "1 September" —
  not a generic error.
- Unstyled, like every screen before [[0193]]. The modal's *wording* is this
  task's and does not get re-decided later.

## Acceptance Criteria

- [ ] **Ships closed.** With `PORTAL_ENABLED=false` ([[0183]]) this slice's
      routes return an empty `404`; with it on, they behave normally — every
      deploy goes straight to production
- [ ] Item 7 measured on the `DAY`-period proxy and written up with the date;
      [[0157]] and this task stop presenting the boundary as AWS-documented
- [ ] Rework issues a new key and deletes the old one in one operation; the user
      is never keyless
- [ ] The old key returns `403` immediately after; the new one returns `200`
- [ ] A second attempt in the same quota period is refused with `409` and
      `next_eligible_at`
- [ ] Reworking on 3 August refuses until 1 September and succeeds on
      1 September — the meeting's worked example, tested
- [ ] Rework is unreachable with a session cookie alone
- [ ] A user who has left the guild is refused on rework, with a message that
      names the server rather than a generic error
- [ ] The modal states the old key dies immediately; confirm is gated on typing
      `delete-key` and cannot double-fire
- [ ] `MONTH` confirmation scheduled for 1 September 2026 if the epic is open

## Notes

- A user who reworks on the last day of a period gets a fresh counter and a
  period reset a day later. Not an exploit — the reset was coming anyway — but
  written down so it is not re-raised as one.
- If the measured AWS rollover instant differs from ours, the dashboard renders
  our date and the counter does its own thing. A UX wrinkle, not a correctness
  bug: the cap is ours to define.
- The rework cap is why a leaked key cannot be invalidated until the 1st. That
  gap is [[0192]], and it is no longer blocked.
