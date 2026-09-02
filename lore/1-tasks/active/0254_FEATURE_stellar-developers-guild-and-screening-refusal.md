---
id: "0254"
title: "Point the portal at the Stellar Developers guild, and refuse an unscreened member as its own answer"
type: FEATURE
status: active
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
      Spawned while verifying [[0195]]'s operator steps. Adam named a target
      guild and asked for a distinct refusal for a member who joined but has not
      accepted the rules. The audit that produced this task also found the
      invite on the landing page pointing at a different guild than the one the
      gate checks; that correction is in scope here.
  - date: 2026-09-02
    status: backlog
    who: akot
    note: >
      Numbered 0252 → 0254 before it was ever written: `origin/develop` moved
      between drafting and creating, and 0252/0253 were taken by [[0210]]'s
      spawn and the `Timeframe::All` finding. The same collision 0179's history
      records twice. The ID was re-picked after the pull, which is the rule
      `lore/CLAUDE.md` pinned in 55550cd the day before.
  - date: 2026-09-02
    status: active
    who: akot
    note: >
      Activated. Branch
      `feat/0254_stellar-developers-guild-and-screening-refusal` cut from
      `develop`.
  - date: 2026-09-02
    status: active
    who: akot
    note: >
      Target guild changed by Adam before any code was written: **Stellar
      Developers, `897514728459468821`** is the official and only Stellar
      server this project refers to. The first draft named Stellar Global
      (`761985725453303838`); that guild, and the measurements taken against
      it, are kept below as the rejected alternative. The change SHRINKS the
      task — `discord.gg/stellardev` already resolves to the target, so the
      invite half of the original scope dissolves into a regression guard.
---

# The Stellar Developers guild, and the member who joined but did not accept the rules

## Summary

**Story:** _as a developer who joined the Stellar Developers Discord but has not
clicked through its rules, I am told exactly that — not that I am not a member —
and the guild the portal checks is the one the invite sends me to._

Two halves of one change, together because neither can be verified without the
other: the gate is pointed at the official guild (`897514728459468821`), and the
state "on the server, inside its screening" stops being collapsed into "not a
member".

**The official guild, decided 2026-09-02 by Adam:** `897514728459468821`,
**Stellar Developers**, `discord.gg/stellardev`. It is the only Stellar server
this project refers to — in the gate, on the landing page, in the docs and in
the refusal copy. Anywhere a second server appears, it is a defect.

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

**Two guild ids are in circulation today**, and only one of them is wrong:

| Where                                                       | Value                 | What it is                                                                |
| ----------------------------------------------------------- | --------------------- | ------------------------------------------------------------------------- |
| SSM `/prices/production/discord-guild-id`                   | `1536303837785362432` | created 2026-08-10 — the `stellar_test` guild. **The one thing to change** |
| `web/portal/src/landing/links.ts` → `discord.gg/stellardev` | `897514728459468821`  | **Stellar Developers** — the official guild. **Already correct**           |

The landing page has been pointing at the right server all along; it is the gate
that points at a test guild. Once SSM moves, the two agree and the loop the
first draft of this task was written to prevent — the card says "join the
server", the button joins a _different_ server, the next attempt refuses again —
cannot form. What remains is to make sure it cannot form later: the two values
are one fact in two places, and nothing checks them against each other.

**Measured 2026-09-02, unauthenticated, from the invite metadata:**

```
GET https://discord.com/api/v10/invites/stellardev?with_counts=true&with_expiration=true

guild.id                  897514728459468821    ← the official guild, confirmed
name                      Stellar Developers
vanity_url_code           stellardev            ← vanity invite, owned by SDF
verification_level        2 (MEDIUM)
approximate_member_count  32873   (1557 online)
expires_at                null                  ← permanent invite
features                  MEMBER_VERIFICATION_GATE_ENABLED  ← Membership Screening ON
                          GUILD_ONBOARDING_HAS_PROMPTS      ← Onboarding also on
                          COMMUNITY, DISCOVERABLE, VANITY_URL, …
```

So on the official guild the screening gate is **provably on**: the state this
task exists to name — joined, has not accepted the rules — is reachable, and
`PendingScreening` is not dead code. That is step 0's first question, answered
without an OAuth round-trip.

`verification_level: 2` (MEDIUM — the account must be older than five minutes on
Discord) gates _posting_, not the member object, so it does not touch `pending`.
It is one more thing between "clicked the invite" and "can use the server", and
the refusal copy should not promise the server is instantly usable.

### The rejected alternative — Stellar Global

The first draft of this task named **Stellar Global** (`761985725453303838`,
created 2020-10-03) as the target, on Adam's word of the same morning. He
reversed it the same day: Stellar Developers is the official and only server.
The measurement taken against Stellar Global is kept because it cost a round of
work and because the next person to propose a second guild will ask the same
question:

```
GET https://discord.com/api/v10/invites/5g2YkszV3D   (the invite the SCF handbook publishes)

guild.id                  761985725453303838
name                      Stellar Global
verification_level        3 (HIGH)
approximate_member_count  3204
features                  MEMBER_VERIFICATION_GATE_ENABLED, GUILD_ONBOARDING_HAS_PROMPTS
```

Its widget is disabled, so the features are visible only through invite
metadata — `discord.gg/5g2YkszV3D`, recorded in this repo since [[0156]]
(`sources/stellar-discord-scf-handbook-rules-and-roles.md`). Screening is on
there too, so the reversal changes nothing about the mechanism; it changes which
snowflake SSM gets and deletes the invite half of the work. **Nothing in the
codebase should name this guild.**

## Step 0 — measure, before any of it is built

Screening being enabled is settled (above). What is **not** settled is the
transport: [[0180]] item 2 ("does the REST member route carry `pending`?") was
never answered — 0180 was cancelled. The documentation does not answer it
either: the field is marked optional, and the only statement about its presence
is written about **gateway events**, not this REST route —

> In `GUILD_` events, `pending` will always be included as true or false. In non
> `GUILD_` events which can only be triggered by non-`pending` users, `pending`
> will not be included.

The code already fails closed on the gap: an absent `pending` is `Unknown`,
which refuses **every** member with "we could not verify your Discord
membership". If the field is absent on this route, pointing SSM at Stellar
Developers issues zero keys and the portal looks broken to everyone — and looks
exactly like a Discord outage. That asymmetry is why the measurement comes first
and not alongside.

One OAuth token, three observations, against `897514728459468821`:

1. a member who has **not** accepted the rules → is `pending` present, and is it
   `true`?
2. the same account **after** accepting → `pending: false`, or the field gone?
3. an account that never joined → `404`, and which `code` (expecting `10007`)?

`scripts/measure-pending-absent.sh` already drives exactly this — run it with
`GUILD=897514728459468821`, whose header comment names this guild as the one
production gates on. Mind its warning: a successful run issues **one real
production key**, and it prints the delete command.

Record the raw bodies in this task. If (1) and (2) show `pending` absent, this
task stops and the gate needs a different mechanism — that is a finding, not a
failure, and it is cheaper here than after the switch.

### Measured — observation 1, 2026-09-02

Local `serve` against the real Discord, `PORTAL_GUILD_ID=897514728459468821`,
signed in with an account that had joined Stellar Developers and had **not**
accepted its rules (Adam):

```
2026-09-02T12:22:01Z INFO serve: prices-api local server listening on http://0.0.0.0:8080
2026-09-02T12:22:21Z INFO prices_api::portal::auth: portal sign-in refused outcome="pending_rules"
```

**The REST route carries `pending`.** This is the finding, and it is the one
0180 item 2 was cancelled without making. The verdict is derivable backwards
from the landing, because `membership()` has one arm per shape of the field:

| what Discord sent | verdict | log |
| --- | --- | --- |
| `pending: true` | `PendingScreening` | `outcome="pending_rules"` ← **observed** |
| `pending: false` | `Member` | sign-in proceeds |
| field absent | `Unknown` | `reason="pending_absent"` warn, `outcome="unknown"` |

`pending_absent` did **not** fire, so the field was present and true. The
branch of this task that would have stopped it — "if the field is absent, the
gate needs a different mechanism" — is closed, and the SSM switch carries no
"every visitor is refused" risk beyond the ordinary.

⚠️ **The raw body is not captured**, only the verdict: nothing in the handler
logs Discord's response, deliberately (ADR 0010 — the registry stores no
membership data, and a member object in CloudWatch is membership data). The
log line is the observation this task has, and it is sufficient because the
three arms above are distinguishable from the outside. Capturing a body would
mean a temporary `tracing` line and a decision about logging member objects
that this task should not make on its own.

### Measured — observation 2, 2026-09-02

The same account, after accepting the server's rules on Discord, signing in
again 25 seconds later:

```
2026-09-02T12:25:05Z INFO prices_api::portal::auth: portal sign-in refused outcome="pending_rules"
2026-09-02T12:25:30Z INFO prices_api::portal::keys: portal issued an API key key_id=ak1dkldyog created=false
```

**The field flips.** `pending` went `true` → `false` across one click of the
rules prompt: the first line is `PendingScreening`, the second can only be
reached through `Membership::Member`, and nothing else changed between them.
Observation 1 alone would have been consistent with a field stuck at `true`;
this is the half that rules that out.

`created=false` is the reconciler adopting an existing key rather than
creating one — `discord-1542110353150967951-key`, created earlier the same
day (13:45 local) by an earlier local run, tagged `ManagedBy=prices-portal`.
A **real production key issued from a laptop**: it is the local-run hazard
`packages/prices-api/README.md` and [[0194]] both name, and it is Adam's to
keep or delete.

### Measured — observation 3, 2026-09-02, and the assumption it breaks

The same account again, after **leaving** Stellar Developers:

```
2026-09-02T12:28:03Z WARN prices_api::portal::auth: sign-in membership check answered Unknown Guild (10004) — is the discord-guild-id parameter right? guild_id=897514728459468821
2026-09-02T12:28:03Z INFO prices_api::portal::auth: portal sign-in refused outcome="not_member"
```

The verdict is right. **The code the task predicted is not**: this expected
`10007` ("Unknown Member") and Discord answered **`10004`** ("Unknown Guild")
— with the guild id correct, freshly proved correct by observations 1 and 2
against the same snowflake minutes earlier.

This settles [[0180]] item 1, unmeasured since 0189, and it contradicts what
`discord.rs` says about the code:

> 10004 usually means the *guild id* is wrong, which is our configuration and
> not the user; the caller logs it loudly for exactly that reason.

The mechanism, once seen, is obvious: under `guilds.members.read` a token
whose account is not in the guild **cannot see the guild**, so Discord's
answer is "Unknown Guild", not "Unknown Member". 10004 is the ORDINARY
non-member reply on this route, not the sign of a mis-seed.

**What it costs today.** Both refusal paths (`auth/mod.rs` and
`auth/issue.rs`) `tracing::warn!` on 10004 asking whether the parameter is
right. In production that fires for **every non-member** — the exact
population the portal expects to refuse most often — telling operators to
check a parameter that is fine, and burying the signal the warn exists for.
A mis-seeded guild id produces the same line as an ordinary refusal, so the
line distinguishes nothing.

Not fixed in this task's diff yet: the warn is 0189's deliberate decision,
made on an assumption this measurement is the first to test. See "Open" in
the notes.

Note this is **not** the same question as Onboarding. `pending` is Membership
Screening — the rules checkbox, which is what Adam asked for. Discord Onboarding
(channel/role prompts) lives in `flags`, bit `COMPLETED_ONBOARDING` (1<<1),
which this service deliberately does not deserialize (ADR 0010 — "the registry
stores no membership data"). Stellar Developers has `GUILD_ONBOARDING_HAS_PROMPTS`
set as well, so both are on and step 0 must say which one an unscreened member is
actually held by. **Out of scope unless step 0 shows the guild gates on
Onboarding instead of screening**, in which case the decision comes back to Adam
before any `flags` field is added.

## Implementation

### 1. The guild — one value moves, the other is guarded

- `aws ssm put-parameter --name /prices/production/discord-guild-id --value 897514728459468821 --type String --overwrite --region eu-central-1`.
  No deploy: the value is read per request through the Parameters and Secrets
  extension. Current version is 1, seeded 2026-08-28.
- `STELLAR_DISCORD_INVITE` in `web/portal/src/landing/links.ts` **stays
  `https://discord.gg/stellardev`** — it already resolves to the official guild.
  No edit; what it needs is a comment saying that the link and the SSM parameter
  are one fact in two places, and that the invite is the vanity code SDF owns.
- **The check that did not exist.** Add the verification to `tools/scripts/` —
  resolve the invite in `links.ts` and assert `guild.id` equals the documented
  official guild:
  `curl -s https://discord.com/api/v10/invites/stellardev | jq -r .guild.id` →
  `897514728459468821`. The drift this task was spawned to fix was invisible
  precisely because nothing compared the two; leaving without the comparison
  leaves the same hole for the next change.
- Anywhere `1536303837785362432` (`stellar_test`) or `761985725453303838`
  (Stellar Global) is named outside `lore/`, it is now wrong. Sweep
  `docs/`, `scripts/`, `packages/`, `web/` and say which is the official guild
  where the value lives. `lore/` records are not rewritten ([[0235]]'s rule).

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
- The doc comment on this module carries the example guild id
  `897514728459468821` already; it is now the real one, not an example. Say so.

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
  `discord.com/channels/897514728459468821` opens the server itself; confirm
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
  the screening sentence. The suite asserts `discord.gg/stellardev` in four
  places today — those assertions become the invite's regression guard and stay.
- `docs/runbooks/portal-oauth-deploy-prep.md` — the guild parameter's value and
  the invite are a matched pair; write the check, and name Stellar Developers as
  the official guild.
- `docs/epics/self-service-onboarding.md` — the abuse-barrier section describes
  the gate as membership + account age. It becomes membership + **screening
  cleared** + account age, which is what it always meant and never said.
- ADR 0010 — if step 0 changes what `pending` means on this route, the ADR's
  reading of "must be a member" needs the measurement appended. Not a rewrite.

## Acceptance Criteria

- [ ] Step 0's three observations against `897514728459468821` are recorded in
      this task with raw response bodies
- [ ] `/prices/production/discord-guild-id` is `897514728459468821`
- [x] A scripted check resolves `STELLAR_DISCORD_INVITE` through the invite API
      and asserts it equals the guild the gate uses — the two values can no
      longer drift unnoticed (`npm run discord:verify-guild [-- --ssm]`)
- [x] No file outside `lore/` names `1536303837785362432` or
      `761985725453303838` as a guild the product uses — the test guild is
      named only as the scratch instrument in `measure-pending-absent.sh` and
      as the runbook's "seeded while building" history
- [x] A member with `pending: true` is refused with a distinct outcome, its own
      query parameter, and its own log line — not `not_member`
- [x] That refusal renders "Access not available" with "Stellar Discord accept
      rules required", says the visitor is already on the server, and its action
      opens the server rather than an invite
- [x] `pending: None` still refuses as `Unknown` and still logs `pending_absent`
      — the fail-closed arm is not touched
- [x] The rework path ([[0191]]) gives an unscreened member the same answer as
      the issue path — both consumers of `membership()`/`decide()` (sign-in in
      `auth/mod.rs`, issue in `auth/issue.rs`) match the new variant
- [ ] A real account walks: join Stellar Developers → refused with the new
      screen → accept the rules → sign in → key issued (**step 0 / phase E,
      Adam's hand**)
- [x] Rust and portal suites green; the `not_member` card no longer carries the
      screening sentence

## Implementation Notes

Three commits on `feat/0254_stellar-developers-guild-and-screening-refusal`,
2026-09-02, one per phase of the plan:

**A — Rust** (`31e2383`). `eligibility.rs`: `Membership::PendingScreening`
and `Eligibility::PendingScreening` for `pending: Some(true)`; `decide()`
returns it before the age check, like every membership verdict. `auth/mod.rs`
`PENDING_RULES_QUERY = "?signin=pending_rules"`, `auth/issue.rs`
`ISSUE_PENDING_RULES_QUERY = "?issue=pending_rules"`, both in the
"distinct literals" and "only redirect targets" tests; `outcome =
"pending_rules"` logged on both paths; the module table in `issue.rs` gains
the row and `not_member` loses its "or has not cleared screening" clause.
`after_sign_in`'s unreachable arm lists the new variant. **Modified tests**:
`portal_auth.rs` `a_member_still_in_screening_cannot_sign_in` →
`…_is_refused_as_pending_rules`, `portal_issue.rs` `a_pending_member_is_refused`
→ `…_as_pending_rules` — each asserted `not_member` for a pending member;
the refusal is unchanged, the landing is split. Unit tests: two new rows
(`a_pending_member_is_refused_as_pending_screening`,
`pending_screening_takes_precedence_over_age`), the `None → Unknown`
assertion untouched. 420 → 424 tests, all green, `cargo fmt --check` clean.

**B — Portal** (`65fece3`). `links.ts`: `STELLAR_DISCORD_GUILD_ID =
'897514728459468821'` and `STELLAR_DISCORD_SERVER =
https://discord.com/channels/<id>` derived from it; `STELLAR_DISCORD_INVITE`
unchanged. `oauthPopup.ts`: `'pending_rules'` in the union and the allow-list.
`app.tsx` `LoginView`: the `notMember` screen became `notMember ||
pendingRules` — one `LoginCard` composition, the callout and the button
chosen by the state ("Stellar Discord accept rules required" / "Open Stellar
Discord" → the server; "membership required" / "Join Stellar Discord" → the
invite). The screening sentence on the `not_member` callout is gone.
Dashboard: `refusedPendingRules`, an `issue-pending-rules` paragraph naming
the server, and the `issue-not-member` paragraph without its screening
clause (and reading the invite from `links.ts` rather than a literal).
`app.spec.tsx`: +3 tests (signed-out screen; dashboard `signin=pending_rules`;
`issue=pending_rules` without a key), the `not_member` tests assert the
sentence is absent. 182/182, lint and typecheck clean.

**C — Guard and docs** (`7a28de8`). `tools/scripts/verify-discord-guild.mjs`
(`npm run discord:verify-guild`): resolves the invite through
`GET /api/v10/invites/<code>`, asserts `guild.id === STELLAR_DISCORD_GUILD_ID`,
reports whether `MEMBER_VERIFICATION_GATE_ENABLED` is set, and with `--ssm`
compares the live parameter. **Run on 2026-09-02 it reports the drift this
task exists for**: invite → `897514728459468821`, screening ON, SSM =
`1536303837785362432`, exit 1 with the `put-parameter` line to run. Not in
CI (network). `measure-pending-absent.sh`: default guild is the official one,
the scratch guild is the documented alternative, and a `pending_rules` line
in the log is a verdict of its own. Runbook §2a names the official guild and
the check; the epic's barrier reads membership + screening cleared + age.

## Design Decisions

### From Plan

1. **Stellar Developers `897514728459468821` is the official and only guild**
   (Adam, 2026-09-02) — see the history entry and "The rejected
   alternative".
2. **One screen, two fillings.** `not_member` and `pending_rules` share the
   `LoginCard` composition from frame `825:1485`; only the callout and the
   button differ. Two copies of the JSX is how two refusals end up two
   pixels apart.
3. **The button opens the server by id, never the invite**, for the pending
   state — an invite tells a member they are already in.
4. **The invite is not edited.** `discord.gg/stellardev` already resolves to
   the official guild; what was missing was the comparison, so the change is
   a check and a comment, not a value.

### Emerged

5. **The guard is not in CI.** It calls discord.com; a Discord hiccup must
   not block an unrelated merge. It is an npm script named in the runbook
   and meant to run when either value changes and at deploy prep. If that
   proves too easy to forget, a scheduled workflow (not a PR gate) is the
   next step — noted, not spawned.
6. **Screening OFF is reported, not enforced**, by the guard: it is SDF's
   setting and [[0170]] owns the alert; failing a check we cannot fix would
   only teach people to ignore it.
7. **`STELLAR_DISCORD_GUILD_ID` lives in `links.ts` as the one source** and
   the server URL is derived from it, so the portal cannot open a guild
   other than the one it names — the guard reads the same constant.
8. **`after_sign_in`'s unreachable arm** gained the new variant rather than a
   wildcard: an exhaustive match is what makes the next variant a compile
   error here rather than a silent plain landing.
9. **Both refusal buttons open in a new tab** (Adam, 2026-09-02) — the
   invite on `not_member` as well as the server on `pending_rules`, not just
   the new one. Each sends the visitor to Discord to perform an errand and
   come back, and the page they leave is the page that explains it; the
   sign-in button stays a same-tab navigation because OAuth returns here by
   itself. `rel="noopener noreferrer"` comes from `DiscordButton`'s
   `target`, so no call site can forget it.

## Notes

- **Sequencing.** Step 0 and items 2-3 are ours and can run now against
  `stellar_test` for the code paths. The SSM switch is the moment the portal
  becomes real for outsiders — it belongs next to [[0179]], and [[0164]]'s
  evidence pass must run after it, not before.
- **The barrier is rented.** `MEMBER_VERIFICATION_GATE_ENABLED` is SDF's setting
  on SDF's server. If they turn it off, every joiner is `pending: false`
  immediately and ADR 0010's barrier silently becomes "has a Discord account".
  Nothing alerts on that today; the epic tracks it as [[0170]].
- **32873 members** on the official guild against 3204 on the alternative — the
  gate is a barrier to bots, not a small-community filter, which is what ADR
  0010 assumed and can now say with a number.
- This task does **not** touch `flags` / Discord Onboarding. See step 0.
- Not blocked by [[0195]]'s open operator step (the two `RETAIN`ed buckets); no
  overlap.
