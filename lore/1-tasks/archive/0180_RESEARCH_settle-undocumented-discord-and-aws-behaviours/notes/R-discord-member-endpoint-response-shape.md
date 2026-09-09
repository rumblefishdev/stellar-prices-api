---
title: "Discord — what GET /users/@me/guilds/{id}/member actually returns"
type: research
status: seed
spawned_from: notes/Q-which-undocumented-behaviours-hold.md
spawns: []
tags: [discord, oauth, membership, guild, screening]
links:
  - "../../../archive/0156_RESEARCH_self-service-auth-assumptions/notes/R-discord-oauth-observable-signals.md"
  - "../../../archive/0156_RESEARCH_self-service-auth-assumptions/sources/discord-oauth-guild-member-and-snowflake.md"
history:
  - date: 2026-08-12
    status: seed
    who: akot
    note: "Created empty with the four measurements to fill; results pending the Discord app + guilds"
---

# Discord — what the member endpoint actually returns

Covers items 1–5. Endpoint under test:

```
GET https://discord.com/api/v10/users/@me/guilds/{guild.id}/member
Authorization: Bearer <user access token>   # user token, NOT a bot token
```

Scope required: `guilds.members.read`. The route is called with the *user's own*
consented token — no bot in the guild, no admin rights.

> **Status: nothing measured yet.** Every "Result" below is a placeholder.
> Fill each in with the observed value and the date it was observed.

---

## 1. Status code when the user is not a guild member

**What the docs give us.** Only the success case is documented. Generic
`404 NOT FOUND` plus JSON error codes `10004` ("Unknown guild") and `10007`
("Unknown member") are all that exist.

**Why it matters.** The membership check is a *negative inferred from an
undocumented error shape*. Both failure directions are real:

- fail **closed** on a `429` → legitimate users are denied a key
- fail **open** on a `404` → the check is void and the abuse barrier is gone

**The rule this must confirm.** Treat only an explicit `10007`/`10004`-style 404
as "not a member". Treat `401` / `403` / `429` / `5xx` as **"unknown, do not
deny"** — a third outcome, rendered differently in the portal ([[0162]]).

**How to measure.** Call it with the non-member account's token against
`stellar_test`.

**Result (date: ______):**

| Case | HTTP status | JSON `code` | Body |
|---|---|---|---|
| Member | | | |
| Non-member | | | |
| Bogus guild id | | | |

---

## 2. Is `pending` present on the REST response?

**What the docs give us.** The field is optional (`pending?`). The presence
guarantee — *"In `GUILD_` events, `pending` will always be included"* — is
written about **gateway events**, not this REST route.

**Why it matters.** `pending === undefined` is a third state and must be handled
as one. Never write `if (member.pending)` and read absent as "cleared".

**Result (date: ______):**

- Present on a screening-enabled guild? →
- Present on a screening-disabled guild? →

---

## 3. Is `flags` populated on that response?

**What the docs give us.** Non-optional in the field table, which *suggests*
always present — but that is inference, not a statement.

**Why it matters.** Required before any rule reads `COMPLETED_ONBOARDING` or
`AUTOMOD_QUARANTINED_USERNAME`.

**Result (date: ______):**

- Present? → · Value observed → · Which bits set →

---

## 4. What `pending` is for a guild without screening enabled

**What the docs give us.** Every documented statement about `pending` is scoped
*"In guilds with Membership Screening enabled"*. Nothing covers the gate-off
case.

**Why it matters.** `pending === false` may mean "cleared the gate" **or**
"there was no gate". Only the guild-level `MEMBER_VERIFICATION_GATE_ENABLED`
feature distinguishes them — and that lives on the guild object, which
`guilds.members.read` does **not** return. So if the real Stellar guild ever
turns screening off, a rule keyed on `pending === false` silently changes
meaning.

**How to measure.** Needs a **second** scratch guild with screening **off** —
`stellar_test` alone cannot answer this.

**Result (date: ______):**

| Guild | Screening | `pending` value |
|---|---|---|
| `stellar_test` | on | |
| scratch | off | |

---

## 5. Consent screen with and without `guilds.members.read`

**What the docs give us.** Nothing — Discord documents no per-scope consent
copy anywhere, so the friction cost of asking for this scope is unknown.

**Why it matters.** Cheap to capture while the app exists, and it is the only
evidence for how much the extra scope costs us in drop-off at sign-in.

**Result (date: ______):** screenshots →

- `identify` only → `sources/consent-identify-only.png`
- `identify` + `guilds.members.read` → `sources/consent-with-members-read.png`

---

## Consequences for other tasks

Fill in once measured — this is what actually leaves the note:

- [[0159]] — error handling branch for the membership check
- [[0162]] — "could not verify" must render differently from "not a member"
- ADR 0010 — only if a finding changes its shape
