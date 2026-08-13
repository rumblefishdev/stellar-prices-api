---
id: "0010"
title: "Discord identity is the account: one active key, gated on guild membership and account age"
status: accepted
deciders: [akot]
related_tasks: ["0156", "0157", "0158", "0159", "0160", "0179", "0180"]
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
  - date: 2026-08-13
    status: accepted
    who: akot
    note: >
      Measurements from 0180 written back. Status unchanged — the decision
      stands and no alternative is reopened. Proportionality now carries
      measured numbers instead of an estimate: $0.55-0.89 per fully-drained key
      all-in, against the $0.38 gateway-only figure this ADR was written on, so
      the "backend cost might dominate by 10x" reversal trigger is retired.
      Three errors corrected on the way: $3.50/million and $0.038/hr are
      us-east-1 rates and we are in eu-central-1 ($3.70 and $0.020); the API
      Gateway cache was called "optional" when it is enabled in production; and
      the pointer to "0180 #6" on the delete-then-create derivation was wrong
      (#6 is nameQuery) — nothing in 0180 measured that derivation, and it is
      now labelled as still a derivation. Correction 1 (nameQuery) closed as
      measured; correction 2 (quota rollover) left open. Added the point 0180's
      cost note raised and nothing had absorbed: the real exposure from a
      drained key is contention on the BE-shared ch-prod-01, which no dollar
      figure will surface.
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
  one-click invite, and `verification_level` is `2` — which Discord's API
  reference glosses as *"must be registered on Discord for longer than 5
  minutes"*, while its own support article describes Medium as requiring a
  **verified email** held *"for longer than five minutes"*. **The two sources
  contradict each other**; the research recorded this rather than resolving it,
  and it must be tested, not cited ([[0180]]).
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
not an identity barrier. At this value the age gate stops scripted signup that
mints an account and claims a key in the same breath, and essentially nothing
else. **Membership is therefore the only remaining gate — and it is a thin one.**
Joining Stellar's guild is a public one-click invite; Rules Screening, in the
research's own words, *"is a click-through rules agreement… a friction gate, not
an identity gate. It costs an attacker one extra click, not an extra identity."*

So state the barrier plainly: **two clicks and a five-minute wait.** Nobody
reading this later should mistake "two gates" for "two barriers" — we have one
thin gate and one speed-bump. What makes that acceptable is not the gate's
strength but the size of the prize behind it (see Proportionality below), and
the fact that a cheaper lever — lowering the quota — is held in reserve.

That is a deliberate trade, not an oversight:

- It refuses **no** legitimate newcomer. A developer who creates a Discord
  account specifically to use this API waits minutes, not days — the failure
  mode of a 7- or 30-day threshold, which would have been invisible to us and
  infuriating to them.
- The exposure it is defending is **$0.55–$0.89 per fully-drained key per
  month** — measured 2026-08-12 ([[0180]] #9); the $0.38 this ADR was written on
  was gateway-only and priced in us-east-1. ~112–182 drained keys reach
  ~$100/month. A stricter gate would still cost more in refused real users than
  in prevented abuse.
- The threshold is an SSM parameter precisely so this can be raised without a
  deploy the moment churn is actually observed.

**When membership is checked: see §8.** An earlier draft of this ADR said
"checked once, at issuance" and meant "once, at sign-in" — those are different
moments, and the tasks split on exactly that seam, which left the gate
unenforceable. §8 replaces that rule: eligibility is proved per action, by
re-authentication, and rework re-checks membership too.

One consequence of §3 survives unchanged: **the registry stores no membership
data** — no `pending`, no `joined_at`. Every check reads Discord live at the
moment it matters, so storing them would be holding data we never re-read,
against a document that deliberately declines to hold an email address.
[[0158]]'s schema needs no membership columns.

### 4. Staging: `stellar_test` first, Stellar Developers later

Adam creates a **`stellar_test`** guild for building and testing the epic.
Integration against the production Stellar guild is a separate conversation with
SDF, tracked as [[0179]].

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
| Stellar Discord (SDF) relationship | **Adam Kot (`akot`)** on our side, executing [[0179]]. No named SDF counterpart is public as of 2026-08-10; the published routes are `communityfund@stellar.org` and `#scf-general` |

This replaces the "someone" placeholder in [[0159]]. The published SDF contact
routes are `communityfund@stellar.org` and `#scf-general` (the handbook says the
channel is faster).

### 7. Non-goal, decided rather than deferred

**A user who leaves the Discord server after issuance keeps their key.** The
member object is read once during the OAuth callback and nothing pushes us an
update afterwards. This is a decision, not an open question.

---

### 8. Eligibility is proved per action, by re-authentication

Settled by Adam, 2026-08-10, after an adversarial audit found the original
"checked once, at issuance" rule unenforceable. It replaces that rule.

**The three answers.**

1. **A key is issued only if the caller is a member of the guild at the moment of
   issuance** — not at the moment of sign-in.
2. **The verdict travels by re-authentication**, not by trusting a session.
3. **An issued key never expires and is never deactivated.** Membership is
   re-checked only when the user asks to **rework** it.

### Why re-authentication rather than a session claim

Sign-in and issuance are two separate HTTP requests handled by two different
tasks. The session cookie carries only the Discord user ID, so an ineligible user
who completes OAuth — which they can, because OAuth only proves a Discord account
exists — holds a valid session and could call the issue endpoint directly. A
signed "eligible" claim in the cookie would fix that, but it would date the
verdict to sign-in time, and it does nothing for rework, which happens days or
weeks later when the user's Discord token is long gone and which we deliberately
do not persist.

So: **the action itself carries the OAuth round-trip.** Clicking "get my key" or
confirming a rework redirects through Discord, and the callback that returns holds
a fresh token with which to ask, right then, whether this person is a member.

This is cheap. Discord does not re-prompt for consent when the user has already
authorized the same scopes, so the second and later round-trips are a redirect,
not a consent screen. And it needs no bot in Stellar's guild — which would have
required SDF's permission and made [[0179]] a hard blocker.

### The shape

`state` is already required for CSRF; it also carries the **intended action**,
signed. The callback completes that action and nothing else.

| Path | Re-auth? | Checks |
| --- | --- | --- |
| Sign in | — | identity only; issues a session |
| **Issue a key** | **yes** | membership + account age, then `CreateApiKey` |
| Reveal the key | no | session only — works forever |
| Usage / dashboard | no | session only — works forever |
| **Rework** | **yes** | membership, then the quota-period cap, then the swap |

For rework the user confirms first (the `delete-key` modal in [[0162]]), and the
re-auth is the gate between confirming and executing.

**Account age is only checked on issuance**, not on rework: an account old enough
once is old enough forever, so re-checking it is noise.

### Consequences

- A user who leaves the guild **keeps their key, indefinitely, and keeps the
  dashboard**. This sharpens the epic's non-goal rather than contradicting it:
  the key does keep working on its own schedule. What they lose is the *right to
  rework*, which is a privilege, not the thing they were issued.
- [[0160]]'s issue and rework endpoints are no longer pure AWS calls; they
  complete an OAuth round-trip. Reveal and usage remain pure. The earlier
  statement that these handlers make no Discord calls at all was wrong and is
  withdrawn.
- Nothing needs to be stored: no Discord tokens, no membership columns in the
  registry. That part of §3 stands.

### Does `pending: true` count as a member?

**No — issuance and rework require `pending === false`.** Recorded as my reading
of "must be a member", flagged here so it can be reversed cheaply if Adam meant
otherwise. Discord itself treats a `pending` member as not yet participating:
they "will initially be restricted from doing any actions in the guild". It is
also the only reading under which this ADR's screening argument, and [[0179]]'s
central question to SDF, mean anything — under the other reading the gate is one
click and screening is decorative.

**This rule must not ship before [[0180]] #2 is measured.** `pending` is an
optional field and its presence on the `guilds.members.read` REST response is
undocumented. If it turns out to be absent in practice, a naive
`pending === false` test would refuse **every** user and take issuance down
completely. Measure first, then choose the behaviour for `undefined` deliberately.

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
by `guilds` carry neither `pending` nor `joined_at` — so `guilds` is strictly
more privacy cost for strictly less signal. (Discord publishes no field table for
the partial guild object, only an example; that field set is read off the example,
not off a specification.) Note this means we
**deliberately diverge from SDF's own SCF Dashboard**, which requests
`identify email connections guilds`.

**Why account age as well.** It is free: no extra scope, no extra call, no
change to the consent screen, since the snowflake is already in the `identify`
response. It is the only mitigation on the list with $0 recurring cost and no
new external dependency. It also covers the case membership does not: an account
minted specifically to join and claim a key.

**Why the threshold is configuration, and why it starts at 5 minutes.** Discord's
only *account-age* verification level is **MEDIUM = 5 minutes**; HIGH's 10 minutes
is **time as a member of the server, not account age** (*"must be a member of the
server for longer than 10 minutes"*) — a distinction an earlier draft of this ADR
got wrong, and one that matters, because it means Discord publishes **no**
account-age threshold above five minutes. Nor did the research find a third-party
API provider publishing account-age gating for free-tier issuance — across four
comparators, **two of which (Etherscan, Discord's own help centre) returned HTTP
403 and could not be read**, so read that as "not found", not as "does not exist".
Rather than invent a number with no precedent, we mirror the value Stellar's own
server uses. The parameter exists so the number can move when evidence arrives,
not so it can be guessed higher now.

**A caveat that undercuts this rationale, and must not be lost.** Discord's own
documentation states that *"Having a verified phone number supersedes all other
requirements"* — so for a phone-verified account, the server's five-minute clock
does not apply at all. The research note headed this *"a real hole in the
argument"*. Mirroring Stellar's setting therefore mirrors a setting that is itself
bypassable; our snowflake check is **not** bypassable, which is the one respect in
which our gate is stronger than the one it copies.

**Why one key is not merely convenient but required.** Quota is charged per
`(usage plan, API key)` — *"Throttling and quota limits apply to requests for
individual API keys that are aggregated across all API stages within a usage
plan."* The nearest thing to a user-shaped field is `customerId`, and AWS gives
it two definitions — *"An AWS Marketplace customer identifier"* (`CreateApiKey`)
and *"The identifier of a customer in AWS Marketplace **or an external system,
such as a developer portal**"* (`GetApiKeys`). The second describes our case
exactly, so `customerId` could plausibly carry a Discord user ID. What decides
the matter is narrower: **no AWS page states that quota or throttling is summed
over `customerId`** — it is documented as a filter for *listing* keys, and §1
says quota applies to "requests for individual API keys". With concurrent keys,
AWS's native per-key monthly quota therefore stops bounding a user's total
consumption and we would have to fan out `GetUsage` per key and sum it ourselves.

**Why the rework cap is what makes one key sufficient — the derivation, not an
assertion.** AWS nowhere documents that a replacement key starts with a clean
quota counter; `DeleteApiKey` says nothing about usage at all. It follows by
construction: quota is counted against an API key, usage is stored **indexed by
API key ID** (`GetUsage.values` maps `{api_key}` → daily `[used, remaining]`),
`CreateApiKey` mints a **new** `id`, and nothing on the `ApiKey` model links a new
key to a deleted one — there is no predecessor, lineage or carry-over field among
`createdDate`, `customerId`, `description`, `enabled`, `id`, `lastUpdatedDate`,
`name`, `stageKeys`, `tags`, `value`. So delete-then-create almost certainly
yields a key ID with no prior usage. **That is the loophole the once-per-quota-
period cap exists to close**, and it is why the cap is load-bearing rather than
tidy. It is a derivation from documented behaviour, not a documented guarantee —
and after [[0180]] it **still is one**. None of that task's nine items measured
delete-then-create directly. (An earlier draft of this line pointed at #6, which
is `nameQuery` matching and has nothing to do with usage counters.)

What #8 measured corroborates the derivation from the adjacent direction:
disabling a key **preserves** its counter, so usage travels with the key `id`
and not with the name or the plan attachment. That is consistent with a fresh
`id` starting clean — it does not prove it. If anyone wants the guarantee rather
than the inference, it is five minutes on a scratch plan: drain a key, delete
it, recreate it under the same name, read `GetUsage`.

**Proportionality — what the abuse is worth. Measured 2026-08-12 ([[0180]] #9);
this paragraph previously carried an estimate.** A fully-drained key (100k
quota, 3 KB responses) costs **$0.55–$0.89/month all-in** in `eu-central-1` —
gateway requests, gateway transfer, Lambda requests and Lambda duration. The
**$0.38** it replaces was gateway-only *and priced in the wrong region*: the
correct request rate here is **$3.70/million, not $3.50**. So ~112–182 drained
keys reach $100/month, where this ADR said 286.

**The suspicion that backend cost dominates was wrong, and the argument
survives.** All-in is **1.4–2.3×** the old figure, not the 10× that would have
made this reasoning worth reopening. But the reason it moved at all is worth
recording: the largest marginal component at the measured p50 is **Lambda
duration, not gateway** ($3.30e-6 against $3.70e-6). "Gateway-only" was not a
conservative simplification — it happened to omit a term of similar size, and
was wrong by roughly the amount that term is worth.

**The API Gateway cache is not optional and we are already paying for it.**
`apiGatewayCacheEnabled: true` at 0.5 GB (`infra/envs/production.json`), which
the AWS Pricing API puts at **$14.60/month** for this region — this ADR
previously called it "optional" at "$27.36/month", and both were wrong; that is
the us-east-1 rate for a cache that is already on. It equals **~19 fully-drained
keys per month, charged whether anyone signs up or not.** The largest
onboarding-adjacent cost in this design is a fixed one, and no abuse gate moves
it.

**One thing the dollar figure does not measure.** ClickHouse is a fixed-price
box (ADR 0007) and our allocation is ~**€1.10/month**, so a drained key costs
almost nothing there *in money* — but `ch-prod-01` is **shared with the BE
team**, and query load is real whether or not anyone bills us for it. A
dollar-denominated proportionality argument is right about the money and silent
about contention. If abuse ever becomes visible, the first symptom will be BE's
queries slowing down, not a line on an invoice — so do not treat "the money is
small" as "the exposure is small".

This is why no paid mitigation is justified, and why the quota — which cuts
worst-case per-key exposure **26×** versus the throttle alone — remains the most
effective control in the design.

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

**Cons:** At an assumed $60/h fully loaded, a 2-minute review costs $2.00 ≈
**2.2–3.6 fully-drained keys** on the measured all-in cost (this read "5.3" on
the old gateway-only $0.38). The rejection gets *weaker* as the true cost rises
— but not nearly enough to matter, and the decisive objection was never the
arithmetic: cost scales with *legitimate* signups while the abuse it prevents
does not, and it deletes the "self-service" property the epic is named after.

**Decision:** REJECTED.

### Alternative 6: Lower the free monthly quota

**Description:** Leave the gates alone and shrink the prize instead — 100k/month
down to 50k or 25k.

**Pros:** Free, and it *saves* money: per-key exposure drops from
**$0.55–$0.89** to **$0.28–$0.45** at 50k and to **$0.14–$0.22** at 25k — every
marginal component scales per call, so the quota scales the exposure linearly
(figures restated 2026-08-12 from the measured all-in cost; they read
$0.38/$0.19/$0.09 when this ADR assumed gateway charges only). It is one number
in the usage plan,
enforced natively by AWS with no code of ours in the path, and it bounds every
key — abusive or not. There is real headroom: our 50k–100k is **5–10×
CoinGecko's** free tier ("10k call credits/mo") and **3.3–6.7×** CoinMarketCap's
("15,000"), so 25k would still be 2.5× CoinGecko.

**Cons:** It says nothing about *who* gets a key, so it is a damage cap rather
than a barrier — orthogonal to the question this ADR is settling. It also
degrades the product for legitimate users, and [[0157]] already fixed 100k with
Adam's sign-off on 2026-08-06.

**Decision:** NOT ADOPTED NOW, and deliberately kept as the **first lever to
pull** if churn is ever observed — cheaper and faster than any gate. Recorded
here because it is the one costed mitigation that is cheaper than the two
adopted, and an ADR that omitted it would misrepresent the analysis.

### Alternative 7: Multiple concurrent keys per account

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
- `pending`, `joined_at` and `roles` become available for finer rules later
  without another consent change. Two caveats: whether `pending` and `flags` are
  actually present on the `guilds.members.read` REST response is **undocumented**
  ([[0180]] #2, #3), and `pending === false` can also mean an admin waved the
  member through (`BYPASSES_VERIFICATION`).
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
- The not-a-member path depends on an **undocumented** error shape ([[0180]] #1).
- `pending === false` is not proof of having passed anything —
  `BYPASSES_VERIFICATION` (*"Member is exempt from guild verification
  requirements"*) means an admin can wave a member through.

### What would reverse this decision

- **SDF disabling Membership Screening on the production guild, or declining the
  integration in [[0179]].** With the age threshold at 5 minutes, screening is
  the second of the barrier's two clicks; losing it takes the gate from "joined
  and accepted the rules" to "joined". That is a small absolute change — which is
  exactly why it matters that the gate was already thin. If it goes, the first
  lever is the free quota (Alternative 6), then the age threshold, then a
  captcha. Whoever picks up [[0179]] should still treat "will you keep screening
  on, and would you tell us if it changed" as a question worth asking, because we
  would not learn the answer from any API response.

  **This reversal trigger is only real if we actually read `pending`** — see
  "Open: does issuance require `pending === false`?" below. If we do not, losing
  screening changes nothing about our behaviour, because we were never observing
  it in the first place.
- **Observed churn.** Raise the SSM threshold first — it is a config change and
  costs nothing. Only if churn continues does captcha (Turnstile) become
  justified.
- Observed *scripted* signup specifically — that is the threat captcha
  addresses, and a 5-minute age gate barely touches it.
- ~~The all-in per-call backend cost turning out to dominate the gateway figure
  by an order of magnitude.~~ **Retired 2026-08-12 — measured, and it does not**
  ([[0180]] #9). All-in is $0.55–$0.89/key against the $0.38 assumed here:
  1.4–2.3×, nowhere near the 10× that would have moved the decision. The figures
  in Proportionality are now measured rather than estimated, so this trigger has
  nothing left to fire on.
- **Replacing it: contention on the shared ClickHouse box.** The measurement
  that retired the trigger above also showed why a dollar figure was the wrong
  instrument — `ch-prod-01` is fixed-price and shared with BE, so abusive query
  load is close to free *to us* and not free *to them*. Watch BE's query
  latency, not our bill.

---

## Corrections this ADR makes to already-written tasks

Both were checked directly against AWS documentation. Neither is fatal. **The
first is now measured and corrected; the second is still open** ([[0180]]).

1. ✅ **`nameQuery` is not documented as a prefix match — and measured
   2026-08-12, it is one.** [[0158]] and [[0160]] build a reconciler and a
   "prefix hazard" guard on that premise. AWS documents one sentence — *"The
   name of queried API keys."* — and states no matching semantics; measurement
   on a scratch plan showed **case-sensitive prefix** matching. So the community
   answer was right, the prefix hazard 0158 wrote as a conditional is **real**,
   and the client-side exact-match filter is **load-bearing, not defence in
   depth** — for the stronger reason that AWS *does* return prefixes, rather
   than the weaker one that AWS says nothing. 0158, 0160 and
   `docs/runbooks/manual-api-key-tier.md` are updated.

   One thing nobody had asked, and it changes code: **a `nameQuery` result still
   paginates.** It comes back with a `position` token like any other list, so a
   reconciler that ranks by earliest `createdDate` off page one can pick a winner
   from a partial list. Page to exhaustion before ranking.
2. ⏳ **The monthly quota reset instant is undocumented — still unmeasured.**
   "1st of the month, 00:00 UTC" appears in [[0157]]/[[0158]]/[[0160]] as if it
   were AWS behaviour. AWS's only statement is an example caption — *"creates a
   usage plan that resets at the beginning of the month"* — with no timezone and
   no instant. `offset` is a **request count**, not a time shift. Keep the rule
   as **our** product decision; do not present it as inherited AWS semantics.
   A real `MONTH` rollover cannot be observed before **1 September 2026**;
   [[0180]] #7 measures a `DAY`-period plan as a proxy for the instant and the
   timezone, which is evidence and not proof — and is enough, because the point
   is to stop citing AWS for a rule AWS never stated.

---

## References

Every source below was fetched on 2026-08-10 and its quoted text verified
against the original.

- [Usage plans and API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-api-usage-plans.html) — quota per API key; not hard limits
- [GetApiKeys](https://docs.aws.amazon.com/apigateway/latest/api/API_GetApiKeys.html) — `nameQuery`, `customerId`
- [QuotaSettings](https://docs.aws.amazon.com/apigateway/latest/api/API_QuotaSettings.html) — `offset` is a request count
- [Amazon API Gateway quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/limits.html) — control-plane rates
- [Amazon API Gateway Pricing](https://aws.amazon.com/api-gateway/pricing/) — $3.50/million, $0.09/GB, $0.038/hr cache. **These are us-east-1 rates.** Our region is `eu-central-1`: **$3.70/million** requests and **$0.020/hr** for a 0.5 GB cache, read from the AWS Pricing API on 2026-08-12 (`EUC1-ApiGatewayRequest`, `EUC1-ApiGatewayCacheUsage:0.5GB`) — see [[0180]] #9
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
