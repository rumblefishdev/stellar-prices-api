---
id: "0189"
title: "Eligibility gate — Stellar Discord membership and minimum account age before a key is issued"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0183", "0156", "0159", "0179", "0180", "0186", "0187", "0191", "0193"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, discord, auth, abuse-prevention, spike, slice-6]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../archive/0159_FEATURE_discord-oauth-sign-in.md"
  - "../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/notes/R-discord-member-endpoint-response-shape.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Sixth slice, the second half of [[0159]], and the new home of [[0180]]
      items 1–5. Those five measurements were a task-shaped blocker in front of
      the entire epic; they are actually the first hour of this one task, and
      nothing before this slice depends on them.
---

# Eligibility gate — membership and account age

## Summary

**Story:** *as the operator, I want a key to be issuable only by a member of the
Stellar Discord whose account is not brand new — so the abuse barrier is real
rather than assumed.*

This is the whole of the epic's abuse story. Until it lands, [[0187]] issues a
key to anyone with a Discord account, which is acceptable on a dev distribution
and must not reach production.

## Step 0 — measure first (was [[0180]] items 1–5)

Five behaviours the design leans on are **undocumented**. Measure them before
writing the branch that depends on them, and write the results into this task
with the date — converting it to a directory with `notes/` if they run long, per
the task-size convention. Prerequisites, all owned by Adam: the Discord app
from [[0186]] extended with `guilds.members.read`, the `stellar_test` guild with
Membership Screening **on** and `verification_level: 2`, **a second scratch guild
with screening off**, and **a second account that is not a member**.

| # | Question | Why it is load-bearing |
| --- | --- | --- |
| 1 | Status code + JSON error code when the user is **not** a member | Fail closed on a `429` and legitimate users are denied; fail open on a `404` and the barrier is void. Highest stakes item in the epic |
| 2 | Is `pending` present on the REST member response? | The docs' presence guarantee is written about gateway events, not this route |
| 3 | Is `flags` populated on that response? | `BYPASSES_VERIFICATION` changes what `pending === false` means |
| 4 | What `pending` means with screening **off** | Needs the second guild — this is the comparison |
| 5 | Consent-screen copy with and without `guilds.members.read` | Friction on the one screen every user sees |

Detail and reasoning: archived
`0180_RESEARCH_.../notes/R-discord-member-endpoint-response-shape.md` and
`notes/G-measurement-runbook.md`. Do not re-derive them.

## Implementation

- **Add `guilds.members.read` to the scope** — in the Developer Portal
  registration as well as the authorize URL; scopes "must be declared in the
  Developer Portal". Still never `guilds` (it returns every server the user is
  in, and its partial guild objects carry neither `pending` nor `joined_at`) and
  never `email`.
- **Membership check:** `GET /users/@me/guilds/{guild.id}/member`, guild id from
  SSM.
- **Three outcomes, not two: eligible, ineligible, unknown.** Only an explicit
  `10007`/`10004`-style `404` is ineligible. `401`/`403`/`429`/`5xx` is unknown —
  do not issue, and **do not tell the user they are not a member**. Step 0 item 1
  pins the exact shape.
- **`pending` is optional (`pending?`).** Treat `undefined` as a third state;
  never read absent as "cleared". And `pending === false` can mean an admin waved
  someone through via `BYPASSES_VERIFICATION`.
- **Minimum account age from the snowflake:** `(BigInt(id) >> 22n) +
  1420070400000n`. Costs no extra scope and no extra consent line — `id` is
  already in the `identify` response. Use `BigInt`; `Number` loses precision
  above 2^53. Threshold **5 minutes**, matching Stellar's own
  `verification_level: 2`.
- **Two SSM parameters, operator-seeded, read at runtime:**

  | Parameter | Value |
  | --- | --- |
  | `/prices/{env}/discord-guild-id` | `stellar_test` while building, `897514728459468821` after [[0179]] |
  | `/prices/{env}/min-account-age-minutes` | `5` |

  **Do not write `new ssm.StringParameter`.** A CloudFormation-managed parameter
  is CDK-owned, so the next `cdk deploy` silently restores the committed value —
  un-flipping production back to the test guild after [[0179]] step 4. The repo's
  precedent is explicit: `SecretsStack` deliberately does not create the secrets,
  and `compute-stack.ts` *reads* `/prices/{env}/ledger-processor/initial-cursor`
  because the operator seeds it at deploy prep. Read via the SSM SDK, **not**
  `valueForStringParameter` — the latter resolves at deploy and would make the
  threshold un-tunable without a redeploy, defeating the point.
  No IAM work needed: `lambda-baseline.ts` already grants `ssm:GetParameter*` on
  `arn:…:parameter/prices/{env}/*`.
- **Eligibility is proved per action, never carried in the session** (ADR 0010
  §8). A signed "eligible" claim would date the verdict to sign-in time and would
  not survive to a rework weeks later.

  | Path | Re-auth | Checks |
  | --- | --- | --- |
  | Sign in | — | identity only |
  | Issue a key | **yes** | membership (`pending === false`) + account age |
  | Reveal / usage | no | session only |
  | Rework ([[0191]]) | **yes** | membership only — age is never re-checked |

  `state` from [[0186]] carries the intended action, signed; the callback
  exchanges the code for a **fresh** token, checks, and only then hands off to
  the issue path. Discord does not re-prompt for consent on repeat authorisation
  of the same scopes, so the second round-trip is a redirect, not a login.
- **Frontend, plain but specific.** Both refusals are fixable by the user, and
  a generic error makes them abandon:
  - **Not a member** — name the server, link `discord.gg/stellardev` (the
    registered vanity code; the other invites SDF publishes are personal and one
    is already dead, [[0179]]), and let retry re-run the round-trip.
  - **Too young** — this is a *wait*, not a rejection. Render the time
    remaining and allow retry in place. **Not a calendar date** — that pattern is
    right for [[0191]]'s weeks-long cap and absurd for five minutes. Do not
    hard-code "5 minutes"; drive the copy from what the backend returns.
  - **"Could not verify" must render differently from "not a member."** A
    Discord outage is not an accusation the user can act on.
  - **The landing page states both prerequisites before the user authenticates.**
    Learning about the membership requirement afterwards means they authorised an
    app for nothing.

  Styling is [[0193]]'s; the wording is this task's.

## Acceptance Criteria

- [ ] **Ships closed**, and this is the slice [[0194]] is waiting on before
      `portal-enabled` may be flipped to `true` — the gate must pass first
- [ ] Items 1–5 measured against `stellar_test` and a screening-off scratch
      guild, results written down with the date
- [ ] Consent screen captured with and without `guilds.members.read`
- [ ] Scope is exactly `identify` + `guilds.members.read`, in the Developer
      Portal as well as the authorize URL
- [ ] A non-member is refused and no key is created
- [ ] A `429` or `5xx` from Discord refuses **without** claiming non-membership,
      and renders as "try again shortly"
- [ ] `pending === undefined` is handled explicitly and does not silently pass
- [ ] An account below the threshold is refused with the time remaining
- [ ] Both SSM parameters are operator-seeded and read at runtime; changing
      `min-account-age-minutes` takes effect **without a redeploy**
- [ ] The guild id survives a `cdk deploy` unchanged
- [ ] Parameter names and the seeding step are in the deploy-prep runbook,
      alongside the mTLS material
- [ ] Issue is unreachable with a session cookie alone — verified by calling it
      directly with nothing else
- [ ] Reveal and usage still work, with no re-auth, for a user who has left the
      guild

## Notes

- The epic's non-goal, worth a code comment: a user who later leaves the server
  keeps their key. Sign-in proves membership at the moment of issuance and
  nothing afterwards.
- Production points at the real Stellar guild only after [[0179]]. Until then
  this gate is real but gated on our own test guild — which [[0164]] is explicit
  is *not* evidence of a functional flow for an outside developer.
