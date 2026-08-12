---
url: https://docs.discord.com/developers/resources/guild.md
title: "Guild Resource + API Reference — excerpts (Guild Member object, Membership Screening, Snowflakes)"
fetched_date: 2026-08-10
fetch_method: "curl -L (raw .md source served by docs.discord.com)"
second_url: https://docs.discord.com/developers/reference.md
note: "EXCERPTS, verbatim. Sections copied unmodified from the two raw .md sources named above; nothing else edited."
---

## Excerpt A — Guild Member Object

Source: https://docs.discord.com/developers/resources/guild.md

### Guild Member Object

<ManualAnchor id="guild-member-object-guild-member-structure" />

###### Guild Member Structure

| Field                           | Type                                                                                       | Description                                                                                                                                                                                                                          |
| ------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| user?                           | [user](/developers/resources/user#user-object) object                                      | the user this guild member represents                                                                                                                                                                                                |
| nick?                           | ?string                                                                                    | this user's guild nickname                                                                                                                                                                                                           |
| avatar?                         | ?string                                                                                    | the member's [guild avatar hash](/developers/reference#image-formatting)                                                                                                                                                             |
| banner?                         | ?string                                                                                    | the member's [guild banner hash](/developers/reference#image-formatting)                                                                                                                                                             |
| roles                           | array of snowflakes                                                                        | array of [role](/developers/topics/permissions#role-object) object ids                                                                                                                                                               |
| joined\_at                      | ?ISO8601 timestamp                                                                         | when the user joined the guild                                                                                                                                                                                                       |
| premium\_since?                 | ?ISO8601 timestamp                                                                         | when the user started [boosting](https://support.discord.com/hc/en-us/articles/360028038352-Server-Boosting-) the guild                                                                                                              |
| deaf                            | boolean                                                                                    | whether the user is deafened in voice channels                                                                                                                                                                                       |
| mute                            | boolean                                                                                    | whether the user is muted in voice channels                                                                                                                                                                                          |
| flags                           | integer                                                                                    | [guild member flags](/developers/resources/guild#guild-member-object-guild-member-flags) represented as a bit set, defaults to `0`                                                                                                   |
| pending?                        | boolean                                                                                    | whether the user has not yet passed the guild's [Membership Screening](/developers/resources/guild#membership-screening-object) requirements                                                                                         |
| permissions?                    | string                                                                                     | total permissions of the member in the channel, including overwrites, returned when in the interaction object                                                                                                                        |
| communication\_disabled\_until? | ?ISO8601 timestamp                                                                         | when the user's [timeout](https://support.discord.com/hc/en-us/articles/4413305239191-Time-Out-FAQ) will expire and the user will be able to communicate in the guild again, null or a time in the past if the user is not timed out |
| avatar\_decoration\_data?       | ?[avatar decoration data](/developers/resources/user#avatar-decoration-data-object) object | data for the member's guild avatar decoration                                                                                                                                                                                        |
| collectibles?                   | ?[collectibles](/developers/resources/user#collectibles) object                            | data for the member's collectibles                                                                                                                                                                                                   |

<Info>
  The field `user` won't be included in the member object attached to `MESSAGE_CREATE` and `MESSAGE_UPDATE` gateway events.
</Info>

<Info>
  In `GUILD_` events, `pending` will always be included as true or false. In non `GUILD_` events which can only be triggered by non-`pending` users, `pending` will not be included.
</Info>

<Info>
  Member objects retrieved from `VOICE_STATE_UPDATE` events will have `joined_at` set as `null` if the member was invited as a guest.
</Info>

<ManualAnchor id="guild-member-object-example-guild-member" />

###### Example Guild Member

```json theme={"system"}
{
  "user": {},
  "nick": "NOT API SUPPORT",
  "avatar": null,
  "banner": null,
  "roles": [],
  "joined_at": "2015-04-26T06:26:56.936000+00:00",
  "deaf": false,
  "mute": false
}
```

<ManualAnchor id="guild-member-object-guild-member-flags" />

###### Guild Member Flags

| Flag                               | Value     | Description                                                                  | Editable |
| ---------------------------------- | --------- | ---------------------------------------------------------------------------- | -------- |
| DID\_REJOIN                        | `1 << 0`  | Member has left and rejoined the guild                                       | false    |
| COMPLETED\_ONBOARDING              | `1 << 1`  | Member has completed onboarding                                              | false    |
| BYPASSES\_VERIFICATION             | `1 << 2`  | Member is exempt from guild verification requirements                        | true     |
| STARTED\_ONBOARDING                | `1 << 3`  | Member has started onboarding                                                | false    |
| IS\_GUEST                          | `1 << 4`  | Member is a guest and can only access the voice channel they were invited to | false    |
| STARTED\_HOME\_ACTIONS             | `1 << 5`  | Member has started Server Guide new member actions                           | false    |
| COMPLETED\_HOME\_ACTIONS           | `1 << 6`  | Member has completed Server Guide new member actions                         | false    |
| AUTOMOD\_QUARANTINED\_USERNAME     | `1 << 7`  | Member's username, display name, or nickname is blocked by AutoMod           | false    |
| DM\_SETTINGS\_UPSELL\_ACKNOWLEDGED | `1 << 9`  | Member has dismissed the DM settings upsell                                  | false    |
| AUTOMOD\_QUARANTINED\_GUILD\_TAG   | `1 << 10` | Member's guild tag is blocked by AutoMod                                     | false    |

<Info>
  BYPASSES\_VERIFICATION allows a member who does not meet verification requirements to participate in a server.
</Info>



## Excerpt B — Membership Screening Object

Source: https://docs.discord.com/developers/resources/guild.md

### Membership Screening Object

In guilds with [Membership Screening](https://support.discord.com/hc/en-us/articles/1500000466882) enabled, when a member joins, [Guild Member Add](/developers/events/gateway-events#guild-member-add) will be emitted but they will initially be restricted from doing any actions in the guild, and `pending` will be true in the [member object](/developers/resources/guild#guild-member-object). When the member completes the screening, [Guild Member Update](/developers/events/gateway-events#guild-member-update) will be emitted and `pending` will be false.

<Warning>
  We are making significant changes to the Membership Screening API specifically related to getting and editing the Membership Screening object. Long story short is that it can be improved. As such, we have removed those documentation. There will **not be** any changes to how pending members work, as outlined above. That behavior will stay the same.
</Warning>

### Incidents Data Object

<ManualAnchor id="incidents-data-object-incidents-data-structure" />

###### Incidents Data Structure

| Field                    | Type               | Description                            |
| ------------------------ | ------------------ | -------------------------------------- |
| invites\_disabled\_until | ?ISO8601 timestamp | when invites get enabled again         |
| dms\_disabled\_until     | ?ISO8601 timestamp | when direct messages get enabled again |
| dm\_spam\_detected\_at?  | ?ISO8601 timestamp | when the dm spam was detected          |
| raid\_detected\_at?      | ?ISO8601 timestamp | when the raid was detected             |

<ManualAnchor id="incidents-data-object-example-incidents-data" />

###### Example Incidents Data

```json theme={"system"}
{
  "invites_disabled_until": "2023-09-01T14:48:02.222000+00:00",
  "dms_disabled_until": null
}
```


## Excerpt C — Snowflakes

Source: https://docs.discord.com/developers/reference.md

## Snowflakes

Discord utilizes Twitter's [snowflake](https://github.com/twitter-archive/snowflake/tree/snowflake-2010) format for uniquely identifiable descriptors (IDs). These IDs are guaranteed to be unique across all of Discord, except in some unique scenarios in which child objects share their parent's ID. Because Snowflake IDs are up to 64 bits in size (e.g. a uint64), they are always returned as strings in the HTTP API to prevent integer overflows in some languages. See [Gateway ETF/JSON](/developers/events/gateway#encoding-and-compression) for more information regarding Gateway encoding.

**Snowflake ID Broken Down in Binary**

```
111111111111111111111111111111111111111111 11111 11111 111111111111
64                                         22    17    12          0
```

**Snowflake ID Format Structure (Left to Right)**

| Field               | Bits     | Number of bits | Description                                                                  | Retrieval                           |
| ------------------- | -------- | -------------- | ---------------------------------------------------------------------------- | ----------------------------------- |
| Timestamp           | 63 to 22 | 42 bits        | Milliseconds since Discord Epoch, the first second of 2015 or 1420070400000. | `(snowflake >> 22) + 1420070400000` |
| Internal worker ID  | 21 to 17 | 5 bits         |                                                                              | `(snowflake & 0x3E0000) >> 17`      |
| Internal process ID | 16 to 12 | 5 bits         |                                                                              | `(snowflake & 0x1F000) >> 12`       |
| Increment           | 11 to 0  | 12 bits        | For every ID that is generated on that process, this number is incremented   | `snowflake & 0xFFF`                 |

### Convert Snowflake to DateTime

<Snowflake />

### Snowflake IDs in Pagination

We typically use snowflake IDs in many of our API routes for pagination. The standardized pagination paradigm we utilize is one in which you can specify IDs `before` and `after` in combination with `limit` to retrieve a desired page of results. You will want to refer to the specific endpoint documentation for details.

It is useful to note that snowflake IDs are just numbers with a timestamp, so when dealing with pagination where you want results from the beginning of time (in Discord Epoch, but `0` works here too) or before/after a specific time you can generate a snowflake ID for that time.

**Generating a snowflake ID from a Timestamp Example**

```bash theme={"system"}
(timestamp_ms - DISCORD_EPOCH) << 22
```

