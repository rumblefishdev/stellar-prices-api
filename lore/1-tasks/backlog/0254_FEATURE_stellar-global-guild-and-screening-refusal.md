---
id: "0254"
title: "Point the portal at the Stellar Global guild, and refuse an unscreened member as its own answer"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0163", "0164", "0170", "0179", "0186", "0189", "0191", "0193", "0195"]
tags:
  [
    layer-backend,
    layer-frontend,
    priority-high,
    effort-medium,
    milestone-M3,
    epic-self-service-onboarding,
    discord,
    external-dependency,
    pre-launch,
  ]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
history:
  - date: 2026-09-02
    status: backlog
    who: akot
    note: >
      Spawned while verifying [[0195]]'s operator steps. Adam named the target
      guild — Stellar Global, 761985725453303838 — and asked for a distinct
      refusal for a member who joined but has not accepted the rules. The audit
      that produced this task also found the invite on the landing page points
      at a THIRD guild; that correction is in scope here.
  - date: 2026-09-02
    status: backlog
    who: akot
    note: >
      Numbered 0252 → 0254 before it was ever written: `origin/develop` moved
      between drafting and creating, and 0252/0253 were taken by [[0210]]'s
      spawn and the `Timeframe::All` finding. The same collision 0179's history
      records twice. The ID was re-picked after the pull, which is the rule
      `lore/CLAUDE.md` pinned in 55550cd the day before.
---

# The Stellar Global guild, and the member who joined but did not accept the rules

## Summary

**Story:** _as a developer who joined the Stellar Discord but has not clicked
through its rules, I am told exactly that — not that I am not a member — and the
guild the portal checks is the one the invite sends me to._

Two halves of one change, together because neither can be verified without the
other: the gate is pointed at the real guild (`761985725453303838`), and the
state "on the server, inside its screening" stops being collapsed into "not a
member".

## Context

[[0189]] built the gate and already reads the right field. The scope is
`identify guilds.members.read`, the call is
`GET /users/@me/guilds/{guild}/member` with the user's own token, and
`eligibility::membership()` maps `pending` to a verdict. What it does **not** do
is tell the two refusals apart on the way out: `Some(true)` returns
`Membership::NotMember`, lands `?signin=not_member` / `?issue=not_member`, and
renders the same "Access not available / Join the server" card as a genuine
non-member. 0189 knew this and left one sentence of copy as the mitigation
("New members may need to complete the server's screening first", Adam,
2026-08-26). Adam's decision of 2026-09-02 replaces the sentence with a state.

**The API is not in question.** Discord documents the field on the guild member
object — `pending?`, "whether the user has not yet passed the guild's Membership
Screening requirements" — and documents the route we already call,
`GET /users/@me/guilds/{guild.id}/member`, as returning a guild member object
under exactly the scope we already hold. Nothing new is integrated here; what
changes is that our own code stops discarding the distinction.

**Three guild ids are in circulation today**, and no two of them agree:

| Where                                                     | Value                 | What it is                                                                    |
| --------------------------------------------------------- | --------------------- | ----------------------------------------------------------------------------- |
| SSM `/prices/production/discord-guild-id`                 | `1536303837785362432` | created 2026-08-10 — the `stellar_test` guild                                 |
| `web/portal/src/landing/links.ts` → `discord.gg/stellardev` | `897514728459468821`  | **"Stellar Developers"** — a real Stellar server, but not the one we check    |
| Adam, 2026-09-02                                          | `761985725453303838`  | created 2020-10-03 — **Stellar Global**, the target                           |

The invite mismatch is invisible today only because SSM points at the test
guild, so nobody has ever walked the refusal. Left alone past the SSM switch it
becomes a loop: the card says "join the server", the button joins a _different_
server, and the next attempt refuses again.

Measured 2026-09-02, unauthenticated, from the invite metadata of
`discord.gg/stellardev`:

```
897514728459468821  MEMBER_VERIFICATION_GATE_ENABLED   ← Membership Screening on
                    GUILD_ONBOARDING_HAS_PROMPTS       ← Onboarding also on
761985725453303838  widget disabled — guild exists, features NOT observable
```

So on Stellar Developers the screening gate is provably on. **On Stellar Global
we cannot see it from outside**, and that is step 0's first question: a guild
with no screening gate answers `pending: false` to everyone who joins, which
makes both this task's new view dead code and ADR 0010's barrier a formality.

## Step 0 — measure, before any of it is built

[[0180]] item 2 ("does the REST member route carry `pending`?") was never
settled — 0180 was cancelled. The documentation does not settle it either: the
field is marked optional, and the only statement about its presence is written
about **gateway events**, not this REST route —

> In `GUILD_` events, `pending` will always be included as true or false. In non
> `GUILD_` events which can only be triggered by non-`pending` users, `pending`
> will not be included.

The code already fails closed on the gap: an absent `pending` is `Unknown`,
which refuses **every** member with "we could not verify your Discord
membership". If the field is absent on this route, pointing SSM at Stellar
Global issues zero keys and the portal looks broken to everyone. That asymmetry
is why the measurement comes first and not alongside.

One OAuth token, three observations, against `761985725453303838`:

1. a member who has **not** accepted the rules → is `pending` present, and is it
   `true`?
2. the same account **after** accepting → `pending: false`, or the field gone?
3. an account that never joined → `404`, and which `code` (expecting `10007`)?

Record the raw bodies in this task. If (1) and (2) show `pending` absent, this
task stops and the gate needs a different mechanism — that is a finding, not a
failure, and it is cheaper here than after the switch.

Note this is **not** the same question as Onboarding. `pending` is Membership
Screening — the rules checkbox, which is what Adam asked for. Discord Onboarding
(channel/role prompts) lives in `flags`, bit `COMPLETED_ONBOARDING` (1<<1),
which this service deliberately does not deserialize (ADR 0010 — "the registry
stores no membership data"). **Out of scope unless step 0 shows Stellar Global
gates on Onboarding instead of screening**, in which case the decision comes
back to Adam before any `flags` field is added.

## Implementation

### 1. The guild, and the invite that must agree with it

- `aws ssm put-parameter --name /prices/production/discord-guild-id --value 761985725453303838 --type String --overwrite --region eu-central-1`.
  No deploy: the value is read per request through the Parameters and Secrets
  extension. Current version is 1, seeded 2026-08-28.
- `STELLAR_DISCORD_INVITE` in `web/portal/src/landing/links.ts` — replace
  `https://discord.gg/stellardev` with an invite that resolves to
  `761985725453303838`. **We do not have that code yet**; it comes from [[0179]]'s
  conversation with SDF, or from the server's own invite if it is public. Verify
  it, do not assume it:
  `curl -s https://discord.com/api/v10/invites/<code> | jq .guild.id`.
- Add that verification to `tools/scripts/` or to the task's evidence — the next
  person to change either value must be able to catch the drift that this task
  exists to fix.
- The two values are one fact in two places. Say so where they live.

### 2. The backend tells the two refusals apart

`packages/prices-api/src/portal/eligibility.rs`:

- `Membership` gains a variant — `PendingScreening` (name it for the Discord
  concept, not for the copy) — returned for `pending: Some(true)`. `Some(false)`
  and the 404 codes are unchanged; `None` stays `Unknown` and keeps the
  `pending_absent` warning.
- `Eligibility` likewise: a variant beside `NotMember`, so `decide()`'s
  precedence (membership before age) is untouched and an unscreened member is
  never told to wait for their account to age.
- **`membership()` is shared with the rework path** ([[0191]] re-proves
  membership on replace, never age). Both callers must handle the new variant —
  a rework by an unscreened member gets the same answer as an issue.

`auth/mod.rs` and `auth/issue.rs`:

- `?signin=pending_rules` and `?issue=pending_rules` beside the existing
  `not_member` (`NOT_MEMBER_QUERY`, `ISSUE_NOT_MEMBER_QUERY`). Constants, like
  their siblings.
- `tracing::info!(outcome = "pending_rules", ...)` so the two are countable
  apart in CloudWatch — the ratio is the only signal we will have that the
  screening step is where people fall out.
- The module header tables in `issue.rs` list every query the page can land on;
  they are documentation and must gain the row.

### 3. The view

`web/portal/src/app/app.tsx` — a screen, not a banner, exactly as `not_member`
is a screen (the comment at its head explains why: the ordinary card's sign-in
button would hand the visitor back to the same refusal).

- Title: **"Access not available"** — unchanged, it is the honest headline for
  both.
- Callout title: **"Stellar Discord accept rules required"** (Adam's wording).
- Body: says the account **is** on the server and what is left — open Discord,
  accept the server's rules, come back. It must not say "join".
- Action: a button to the **server**, not to the invite — a member following an
  invite lands back on a "you're already a member" screen.
  `discord.com/channels/761985725453303838` opens the server itself; confirm
  during step 0 that it lands an unscreened member on the screening prompt.
- `Callout variant="neutral"`, matching `not_member`: nothing failed.
- Remove the "New members may need to complete the server's screening first."
  sentence from the `not_member` card — its whole reason for existing was that
  this state had nowhere else to go.
- `oauthPopup.ts` — `pending_rules` joins the outcome union and the accepted
  list, or the popup drops it.

### 4. Tests and docs

- `eligibility.rs` unit tests: the decision table gains rows for the new variant
  on both `decide()` and `membership()`; the `pending: None` → `Unknown`
  assertion must stay, it is the fail-closed guarantee.
- `app.spec.tsx`: `/?signin=pending_rules` and `/?issue=pending_rules` render
  the new screen; `not_member` still renders the old one and no longer carries
  the screening sentence.
- `docs/runbooks/portal-oauth-deploy-prep.md` — the guild parameter's value and
  the invite are a matched pair; write the check.
- `docs/epics/self-service-onboarding.md` — the abuse-barrier section describes
  the gate as membership + account age. It becomes membership + **screening
  cleared** + account age, which is what it always meant and never said.
- ADR 0010 — if step 0 changes what `pending` means on this route, the ADR's
  reading of "must be a member" needs the measurement appended. Not a rewrite.

## Acceptance Criteria

- [ ] Step 0's three observations against `761985725453303838` are recorded in
      this task with raw response bodies, including whether the guild has
      Membership Screening enabled at all
- [ ] `/prices/production/discord-guild-id` is `761985725453303838`
- [ ] `STELLAR_DISCORD_INVITE` resolves to `761985725453303838`, verified by the
      invite API and not by reading the link
- [ ] A member with `pending: true` is refused with a distinct outcome, its own
      query parameter, and its own log line — not `not_member`
- [ ] That refusal renders "Access not available" with "Stellar Discord accept
      rules required", says the visitor is already on the server, and its action
      opens the server rather than an invite
- [ ] `pending: None` still refuses as `Unknown` and still logs `pending_absent`
      — the fail-closed arm is not touched
- [ ] The rework path ([[0191]]) gives an unscreened member the same answer as
      the issue path
- [ ] A real account walks: join Stellar Global → refused with the new screen →
      accept the rules → sign in → key issued
- [ ] Rust and portal suites green; the `not_member` card no longer carries the
      screening sentence

## Notes

- **Sequencing.** Step 0 and items 2-3 are ours and can run now against
  `stellar_test` for the code paths. The SSM switch is the moment the portal
  becomes real for outsiders — it belongs next to [[0179]], and [[0164]]'s
  evidence pass must run after it, not before.
- **The barrier is rented.** `MEMBER_VERIFICATION_GATE_ENABLED` is SDF's setting
  on SDF's server. If they turn it off, every joiner is `pending: false`
  immediately and ADR 0010's barrier silently becomes "has a Discord account".
  Nothing alerts on that today; the epic tracks it as [[0170]].
- This task does **not** touch `flags` / Discord Onboarding. See step 0.
- Not blocked by [[0195]]'s open operator step (the two `RETAIN`ed buckets); no
  overlap.
