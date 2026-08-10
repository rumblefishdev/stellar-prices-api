---
title: "Discord platform verification and gating mechanics"
type: research
status: developing
spawned_from: notes/Q-do-the-two-flagged-auth-assumptions-hold.md
spawns:
  - notes/S-account-model-and-abuse-barrier.md
tags: [discord, auth, abuse-prevention, verification]
links: []
history:
  - date: 2026-08-10
    status: developing
    who: claude
    note: "Researched Discord account-level and server-level verification mechanics"
---

# Discord platform verification and gating mechanics

Scope: what **Discord the platform** requires of a new account, and what **a server
owner** can require of a new member. Deliberately excludes: what Stellar's server is
actually configured to do, which OAuth scope we should request, AWS-side quota design,
and mitigation choices. Those are covered by sibling notes.

All sources fetched 2026-08-10. `support.discord.com` HTML is Cloudflare-blocked to
automated fetches; article bodies were retrieved via the public Zendesk Help Center JSON
API (`/api/v2/help_center/en-us/articles/{id}.json`), which serves the same article body.
The canonical human URL is cited in each case.

---

## Bottom line for the epic's abuse argument

Discord's own account-creation barrier is **weak and is not, by default, a verification
gate**: an account can be created and used to chat before email is verified, and phone is
not required at signup. The meaningful barriers are all **server-side** and **opt-in by
the server owner** — verification level, Rules Screening, role gating. Therefore the
question "does Discord verify new accounts?" resolves to "not really; the server might",
which pushes the whole load onto whatever Stellar's server has configured.

Of the server-side barriers, exactly one is cleanly observable through the API on a guild
member object: **`pending`** (Rules Screening not yet accepted). Verification-level
compliance is **not** exposed as a field on the member object — see §2.4.

---

## 1. What Discord requires to create and use an account

### 1.1 Account creation and email

Registration is via the registration page or in-app. The published guidance on the
email is about *recoverability*, not verification:

> "When registering your Discord account, please use an email address you can access and
> maintain access to. If you want to change the email address of your Discord account,
> you'll need access to your original email."

> Source: [Getting Started](https://support.discord.com/hc/en-us/articles/360033931551-Getting-Started) — fetched 2026-08-10

Email verification is explicitly **not required to use the account**. Discord calls the
verified state "claiming" the account, and states plainly that an unclaimed account can
still chat:

> "If your new account is not verified, your username will be given a random 5 digits to
> the end of your username after 7 days from account creation.
>
> You'll be able to enjoy all of the chat functions Discord has to offer, but it's
> important to claim your account by verifying your email address."

This is the single most load-bearing quote in this note: **an unverified account can use
Discord's chat functions.** The same article notes the only hard consequence is
server-side: "Some servers are locked behind a security Verification wall that requires a
verified account before you can access chat functions."

> Source: [Getting Started](https://support.discord.com/hc/en-us/articles/360033931551-Getting-Started) — fetched 2026-08-10

Caveat on staleness: the quoted passage still refers to a "Discriminator (those four
digits next to your user name)", which is legacy terminology from before Discord's
username migration. The article's `updated_at` is 2026-08-10, but this paragraph reads as
un-revised. Treat the *mechanism* (unverified accounts work) as current; treat the
*discriminator detail* as probably stale. Not resolvable from the source.

### 1.2 Phone verification

Phone is presented as optional and additive, added from user settings after the fact:

> "Additionally, you can also verify your phone number to your Discord account!"

> Source: [Getting Started](https://support.discord.com/hc/en-us/articles/360033931551-Getting-Started) — fetched 2026-08-10

Phone verification becomes *mandatory* only when Discord's risk systems demand it, or when
a server's verification level demands it:

> "In order to use Discord and check out Discord servers, you may be required to verify
> your email and phone number as part of our safety system to keep users and communities
> safe."

> "We may require you to verify your account using a valid phone number. The phone number
> you use cannot be associated with an existing Discord account."

The triggers Discord lists for a "Verification Required" prompt are behavioural, not
signup-time:

> "joined multiple servers in a short period of time" / "direct messaged multiple users
> who are not your friends" / "sent messages or posted content to communities faster than
> a human can manage" / "used third party clients or modifications to Discord"

Constraints that make phone-verified accounts genuinely costly to farm:

> "VOIP, Burner, or Landline numbers cannot be used."

> "Rate Limit/Recently Used: the phone number was recently used and is currently in a
> timeout period." — remedy: "Please wait 24-48 hrs before verifying your phone number
> once more."

> Source: [How to Verify Your Discord Account](https://support.discord.com/hc/en-us/articles/6181726888215-How-to-Verify-Your-Discord-Account) — fetched 2026-08-10

**Reading for us:** one phone number = one Discord account is enforced, and VOIP/burner
numbers are rejected. That is a real anti-alt property — but it only binds accounts that
were *forced* to verify. A freshly created, never-flagged account has neither a verified
email nor a phone attached.

### 1.3 What an unverified / suspicious account cannot do

Discord has a separate degraded state, "Limited Access", applied on suspicion:

> "If Discord detects suspicious account behavior on your account, we may place your
> account in Limited Access."

> "If your account is in Limited Access, you will not be able to join new servers or
> initiate new direct messages."

In Limited Access, a user can "Message your existing friends" but can't "Send outgoing
friend requests", "Direct message any new friends added while you were in Limited Access",
or "Join new servers". Notably: "Other users will not see any indication that your account
is in Limited Access."

> Source: [Limited Access FAQ](https://support.discord.com/hc/en-us/articles/6461420677527-Limited-Access-FAQ) — fetched 2026-08-10

This state is **not exposed on the guild member object** — not stated in any source that it
is observable via the API, and it does not appear in the guild member field table (§2.4).

---

## 2. Server Verification Levels

### 2.1 API enum — exact values

| Level | Integer | Description (verbatim from API docs) |
|-------|---------|--------------------------------------|
| `NONE` | 0 | "unrestricted" |
| `LOW` | 1 | "must have verified email on account" |
| `MEDIUM` | 2 | "must be registered on Discord for longer than 5 minutes" |
| `HIGH` | 3 | "must be a member of the server for longer than 10 minutes" |
| `VERY_HIGH` | 4 | "must have a verified phone number" |

Exposed on the guild object as `verification_level` (integer).

> Source: [Guild Resource — Discord Developer Documentation](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

### 2.2 The support article's wording is cumulative; the API's is not

The API table above reads as if each level imposes only its own row. The user-facing
article makes clear the levels are **cumulative**:

> **Low:** "the user must have a verified email on their Discord account before they're
> approved to participate in the server."

> **Medium:** "Medium server verification settings will require your Discord account to
> have a verified email. It must be verified for longer than five minutes before you are
> able to start chatting in the server."

> **High:** "Including requiring a verified email AND being registered on Discord for more
> than 5 minutes. You must also be present in the server for longer than 10 minutes."

> **Highest:** "In addition to everything above (verified email, registered user for 5+
> minutes, and server member for 10+ minutes), they will also need to have a verified
> phone number attached to their Discord account."

> **None:** "Selecting None will forego any verification security, and anyone who enters
> your server will be able to chat immediately upon entering."

> Source: [Verification Levels](https://support.discord.com/hc/en-us/articles/216679607-Verification-Levels) — fetched 2026-08-10

Note the discrepancy between the two sources on *what* the 5-minute clock measures. The
API says MEDIUM = "registered on Discord for longer than 5 minutes"; the support article
says Medium = email "verified for longer than five minutes" but then describes High as
"registered on Discord for more than 5 minutes". The sources are not consistent. Not
resolvable from either source; if it matters, it must be tested.

### 2.3 Phone verification supersedes everything — a real hole in the argument

> "Having a verified phone number supersedes all other requirements. This means that if a
> user has a verified phone number on their Discord account, they can participate in
> servers with any verification level, without needing to meet the email verification or
> time-based requirements."

And the FAQ restates it: "Q: Do I still need to meet other requirements if I have a
verified phone number? A: No. Having a verified phone number supersedes all other
requirements."

> Source: [Verification Levels](https://support.discord.com/hc/en-us/articles/216679607-Verification-Levels) — fetched 2026-08-10

**Reading for us:** the time-based components of MEDIUM/HIGH — the ones that would actually
slow down a churn attack — are bypassed entirely by a phone-verified account. Any
account-age or dwell-time barrier from verification levels evaporates for an attacker
willing to attach a phone number.

Verification levels are also **server-wide and text-oriented only**:

> "Verification Levels refer to the levels of security a user must meet before they're
> allowed to send text messages in a channel."

> "Q: Can different channels have different verification levels? A: No, verification
> levels are set server-wide and apply to all channels within that server."

> "Q: What happens if a user doesn't meet the verification requirements? A: Users who don't
> meet the verification requirements won't be able to send text messages in the server
> channels or join voice channels."

> Source: [Verification Levels](https://support.discord.com/hc/en-us/articles/216679607-Verification-Levels) — fetched 2026-08-10

**Critical for us:** verification level gates *talking*, not *joining*. A member who fails
the verification level is still a member. If we ask Discord "is this user a member of guild
X", the answer is yes regardless.

### 2.4 Verification-level compliance is NOT on the guild member object

The guild member object fields are:

```
user?, nick?, avatar?, banner?, roles, joined_at, premium_since?, deaf, mute,
flags, pending?, permissions?, communication_disabled_until?,
avatar_decoration_data?, collectibles?
```

There is no field expressing "this member satisfies the guild's verification level".

> Source: [Guild Resource — Guild Member Object](https://docs.discord.com/developers/resources/guild#guild-member-object) — fetched 2026-08-10

There is one adjacent signal — a per-member *exemption* flag:

> `BYPASSES_VERIFICATION` | `1 << 2` | "Member is exempt from guild verification
> requirements" | Editable: true

with the accompanying note: "BYPASSES_VERIFICATION allows a member who does not meet
verification requirements to participate in a server."

> Source: [Guild Resource — Guild Member Flags](https://docs.discord.com/developers/resources/guild#guild-member-flags) — fetched 2026-08-10

That flag tells us a moderator manually waved someone through. It does not tell us whether
an ordinary member passes.

---

## 3. Membership Screening / Rules Screening and the `pending` flag

### 3.1 What it is, from the user-facing side

Discord's current product name is **Rules Screening**; the API name is **Membership
Screening**. It requires a Community-enabled server:

> "In order to see this feature, you must enable Community for your Discord server."

> "Rules screening allows you to set up rules that new members must explicitly agree to
> before they can talk, react, or DM Other members."

> "We've ensured that pending members are not able to talk, DM server members, or react
> until they've accepted the rules."

Two important qualifiers:

> "Note: Manually verifying a member (Server Settings > Members) will bypass this
> requirement."

> "Q: Are there other types of membership screening capabilities? A: Rules screening is
> currently our only available membership screening capability."

And Discord itself acknowledges the friction cost, which is relevant to whether a
well-run server will even have it on:

> "However, do note that it will add another step to joining your server which can
> sometimes lead to a drop off in joiners."

> Source: [Rules Screening FAQ](https://support.discord.com/hc/en-us/articles/1500000466882-Rules-Screening-FAQ) — fetched 2026-08-10

**Reading for us:** Rules Screening is a click-through rules agreement. It is a *friction*
gate, not an *identity* gate. It costs an attacker one extra click, not an extra identity.
It is also proof-of-nothing about the account behind it.

### 3.2 `pending` — exact semantics

Field definition, verbatim:

> `pending?` | `boolean` | "whether the user has not yet passed the guild's Membership
> Screening requirements"

The `?` marks it **optional** — it may be absent from the object.

Lifecycle, verbatim:

> "In guilds with Membership Screening enabled, when a member joins, Guild Member Add will
> be emitted but they will initially be restricted from doing any actions in the guild, and
> `pending` will be `true` in the member object. When the member completes the screening,
> Guild Member Update will be emitted and `pending` will be `false`."

Presence rule, verbatim:

> "In `GUILD_` events, `pending` will always be included as true or false. In non `GUILD_`
> events which can only be triggered by non-`pending` users, `pending` will not be
> included."

And from the Add Guild Member endpoint:

> "For guilds with Membership Screening enabled, this endpoint will default to adding new
> members as `pending` in the guild member object. Members that are `pending` will have to
> complete membership screening before they become full members that can talk."

> Source: [Guild Resource — Membership Screening Object](https://docs.discord.com/developers/resources/guild#membership-screening-object) — fetched 2026-08-10

### 3.3 What the docs do NOT say — flag this

The task asked explicitly whether `pending` is only meaningful when the guild has
Membership Screening enabled, and what value it takes otherwise.

**Not stated in the source.** Every documented statement about `pending` is scoped with
"In guilds with Membership Screening enabled" / "For guilds with Membership Screening
enabled". The docs describe the presence rule only in terms of *gateway event class*
(`GUILD_` vs non-`GUILD_`), never in terms of the REST response for
`GET /users/@me/guilds/{guild.id}/member`. For a guild **without** screening, whether the
field is `false` or simply absent is undocumented.

Practical consequence: code must treat `pending` as `boolean | undefined` and must not read
`pending === false` as "this user cleared a screening gate" — in a guild with no screening
configured, that same value (or absence) means "there was no gate". `pending` distinguishes
*cleared* from *not yet cleared*; it does not distinguish *cleared* from *no gate existed*.
The only documented way to tell those apart is the guild-level feature flag:

> `MEMBER_VERIFICATION_GATE_ENABLED` | "guild has enabled Membership Screening"

> Source: [Guild Resource — Guild Features](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

Reading `features` requires a guild object, which `guilds.members.read` does not return —
the guild's posture would have to be established out-of-band (once, manually) or via a bot.
Whether we can read it at all is a scope question and belongs to the sibling note.

### 3.4 The Membership Screening API is explicitly unstable

> "We are making significant changes to the Membership Screening API specifically related to
> getting and editing the Membership Screening object. Long story short is that it can be
> improved. As such, we have removed those documentation. There will **not be** any changes
> to how pending members work, as outlined above. That behavior will stay the same."

> Source: [Guild Resource — Membership Screening Object](https://docs.discord.com/developers/resources/guild#membership-screening-object) — fetched 2026-08-10

So: reading/editing the screening *object* is undocumented and in flux; the `pending`
*member* behaviour is explicitly promised stable. If we depend on anything here, depend on
`pending` only.

---

## 4. Server Onboarding

### 4.1 What it is, and how it differs from Rules Screening

Community Onboarding is a **channel/role self-selection** flow, not a barrier:

> "With Community Onboarding, new members get to pick out their own roles and channels and
> enjoy a personalized channel list in your server by answering a few simple questions."

Per-question controls:

> "Ask before a member joins - this shows your questions and answers to new members right
> before they join your server."

> "Make Required - members must answer the question to proceed into your server."

Setup constraints:

> "You must have selected at least 7 Default Channels" and "At least 5 of these channels
> must allow @everyone to View and Send Messages"

> Source: [Community Onboarding FAQ](https://support.discord.com/hc/en-us/articles/11074987197975-Community-Onboarding-FAQ) — fetched 2026-08-10

**The direction of the feature is the finding.** Discord actively markets Onboarding as a
*replacement for* verification friction, and tells admins to remove their gates:

> "5. Remove verification steps that overwhelm or lock new members from joining your server"

> "At this stage, consider removing bot-powered restrictions that add confusion and turn
> away new members trying to join your server."

> "'But Discord, wouldn't that make my server more vulnerable to raiders also trying to join
> my server?!' To that, we have the solution: Raid Protection"

> Source: [Community Onboarding FAQ](https://support.discord.com/hc/en-us/articles/11074987197975-Community-Onboarding-FAQ) — fetched 2026-08-10

That is Discord's official recommendation to community servers: swap the human-visible gate
for invisible ML-based raid detection. A large, well-run server following current Discord
guidance is therefore *less* likely to have a screening gate we can observe, not more. That
cuts directly against the epic's assumption.

Also note, relevant to any "leaves and rejoins" reasoning:

> "Q: Will members who leave my server and rejoin have to go through my onboarding process
> again? A: Yes."

> Source: [Community Onboarding FAQ](https://support.discord.com/hc/en-us/articles/11074987197975-Community-Onboarding-FAQ) — fetched 2026-08-10

### 4.2 Is onboarding API-observable?

Partly, and via a different mechanism than `pending` — the `flags` bitfield on the guild
member object:

| Flag | Value | Description (verbatim) |
|------|-------|------------------------|
| `DID_REJOIN` | `1 << 0` | "Member has left and rejoined the guild" |
| `COMPLETED_ONBOARDING` | `1 << 1` | "Member has completed onboarding" |
| `STARTED_ONBOARDING` | `1 << 3` | "Member has started onboarding" |
| `STARTED_HOME_ACTIONS` | `1 << 5` | "Member has started Server Guide new member actions" |
| `COMPLETED_HOME_ACTIONS` | `1 << 6` | "Member has completed Server Guide new member actions" |

`flags` itself: "guild member flags represented as a bit set, defaults to 0".

> Source: [Guild Resource — Guild Member Flags](https://docs.discord.com/developers/resources/guild#guild-member-flags) — fetched 2026-08-10

The guild-level configuration object exists too:

> Guild Onboarding: `guild_id`, `prompts`, `default_channel_ids`, `enabled` ("Whether
> onboarding is enabled in the guild"), `mode` (`ONBOARDING_DEFAULT` = 0, "Counts only
> Default Channels towards constraints"; `ONBOARDING_ADVANCED` = 1, "Counts Default Channels
> and Questions towards constraints"). Prompts carry a `required` field and `in_onboarding`
> ("Indicates whether the prompt is present in the onboarding flow").

> Source: [Guild Resource — Guild Onboarding Object](https://docs.discord.com/developers/resources/guild#guild-onboarding-object) — fetched 2026-08-10

Two gaps, both material:

- **Not stated in the source** whether an incomplete onboarding *restricts* the member the
  way `pending` does. The docs describe `COMPLETED_ONBOARDING` as a status bit only; nothing
  says the member is action-restricted while it is unset. The support article's "must answer
  the question to proceed into your server" is the closest statement, and it is about the
  join UI, not enforcement of an already-joined member.
- **Not stated in the source** whether `flags` is populated on the response to
  `GET /users/@me/guilds/{guild.id}/member`. The endpoint is documented only as "Returns a
  guild member object for the current user", with no field-level caveats. `flags` is
  non-optional in the field table (no `?`), which suggests it is always present — but that is
  an inference, not a documented guarantee. **Must be verified empirically before anyone
  designs a rule on it.**

> Source: [User Resource — Get Current User Guild Member](https://docs.discord.com/developers/resources/user) — fetched 2026-08-10

`COMPLETED_ONBOARDING` is a weaker signal than `pending` in any case: it proves the user
clicked through a channel picker, not that they cleared a barrier.

---

## 5. Role gating

Yes, a server can gate everything behind a role, and yes, role state is on the member
object — but it is uninterpretable from outside the server.

Mechanism: guild-level base permissions per role, overridden per channel.

> "Permissions are a way to limit and grant certain abilities to users in Discord. A set of
> base permissions can be configured at the guild level for different roles."

> `VIEW_CHANNEL` (`0x0000000000000400`): "Allows guild members to view a channel, which
> includes reading messages in text channels and joining voice channels"

> `SEND_MESSAGES` (`0x0000000000000800`): "Allows for sending messages in a channel and
> creating threads in a forum"

> "The `@everyone` role has the same ID as the guild it belongs to."

Channel-level overwrites grant or deny per role or per member, overriding guild-level
settings.

> Source: [Permissions — Discord Developer Documentation](https://docs.discord.com/developers/topics/permissions) — fetched 2026-08-10

So the classic gate is: deny `VIEW_CHANNEL` to `@everyone` on all real channels, allow it to
a "verified" role, and have a bot assign that role after some challenge. Discord confirms
this pattern exists and recommends against it:

> "Q: I currently use a role gating through a third-party bot to have members agree to rules
> should, I use this instead? A: We recommend switching to our Rules Screening because in
> order to DM or talk in the server, members must agree to the rules. Existing bot gates do
> not protect against DMs to server members and are confusing for new members."

> "If you have a bot role gate currently enabled, make sure you re-enable perms on @everyone
> when you enact Rules Screening to ensure it works properly."

> Source: [Rules Screening FAQ](https://support.discord.com/hc/en-us/articles/1500000466882-Rules-Screening-FAQ) — fetched 2026-08-10

Observability: the member object carries `roles` — "array of role object ids". That is a
list of **opaque snowflake IDs**. Nothing in the guild member object names the roles or says
what they mean.

> Source: [Guild Resource — Guild Member Object](https://docs.discord.com/developers/resources/guild#guild-member-object) — fetched 2026-08-10

**Reading for us:** to use roles as a barrier signal, we would have to hardcode (as SSM
config, like the guild ID) the specific role snowflake that Stellar's server treats as
"verified", and re-check it whenever they restructure their roles. That is a standing
coupling to another org's server configuration, and it silently degrades to "allow everyone"
if the role is renumbered. Higher maintenance cost than `pending`, with the same
proof-of-nothing property unless the role is behind a real challenge.

Role hierarchy is otherwise a moderation construct, not a verification one:

> "Members can only affect users with roles lower than their highest role."

> Source: [Discord Roles and Permissions](https://support.discord.com/hc/en-us/articles/214836687-Discord-Roles-and-Permissions) — fetched 2026-08-10

---

## 6. What Discord publishes about churning throwaway accounts

### 6.1 Raid Protection — automatic, ML-based, and it does add CAPTCHA

> "Our Raid Protection system provides protection from join-raids by using machine learning
> to evaluate various signals that are likely indicative of an upcoming raid and taking
> automated actions to safeguard your server."

> "When a raid has been detected, we'll automatically take action against suspicious joiners
> by sending you an alert to a dedicated channel of your choice and require CAPTCHA for new
> joiners within the next hour to prevent raiders from joining your server. You can disable
> CAPTCHA at any time."

Availability caveat, as published:

> "We're gradually rolling out Raid Protection alerts to community servers! At this time,
> availability may be limited."

> Source: [How to Protect Your Server from Raids 101](https://support.discord.com/hc/en-us/articles/10989121220631-How-to-Protect-Your-Server-from-Raids-101) — fetched 2026-08-10

**Reading for us:** Raid Protection is burst-shaped. It reacts to "a large number of users or
bots join a server at once". A patient attacker registering ten accounts over ten days is not
a raid and would not trip it. Nothing published claims otherwise.

### 6.2 AutoMod — content moderation, plus a username quarantine

AutoMod is described as message-content filtering, not identity filtering:

> "AutoMod is a system of multiple content filters designed to make content moderation easier
> and less work for moderators."

> "AutoMod prevents unwanted messages from being posted in your Community across all of your
> #text-channels."

The one identity-adjacent capability is username blocking:

> "You can also customize words or phrases you don't want visible in members' usernames or
> server nicknames while in your server. Usernames or members who have server nicknames that
> contain these blocked words will be required to update their server nickname before they can
> talk or interact with other server members."

> Source: [AutoMod FAQ](https://support.discord.com/hc/en-us/articles/4421269296535-AutoMod-FAQ) — fetched 2026-08-10

API-side, AutoMod actions are:

| Action | Value | Description (verbatim) |
|--------|-------|------------------------|
| `BLOCK_MESSAGE` | 1 | blocks a member's message before it is posted |
| `SEND_ALERT_MESSAGE` | 2 | logs user content to a specified channel |
| `TIMEOUT` | 3 | timeout user for a specified duration |
| `BLOCK_MEMBER_INTERACTION` | 4 | prevents a member from using text, voice, or other interactions |

Trigger types: `KEYWORD` (1), `SPAM` (3), `KEYWORD_PRESET` (4), `MENTION_SPAM` (5),
`MEMBER_PROFILE` (6).

> Source: [Auto Moderation — Discord Developer Documentation](https://docs.discord.com/developers/resources/auto-moderation) — fetched 2026-08-10

The quarantine state is reflected on the member object only for the profile-name case, via
`AUTOMOD_QUARANTINED_USERNAME` (`1 << 7`, "Member's username, display name, or nickname is
blocked by AutoMod") and `AUTOMOD_QUARANTINED_GUILD_TAG` (`1 << 10`).

> Source: [Guild Resource — Guild Member Flags](https://docs.discord.com/developers/resources/guild#guild-member-flags) — fetched 2026-08-10

Moderator-facing, quarantine appears as a Members-page signal:

> "Quarantined: Shows members who are currently quarantined by Automod and are not able to
> participate in voice, stage, or text channels."

Alongside two other signals: "Unusual DM Activity: Shows members who may have sent a high
volume of DMs to non-friend server members" and "Unusual Account Activity: Shows members who
are engaged in suspected spam activity."

The same page confirms moderators can *see* account age — but only in the moderator UI:

> "The age of their Discord account" (listed among the fields shown on the Members page)

> Source: [Members Page](https://support.discord.com/hc/en-us/articles/15946797617431-Members-Page) — fetched 2026-08-10

### 6.3 Security Actions — incident response, not admission control

> "Security Actions can be used to secure your server in the event of an incident. This allows
> you to not only pause invites, but also pause DM's between non-friends that are members
> within the server."

> Source: [Activity Alerts + Security Actions](https://support.discord.com/hc/en-us/articles/17439993574167-Activity-Alerts-Security-Actions) — fetched 2026-08-10

### 6.4 Platform-wide anti-spam posture

> "Many spammers are caught after they send only a few messages and, in the most extreme
> cases, we catch spammers before they are able to send a single message."

> "Thanks to community reporting, our ability to identify bad actors has increased by 1000%,
> allowing us to more rapidly discover and remove spammers while also improving our automated
> detection models."

> "We are currently testing a system that monitors servers for inauthentic behavior from new
> members, and proactively puts the server into safe mode, requiring captchas to engage with
> the community for a period of time."

> Source: [How We're Fighting Spammers on Discord](https://discord.com/safety/how-discord-is-fighting-spam) — fetched 2026-08-10

Note the framing: all of it is about **spamming behaviour after account creation**. Nothing
Discord publishes claims that account *creation* is hard, or that alt accounts are detected
at signup. There is no published alt-linking or device-fingerprint claim in any source
fetched here. Do not assume one exists.

Transparency-report figures on accounts disabled for spam were **not verifiable**: the
Transparency Hub landing page serves the data as downloadable PDFs, and no figures were
extractable from the fetched HTML.

> Source: [Transparency Hub | Discord Safety](https://discord.com/safety-transparency) — fetched 2026-08-10

---

## 7. Free signal: account age from the snowflake

Confirming the parent task's claim, since it changes the cost of one mitigation.

> "Discord utilizes Twitter's snowflake format for uniquely identifiable descriptors (IDs)."

Discord Epoch: `1420070400000` (ms), "the first second of 2015".

| Field | Bits | # of Bits | Description | Retrieval |
|-------|------|-----------|-------------|-----------|
| Timestamp | 63 to 22 | 42 | "Milliseconds since Discord Epoch" | `(snowflake >> 22) + 1420070400000` |
| Internal worker ID | 21 to 17 | 5 | | `(snowflake & 0x3E0000) >> 17` |
| Internal process ID | 16 to 12 | 5 | | `(snowflake & 0x1F000) >> 12` |
| Increment | 11 to 0 | 12 | "For every ID generated on that process, this number is incremented" | `snowflake & 0xFFF` |

> Source: [API Reference — Snowflakes](https://docs.discord.com/developers/reference) — fetched 2026-08-10

Confirmed: the user ID we already receive from `identify` encodes account creation time
exactly, with no extra scope and no extra consent. An account-age minimum is free to
implement and free in consent-screen terms. (Whether it is the right mitigation is the
sibling note's call.)

---

## 8. Summary table — what a server can require, and what we can see

| Barrier | Configured by | What it actually proves | Visible on guild member object? |
|---|---|---|---|
| Discord email verification | Discord (optional to user) | Control of an email address | No — `verified` is on the **user** object and requires the `email` scope |
| Discord phone verification | Discord, on suspicion only | Control of a non-VOIP number, 1 per account | No |
| Verification level LOW–VERY_HIGH | Server owner | Email / time / phone — **all bypassed by phone verification** | No |
| Rules Screening (`pending`) | Server owner, Community only | Clicked "I agree" | **Yes — `pending`** (optional field; undocumented when screening is off) |
| Onboarding | Server owner, Community only | Answered a channel-picker question | Partly — `flags` bits `COMPLETED_ONBOARDING` / `STARTED_ONBOARDING` |
| Role gate | Server owner + bot | Whatever the bot asked for | `roles` as opaque snowflakes; meaning must be hardcoded |
| Raid Protection / AutoMod | Discord + server owner | Nothing at admission; reacts to bursts and content | Only `AUTOMOD_QUARANTINED_*` flags |
| Account age | — | Snowflake creation time | Derivable from the user ID under `identify` alone |

> Source: composite of all sources cited above — fetched 2026-08-10. The `verified` row is
> from [User Resource — User Object](https://docs.discord.com/developers/resources/user),
> which gives `verified` = "whether the email on this account has been verified" with
> required OAuth2 scope `email` — fetched 2026-08-10

---

## 9. Open items this note could not close

1. **`pending` for guilds without Membership Screening** — value or absence is not stated in
   the source. Needs an empirical test against a guild with `MEMBER_VERIFICATION_GATE_ENABLED`
   off.
2. **Which optional fields `GET /users/@me/guilds/{guild.id}/member` actually returns** —
   particularly `flags` and `pending`. Docs say only "Returns a guild member object". Needs an
   empirical test.
3. **Whether an incomplete onboarding restricts a member** the way `pending` does — not stated
   in the source.
4. **The MEDIUM 5-minute clock** — API docs and support article disagree on whether it measures
   time since registration or time since email verification.
5. **Transparency-report spam figures** — published only as PDFs; not fetched.
6. Whether the "claim your account" paragraph in Getting Started is current — it still uses
   pre-migration discriminator terminology despite a 2026-08-10 update timestamp.

---

## Archived sources

- `sources/discord-verification-levels.md`
- `sources/discord-rules-screening-faq.md`
- `sources/discord-community-onboarding-faq.md`
- `sources/discord-api-guild-resource-excerpts.md`
