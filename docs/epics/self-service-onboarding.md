# Epic: Self-Service Onboarding (Prices API)

## Summary

Let external developers get an API key for the Prices API without a human in the loop — a
small portal where a user signs in with Discord, gets a key shown on-screen, and lands on a
dashboard showing their usage against quota. Plus a quickstart guide and example queries.
Delivered in Tranche 3 (Production Launch, Weeks 10–13).

## Why this exists

"Self-service onboarding" is a named deliverable in the RFP (`RFP/01-prices-api.md`) and
appears as an acceptance criterion in every iteration of our design response
(`notes/prices-api-design-after-2nd-review.md`, `notes/price-api-reviewer-response.md`,
`notes/prices-api-design-after-review.md`). Reviewer sign-off criterion, verbatim:

> Onboarding portal accessible at documented URL; self-service API key request flow functional.

## Agreed scope (settled — from the design docs, do not relitigate)

- Static portal hosted on **S3 + CloudFront**, alongside the OpenAPI/Swagger UI docs
- **Sign-in via Discord OAuth** — matches how other Stellar/SCF-ecosystem services
  authenticate against the Stellar Discord. This replaces the earlier "anonymous form"
  assumption and is the primary abuse-prevention mechanism (see below)
- Key issued automatically on first sign-in, no manual approval step
- **Quickstart guide**
- **Example queries**
- Keys are provisioned through **API Gateway usage plans**
- Every API request requires a key; missing key → **`403`**
- Delivered and demoed as part of **Tranche 3**

## Auth & key handling (resolves several previously-open items)

- **Login:** Discord OAuth. A user must have a real Discord account to request a key — no
  email/captcha flow needed on top of this.
  - Residual risk: someone can still spin up throwaway Discord accounts. ~~**Unverified
    assumption (confirm before build):**~~ our understanding is Stellar's own Discord requires
    some form of verification for new accounts/members, which makes churning through
    disposable accounts non-trivial — this is part of why we're not building anything extra
    (captcha, email, etc.) on top of Discord login. If that verification turns out not to be
    there, this residual risk is bigger than assumed and worth revisiting.
  - **RESOLVED 2026-08-10 — see ADR 0010 (task 0156). The assumption was half right, and
    the half that was wrong mattered more.** Stellar's Discord _does_ have Membership
    Screening enabled, so the gate exists. But joining is a public one-click invite and the
    server's verification level is "registered on Discord for longer than 5 minutes" — and,
    decisively, **the flow described above would never have observed any of it**: Discord
    OAuth under `identify` authenticates a Discord _account_, and exposes no email-verified
    flag (that needs the `email` scope), no phone field at all, and no server membership.
    SDF's own SCF Dashboard does not treat a Discord account as sufficient either.
    **Scope therefore grows by two gates:** the flow now requests
    `identify` + `guilds.members.read` and requires **membership of the Stellar Discord**,
    plus a **minimum Discord account age of 5 minutes** derived from the user's snowflake
    (free — no extra scope), matching Stellar's own server setting. Both are checked
    **once, at issuance** — nothing re-checks them, which is the consistent extension of
    the non-goal directly below. Captcha, email confirmation and manual approval were
    costed and declined; a fully-drained key is worth ~$0.38/month, so no paid mitigation
    pays for itself. Downstream effects are in tasks 0159, 0162, 0163 and 0164.
    **Read the residual risk above as unchanged in size, not eliminated:** at a 5-minute
    threshold the age check is a speed-bump, so the barrier is effectively "joined a public
    Discord server and accepted its rules". That is proportionate to the exposure, but it
    rests on SDF keeping Membership Screening enabled — tracked as task 0170.
  - **Account leaving the Discord server after key issuance:** not actively handled — a key,
    once issued, keeps working on its own schedule regardless of the user's later Discord
    membership status. This is a conscious "not solving this now" rather than an oversight;
    revisit only if it turns out to be exploited in practice.
- **Account model:** the Discord identity _is_ the account. A signed-in user lands on a
  dashboard tied to their Discord ID. ~~**Recommendation:**~~ **Confirmed 2026-08-10 (ADR
  0010): one active key per Discord account.** This also resolves the contradiction between
  this line and "Out of scope" below, which already stated it as settled — the "Out of
  scope" reading was correct. The confirmation is stronger than "keeps the abuse story
  simple": AWS charges quota per `(usage plan, API key)` and has **no principal that
  aggregates keys**, so a multi-key model would force us to fan out `GetUsage` per key and
  sum it ourselves — precisely the work the rotation cap below exists to avoid. Note AWS
  will not enforce one-key-per-account for us; the registry owns that invariant.
- **Key delivery:** shown on-screen immediately after the Discord sign-in completes the
  request, **and viewable again later on the dashboard** — not a one-time reveal. This is
  simpler than the "shown once" pattern common elsewhere, and it's workable here because AWS
  API Gateway keys aren't stored as one-way hashes: the raw value can be fetched at any time
  via `GetApiKey` (with `includeValue=true`). Implementation-wise this reuses the same
  authenticated backend-endpoint pattern as the `GetUsage` dashboard call above — no need to
  separately store the raw key ourselves.
- **Usage dashboard:** yes, this is feasible. AWS API Gateway usage plans expose per-key
  consumption via the `GetUsage` API (`apigateway:GetUsage`), which returns request counts
  against the plan's quota over a date range. This is a server-side AWS SDK call requiring
  IAM credentials — it **cannot** be called from the browser directly. Implementation
  implication: needs a small backend endpoint (new Lambda, not currently in the component
  list) that the logged-in session calls, which calls `GetUsage` on the user's behalf and
  returns the numbers for the dashboard to render.
- **Storage implication:** still need a small table mapping Discord user ID → API Gateway
  key ID / usage plan, so the dashboard knows which key's usage to look up for the logged-in
  user. This doesn't exist in the current schema and needs to be added.
- **Rotation/revocation:** yes, the dashboard offers a "generate new key" action — but it's
  **rate-limited to once per calendar month**, specifically so a user can't burn their monthly
  quota and rotate to a fresh key to reset the counter mid-month. A new key is only issuable
  once the current one's monthly quota period has rolled over.
  - **Why this instead of cross-key quota tracking:** rotating a key means deleting the old
    API Gateway key resource and creating a new one (a new `apiKeyId`), which gets a clean
    quota counter in AWS's own tracking — `GetUsage`/quota is scoped to
    `(usagePlanId, apiKeyId)`. Summing usage across a user's entire key history to enforce a
    true cumulative cap is real work (needs its own aggregation logic, not just AWS's native
    quota). Capping rotation frequency to once/month sidesteps that: AWS's native per-key
    monthly quota is enough on its own once rotation can't happen more often than the quota
    resets anyway.
  - **When the next rework becomes available (settled 2026-08-07):** the boundary is the
    **first day of the month following the last rework, 00:00 UTC**. Worked example: a key
    reworked on **3 August** cannot be reworked again until **1 September**.
    - **Correction 2026-08-10 (task 0156):** "the same instant the AWS quota period rolls
      over" was asserted here and inherited by tasks 0157/0158/0160 — **AWS does not
      document it.** Its only statement anywhere is an example caption, "creates a usage
      plan that resets at the beginning of the month", with no timezone and no instant; and
      `offset` is a _request count_, not a way to shift the reset day. The boundary above
      stands as **our own product rule** — it is sound and gives one date to render — but
      the claim that our date and AWS's coincide is unverified until measured (task 0171).
      If they turn out to differ, we render our date and the quota counter does its own
      thing; that is a UX wrinkle, not a correctness bug, because the cap is ours to define.
  - **Rework is a swap, not a delete-and-wait (settled 2026-08-07):** the old key is
    deleted and a new one issued in the same operation, so a user is never left without
    a working key. The cap blocks the _next_ rework, not the replacement. Eligibility is
    measured from `coalesce(last_rotated_at, created_at)`, so a key issued inside the
    current period cannot be reworked either — without that fallback a user could take a
    key, exhaust the quota and rework into a clean counter within the same period, which
    is the loophole this cap exists to close.
  - **Rework flow on the dashboard (settled 2026-08-07):** the action opens a modal that
    states plainly that the current key is deleted and **stops working immediately** —
    anything using it breaks the moment the user confirms. The confirm button stays disabled
    until the user types the phrase **`delete-key`**. A refused rework (one already performed
    in the current quota period) renders the next eligible date, not a generic error.
  - **Revocation is not covered.** This heading says "Rotation/revocation", but only rotation
    is specified above and rotation does not appear in the acceptance criteria below. A user
    whose key leaks mid-period cannot invalidate it and must wait for the period boundary.
    API Gateway exposes `UpdateApiKey(enabled=false)`, which invalidates a key in one call
    without touching the quota counter, so this is cheap to add and does not need to share
    the rework cap. Recorded here as a known gap, deliberately deferred.

## Rate limiting — override the design doc's default

The design docs currently list **100 req/s per key** (global burst capped at 1000 req/s).
**Do not implement this number as-is** — it was carried through multiple drafts without
justification and has real problems:

- It equals the sustained load the _entire system_ is load-tested against (100 req/s is the
  acceptance-criteria target for the whole API), so one unreviewed, self-issued key can
  consume the full capacity we've proven the system can handle.
- 10 keys running flat-out would saturate the 1000 req/s global burst ceiling for everyone.
- The default RDS instance (`db.t4g.micro`, burstable, 1 GB RAM) is explicitly called out
  elsewhere in the design as unable to sustain continuous load without throttling — 100 req/s
  of reads from a single free key is a real risk to it.
- Cost exposure: API Gateway bills per request regardless of cache hits (~$3.50/million).
  One key sustained at 100 req/s continuously is ~259M requests/month, roughly $900/month,
  from a single no-approval signup.
- Comparable public price APIs are far below even a conservative-sounding number:
  - CoinGecko public/unauthenticated: ~5–15 calls/min (~0.1–0.25 req/s)
  - CoinGecko free registered ("Demo") plan: ~30 calls/min (~0.5 req/s)
  - CoinMarketCap free ("Basic") plan: bursts to 50 req/min (~0.83 req/s), but the binding
    constraint is a **15,000 calls/month** cap (~500/day) — the per-minute number barely
    matters next to the monthly ceiling

**Implement instead:**

- Default free-tier limit for self-issued keys: **1 req/s per key** (60 req/min) — already
  roughly 2x CoinGecko's free registered tier, so generous by industry standards without
  being able to single-handedly load-test our own production system
- **Add a monthly request quota** (e.g. 50,000–100,000 calls/month) alongside the per-second
  throttle. A per-second limit alone doesn't stop a key idling at 1 req/s from generating
  2.6M requests/month — CoinMarketCap's numbers above show the monthly cap is what actually
  bounds cost and abuse in practice, not the burst rate
- Anything higher requires actual human contact, not a bigger number available through the
  automatic form. **Resolved: this is a fully manual, out-of-band process for now** — someone
  on our side creates a key with a higher quota by hand (AWS console/CLI), and payment is
  collected through a normal bank transfer, outside the product entirely. No self-serve
  upgrade flow, no in-app billing, nothing to build here — explicitly a future problem
- This is a rate-limit-only change — it does not affect the `403`-without-key behavior or
  the usage-plan mechanism already agreed

## Out of scope

- Billing / paid tiers — not in the original RFP scope. Higher-quota keys are handled
  entirely by hand (manual AWS key creation + bank transfer for payment) — no in-app
  upgrade flow or billing integration
- Org/team accounts — one key per Discord account only
- Handling Discord membership changes after a key is issued (e.g. user leaves the server) —
  the key keeps working regardless; not solved by this epic

All open questions from earlier drafts of this doc are now resolved — see "Auth & key
handling" and "Rate limiting" above.

## Acceptance criteria

1. Onboarding portal accessible at a documented URL (carried from the design doc)
2. Self-service flow functional end-to-end: Discord sign-in → key issued and shown on-screen
   → key works against the live API (carried from the design doc, updated for Discord auth)
3. Quickstart guide and example queries present and accurate against the live API
   (carried from the design doc)
4. Dashboard shows the signed-in user's current usage against their rate limit/quota, sourced
   from `GetUsage` (new — from this epic's scope)
5. Default key limits confirmed as 1 req/s + monthly quota, not the design doc's 100 req/s
   (new — from this epic's scope)

## Source docs

- `RFP/01-prices-api.md` — original deliverable line item
- `notes/prices-api-design-after-2nd-review.md` §2 (component table), §6 (throttling), §9
  Tranche 3 (lines ~875, ~905)
- `notes/price-api-reviewer-response.md` — earlier milestone-based version of the same scope
