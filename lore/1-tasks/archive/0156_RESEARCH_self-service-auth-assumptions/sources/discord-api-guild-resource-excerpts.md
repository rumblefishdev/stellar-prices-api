---
url: https://docs.discord.com/developers/resources/guild
title: "Guild Resource - Discord Developer Documentation"
fetched_date: 2026-08-10
note: "EXCERPTS ONLY, not the full page (the page is very large). Sections reproduced verbatim, HTML tables flattened to pipe-separated rows."
---

# Guild Resource — excerpts

## Verification Level enum

```
Verification Level

Level | Integer | Description |
NONE | 0 | unrestricted |
LOW | 1 | must have verified email on account |
MEDIUM | 2 | must be registered on Discord for longer than 5 minutes |
HIGH | 3 | must be a member of the server for longer than 10 minutes |
VERY_HIGH | 4 | must have a verified phone number |
```

## Guild Features (selected)

```
COMMUNITY | guild can enable welcome screen, Membership Screening, stage channels and discovery, and receives community updates |
DISCOVERABLE | guild is able to be discovered in the directory |
MEMBER_VERIFICATION_GATE_ENABLED | guild has enabled Membership Screening |
VERIFIED | guild is verified |
WELCOME_SCREEN_ENABLED | guild has enabled the welcome screen |
COMMUNITY | Administrator | Enables Community Features in the guild |
DISCOVERABLE | Administrator* | Enables discovery in the guild, making it publicly listed |
```

## Guild Member Object (field table)

```
Field | Type | Description |
user? | user object | the user this guild member represents |
nick? | ?string | this user’s guild nickname |
avatar? | ?string | the member’s guild avatar hash |
banner? | ?string | the member’s guild banner hash |
roles | array of snowflakes | array of role object ids |
joined_at | ?ISO8601 timestamp | when the user joined the guild |
premium_since? | ?ISO8601 timestamp | when the user started boosting the guild |
deaf | boolean | whether the user is deafened in voice channels |
mute | boolean | whether the user is muted in voice channels |
flags | integer | guild member flags represented as a bit set, defaults to 0 |
pending? | boolean | whether the user has not yet passed the guild’s Membership Screening requirements |
permissions? | string | total permissions of the member in the channel, including overwrites, returned when in the interaction object |
communication_disabled_until? | ?ISO8601 timestamp | when the user’s timeout will expire and the user will be able to communicate in the guild again, null or a time in the past if the user is not timed out |
avatar_decoration_data? | ?avatar decoration data object | data for the member’s guild avatar decoration |
collectibles? | ?collectibles object | data for the member’s collectibles |

The field user won’t be included in the member object attached to MESSAGE_CREATE and MESSAGE_UPDATE gateway events.

In GUILD_ events, pending will always be included as true or false. In non GUILD_ events which can only be triggered by non-pending users, pending will not be included.

```

## Guild Member Object footnotes

```
In GUILD_ events, pending will always be included as true or false. In non GUILD_ events which can only be triggered by non-pending users, pending will not be included.
In guilds with Membership Screening enabled, when a member joins, Guild Member Add will be emitted but they will initially be restricted from doing any actions in the guild, and pending will be true in the member object. When the member completes the screening, Guild Member Update will be emitted and pending will be false.
For guilds with Membership Screening enabled, this endpoint will default to adding new members as pending in the guild member object. Members that are pending will have to complete membership screening before they become full members that can talk.
```

## Guild Member Flags

```
Guild Member Flags

Flag | Value | Description | Editable |
DID_REJOIN | 1 << 0 | Member has left and rejoined the guild | false |
COMPLETED_ONBOARDING | 1 << 1 | Member has completed onboarding | false |
BYPASSES_VERIFICATION | 1 << 2 | Member is exempt from guild verification requirements | true |
STARTED_ONBOARDING | 1 << 3 | Member has started onboarding | false |
IS_GUEST | 1 << 4 | Member is a guest and can only access the voice channel they were invited to | false |
STARTED_HOME_ACTIONS | 1 << 5 | Member has started Server Guide new member actions | false |
COMPLETED_HOME_ACTIONS | 1 << 6 | Member has completed Server Guide new member actions | false |
AUTOMOD_QUARANTINED_USERNAME | 1 << 7 | Member’s username, display name, or nickname is blocked by AutoMod | false |
DM_SETTINGS_UPSELL_ACKNOWLEDGED | 1 << 9 | Member has dismissed the DM settings upsell | false |
AUTOMOD_QUARANTINED_GUILD_TAG | 1 << 10 | Member’s guild tag is blocked by AutoMod | false |
```

## Membership Screening Object

```
In guilds with Membership Screening enabled, when a member joins, Guild Member Add will be emitted but they will initially be restricted from doing any actions in the guild, and pending will be true in the member object. When the member completes the screening, Guild Member Update will be emitted and pending will be false.

We are making significant changes to the Membership Screening API specifically related to getting and editing the Membership Screening object. Long story short is that it can be improved. As such, we have removed those documentation. There will not be any changes to how pending members work, as outlined above. That behavior will stay the same.

​

Incidents Data Object

Incidents Data Structure

Field | Type | Description |
invites_disabled_until | ?ISO8601 timestamp | when invites get enabled again |
```

## Add Guild Member (pending note)

```
For guilds with Membership Screening enabled, this endpoint will default to adding new members as pending in the guild member object. Members that are pending will have to complete membership screening before they become full members that can talk.
```

