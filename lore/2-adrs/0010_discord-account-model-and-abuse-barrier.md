---
id: "0010"
title: "Discord identity is the account: one active key, gated on guild membership and account age"
status: accepted
deciders: [akot]
related_tasks: ["0156", "0157", "0158", "0159", "0160", "0170", "0171"]
related_adrs: ["0007", "0008"]
tags: [discord, oauth, auth, abuse-prevention, account-model, api-keys, usage-plan, epic-self-service-onboarding]
links:
  - "../../docs/epics/self-service-onboarding.md"
  - "../1-tasks/active/0156_RESEARCH_self-service-auth-assumptions/notes/S-account-model-and-abuse-barrier.md"
history:
  - date: 2026-08-10
    status: accepted
    who: akot
    note: >
      Settles the two items the Self-Service Onboarding epic flagged "confirm
      before build". Scope and mitigation decided by Adam: guild membership
      required plus an account-age minimum. Written from 0156's five research
      notes, every source of which was re-verified against the original.
---

# ADR 0010: Discord identity is the account

**Related:**
- [Task 0156: Confirm the self-service auth assumptions](../1-tasks/active/0156_RESEARCH_self-service-auth-assumptions/README.md)
- [S-account-model-and-abuse-barrier](../1-tasks/active/0156_RESEARCH_self-service-auth-assumptions/notes/S-account-model-and-abuse-barrier.md) — the reasoning behind this ADR
- Builds against: [[0158]] (registry), [[0159]] (sign-in), [[0160]] (backend endpoints)

---

## Context

The Self-Service Onboarding epic declared its scope settled except for two items
it marked "confirm before build":

1. Whether Stellar's Discord actually verifies new accounts — the load-bearing
   assumption behind building no captcha and no email layer.
2. Whether one active key per Discord account is the agreed account model.

The epic stated the first plainly as an **unverified assumption**, and noted
that if the verification is not there, "this residual risk is bigger than
assumed and worth revisiting". Nothing else in the epic covers that gap.

Task 0156 investigated both. The findings that changed the decision:

- **Stellar Developers (`897514728459468821`) does have Membership Screening
  enabled** — so the epic's assumption was not baseless. But joining is a public
  one-click invite, and `verification_level` is `2` = *"must be registered on
  Discord for longer than 5 minutes"*.
- **Under the `identify` scope our flow observes none of it.** `verified` (email
  verified) requires the **`email`** scope; there is **no phone field on the
  OAuth2 User object at all**; guild membership sits behind a separate scope and
  a separate call. A bare `identify` flow observes exactly one thing: that a
  Discord account exists.
- **SDF's own SCF Dashboard does not treat a Discord account as sufficient** for
  anything with a cost — its verified-member flow adds linked-role social
  verification and Stellar wallet authentication on top.
- **AWS charges quota per `(usage plan, API key)` and has no principal that sums
  keys**, so multi-key would require our own aggregation — exactly the work the
  epic's rework cap exists to avoid.

---

## Decision

### 1. The Discord identity *is* the account

No separate registration, no email, no password, no manual approval. The Discord
user ID is the account key and the registry key ([[0158]]).

### 2. One active key per Discord account

Confirmed, not merely recommended. The epic's "Out of scope" phrasing ("one key
per Discord account only") is the correct reading; the "Recommendation" phrasing
under "Auth & key handling" is superseded by this ADR.

**AWS will not enforce this for us.** A key may sit in up to 10 usage plans; a
plan holds arbitrarily many keys; `name` is optional and not unique — only key
*values* are unique and enforced. One-active-key is an invariant the registry
owns in application code.

### 3. The abuse barrier is guild membership plus account age

**OAuth scope: `identify` + `guilds.members.read`. Never `guilds`.**

- Membership is checked against `GET /users/@me/guilds/{guild.id}/member`.
- **The guild ID is per-environment configuration in SSM, not a constant.**
- A **minimum Discord account age** is derived from the snowflake in the
  `identify` response — `(snowflake >> 22) + 1420070400000`, Discord epoch
  `1420070400000`. **The threshold is an SSM parameter**, not a literal.

**Threshold: 5 minutes — matching Stellar's own `verification_level: 2`**
(*"must be registered on Discord for longer than 5 minutes"*). Decided by Adam,
2026-08-10: we do not set a stricter bar than the server whose gate we depend
on.

**Be honest about what that buys.** Five minutes is Discord's raid speed-bump,
not an identity barrier — the research found no precedent above ten minutes
anywhere, including Discord's own product. At this value the age gate stops
scripted signup that mints an account and claims a key in the same breath, and
essentially nothing else. **Membership therefore carries the abuse story
alone**, and membership costs one public click plus a rules checkbox.

That is a deliberate trade, not an oversight:

- It refuses **no** legitimate newcomer. A developer who creates a Discord
  account specifically to use this API waits minutes, not days — the failure
  mode of a 7- or 30-day threshold, which would have been invisible to us and
  infuriating to them.
- The exposure it is defending is **$0.38 per fully-drained key per month**;
  286 drained keys reach ~$100/month. A stricter gate would cost more in
  refused real users than in prevented abuse.
- The threshold is an SSM parameter precisely so this can be raised without a
  deploy the moment churn is actually observed.

**Membership is checked once, at issuance. Decided by Adam, 2026-08-10.**
Neither the dashboard, the reveal path, nor rework re-checks it. This is the
consistent extension of the epic's existing non-goal (§7): a key, once issued,
keeps working regardless of later Discord state. Two consequences worth stating:

- **The registry stores no membership data** — no `pending`, no `joined_at`.
  Nothing re-reads them, so storing them would be holding data we never use,
  against a document that deliberately declines to hold an email address.
  [[0158]]'s schema needs no membership columns.
- Every portal call after sign-in is cheaper and cannot fail on a Discord
  outage. Only issuance depends on Discord being up.

### 4. Staging: `stellar_test` first, Stellar Developers later

Adam creates a **`stellar_test`** guild for building and testing the epic.
Integration against the production Stellar guild is a separate conversation with
SDF, tracked as [[0170]].

`stellar_test` must mirror the production posture or the tests prove nothing.
Membership Screening requires Community to be enabled first (*"In order to see
this feature, you must enable Community for your Discord server"*). Configure at
minimum:

- Community enabled → Rules Screening **on**, so `pending` exercises the real
  code path
- `verification_level` = `2`, matching Stellar Developers

### 5. Not building

Captcha, email confirmation, and manual first-key approval are costed and
declined. See [Alternatives Considered](#alternatives-considered).

### 6. Owners

| Responsibility | Owner |
|---|---|
| `stellar_test` guild — creation and configuration | Adam Kot (`akot`) |
| Discord application registration + redirect-URI lifecycle | Adam Kot (`akot`) |
| Stellar Discord (SDF) relationship | Tracked as [[0170]]; no named SDF counterpart is public as of 2026-08-10 |

This replaces the "someone" placeholder in [[0159]]. The published SDF contact
routes are `communityfund@stellar.org` and `#scf-general` (the handbook says the
channel is faster).

### 7. Non-goal, decided rather than deferred

**A user who leaves the Discord server after issuance keeps their key.** The
member object is read once during the OAuth callback and nothing pushes us an
update afterwards. This is a decision, not an open question.

---

## Rationale

**Why membership rather than `identify` alone.** `identify` alone observes only
that a Discord account exists, which is very close to no barrier: Discord's own
Getting Started documentation states that an unverified account *"will be able
to enjoy all of the chat functions Discord has to offer"*. Requiring membership
at least binds a key to an account that has joined the server and cleared
whatever gate the server operates.

**Why `guilds.members.read` and never `guilds`.** Settled by the docs' own scope
definitions without further argument: `guilds` is *"all of a user's guilds"*,
`guilds.members.read` is one named guild. And the partial guild objects returned
by `guilds` **carry neither `pending` nor `joined_at`** — so `guilds` is
strictly more privacy cost for strictly less signal. Note this means we
**deliberately diverge from SDF's own SCF Dashboard**, which requests
`identify email connections guilds`.

**Why account age as well.** It is free: no extra scope, no extra call, no
change to the consent screen, since the snowflake is already in the `identify`
response. It is the only mitigation on the list with $0 recurring cost and no
new external dependency. It also covers the case membership does not: an account
minted specifically to join and claim a key.

**Why the threshold is configuration, and why it starts at 5 minutes.** Discord's
own account-age thresholds are **5 minutes** (MEDIUM) and **10 minutes** (HIGH),
and no third-party API provider publishes account-age gating for free-tier
issuance at all. Rather than invent a number with no precedent, we mirror the
value Stellar's own server uses. The parameter exists so the number can move
when evidence arrives, not so it can be guessed higher now.

**Why one key is not merely convenient but required.** Quota is charged per
`(usage plan, API key)` — *"Throttling and quota limits apply to requests for
individual API keys that are aggregated across all API stages within a usage
plan."* The nearest thing to a user-shaped field, `customerId`, is documented
purely as *"An AWS Marketplace customer identifier"*. With concurrent keys,
AWS's native per-key monthly quota stops bounding a user's total consumption and
we would have to fan out `GetUsage` per key and sum it ourselves.

**Proportionality — what the abuse is worth.** A fully-drained key (100k quota,
3 KB responses, us-east-1) costs **$0.38/month**; 286 drained keys reach
~$100/month. Optional API Gateway caching alone is $27.36/month = 72 abusive
keys. This is why no paid mitigation is justified, and why the quota — which
cuts worst-case per-key exposure **26×** versus the throttle alone — remains the
most effective control in the design.

---

## Alternatives Considered

### Alternative 1: `identify` alone, no membership check

**Description:** Accept the epic as written — Discord login is the only barrier.

**Pros:** Minimal consent screen; no dependency on another org's server config.

**Cons:** Observes only that a Discord account exists. Discord's own docs
confirm unverified accounts are fully functional, there is no phone field to
read, and `verified` costs the `email` scope. The epic's stated barrier would be
invisible to the software.

**Decision:** REJECTED — the residual risk the epic accepted was premised on a
barrier we would never observe.

### Alternative 2: `identify` + `guilds`

**Description:** Request the broad scope, as SDF's own SCF Dashboard does.

**Pros:** Ecosystem precedent; one call; no guild ID configuration.

**Cons:** Returns every server the user belongs to (up to 200) — data we have no
reason to see — and the partial guild objects **lack `pending` and `joined_at`**.
Strictly more privacy cost for strictly less signal.

**Decision:** REJECTED.

### Alternative 3: Captcha (Turnstile / hCaptcha / reCAPTCHA)

**Pros:** Turnstile's free plan publishes *"Unlimited challenges (traffic or
verification requests)"*.

**Cons:** Raises the cost of *automating* signups; does nothing against a human
churning accounts, which is the threat in question. hCaptcha Pro at $139/month
is 366 abusive keys' worth; its free-tier volume cap is undocumented.
reCAPTCHA's free allowance is *"per organization"* and exhaustion returns a hard
`Resource Exhausted (429)` — an availability coupling on key issuance.

**Decision:** REJECTED for now. If *scripted* signup is ever observed, use
Turnstile — the only one with a published volume-unlimited free tier and a
failure mode we control.

### Alternative 4: Email confirmation

**Cons:** Cannot ship without an AWS Support ticket — the SES sandbox permits
*"a maximum of 200 messages per 24-hour period"* and only *"to verified email
addresses and domains"*. Needs a sending domain, DKIM, and bounce handling. And
it re-proves something Discord already holds: Discord's LOW verification level
is literally *"must have verified email on account"*.

**Decision:** REJECTED — the most work for the least proof.

### Alternative 5: Manual approval for the first key

**Cons:** At an assumed $60/h fully loaded, a 2-minute review costs $2.00 ≈ 5.3
fully-drained keys. Cost scales with *legitimate* signups while the abuse it
prevents does not, and it deletes the "self-service" property the epic is named
after.

**Decision:** REJECTED.

### Alternative 6: Multiple concurrent keys per account

**Cons:** AWS quota is per `(usage plan, API key)` with no aggregating
principal, so a per-user cap becomes our own fan-out and summation. Also breaks
the rework cap, which is coherent only under one key.

**Decision:** REJECTED.

---

## Consequences

### Positive

- The abuse barrier is now something the software actually observes, rather than
  an assumption about a server we never contact.
- Two independent gates with $0 recurring cost and no new vendor.
- `pending`, `joined_at`, `roles` and `flags` become available for finer rules
  later without another consent change.
- One-key is confirmed, so [[0158]]'s schema and [[0160]]'s rework cap are
  correct as designed and need no reshaping.
- The guild ID being configuration lets `stellar_test` and production differ
  without a code change.

### Negative

- **We take a dependency on another organisation's server configuration.** If
  SDF disables Membership Screening, restructures roles, or migrates the guild,
  our issuance changes behaviour with no notice. Discord actively markets
  Onboarding as a *replacement* for verification friction — step 5 of its setup
  guide is *"Remove verification steps that overwhelm or lock new members"* — so
  a well-run server is **less** likely to keep an observable gate over time.
- The consent screen gains a scope line, permanently.
- A legitimate newcomer with a brand-new Discord account waits ~5 minutes.
  Deliberately small; the cost of a larger threshold falls almost entirely on
  real users.
- **The age gate is close to symbolic at this value.** Anyone reading this ADR
  later should not mistake "we have two gates" for "we have two barriers" — we
  have one barrier (membership) and one speed-bump.
- The not-a-member path depends on an **undocumented** error shape ([[0171]] #1).
- `pending === false` is not proof of having passed anything —
  `BYPASSES_VERIFICATION` (*"Member is exempt from guild verification
  requirements"*) means an admin can wave a member through.

### What would reverse this decision

- **SDF disabling Membership Screening on the production guild, or declining the
  integration in [[0170]].** This is now the load-bearing risk, not one of
  several: with the age threshold at 5 minutes, screening is the only part of
  the barrier that costs an abuser anything beyond a click. If it goes away,
  the barrier is "joined a public server" and we have **nothing else** — at
  which point the age threshold must be raised, or a captcha added, or the free
  quota lowered. Whoever picks up [[0170]] should treat "will you keep screening
  on, and would you tell us if it changed" as the question that matters most.
- **Observed churn.** Raise the SSM threshold first — it is a config change and
  costs nothing. Only if churn continues does captcha (Turnstile) become
  justified.
- Observed *scripted* signup specifically — that is the threat captcha
  addresses, and a 5-minute age gate barely touches it.
- The all-in per-call backend cost ([[0171]] #7) turning out to dominate the
  gateway figure by an order of magnitude — every exposure number in the
  rationale scales with it, and a $0.38/key threat model becoming a $4/key one
  changes what mitigation is proportionate.

---

## Corrections this ADR makes to already-written tasks

Both were checked directly against AWS documentation. Neither is fatal; both
must be corrected before build ([[0171]]).

1. **`nameQuery` is not documented as a prefix match.** [[0158]] and [[0160]]
   build a reconciler and a "prefix hazard" guard on that premise. AWS documents
   one sentence — *"The name of queried API keys."* — and states no matching
   semantics. The client-side exact-match filter is therefore **load-bearing,
   not defence in depth**.
2. **The monthly quota reset instant is undocumented.** "1st of the month,
   00:00 UTC" appears in [[0157]]/[[0158]]/[[0160]] as if it were AWS behaviour.
   AWS's only statement is an example caption — *"creates a usage plan that
   resets at the beginning of the month"* — with no timezone and no instant.
   `offset` is a **request count**, not a time shift. Keep the rule as **our**
   product decision; do not present it as inherited AWS semantics.

---

## References

Every source below was fetched on 2026-08-10 and its quoted text verified
against the original.

- [Usage plans and API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-api-usage-plans.html) — quota per API key; not hard limits
- [GetApiKeys](https://docs.aws.amazon.com/apigateway/latest/api/API_GetApiKeys.html) — `nameQuery`, `customerId`
- [QuotaSettings](https://docs.aws.amazon.com/apigateway/latest/api/API_QuotaSettings.html) — `offset` is a request count
- [Amazon API Gateway quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/limits.html) — control-plane rates
- [Amazon API Gateway Pricing](https://aws.amazon.com/api-gateway/pricing/) — $3.50/million, $0.09/GB, $0.038/hr cache
- [Discord — OAuth2](https://docs.discord.com/developers/topics/oauth2) — scope definitions
- [Discord — User Resource](https://docs.discord.com/developers/resources/user) — `verified` requires `email`; no phone field
- [Discord — Guild Resource](https://docs.discord.com/developers/resources/guild) — verification levels, `pending`, member flags
- [Discord — API Reference](https://docs.discord.com/developers/reference) — snowflake epoch and formula
- [Discord invite API — `stellardev`](https://discord.com/api/v10/invites/stellardev?with_counts=true) — guild `897514728459468821`, `verification_level: 2`, `MEMBER_VERIFICATION_GATE_ENABLED`
- [Verification Levels](https://support.discord.com/hc/en-us/articles/216679607-Verification-Levels) — phone supersedes all other requirements
- [Rules Screening FAQ](https://support.discord.com/hc/en-us/articles/1500000466882-Rules-Screening-FAQ) — Community required for screening
- [SES sandbox / production access](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html) — 200 msgs/24h, verified recipients only
- [Cloudflare Turnstile plans](https://developers.cloudflare.com/turnstile/plans/) — unlimited challenges on Free
- [Google reCAPTCHA billing](https://docs.cloud.google.com/recaptcha/docs/billing-information) — 10,000/month per organization
