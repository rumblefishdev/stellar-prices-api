---
title: "Stellar Discord server posture, as observable on 2026-08-10"
type: research
status: developing
spawned_from: notes/Q-do-the-two-flagged-auth-assumptions-hold.md
spawns:
  - notes/S-account-model-and-abuse-barrier.md
tags: [discord, stellar, abuse-prevention, ownership]
links: []
history:
  - date: 2026-08-10
    status: developing
    who: claude
    note: "Established what is publicly observable about the Stellar Discord server"
---

# Stellar Discord server posture, as observable on 2026-08-10

Scope of this note: **the server as it actually exists, and who owns the
relationship.** Discord platform mechanics, OAuth scope semantics, AWS, and
mitigation design are covered by sibling notes.

## Headline

The official server is **Stellar Developers**, guild ID
**`897514728459468821`**, joinable by a public one-click vanity invite with no
application or approval. It *does* have Membership Screening enabled
(`MEMBER_VERIFICATION_GATE_ENABLED`) and `verification_level: 2`. The guild ID is
trivially obtainable from public, unauthenticated endpoints.

The epic's claim that other Stellar/SCF-ecosystem services authenticate via
Discord OAuth is **verified** — the SCF Dashboard is a live, working example,
operated by SDF. It requests `identify email connections guilds`.

## 1. What is the official Stellar Discord, and where is the invite published

Four distinct invite URLs are published across SDF properties. **All four resolve
to the same guild, `897514728459468821` ("Stellar Developers").**

| Published at | Invite URL | Resolves to |
|---|---|---|
| `developers.stellar.org` (homepage, "Developer resources") | `https://discord.gg/stellardev` | `897514728459468821` |
| `stellar.org/community` and `stellar.org/connect` | `https://discord.gg/st7Mxd58BV` | `897514728459468821` |
| `communityfund.stellar.org` (homepage) | `https://discord.com/invite/ShHGRudAGv` | `897514728459468821` |
| `stellar/scf-handbook` (GitHub, FAQ) | `https://discord.gg/stellardev` | `897514728459468821` |

On the docs homepage the link text is **"Stellar Developer Discord"**, described
as "Ask questions and engage with other Stellar devs."

> Source: [Developer Tools, SDKs & Core Resources for Building | Stellar Docs](https://developers.stellar.org/) — fetched 2026-08-10

On stellar.org the wording is **"Join the conversation on the Stellar Discord,
ask questions, and post some memes."**

> Source: [Stellar | The Stellar Community](https://stellar.org/community) — fetched 2026-08-10
> Source: [Stellar | Connect](https://stellar.org/connect?locale=en) — fetched 2026-08-10

The SCF handbook publishes **"Join the [Stellar Developers Discord Server](https://discord.gg/stellardev)
to connect with other folks in the Stellar ecosystem"** and directs support to
**"ask it in the `#scf-general` channel"**.

> Source: [scf-handbook/additional-support/faq.md at main · stellar/scf-handbook](https://github.com/stellar/scf-handbook/blob/main/additional-support/faq.md) — fetched 2026-08-10

**Detail worth recording:** only `stellardev` is the guild's registered vanity
code (`"vanity_url_code": "stellardev"`). The other three are **personal invites
created by individual Discord accounts** — the invite objects carry an `inviter`
field naming users `.aythia`, `ankeliu`, and `gd2000`. They are permanent
(`expires_at: null`) but they are individual-account artefacts, not
organisational ones, and can be revoked by their creators or by the member
leaving. **Use `discord.gg/stellardev` if we ever hard-code an invite.**

> Source: [Discord invite API — code `st7Mxd58BV`](https://discord.com/api/v10/invites/st7Mxd58BV?with_counts=true&with_expiration=true) — fetched 2026-08-10
> Source: [Discord invite API — code `ShHGRudAGv`](https://discord.com/api/v10/invites/ShHGRudAGv?with_counts=true&with_expiration=true) — fetched 2026-08-10
> Source: [Discord invite API — code `PacPQu9URv`](https://discord.com/api/v10/invites/PacPQu9URv?with_counts=true) — fetched 2026-08-10

**Adjacent servers that are NOT the one we mean.** The handbook lists two "Other
official Stellar Discord servers": Stellar Global (`discord.gg/5g2YkszV3D`,
guild `761985725453303838`, 3,189 members) and Stellar Quest
(`discord.gg/aWmhSXfRG3`, guild `763798356484161566`, name "Lumenauts", 1,172
members). There is also a **"Stellar Community Fund [Archived]"** guild
(`831188872536784947`, 2,041 members, `verification_level: 4`) — SCF discussion
migrated into the main Stellar Developers server as `#scf-general`. If we
configure a guild ID, it must be `897514728459468821` and nothing else.

> Source: [scf-handbook navigating-discord/README.md (raw)](https://raw.githubusercontent.com/stellar/scf-handbook/main/additional-support/navigating-discord/README.md) — fetched 2026-08-10
> Source: [Discord invite API — code `22jPRDaczh`](https://discord.com/api/v10/invites/22jPRDaczh?with_counts=true) — fetched 2026-08-10

**One published invite is already broken.** The stellar.org page for the weekly
Developer and Protocol Meeting publishes `https://discord.com/invite/hAZTTvtq?event=1260569514056093706`;
that code returns `{"code": 10006}` (Unknown Invite). Evidence that
SDF-published invite links rot.

> Source: [Stellar | Stellar Developer and Protocol Meeting](https://stellar.org/community/events/developer-and-protocol-meeting) — fetched 2026-08-10

## 2. Is joining open or gated?

**Open.** No application, no approval, no referral. Every published route is a
one-click permanent invite, and the server is additionally listed in Discord's
public Server Discovery directory with a "Join Server" button.

> Source: [Stellar Developers - Discord Servers](https://discord.com/servers/stellar-developers-897514728459468821) — fetched 2026-08-10

SDF publishes nothing describing a joining barrier — the wording everywhere is
invitational ("Join the conversation", "Ask questions and engage"). The guild
object confirms `DISCOVERABLE` ("guild is able to be discovered in the
directory") and `CONSIDERED_EXTERNALLY_DISCOVERABLE`.

> Source: [Guild Resource — Discord Developer Docs](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

The Discovery listing gives the server creation date as **October 12th, 2021**,
activity "Like a busy coffee shop", categories Programming / Cryptocurrency /
Science & Tech, language English.

> Source: [Stellar Developers - Discord Servers](https://discord.com/servers/stellar-developers-897514728459468821) — fetched 2026-08-10

## 3. Rules acceptance, verification, onboarding for new members

Three things are publicly observable, and one important thing is not.

**(a) The invite lands in a `rules` channel.** Every invite to this guild carries
`"channel": {"id": "900373252420030465", "type": 0, "name": "rules"}` — the
invite's target channel is the rules channel. That is a design signal, not an
enforcement mechanism.

**(b) Membership Screening is enabled.** The guild's features include
`MEMBER_VERIFICATION_GATE_ENABLED`, which Discord documents as **"guild has
enabled Membership Screening"**. It also carries `GUILD_ONBOARDING`,
`GUILD_ONBOARDING_HAS_PROMPTS`, `GUILD_ONBOARDING_EVER_ENABLED`,
`WELCOME_SCREEN_ENABLED`, `GUILD_SERVER_GUIDE`, and `AUTO_MODERATION`.

> Source: [Discord invite API — code `stellardev`](https://discord.com/api/v10/invites/stellardev?with_counts=true&with_expiration=true) — fetched 2026-08-10
> Source: [Guild Resource — Discord Developer Docs](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

**This is the single most load-bearing fact in this note for the epic's abuse
argument:** Membership Screening is what makes the `pending` field on a guild
member object meaningful. The gate the epic assumes exists, does exist at the
server level. Whether our OAuth flow can *see* it is a separate question and is
the sibling note's case.

**(c) `verification_level: 2`.** Discord documents level 2 as **MEDIUM — "must be
registered on Discord for longer than 5 minutes"**.

> Source: [Guild Resource — Discord Developer Docs](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

Read plainly: the server-level verification setting is a **five-minute account
age requirement**. It is not an email requirement (level 1) and not a phone
requirement (level 4). As an anti-churn barrier this is close to nil on its own —
the archived SCF guild is set to level 4, so SDF clearly knows level 4 exists and
has chosen not to use it on the main server.

**(d) Published rules.** SDF publishes a rules document in the SCF handbook. It
is enforced socially, by humans, after the fact:

> "Members with a `@Mod` or `@Admin` role have the authority to enforce these
> rules to maintain a safe and healthy server and can ban members who violate the
> rules."

And, directly relevant to our abuse model:

> "don't create sock-puppet accounts to hold conversations about your project."

> Source: [scf-handbook discord-rules-and-guidelines.md (raw)](https://raw.githubusercontent.com/stellar/scf-handbook/main/additional-support/navigating-discord/discord-rules-and-guidelines.md) — fetched 2026-08-10

Caveat on that document: its rule 6 says *"This Discord server is for the Stellar
Community Fund-related projects only. For all developer talk, join the Stellar
Developers Discord"* — so the text was written for the now-archived SCF guild and
has not been fully rewritten for the merged server. Treat it as indicative of
SDF's posture, not as a verbatim description of the current server's rules
channel.

**(e) Published roles.** The handbook lists the role taxonomy, including `Admin`
("Administrator of the server"), `SDF` ("Employee of the Stellar Development
Foundation"), `Mod`, `Verified`, `Voter`, `Expert`, and per-round `Candidate`/
`Winner` roles. `Everyone` is defined as **"Everyone in the server without a
role"** — i.e. the default state of a new joiner is roleless, and general
channels (`#general`, `#support`, `#project-announcements`) are not role-gated.
Only `#verified-panel`, `#roles`, `#meeting` (Verified-only) and `#admin`
(Admin/Mod-only) are marked as restricted.

> Source: [scf-handbook channels-and-roles.md (raw)](https://raw.githubusercontent.com/stellar/scf-handbook/main/additional-support/navigating-discord/channels-and-roles.md) — fetched 2026-08-10

**Non-observability, stated explicitly.** Whether Membership Screening is
configured with real friction (a multi-question form, a manual approval queue) or
is a single "I agree to the rules" checkbox is **not observable from public
sources as of 2026-08-10**. The invite API exposes the *feature flag*, not the
screening form's contents. Likewise, the actual text of the `rules` channel, the
Onboarding prompts, the AutoMod rule set, and any verification bot are **not
observable from public sources as of 2026-08-10** — they require being a member
of the guild. Reading them costs one person one click; nobody has done it yet.

## 4. Can the guild ID be determined from public information?

**Yes, trivially, unauthenticated.** `GET https://discord.com/api/v10/invites/{code}?with_counts=true`
returns the guild object for any valid invite code. No token, no app registration.

Raw values returned for `stellardev` on 2026-08-10:

```
guild.id                        897514728459468821
guild.name                      "Stellar Developers"
guild.description               "Stellar is where blockchain meets the real world."
guild.verification_level        2
guild.vanity_url_code           "stellardev"
guild.nsfw_level                0
guild.premium_tier              3
guild.premium_subscription_count 17
guild_id                        897514728459468821
channel                         { id: 900373252420030465, type: 0, name: "rules" }
expires_at                      null
approximate_member_count        32419
approximate_presence_count      1362
profile.tag                     "XLM"
```

Features asked about specifically:

```
MEMBER_VERIFICATION_GATE_ENABLED   present
COMMUNITY                          present
WELCOME_SCREEN_ENABLED             present
DISCOVERABLE                       present
```

Full `guild.features` array as returned (order varies per response; set is
stable across the three live invite codes):

```
AGE_VERIFICATION_LARGE_GUILD, MAX_FILE_SIZE_100_MB, AUDIO_BITRATE_128_KBPS,
ANIMATED_BANNER, ANIMATED_ICON, TIERLESS_BOOSTING_SYSTEM_MESSAGE,
AUDIO_BITRATE_384_KBPS, CONSIDERED_EXTERNALLY_DISCOVERABLE, GUILD_ONBOARDING,
STAGE_CHANNEL_VIEWERS_300, ENABLED_DISCOVERABLE_BEFORE, NEWS,
STAGE_CHANNEL_VIEWERS_50, VIDEO_QUALITY_720_60FPS, GUILD_WEB_PAGE_VANITY_URL,
AUTO_MODERATION, STAGE_CHANNEL_VIEWERS_150, COMMUNITY_EXP_MEDIUM,
PREVIEW_ENABLED, ACTIVITY_FEED_DISABLED_BY_USER, ROLE_ICONS,
GUILD_ONBOARDING_HAS_PROMPTS, INVITE_SPLASH, CHANNEL_ICON_EMOJIS_GENERATED,
SOUNDBOARD, MEMBER_VERIFICATION_GATE_ENABLED, VIDEO_BITRATE_ENHANCED, GUILD_TAGS,
GUILD_SERVER_GUIDE, VIDEO_QUALITY_1080_60FPS, GUILD_ONBOARDING_EVER_ENABLED,
WELCOME_SCREEN_ENABLED, TIERLESS_BOOSTING, VANITY_URL, MAX_FILE_SIZE_50_MB,
AUDIO_BITRATE_256_KBPS, DISCOVERABLE, BANNER, COMMUNITY
```

> Source: [Discord invite API — code `stellardev`](https://discord.com/api/v10/invites/stellardev?with_counts=true&with_expiration=true) — fetched 2026-08-10

The response also carries a `liveliness.msg_activity_bins` array with
`last_updated_ts: "2026-08-09T00:22:27+00:00"` — ~32.4k members, ~1.36k online,
and low double-digit messages per bin. A real but not enormous community.

**Practical note for [[0158]]/[[0160]]:** the ID is a stable snowflake and does
not need to be discovered at runtime. Confirming it once and putting
`897514728459468821` in SSM is correct; there is no need to build invite
resolution into the service.

## 5. Who runs it?

**Owning organisation: the Stellar Development Foundation (SDF).** Evidenced by
SDF publishing the invite on its own properties (stellar.org,
developers.stellar.org, communityfund.stellar.org), by the role taxonomy carrying
an explicit `SDF` role defined as "Employee of the Stellar Development
Foundation", and by SDF hiring for the function.

The SDF Community Manager job description makes ownership explicit:

> "Own Discord membership numbers, identify ways to increase Discord community
> engagement, optimize Discord community channels, and manage moderator and
> ambassador roles in the ecosystem"

> Source: [Community Manager @ Stellar Development Foundation | Blockchain Association Job Board](https://jobs.theblockchainassociation.org/companies/stellar-development-foundation/jobs/32250422-community-manager) — fetched 2026-08-10

**Published contact routes** (in descending order of usefulness to us):

1. **`communityfund@stellar.org`** — the only public email address found.
   Handbook wording: *"Email communityfund@stellar.org, or (for a faster
   response), send a message in `#scf-general` in the Stellar Developers
   Discord."*
   > Source: [scf-handbook/additional-support/faq.md](https://github.com/stellar/scf-handbook/blob/main/additional-support/faq.md) — fetched 2026-08-10
2. **`#scf-general`** in the Stellar Developers Discord — the handbook's own
   preferred, faster route.
3. **`#support`** — handbook describes it as *"For all questions, reports, or
   other support requests"*.
   > Source: [scf-handbook channels-and-roles.md (raw)](https://raw.githubusercontent.com/stellar/scf-handbook/main/additional-support/navigating-discord/channels-and-roles.md) — fetched 2026-08-10
4. **The SDF DevRel team**, which hosts the weekly Developer and Protocol Meeting
   on Discord every Thursday — a standing, low-friction venue to raise this.
   > Source: [Stellar | Stellar Developer and Protocol Meeting](https://stellar.org/community/events/developer-and-protocol-meeting) — fetched 2026-08-10

**No named individual is public.** SDF publishes team functions (Community
Manager, DevRel team), not a named Discord owner or admin contact. The individual
Discord usernames surfaced as invite creators (`.aythia` / "bri", `ankeliu` /
"anke.xlm", `gd2000` / "Gemma", `kalepail`) are incidental artefacts of invite
objects, not published points of contact, and should not be treated as such.

**Recommendation for the ADR's "owner" line:** the owner of the *Stellar Discord
relationship* on our side must be one of our people — this cannot be delegated to
SDF. The correct external route to name alongside them is
`communityfund@stellar.org` plus `#scf-general`; the correct internal statement is
"no named SDF counterpart is public as of 2026-08-10". [[0159]]'s "someone" needs
one of Adam or Oskar written into it; that is a decision for the epic owner, not
a research finding.

## 6. Verifying the epic's claim about ecosystem precedent

**Claim under test:** *"Sign-in via Discord OAuth — matches how other Stellar/SCF-ecosystem
services authenticate against the Stellar Discord."*

**Verdict: VERIFIED, with a caveat that cuts against us.**

The SCF Dashboard at `communityfund.stellar.org` authenticates via Discord OAuth
and nothing else. Requesting `/dashboard` unauthenticated produces this redirect
chain (reproduced with `curl`, unauthenticated, 2026-08-10):

```
GET https://communityfund.stellar.org/dashboard
  308 -> /dashboard/award-rounds
  307 -> /dashboard/login?callbackUrl=%2Fdashboard%2Faward-rounds
  307 -> https://discord.com/api/oauth2/authorize
            ?scope=identify+email+connections+guilds
            &response_type=code
            &client_id=917408694822658160
            &redirect_uri=https%3A%2F%2Fcommunityfund.stellar.org%2Fapi%2Fauth%2Fcallback%2Fdiscord
  302 -> https://discord.com/oauth2/authorize?...(same params)
  200
```

> Source: [Stellar Community Fund dashboard login redirect chain](https://communityfund.stellar.org/dashboard) — fetched 2026-08-10

The OAuth application behind `client_id=917408694822658160` is confirmed as SDF's
via Discord's public application RPC endpoint:

```json
{"id":"917408694822658160","name":"Stellar Community Fund",
 "description":"Build, Engage, Launch, and Grow.",
 "terms_of_service_url":"https://www.stellar.org/terms-of-service",
 "privacy_policy_url":"https://www.stellar.org/privacy-policy",
 "bot_public":false,"is_verified":false,"is_discoverable":false}
```

> Source: [Discord application RPC — 917408694822658160](https://discord.com/api/v10/applications/917408694822658160/rpc) — fetched 2026-08-10

So: a first-party SDF service, in production, uses Discord OAuth as its sole
sign-in. The epic's precedent claim stands. Discord identity is also load-bearing
for SCF governance — the handbook's verified-member flow is *"Join the Stellar
Developer Discord"*, then *"Verify at least one social account on your Discord
using linked roles"*, then *"Register on the SCF Dashboard"* and *"Authenticate
with a Stellar wallet address"*, with *"Both Verified and Pathfinder roles …
granted automatically based on verifying information in the SCF Dashboard."*

> Source: [How to Become Verified | Stellar Community Fund - Handbook](https://stellar.gitbook.io/scf-handbook/governance/verified-members/how-to-become-verified) — fetched 2026-08-10

**Three caveats the ADR must carry:**

1. **SCF requests `guilds` — the scope our task forbids.** The precedent is real
   but it is a precedent for the *broad* scope. "Matching how other services do
   it" is therefore not an argument for our scope choice; if anything, matching
   SCF exactly would be a regression. Our note on scopes should say we
   deliberately diverge from the ecosystem precedent here.
2. **SCF's Discord identity is not doing abuse-prevention work.** It is a
   convenience login; the actual sybil resistance in SCF's design comes from the
   *additional* layers — linked-role social verification and Stellar wallet
   authentication. SCF does **not** treat "has a Discord account" as sufficient.
   That is the opposite of the epic's position and is the most important thing on
   this page for the abuse argument.
3. **SCF does not appear to gate on Stellar-guild membership either** — nothing
   published says the dashboard checks that you are in `897514728459468821`,
   and `guilds` returning the user's server list is not evidence that it does.
   Whether SCF actually checks is **not observable from public sources as of
   2026-08-10**.

**Other ecosystem services checked, with no Discord OAuth found:** Stellar Quest
(`quest.stellar.org`) returns HTTP 200 with no redirect to Discord OAuth on `/`
or `/login`, and no Discord sign-in control was identifiable in the fetched page.
> Source: [Stellar Quest - Launch Your Blockchain Education](https://quest.stellar.org/) — fetched 2026-08-10

So the honest statement is: **one** first-party SDF example, not a plural
"services". The epic's plural phrasing overstates it.

## What could NOT be established from public sources

Each of these is a genuine gap, not an omission:

1. **The contents of the Membership Screening form.** We know the gate is
   enabled; we do not know whether it is one checkbox or a real questionnaire,
   nor whether it is manually approved. **Not observable from public sources as of
   2026-08-10.** Requires guild membership to see. *This is the cheapest
   outstanding item and it decides how much the `pending` field is worth.*
2. **The Onboarding prompts and Server Guide contents.** Feature flags present;
   contents **not observable from public sources as of 2026-08-10**.
3. **The `rules` channel text and whether acceptance is enforced.** The published
   handbook rules are legacy text written for the archived SCF guild.
   **Not observable from public sources as of 2026-08-10.**
4. **The AutoMod configuration.** `AUTO_MODERATION` is on; its rules are
   **not observable from public sources as of 2026-08-10**.
5. **Any role-gating a fresh account cannot clear.** The handbook implies general
   channels are open to roleless members, but the live channel permission matrix
   is **not observable from public sources as of 2026-08-10**.
6. **Whether the SCF Dashboard actually enforces guild membership** despite
   requesting `guilds`. **Not observable from public sources as of 2026-08-10.**
7. **A named SDF individual owning the Discord.** SDF publishes roles, not names.
   **No named person is public as of 2026-08-10.**
8. **Any published SDF policy on third-party services authenticating against
   their Discord** — no developer/partner policy, no rate limits, no permission
   requirement was found. **Not observable from public sources as of 2026-08-10.**
   We should not assume SDF has an opinion, nor that they have none.

## Implications for the epic (for the ADR to arbitrate)

- The barrier the epic relies on — "throwaway Discord accounts are non-trivial to
  churn against Stellar's server" — is **partly real**: Membership Screening is
  genuinely enabled. But joining is one public click with no approval, and the
  server-level verification setting is a **five-minute account age**, which is
  not a meaningful churn cost.
- More decisively: **the barrier lives at the guild level, and our epic's flow
  never touches the guild.** Establishing that the gate exists is not the same as
  benefiting from it. That is the sibling note's determination to make, but this
  note's evidence points one way.
- **SDF's own service does not rely on Discord identity alone** for anything with
  a cost attached. That is the strongest available evidence that the epic's
  single-barrier model is thinner than assumed.
