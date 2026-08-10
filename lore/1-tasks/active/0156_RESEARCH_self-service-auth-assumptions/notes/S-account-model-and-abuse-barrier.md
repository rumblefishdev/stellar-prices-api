---
title: "Decision: Discord identity is the account, membership is the barrier, one active key"
type: synthesis
status: mature
spawned_from: notes/Q-do-the-two-flagged-auth-assumptions-hold.md
spawns:
  - ../../../../2-adrs/0010_discord-account-model-and-abuse-barrier.md
tags: [discord, auth, abuse-prevention, account-model, decision]
links:
  - "../../../../../docs/epics/self-service-onboarding.md"
history:
  - date: 2026-08-10
    status: seed
    who: claude
    note: "Drafted from the five research notes"
  - date: 2026-08-10
    status: mature
    who: akot
    note: >
      Scope and mitigation settled by Adam: membership in the Stellar guild is
      required, plus an account-age minimum. Test guild owned by Adam.
---

# Decision: the account model and what the abuse barrier actually is

Synthesises [R-discord-platform-verification-mechanics](R-discord-platform-verification-mechanics.md),
[R-discord-oauth-observable-signals](R-discord-oauth-observable-signals.md),
[R-stellar-discord-server-posture](R-stellar-discord-server-posture.md),
[R-apigw-usage-plan-quota-mechanics](R-apigw-usage-plan-quota-mechanics.md) and
[R-abuse-mitigation-options-costed](R-abuse-mitigation-options-costed.md).

Every source cited in those notes was re-fetched and checked against the quoted
text before this synthesis was written — see
[Source verification](#source-verification) at the end.

## Question 1 — does the epic's abuse barrier hold?

**No, as written. The epic's chain breaks at a step it does not mention.**

The epic argues: Discord login is the only barrier → it suffices because
Stellar's Discord verifies new members → therefore no captcha/email layer is
needed. Both halves fail, but not in the way the epic anticipated.

### 1a. The barrier exists — and is thinner than assumed

Stellar Developers (`897514728459468821`, 32,419 members) **does** have
Membership Screening enabled: the public invite API returns
`MEMBER_VERIFICATION_GATE_ENABLED` in `guild.features`. So `pending` is a real,
meaningful signal on that guild — the epic's assumption is not baseless.

But the rest of the posture is weak:

| Property | Value | What it costs an abuser |
|---|---|---|
| Joining | Public one-click invite (`discord.gg/stellardev`), listed in Server Discovery | One click |
| `verification_level` | `2` = MEDIUM = "must be registered on Discord for longer than 5 minutes" | Five minutes |
| Membership Screening | Enabled; a click-through rules agreement | One click |
| Role gating | General channels open to roleless members | Nothing |

Discord's own docs add that **"Having a verified phone number supersedes all
other requirements"** — so even the five-minute clock evaporates for a
phone-verified account.

### 1b. The decisive finding: `identify` observes none of it

This is the part the epic misses entirely. Discord OAuth authenticates a
*Discord account*, and under the `identify` scope we can observe **no account-
quality signal whatsoever**:

- `verified` (email verified) is documented as requiring the **`email`** scope,
  not `identify`. The User Structure table carries a "Required OAuth2 Scope"
  column and lists `verified?` against `email`.
- **There is no phone field on the OAuth2 User object at all**, under any scope.
  Discord's account-level phone verification is invisible to us.
- Guild membership, `pending`, `joined_at` and `roles` all live behind a
  *separate scope and a separate API call*.

So a bare `identify` flow observes exactly one thing: **that a Discord account
exists.** The barrier the epic leans on is real but was, as specified,
completely invisible to the software.

### 1c. SDF's own service does not rely on Discord identity alone

The epic claims Discord OAuth "matches how other Stellar/SCF-ecosystem services
authenticate". That is **verified but only singular** — the SCF Dashboard is one
first-party example, and its redirect chain was reproduced unauthenticated.

The caveat matters more than the claim. SCF requests
`scope=identify email connections guilds`, and its *verified member* flow
requires, on top of Discord: linked-role social verification **and** Stellar
wallet authentication. **SDF does not treat a Discord account as sufficient for
anything with a cost attached.** That is the strongest available evidence
against the epic's single-barrier model.

### 1d. What the abuse is actually worth

Costed against current AWS pricing (us-east-1, REST, `$3.50/million` +
`$0.09/GB`, 3 KB responses):

| Fully-drained keys (100k quota) | Cost/month |
|---|---|
| 1 | **$0.38** |
| 10 | $3.76 |
| 100 | $37.57 |
| 286 | ~$100 |

For calibration: optional API Gateway caching at `$0.038/hr` is **$27.36/month
flat = 72 abusive keys**. Any mitigation with a licence cost above ~$25/month
costs more than the abuse it prevents.

**Caveat that must not be lost:** these are *gateway-only* figures. Backend cost
per call is unpriced and probably dominant. See
[Open items](#open-items-carried-into-the-adr).

The most effective control is already in the epic and is not a new idea: the
**monthly quota**, which cuts worst-case per-key exposure 26× versus the
per-second throttle alone (1 req/s sustained = 2.59M calls = $9.07/month;
capped at 100k = $0.38).

## Decision on Question 1

**Require Stellar guild membership, and add an account-age minimum.**
(Settled by Adam, 2026-08-10.)

- **Scope: `identify` + `guilds.members.read`. Never `guilds`.** Justified by
  the docs' own scope definitions without further argument — `guilds` is defined
  as "all of a user's guilds", `guilds.members.read` as one named guild. And the
  partial guild objects returned by `guilds` **carry neither `pending` nor
  `joined_at`**, so `guilds` is strictly more privacy cost for strictly less
  signal.
- **Account-age minimum from the snowflake.** Free: `id` is `identify`-scope and
  the timestamp is pure arithmetic — `(snowflake >> 22) + 1420070400000`, epoch
  `1420070400000`. No extra scope, no extra call, no consent-screen change.
  Threshold lives in SSM, not in code.
  - **Threshold set to 5 minutes**, matching Stellar's `verification_level: 2`
    (Adam, 2026-08-10) — we do not set a stricter bar than the server whose
    gate we depend on, and it refuses no legitimate newcomer.
  - **Consequence, stated plainly: at 5 minutes the age gate is a speed-bump,
    not a barrier.** Discord's own thresholds are 5 and 10 minutes and no
    third-party provider publishes age gating at all, so there is no precedent
    to appeal to for anything higher. **Membership carries the abuse story
    alone** — and membership is one public click plus a rules checkbox. That is
    proportionate to a $0.38/key exposure, but it means [[0169]]'s question
    "will SDF keep screening on" is the one that actually matters.
- **Membership is checked once, at issuance** (Adam, 2026-08-10). Nothing
  re-checks it. This extends the epic's existing non-goal consistently, keeps a
  Discord outage from breaking the dashboard for existing users, and means the
  registry stores **no** membership data — so [[0158]]'s schema is untouched.
- **Not building:** captcha, email confirmation, manual approval. Costed and
  declined — see [R-abuse-mitigation-options-costed](R-abuse-mitigation-options-costed.md) §6.
  Email confirmation is the worst of both: most work (SES production access via
  AWS Support, domain, DKIM, bounce handling) for the least proof, and it
  re-establishes a fact Discord already holds.

### Staging: test guild first, Stellar guild later

Adam creates a **`stellar_test`** guild for building and testing the epic. The
production Stellar guild integration is a separate conversation with SDF and is
spawned as its own task.

**This makes the guild ID per-environment configuration, not a constant** — which
is what [[0158]]/[[0159]] should build against anyway. `stellar_test` in dev,
`897514728459468821` in production, both from SSM.

**`stellar_test` must mirror the production posture or the tests prove nothing.**
Membership Screening requires Community to be enabled on the guild
(*"In order to see this feature, you must enable Community for your Discord
server"*). Configure, at minimum:

- Community enabled → then Rules Screening ON, so `pending` behaves as it does
  on the real guild
- `verification_level` = `2`, matching Stellar Developers

Without those, `pending` will not exercise the code path that matters.

## Question 2 — one active key per Discord account?

**Confirmed. And AWS forces it — the epic is right, but not for the reason it
gives.**

The epic contradicts itself (a "Recommendation … confirm before build" under
"Auth & key handling", settled fact under "Out of scope"). The second reading is
correct. The AWS mechanics settle it:

- **Quota is charged per (usage plan, API key).** *"A quota limit sets the
  target maximum number of requests with a given API key…"* and *"Throttling and
  quota limits apply to requests for individual API keys that are aggregated
  across all API stages within a usage plan."*
- **There is no AWS principal that aggregates keys.** The nearest candidate,
  `customerId`, is documented purely as *"An AWS Marketplace customer
  identifier"* and is a listing filter on `GetApiKeys`. No page states that
  quota aggregates over it.

So under a multi-key model, bounding one user's total consumption becomes our
own fan-out over N `GetUsage` calls plus our summation — **precisely the
aggregation work the epic's rework cap exists to avoid.** The rework cap is
coherent only under one key. Confirmed as stated.

Two consequences the tasks must carry:

1. **AWS will not enforce one-key-per-account for us.** A key may sit in up to
   10 usage plans; a plan holds arbitrarily many keys; `name` is optional and
   **not** unique (only *values* are unique and enforced). One-active-key is an
   invariant [[0158]]'s registry must own.
2. **Delete-then-create resetting the quota counter is a derivation, not a
   documented guarantee.** `DeleteApiKey` says nothing about usage. It follows
   from usage being indexed by a key ID that `CreateApiKey` re-mints, with no
   lineage field on `ApiKey`. The ADR should *show* the derivation rather than
   assert the behaviour — it is exactly the loophole the cap closes.

## Two claims in already-written tasks that the sources do not support

Both were checked directly against AWS documentation and both are wrong as
written. Neither is fatal; both need correcting before build.

1. **`nameQuery` is not documented as a prefix match.** [[0158]] and [[0160]]
   build a reconciler and a "prefix hazard" guard on the premise that
   `GetApiKeys(nameQuery=…)` matches by prefix. AWS documents one sentence —
   *"The name of queried API keys."* — and says nothing about matching
   semantics. The prefix behaviour is community knowledge, not an AWS contract.
   The client-side exact-match filter those tasks specify is therefore
   **load-bearing, not belt-and-braces**, and the reasoning should say so.
2. **The monthly quota reset instant is undocumented.** [[0157]]/[[0158]]/[[0160]]
   state "1st of the month, 00:00 UTC". AWS never says this. The only statement
   anywhere is an example caption — *"creates a usage plan that resets at the
   beginning of the month"* — with no timezone and no instant. `offset` is a
   **request count**, not a time shift (*"The number of requests subtracted from
   the given limit in the initial time period"*). The worked example
   "reworked 3 August → next 1 September" is a product decision we can keep, but
   it must be stated as *our* rule, not as AWS behaviour we inherit.

## Non-goal, recorded as a decision

**A user who leaves the Discord server after issuance keeps their key.** The
epic resolves this deliberately. The research confirms it is also the only
practical option: the member object is read once during the OAuth callback and
nothing pushes us an update afterwards. Recorded in the ADR as a decision, not
an open question.

## Open items carried into the ADR

Cheap to settle once `stellar_test` and the Discord app exist — which makes app
registration a **dependency of finalising the implementation**, not a
consequence of it.

| # | Unknown | Why it matters |
|---|---|---|
| 1 | Exact status code when the user is not a guild member | The membership check is a *negative* inferred from an undocumented error shape. Only `404` + error codes `10004`/`10007` are documented generically |
| 2 | Is `pending` present on the `guilds.members.read` REST response? | Field is optional (`?`); the docs' presence guarantee is written about **gateway events**, not this route. `pending === undefined` must be a third state |
| 3 | Is `flags` populated on that response? | Non-optional in the field table, but that is inference. Any rule using `COMPLETED_ONBOARDING` depends on it |
| 4 | `nameQuery` prefix vs exact | See above |
| 5 | Quota reset instant and timezone | See above |
| 6 | Does `UpdateApiKey(enabled=false)` preserve/zero usage counters? | Undocumented. The delete-then-create derivation does **not** carry over, since the key `id` survives |
| 7 | All-in per-call backend cost | Every cost figure here is gateway-only. If backend dominates 10×, exposure scales 11× |

Also unresolved and **not** settleable by us: whether `pending === false` proves
anything. `BYPASSES_VERIFICATION` (`1 << 2`, *"Member is exempt from guild
verification requirements"*) means an admin can wave a member through.

A further caution from Discord's own guidance: Community Onboarding is marketed
as a *replacement* for verification friction — step 5 of the setup guide is
literally *"Remove verification steps that overwhelm or lock new members"*. A
well-run server following current Discord advice is **less** likely to keep an
observable gate. Our dependency on `MEMBER_VERIFICATION_GATE_ENABLED` staying on
is a dependency on another organisation's product decision.

## Source verification

Every URL cited across the five research notes was re-fetched and the quoted
text compared against the source before this note was written.

| Note | Sources | Result |
|---|---|---|
| AWS usage plans | 18 URLs | All quotes exact |
| Discord OAuth | 12 URLs | All quotes exact, including the `verified` → `email` scope finding |
| Stellar Discord | invite API, SCF redirect chain, handbook, docs | Live endpoints re-run independently; byte-identical |
| Discord platform | support articles via the public Help Center JSON API | All quotes exact |
| Mitigation costs | 15 URLs | Prices exact; all arithmetic recomputed and correct |

One genuine contradiction **within Discord's own documentation** was found and
is recorded rather than resolved: the API reference gives MEDIUM as *"must be
registered on Discord for longer than 5 minutes"*, while the support article
gives Medium as email *"verified for longer than five minutes"*. The sources
disagree; it must be tested, not cited.
