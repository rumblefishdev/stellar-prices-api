---
title: "What Discord OAuth lets us observe, per scope"
type: research
status: developing
spawned_from: notes/Q-do-the-two-flagged-auth-assumptions-hold.md
spawns:
  - notes/S-account-model-and-abuse-barrier.md
tags: [discord, oauth, scopes, snowflake, auth]
links: []
history:
  - date: 2026-08-10
    status: developing
    who: claude
    note: "Researched Discord OAuth2 scopes, user/member objects and snowflake timestamps"
---

# What Discord OAuth lets us observe, per scope

Scope of this note: **the Discord OAuth2 API surface only** — what our callback can
observe about a user, per scope, and what it demonstrably cannot. The posture of
Stellar's own Discord server, verification levels, screening configuration and
mitigation design are covered elsewhere in this task.

All API facts below come from `docs.discord.com/developers` (the canonical host —
`discord.com/developers/docs/*` 301-redirects there as of 2026-08-10). Every page
cited was fetched; the three most central are archived verbatim under
`sources/discord-oauth-*.md`.

**Headline for the epic:** Discord OAuth authenticates a *Discord account*. Under
`identify` alone we observe **no verification signal of any kind** — not email
verification, not phone, not server membership. The `verified` flag is gated behind
the `email` scope, there is no phone field on the OAuth user object at all, and
guild-level barriers (screening, verification level, roles) are invisible unless we
request a guild scope and call the guild endpoint ourselves.

---

## 1. Authorization code grant — endpoints, `state`, redirect URI, PKCE

The three OAuth2 URLs, verbatim from the docs table:

| URL | Description |
| --- | --- |
| `https://discord.com/oauth2/authorize` | Base authorization URL |
| `https://discord.com/api/oauth2/token` | Token URL |
| `https://discord.com/api/oauth2/token/revoke` | Token Revocation URL |

> "In accordance with the relevant RFCs, the token and token revocation URLs will
> only accept a content type of `application/x-www-form-urlencoded`. JSON content is
> not permitted and will return an error."

Authorization request (docs' own example):

```
https://discord.com/oauth2/authorize?response_type=code&client_id=157730590492196864&scope=identify%20guilds.join&state=15773059ghq9183habn&redirect_uri=https%3A%2F%2Fnicememe.website&prompt=consent&integration_type=0
```

> "`client_id` is your application's `client_id`. `scope` is a list of OAuth2 scopes
> separated by url encoded spaces (`%20`). `redirect_uri` is whatever URL you
> registered when creating your application, url-encoded. `state` is the unique
> string mentioned in State and Security."

Token exchange: POST to the token URL with `grant_type=authorization_code`, `code`,
`redirect_uri`, authenticated as the client.

> "All calls to the OAuth2 endpoints require either HTTP Basic authentication or
> `client_id` and `client_secret` supplied in the form data body."

Response: `access_token`, `token_type: "Bearer"`, `expires_in` (`604800` in the
example — 7 days), `refresh_token`, `scope`.

On `state`:

> "`state` is sent in the authorization request and returned back in the response and
> should be a value that binds the user's request to their authenticated state."

> "While Discord does not require the use of the `state` parameter, we support it and
> highly recommend that you implement it for the security of your own applications
> and data."

**`state` is therefore optional per Discord and mandatory per us** — it is our only
CSRF defence on the callback.

> Source: [OAuth2 — Discord Developer Documentation](https://docs.discord.com/developers/topics/oauth2) — fetched 2026-08-10

**Redirect URI exact-match:** the docs say the `redirect_uri` is "whatever URL you
registered when creating your application" and, for the extended bot flow, "we will
also automatically redirect the user to the first URI in your application's
registered list unless `redirect_uri` is specified" — i.e. registration is required
and there is a registered *list*. An explicit statement that matching is
character-exact (no prefix/subpath matching) is **not stated in the source**. Treat
"must be pre-registered" as documented and "exact match" as an assumption to verify
empirically when the app is registered ([[0159]]).

> Source: [OAuth2 — Discord Developer Documentation](https://docs.discord.com/developers/topics/oauth2) — fetched 2026-08-10

**PKCE:** the OAuth2 topic page does **not** mention PKCE, `code_challenge` or
`code_verifier` anywhere (zero occurrences in the fetched page source). PKCE *is*
documented, but only in the Social SDK mobile guide:

> "**PKCE (Proof Key for Code Exchange)** is **mandatory** for all mobile apps using
> deep links, regardless of whether you're using a public or confidential client"

and the server-side exchange is shown passing `code_verifier`:

> "Your server must include the `code_verifier` parameter when exchanging the
> authorization code for an access token"

So the token endpoint accepts `code_verifier`; PKCE is supported and *required* only
for mobile custom-URL-scheme redirects. For a confidential server-side web client
with an HTTPS redirect — our case — **PKCE is not documented as required**. Whether
Discord *enforces* a `code_challenge` sent on the authorize call is not stated in the
source.

> Source: [Account Linking on Mobile — Discord Social SDK](https://docs.discord.com/developers/discord-social-sdk/development-guides/account-linking-on-mobile) — fetched 2026-08-10

Useful adjunct: `GET /oauth2/@me` ("Get Current Authorization Information") returns
`application`, `scopes` (array of strings — "the scopes the user has authorized the
application for"), `expires`, and `user?` — "the user who has authorized, if the user
has authorized with the `identify` scope". This lets the backend verify what was
actually granted rather than trusting what it asked for.

> Source: [OAuth2 — Discord Developer Documentation](https://docs.discord.com/developers/topics/oauth2) — fetched 2026-08-10

---

## 2. Scopes — the full list, and the four that matter

Discord documents 29 scopes. Preamble, verbatim:

> "These are a list of all the OAuth2 scopes that Discord supports. Some scopes
> require approval from Discord to use. Requesting them from a user without approval
> from Discord may cause errors or undocumented behavior in the OAuth2 flow."

Full list (name — documented description, verbatim):

| Name | Description |
| --- | --- |
| `activities.read` | allows your app to fetch data from a user's "Now Playing/Recently Played" list — not currently available for apps |
| `activities.write` | allows your app to update a user's activity - not currently available for apps (NOT REQUIRED FOR GAMESDK ACTIVITY MANAGER) |
| `applications.builds.read` | allows your app to read build data for a user's applications |
| `applications.builds.upload` | allows your app to upload/update builds for a user's applications - only available to approved partners |
| `applications.commands` | allows your app to add commands to a guild - included by default with the `bot` scope |
| `applications.commands.update` | allows your app to update its commands using a Bearer token - client credentials grant only |
| `applications.commands.permissions.update` | allows your app to update permissions for its commands in a guild a user has permissions to |
| `applications.entitlements` | allows your app to read entitlements for a user's applications |
| `applications.store.update` | allows your app to read and update store data (SKUs, store listings, achievements, etc.) for a user's applications |
| `bot` | for oauth2 bots, this puts the bot in the user's selected guild by default |
| `connections` | allows `/users/@me/connections` to return linked third-party accounts |
| `dm_channels.read` | allows your app to see information about the user's DMs and group DMs - only available to approved partners |
| `email` | enables `/users/@me` to return an `email` |
| `gdm.join` | allows your app to join users to a group dm |
| `guilds` | allows `/users/@me/guilds` to return basic information about all of a user's guilds |
| `guilds.join` | allows `/guilds/{guild.id}/members/{user.id}` to be used for joining users to a guild |
| `guilds.members.read` | allows `/users/@me/guilds/{guild.id}/member` to return a user's member information in a guild |
| `identify` | allows `/users/@me` without `email` |
| `identify.premium` | allows your app to read a user's Nitro subscription type as defined by `premium_type` on the User object - only available to approved partners |
| `messages.read` | for local rpc server api access, this allows you to read messages from all client channels (otherwise restricted to channels/guilds your app creates) |
| `relationships.read` | Allows your app to access a user's Discord Friends list, their pending requests, and blocked users. This scope is part of our Social SDK - submit for access here. Social SDK Terms apply, including Section 5(a)(ii) to the data you obtain |
| `role_connections.write` | allows your app to update a user's connection and metadata for the app |
| `rpc` | for local rpc server access, this allows you to control a user's local Discord client - only available to approved partners |
| `rpc.activities.write` | for local rpc server access, this allows you to update a user's activity - only available to approved partners |
| `rpc.notifications.read` | for local rpc server access, this allows you to receive notifications pushed out to the user - only available to approved partners |
| `rpc.voice.read` | for local rpc server access, this allows you to read a user's voice settings and listen for voice events - only available to approved partners |
| `rpc.voice.write` | for local rpc server access, this allows you to update a user's voice settings - only available to approved partners |
| `voice` | allows your app to connect to voice on user's behalf and see all the voice members - only available to approved partners |
| `webhook.incoming` | this generates a webhook that is returned in the oauth token response for authorization code grants |

> Source: [OAuth2 — Discord Developer Documentation](https://docs.discord.com/developers/topics/oauth2) — fetched 2026-08-10

Note `identify.premium` is "only available to approved partners" — so `premium_type`
(Nitro status, an occasionally-suggested cost-to-abuse signal) is **not available to
us** without Discord approval.

### The four relevant scopes, side by side

| Scope | Doc description (verbatim) | Endpoint unlocked | What we get |
| --- | --- | --- | --- |
| `identify` | "allows `/users/@me` without `email`" | `GET /users/@me` | User object minus `verified`/`email`/`premium_type` |
| `email` | "enables `/users/@me` to return an `email`" | same endpoint, more fields | `email` **and** `verified` |
| `guilds` | "allows `/users/@me/guilds` to return basic information about all of a user's guilds" | `GET /users/@me/guilds` | **Every** guild the user is in |
| `guilds.members.read` | "allows `/users/@me/guilds/{guild.id}/member` to return a user's member information in a guild" | `GET /users/@me/guilds/{guild.id}/member` | Guild member object for **one named guild** |

The scope descriptions themselves settle the `guilds` vs `guilds.members.read`
question the task raised: `guilds` is defined as "all of a user's guilds";
`guilds.members.read` is defined per-guild. **`guilds` is strictly more data than we
need and should not be requested.**

Also relevant: scopes "must be declared in the Developer Portal".

> "Scopes define what your app is allowed to do. They are requested during the OAuth2
> authorization flow and must be declared in the Developer Portal."

> Source: [OAuth2 and Permissions — Discord Developer Documentation](https://docs.discord.com/developers/platform/oauth2-and-permissions) — fetched 2026-08-10

---

## 3. `GET /users/@me` — the User object, field by field

Route: `GET /users/@me`.

> "Returns the [user] object of the requester's account. For OAuth2, this requires
> the `identify` scope, which will return the object *without* an email, and
> optionally the `email` scope, which returns the object *with* an email if the user
> has one."

The documented User Structure — note the docs carry a **"Required OAuth2 Scope"
column**, which answers the question directly:

| Field | Type | Description | Required OAuth2 Scope |
| --- | --- | --- | --- |
| `id` | snowflake | the user's id | `identify` |
| `username` | string | the user's username, not unique across the platform | `identify` |
| `discriminator` | string | the user's Discord-tag | `identify` |
| `global_name` | ?string | the user's display name, if it is set | `identify` |
| `avatar` | ?string | the user's avatar hash | `identify` |
| `bot?` | boolean | whether the user belongs to an OAuth2 application | `identify` |
| `system?` | boolean | whether the user is an Official Discord System user (part of the urgent message system) | `identify` |
| `mfa_enabled?` | boolean | whether the user has two factor enabled on their account | `identify` |
| `banner?` | ?string | the user's banner hash | `identify` |
| `accent_color?` | ?integer | the user's banner color encoded as an integer representation of hexadecimal color code | `identify` |
| `locale?` | string | the user's chosen language option | `identify` |
| `verified?` | boolean | whether the email on this account has been verified | `email` |
| `email?` | ?string | the user's email | `email` |
| `flags?` | integer | the flags on a user's account | `identify` |
| `premium_type?` | integer | the type of Nitro subscription on a user's account | `identify.premium` |
| `public_flags?` | integer | the public flags on a user's account | `identify` |
| `avatar_decoration_data?` | ?avatar decoration data object | data for the user's avatar decoration | `identify` |
| `collectibles?` | ?collectibles object | data for the user's collectibles | `identify` |
| `primary_guild?` | ?user primary guild object | the user's primary guild | `identify` |

> Source: [User Resource — Discord Developer Documentation](https://docs.discord.com/developers/resources/user) — fetched 2026-08-10

**Answering the four specific questions:**

- **`verified` (email verified):** requires the **`email`** scope. The docs list it
  as `verified? | boolean | whether the email on this account has been verified |
  email`. Under `identify` alone we do **not** see it. This is load-bearing: the
  cheapest "is this a real account" signal Discord exposes costs us the `email`
  scope, which also hands us the actual address — a data-minimisation cost and a PII
  storage question ([[0158]]).
- **`email`:** requires the **`email`** scope, per the table and per the Get Current
  User note above ("*without* an email … *with* an email if the user has one").
- **`mfa_enabled`:** available under **`identify`** — "whether the user has two
  factor enabled on their account". It is optional (`?`), so may be absent; the docs
  do not state when it is omitted — **not stated in the source**.
- **`phone`:** there is **no phone field on the OAuth2 User object**. The string
  "phone" does not appear anywhere on the User Resource page (0 occurrences in the
  fetched page). Discord account-level phone verification is therefore **invisible to
  our flow** — we cannot observe it under any scope documented here.

> Source: [User Resource — Discord Developer Documentation](https://docs.discord.com/developers/resources/user) — fetched 2026-08-10

---

## 4. `GET /users/@me/guilds` (scope `guilds`)

Route: `GET /users/@me/guilds`.

> "Returns a list of partial [guild] objects the current user is a member of. For
> OAuth2, requires the `guilds` scope."

> "This endpoint returns 200 guilds by default, which is the maximum number of guilds
> a non-bot user can join. Therefore, pagination is **not needed** for integrations
> that need to get a list of the users' guilds."

Query params: `before` (snowflake), `after` (snowflake), `limit` (integer, 1-200,
default 200), `with_counts` (boolean, default false — "include approximate member and
presence counts in response").

The docs give **no field table** for the partial guild object here, only an example —
so the field set below is what the documented example contains, not a specification:

```json
{
  "id": "80351110224678912",
  "name": "1337 Krew",
  "icon": "8342729096ea3675442027381ff50dfe",
  "banner": "bb42bdc37653b7cf58c4c8cc622e76cb",
  "owner": true,
  "permissions": "36953089",
  "features": ["COMMUNITY", "NEWS", "ANIMATED_ICON", "INVITE_SPLASH", "BANNER", "ROLE_ICONS"],
  "approximate_member_count": 3268,
  "approximate_presence_count": 784
}
```

> Source: [User Resource — Discord Developer Documentation](https://docs.discord.com/developers/resources/user) — fetched 2026-08-10

Two things to note for the scope decision:

1. The partial guild object **has no `pending` and no `joined_at`** — it tells us the
   user is a member, and nothing about whether they cleared screening or when they
   joined. So `guilds` costs strictly more privacy and returns strictly less of the
   signal we actually want.
2. It returns every server the user is in, up to 200 — an unnecessary and
   uncomfortable disclosure for a rate-limit key signup.

**Conclusion: `guilds` is the wrong scope on both privacy and utility grounds.**

---

## 5. `GET /users/@me/guilds/{guild.id}/member` (scope `guilds.members.read`)

Exact route, verbatim from the docs' route badge:

```
GET /users/@me/guilds/{guild.id}/member
```

> "Returns a [guild member] object for the current user. Requires the
> `guilds.members.read` OAuth2 scope."

> Source: [User Resource — Discord Developer Documentation](https://docs.discord.com/developers/resources/user) — fetched 2026-08-10

### Guild Member object — full documented structure

| Field | Type | Description |
| --- | --- | --- |
| `user?` | user object | the user this guild member represents |
| `nick?` | ?string | this user's guild nickname |
| `avatar?` | ?string | the member's guild avatar hash |
| `banner?` | ?string | the member's guild banner hash |
| `roles` | array of snowflakes | array of role object ids |
| `joined_at` | ?ISO8601 timestamp | when the user joined the guild |
| `premium_since?` | ?ISO8601 timestamp | when the user started boosting the guild |
| `deaf` | boolean | whether the user is deafened in voice channels |
| `mute` | boolean | whether the user is muted in voice channels |
| `flags` | integer | guild member flags represented as a bit set, defaults to `0` |
| `pending?` | boolean | whether the user has not yet passed the guild's Membership Screening requirements |
| `permissions?` | string | total permissions of the member in the channel, including overwrites, returned when in the interaction object |
| `communication_disabled_until?` | ?ISO8601 timestamp | when the user's timeout will expire and the user will be able to communicate in the guild again, null or a time in the past if the user is not timed out |
| `avatar_decoration_data?` | ?avatar decoration data object | data for the member's guild avatar decoration |
| `collectibles?` | ?collectibles object | data for the member's collectibles |

Confirmations against the task's three fields:

- **`pending` — present, but optional.** Doc line, verbatim:
  > `pending?` | `boolean` | "whether the user has not yet passed the guild's
  > Membership Screening requirements"

  **Critical caveat, verbatim from the docs' own Info callout:**
  > "In `GUILD_` events, `pending` will always be included as true or false. In non
  > `GUILD_` events which can only be triggered by non-`pending` users, `pending`
  > will not be included."

  That callout is written about **gateway events**, not about this REST route.
  Whether `pending` is always present on the `guilds.members.read` REST response is
  **not stated in the source**. The `?` in the structure table means it is optional
  in general. So `pending === undefined` must be handled as a distinct third state
  in our code — do **not** write `if (member.pending)` and treat absent as "cleared".
  This wants an empirical check once the app is registered.

  Semantics, verbatim from the Membership Screening Object section:
  > "In guilds with Membership Screening enabled, when a member joins, Guild Member
  > Add will be emitted but they will initially be restricted from doing any actions
  > in the guild, and `pending` will be true in the member object. When the member
  > completes the screening, Guild Member Update will be emitted and `pending` will
  > be false."

  So the epic's assumed barrier, *if* the server has screening enabled, is directly
  observable as `pending === false`. If the server has screening **off**, `pending`
  is false (or absent) for everyone and carries no information — it does not
  distinguish a fresh throwaway from an established member.

- **`joined_at` — present and non-optional**, typed `?ISO8601 timestamp` (nullable,
  but always in the object). One documented nulling case:
  > "Member objects retrieved from `VOICE_STATE_UPDATE` events will have `joined_at`
  > set as `null` if the member was invited as a guest."

  Not applicable to this route. This is a usable "time on server" signal.

- **`roles` — present and non-optional**, "array of role object ids". Note it is
  **IDs only**, not role names — so any role-gate rule needs the role snowflake as
  configuration alongside the guild ID.

- **`user` — optional (`user?`).** The docs' only stated omission case is
  > "The field `user` won't be included in the member object attached to
  > `MESSAGE_CREATE` and `MESSAGE_UPDATE` gateway events."

  Not this route. Regardless, we do not need it here — we already have the user ID
  from `/users/@me`. Do not depend on `member.user.id`; use the `identify` response
  as the identity of record.

**Bonus signal in `flags`** — the documented Guild Member Flags bit set includes some
directly relevant bits:

| Flag | Value | Description |
| --- | --- | --- |
| `DID_REJOIN` | `1 << 0` | Member has left and rejoined the guild |
| `COMPLETED_ONBOARDING` | `1 << 1` | Member has completed onboarding |
| `BYPASSES_VERIFICATION` | `1 << 2` | Member is exempt from guild verification requirements |
| `STARTED_ONBOARDING` | `1 << 3` | Member has started onboarding |
| `IS_GUEST` | `1 << 4` | Member is a guest and can only access the voice channel they were invited to |
| `STARTED_HOME_ACTIONS` | `1 << 5` | Member has started Server Guide new member actions |
| `COMPLETED_HOME_ACTIONS` | `1 << 6` | Member has completed Server Guide new member actions |
| `AUTOMOD_QUARANTINED_USERNAME` | `1 << 7` | Member's username, display name, or nickname is blocked by AutoMod |
| `DM_SETTINGS_UPSELL_ACKNOWLEDGED` | `1 << 9` | Member has dismissed the DM settings upsell |
| `AUTOMOD_QUARANTINED_GUILD_TAG` | `1 << 10` | Member's guild tag is blocked by AutoMod |

`COMPLETED_ONBOARDING` (`1 << 1`) is a second, independent observable of "cleared the
server's front door" — worth checking alongside `pending`, since Onboarding and
Membership Screening are different features. `AUTOMOD_QUARANTINED_USERNAME` (`1 << 7`)
is effectively "Discord's AutoMod already thinks this account is suspicious" and is
free to read once we have the member object.

Also documented, with a caution attached:
> "`BYPASSES_VERIFICATION` allows a member who does not meet verification
> requirements to participate in a server."

i.e. an admin can exempt a member — so `pending === false` does not universally imply
"passed screening", it can also mean "was waved through".

> Source: [Guild Resource — Discord Developer Documentation](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

### What happens if the user is NOT a member of that guild

**The docs do not state the response for this case on this route — not stated in the
source.** The `Get Current User Guild Member` section documents only the success
behaviour, with no error table.

What the docs *do* provide is the generic status-code definition and the relevant
JSON error codes, which strongly imply a 404:

> `404 (NOT FOUND)` — "The resource at the location specified doesn't exist."

JSON error codes: `10004` — "Unknown guild"; `10007` — "Unknown member".

> Source: [Opcodes and Status Codes — Discord Developer Documentation](https://docs.discord.com/developers/topics/opcodes-and-status-codes) — fetched 2026-08-10

**Engineering consequence:** the "is this user in the Stellar server?" check is a
*negative* result inferred from an error response whose exact form Discord does not
document. Implement it defensively — treat only an explicit `10007`/`10004`-style 404
as "not a member", treat 401/403/429/5xx as "unknown, do not deny", and **verify the
actual status code empirically before shipping any rule that gates key issuance on
it**. Silently failing closed on a 429 would deny legitimate users; failing open on a
404 would void the check entirely.

Two more consequences worth carrying into the ADR:

1. **The check is a snapshot at issuance.** The member object is read once during the
   OAuth callback. Nothing pushes us an update if the user later leaves or is banned
   — consistent with the epic's conscious non-goal, but the ADR should say so
   explicitly rather than leave it implied.
2. **The guild ID is configuration.** The route is parameterised by `{guild.id}`;
   nothing about it is a constant of the Discord API. Per the task, this belongs in
   SSM with the rest.

---

## 6. Snowflake → account creation timestamp (no extra scope)

> "Discord utilizes Twitter's snowflake format for uniquely identifiable descriptors
> (IDs). These IDs are guaranteed to be unique across all of Discord, except in some
> unique scenarios in which child objects share their parent's ID. Because Snowflake
> IDs are up to 64 bits in size (e.g. a uint64), they are always returned as strings
> in the HTTP API to prevent integer overflows in some languages."

Bit layout, verbatim:

```
111111111111111111111111111111111111111111 11111 11111 111111111111
64                                         22    17    12          0
```

| Field | Bits | Number of bits | Description | Retrieval |
| --- | --- | --- | --- | --- |
| Timestamp | 63 to 22 | 42 bits | Milliseconds since Discord Epoch, the first second of 2015 or 1420070400000. | `(snowflake >> 22) + 1420070400000` |
| Internal worker ID | 21 to 17 | 5 bits | | `(snowflake & 0x3E0000) >> 17` |
| Internal process ID | 16 to 12 | 5 bits | | `(snowflake & 0x1F000) >> 12` |
| Increment | 11 to 0 | 12 bits | For every ID that is generated on that process, this number is incremented | `snowflake & 0xFFF` |

- **Discord epoch constant: `1420070400000`** ms (Unix epoch ms for 2015-01-01
  00:00:00 UTC), stated verbatim in the table above.
- **Formula: `(snowflake >> 22) + 1420070400000`** — yields creation time in Unix ms.
- The docs' own worked example: `175928847299117063` → `2016-04-30 11:18:25.796 UTC`.
- Inverse (for completeness): `(timestamp_ms - DISCORD_EPOCH) << 22`.

> Source: [API Reference — Discord Developer Documentation](https://docs.discord.com/developers/reference) — fetched 2026-08-10

**Scope cost: zero.** `id` is documented as `identify`-scope on the User object
(§3), and the timestamp is a pure arithmetic function of that ID. Deriving account
age therefore requires **no additional scope, no additional API call, and no change
to the consent screen** beyond the `identify` we already need.

Implementation note: the value is a 64-bit integer delivered as a **string** ("they
are always returned as strings in the HTTP API to prevent integer overflows in some
languages"). In JS/TS use `BigInt(id) >> 22n` — `Number` will silently lose precision
above 2^53.

This is the cheapest mitigation lever available and it is available under the scope
set we would request anyway.

---

## 7. Rate limits

Discord's rate-limit model, verbatim:

> "Rate limits exist across Discord's APIs to prevent spam, abuse, and service
> overload. Limits are applied to individual bots and users both on a per-route basis
> and globally. Individuals are determined using a request's authentication—for
> example, a bot token for a bot."

> "**Per-route rate limits** exist for many individual endpoints, and may include the
> HTTP method (`GET`, `POST`, `PUT`, or `DELETE`). In some cases, per-route limits
> will be shared across a set of similar endpoints, indicated in the
> `X-RateLimit-Bucket` header."

Global:

> "All bots can make up to 50 requests per second to our API. **If no authorization
> header is provided, then the limit is applied to the IP address.** This is
> independent of any individual rate limit on a route."

**Numeric per-route limits for `/users/@me`, `/users/@me/guilds` and
`/users/@me/guilds/{guild.id}/member` are not published — not stated in the source.**
The docs explicitly instruct against hardcoding them:

> "Because rate limits depend on a variety of factors and are subject to change,
> **rate limits should not be hard coded into your app**. Instead, your app should
> parse response headers to prevent hitting the limit, and to respond accordingly in
> case you do."

Headers to read: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`,
`X-RateLimit-Reset-After`, `X-RateLimit-Bucket`, plus `X-RateLimit-Global` and
`X-RateLimit-Scope` on 429s. On exceeding, HTTP 429 with `retry_after` (float,
seconds) and a `Retry-After` header.

The one hard number that matters operationally:

> "IP addresses that make too many invalid HTTP requests are automatically and
> temporarily restricted from accessing the Discord API. Currently, this limit is
> **10,000 per 10 minutes**. An invalid request is one that results in **401**,
> **403**, or **429** statuses."

One exclusion, added on verification of this note: 429s carrying
`X-RateLimit-Scope: shared` are **not** counted toward that 10,000. So the ban
risk comes from our own 401/403s — a bad client secret or a replayed code — not
from being rate limited on a shared bucket.

> Source: [Rate Limits — Discord Developer Documentation](https://docs.discord.com/developers/topics/rate-limits) — fetched 2026-08-10

**Is OAuth token exchange rate limited?** The rate-limits page makes no mention of the
token endpoint, and the OAuth2 page mentions rate limits only in passing (bots have
"an entirely separate set of rate limits"; "Be mindful of our Rate Limits!" regarding
webhooks). **Whether `POST /api/oauth2/token` carries its own limit is not stated in
either source.** Practically: our token exchange is authenticated with client
credentials, and the 50 rps global figure is documented for bots, so the applicable
bucket for a confidential OAuth client is undocumented. Since our issuance volume is
human-paced this is not a capacity risk — but the 401-driven Cloudflare ban is a real
availability risk if a bug loops on a rejected exchange. Bound the retry logic.

Relevance to the abuse story: none of these limits constrain an *attacker*
meaningfully — they constrain **us**, per our own client credentials/IP. Discord's
rate limits are not part of the anti-abuse chain for self-issued keys.

---

## 8. The consent screen — what the user is shown

The docs say only this about what the user sees:

> "When someone navigates to this URL, they will be prompted to authorize your
> application for the requested scopes. On acceptance, they will be redirected to
> your `redirect_uri`, which will contain an additional querystring parameter,
> `code`."

And, on re-authorisation:

> "`prompt` controls how the authorization flow handles existing authorizations. If a
> user has previously authorized your application with the requested scopes and
> prompt is set to `consent`, it will request them to reapprove their authorization.
> If set to `none`, it will skip the authorization screen and redirect them back to
> your redirect URI without requesting their authorization. For passthrough scopes,
> like `bot` and `webhook.incoming`, authorization is always required."

> Source: [OAuth2 — Discord Developer Documentation](https://docs.discord.com/developers/topics/oauth2) — fetched 2026-08-10

**Whether adding `guilds.members.read` changes the wording, line count, or appearance
of the authorization prompt is not stated in the source.** Discord documents no
per-scope consent-screen copy anywhere in the developer docs; a targeted search of
`docs.discord.com` for consent-screen text per scope returned nothing authoritative.
The only documented facts are (a) the prompt lists "the requested scopes", and (b)
`prompt=none` can skip it entirely for already-authorised scope sets.

**Inference we are entitled to draw** (and should label as inference in the ADR): since
the screen enumerates requested scopes, adding a scope adds a line. Since
`guilds.members.read` is defined per-guild rather than "all of a user's guilds", its
line should read narrower than `guilds` would — but the exact string is unverified.
The cheap way to settle it: build the authorize URL for our app once it is registered
([[0159]]) and screenshot the screen with and without the scope. That takes minutes
and removes the last unknown in the scope decision.

---

## Bottom line for the ADR

1. **`identify` alone observes nothing about account quality.** Not email
   verification (`email` scope), not phone (no such field, any scope), not server
   membership (needs a guild scope + a call). The epic's stated barrier is invisible
   to a bare `identify` flow. This directly answers the acceptance criterion "does
   our OAuth flow observe that barrier, or only Discord account existence?" — **only
   account existence.**
2. **If a membership check is warranted, `guilds.members.read` is the correct scope**
   and the docs' own scope definitions justify it over `guilds` without further
   argument: `guilds` is defined as "all of a user's guilds", `guilds.members.read`
   as one named guild — and the partial guild objects from `guilds` do not even carry
   `pending` or `joined_at`.
3. **`pending`, `joined_at`, `roles` and `flags` are all real, documented signals** —
   but `pending` is only meaningful if Stellar's server actually has Membership
   Screening enabled (separate line of investigation), `pending` is an optional field
   whose REST-response presence is undocumented, and `BYPASSES_VERIFICATION` means
   `pending === false` is not proof of having passed anything.
4. **Account age is genuinely free** — pure arithmetic on the `identify`-scope `id`,
   epoch `1420070400000`, formula `(snowflake >> 22) + 1420070400000`. No scope, no
   call, no consent-screen change. This is the mitigation with the best
   cost/benefit ratio available.
5. **Discord's rate limits protect Discord, not us.** They do not constrain an
   attacker churning accounts; they constrain our own client, and the
   10,000-invalid-requests-per-10-minutes Cloudflare ban is an availability risk to
   design retries around.

---

## Open items this note could not close from primary sources

| Question | Status |
| --- | --- |
| Is `redirect_uri` matching character-exact? | Not stated in the source — registration required, matching semantics undocumented |
| Does Discord enforce/accept PKCE for confidential web clients? | Documented as mandatory for **mobile deep links** only; web-client behaviour not stated |
| Is `pending` always present on the `guilds.members.read` REST response? | Not stated — docs' presence guarantee is written about gateway events; field is `?` optional |
| Exact status code when the user is not a guild member | Not stated on the route; generic `404 NOT FOUND` + error codes `10004`/`10007` documented |
| Numeric per-route limits on `/users/@me*` | Not published; docs say do not hardcode, parse headers |
| Is `POST /api/oauth2/token` rate limited, and how? | Not stated in either the OAuth2 or Rate Limits page |
| Does adding `guilds.members.read` change the consent screen? | Not stated in any Discord doc; settle empirically after app registration ([[0159]]) |

All seven are cheap to settle empirically once the Discord application exists — which
makes app registration ([[0159]]) a dependency of finalising the scope decision, not
merely a consequence of it.

---

## URLs fetched (all read in full, 2026-08-10)

- `https://docs.discord.com/developers/topics/oauth2` and its raw source `https://docs.discord.com/developers/topics/oauth2.md`
- `https://docs.discord.com/developers/resources/user` and `https://docs.discord.com/developers/resources/user.md`
- `https://docs.discord.com/developers/resources/guild.md`
- `https://docs.discord.com/developers/reference.md`
- `https://docs.discord.com/developers/topics/rate-limits.md`
- `https://docs.discord.com/developers/topics/opcodes-and-status-codes.md`
- `https://docs.discord.com/developers/platform/oauth2-and-permissions.md`
- `https://docs.discord.com/developers/discord-social-sdk/development-guides/account-linking-on-mobile.md`
- `https://docs.discord.com/llms.txt` (documentation index, used to discover the two pages above)

Note on hosts: `discord.com/developers/docs/topics/oauth2` returns `301 Moved
Permanently` to `docs.discord.com/developers/topics/oauth2`. `docs.discord.com` serves
a raw markdown source for every page at `<path>.md`; those were used to avoid
rendering loss in tables. Archived verbatim in `sources/discord-oauth-*.md`.
