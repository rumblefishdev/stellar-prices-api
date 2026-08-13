---
title: "Measurement runbook — ordered steps, and who has to do each"
type: generation
status: developing
spawned_from: notes/Q-which-undocumented-behaviours-hold.md
spawns: []
tags: [runbook, discord, aws, api-gateway, throttling]
links:
  - "../../../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-08-12
    status: developing
    who: akot
    note: "Runbook written at task activation; steps 1-3 executable, results go into the three R- notes"
---

# Measurement runbook

Ordered so that nothing waits on the Discord setup that does not have to.
Results are written into the three `R-` notes, not here — this file is the
*procedure*, kept executable without re-reading the reasoning.

**Division of labour:** steps marked **[Adam]** need a human in a browser or an
account only Adam owns. Steps marked **[Claude]** are scriptable and can be
delegated in-session.

---

## Step 0 — before anything **[Adam]**

The task's manual prerequisite. Nothing Discord-related exists in the repo yet —
`infra/src/lib/stacks/api-gateway-stack.ts:396` mentions Discord only in a
comment, and there are no SSM parameters or secrets.

- [ ] Register the application in the Discord Developer Portal
- [ ] Declare scopes `identify` + `guilds.members.read` (**never** `guilds`)
- [ ] Add a redirect URI usable locally (e.g. `http://localhost:8787/callback`)
- [ ] Keep the client secret out of the repo — SSM/Secrets Manager or a local
      env file that is gitignored
- [ ] `stellar_test` guild: Membership Screening **on**
- [ ] **Second scratch guild: Membership Screening off** — item 4 is a
      comparison and one guild cannot answer it
- [ ] **Second Discord account that is NOT a member of `stellar_test`** —
      without it item 1 (the 404 shape) is unmeasurable
- [ ] Confirm AWS access: profile `stellar` is configured; a scratch usage plan
      needs create/update rights on `apigateway`

---

## Step 1 — cost, first **[Claude]**

Runs **now**, before any setup: no Discord app, no scratch plan, no guild.
Arithmetic over CloudWatch metrics the deployed `prices-api` already emits.

- [ ] Pull `Duration` p50/p95, invocation count, response sizes and log-ingest
      volume over a stated window
- [ ] Allocate the fixed ClickHouse box cost per request (ADR 0007 — it is not
      usage-priced; say plainly that this is an allocation)
- [ ] Multiply by the monthly quota → cost of one fully-drained key
- [ ] Compare against ADR 0010's $0.38 → does proportionality survive?

→ results into [R-all-in-per-call-cost.md](R-all-in-per-call-cost.md)

---

## Step 2 — AWS, on a scratch usage plan **[Claude]**, credentials **[Adam]**

> ⚠️ **Never the production plan.** `pricing-api-free-<env>` is real and its ID
> is published to SSM (`api-gateway-stack.ts:416`).
>
> ⚠️ **Throttling is the real constraint.** `UpdateUsagePlan` is **1 request per
> 20 seconds per account, non-adjustable**, and the whole control plane shares a
> **10 rps / burst 40** budget **with our deploys**. A careless loop here slows
> CI for everyone. Sleep between calls; never retry tightly.

**2a — `nameQuery` (item 6).** Create keys with deliberately overlapping names
(`disc-111111111111111111`, `…-old`, `disc-1111`), then `GetApiKeys` with: the
exact name, a strict prefix, a substring that is not a prefix, and a
case-differing variant. Record which keys come back for each.

**2b — quota rollover (item 7).** A real `MONTH` rollover cannot be seen before
1 September and we are not waiting. Create the scratch plan with period
**`DAY`**, drain some quota, and watch the reset instant. Use `UpdateUsage`
(`op:replace` on `/remaining`) to construct states cheaply instead of making
real calls.

**2c — `enabled=false` (item 8).** Consume known quota → `GetUsage` →
`UpdateApiKey(enabled=false)` → `GetUsage` → re-enable → `GetUsage`. Verdict:
preserved, frozen or zeroed.

**2d — tear down the scratch plan and its keys.**

→ results into [R-apigw-namequery-quota-and-disable.md](R-apigw-namequery-quota-and-disable.md)

---

## Step 3 — Discord, after Step 0 **[Adam]** for the browser half

**3a — get two user tokens.** A throwaway local OAuth callback is enough; the
tokens are short-lived and must not be committed. One token from the member
account, one from the non-member account.

**3b — capture the consent screen (item 5)** while authorising: once with
`identify` only, once with `identify` + `guilds.members.read`. Screenshots into
`sources/`. Do this now — it is free while the browser flow is open, and
awkward to reproduce later.

**3c — call the endpoint in four combinations [Claude]:**

```
GET /api/v10/users/@me/guilds/{guild.id}/member
Authorization: Bearer <user token>
```

| Token | Guild | Answers |
|---|---|---|
| member | `stellar_test` (screening on) | items 2, 3 |
| **non-member** | `stellar_test` | **item 1 — the 404 shape** |
| member | scratch (screening **off**) | item 4 |
| member | bogus guild id | item 1 — distinguishes `10004` from `10007` |

Record full status, JSON `code`, and whether `pending` / `flags` are present —
**absent is a result, not a blank**.

→ results into [R-discord-member-endpoint-response-shape.md](R-discord-member-endpoint-response-shape.md)

---

## Step 4 — write the findings back **[Claude]**

The measurements are not the deliverable. These edits are:

- [ ] [[0158]] + [[0160]] — `nameQuery` reasoning; client-side exact-match
      filter documented as **load-bearing, not defence in depth**
- [ ] `docs/runbooks/manual-api-key-tier.md` — whether `--name-query` returns as
      a server-side prefilter, per [[0157]]'s open invitation
- [ ] [[0157]] / [[0158]] / [[0160]] — quota-period wording restated as **our
      product decision**, not inherited AWS behaviour
- [ ] [[0160]] "Open" — revocation now costable, record the `enabled=false` verdict
- [ ] [[0159]] — the fail-closed/fail-open rule for the membership check
- [ ] [[0162]] — "could not verify" renders differently from "not a member"
- [ ] ADR 0010 — only if a finding changes its shape (cost is the likely one)

---

## Parallel, not blocked by any of this

[[0179]] step 3 — verify the membership call against the **real** Stellar guild
(`897514728459468821`) with one account already a member. Under our control, no
SDF involvement, and if `pending`/`flags` behave differently there than on
`stellar_test`, that changes the code. Cheaper to learn now than at launch.
