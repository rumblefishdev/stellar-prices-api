---
id: "0170"
title: "Contact the Stellar Discord owner (SDF) to agree production guild integration and test it end to end"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0156", "0159", "0160", "0162", "0163", "0164", "0171"]
tags: [layer-docs, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, discord, external-dependency, pre-launch]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
history:
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      Spawned from 0156. ADR 0010 gates key issuance on membership of the
      Stellar guild, but development and testing run against a self-owned
      `stellar_test` guild. This task is the external half: agreeing the
      production integration with SDF and proving it works against the real
      guild.
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      Kept as one task rather than split, by Adam's decision. Tag corrected
      `blocks-launch` → `pre-launch`: the membership check uses the user's own
      OAuth token against a public guild ID, so nothing here is a technical
      blocker on shipping. Added a sequencing section — step 3 (verify against
      the real guild) is under our control and should run early with [[0171]],
      not wait on SDF's reply.
---

# Stellar Discord integration with SDF

## Summary

[ADR 0010](../../2-adrs/0010_discord-account-model-and-abuse-barrier.md) gates
API key issuance on membership of the Stellar Developers Discord
(`897514728459468821`). Build and test run against a `stellar_test` guild Adam
owns, so nothing blocks development.

Small in effort, external in dependency: the long pole is someone else's
response time, which is why it is filed early rather than at cutover.

**Deliberately kept as one task** (Adam, 2026-08-10) rather than split into
"verify against the real guild" and "talk to SDF", even though only the first
is under our control. Read the sequencing section below before assuming the
whole thing waits on a reply.

## Context

0156 established what is publicly observable and, just as importantly, what is
not. The relevant findings:

- The guild is **open-join** — a public one-click vanity invite
  (`discord.gg/stellardev`), plus a Server Discovery listing. 32,419 members as
  of 2026-08-10.
- **Membership Screening is enabled** (`MEMBER_VERIFICATION_GATE_ENABLED` in
  `guild.features`), which is what makes the `pending` field meaningful. This is
  the single fact ADR 0010's barrier rests on.
- `verification_level` is `2` — *"must be registered on Discord for longer than
  5 minutes"*.
- **No named SDF individual is public.** SDF publishes functions (Community
  Manager, DevRel), not a Discord owner. The published routes are
  `communityfund@stellar.org` and `#scf-general` (the SCF handbook says the
  channel is faster).
- **SDF publishes no policy on third parties authenticating against their
  Discord** — no permission process, no rate limits, no stated position. We
  should not assume they have an opinion, nor that they have none.

The dependency is real and asymmetric: if SDF turns screening off, restructures
roles, or migrates the guild, our issuance changes behaviour with no notice to
us. Discord actively markets Onboarding as a *replacement* for verification
friction — its own setup guide step 5 is *"Remove verification steps that
overwhelm or lock new members"* — so drift in that direction is the expected
case, not a tail risk.

## Implementation

**Make contact**

- Open via `#scf-general` in the Stellar Developers Discord (handbook says
  faster), with `communityfund@stellar.org` as the written fallback. Consider
  the weekly Developer and Protocol Meeting as a standing venue.
- Identify a named counterpart and record them here — the whole point is to
  replace "SDF" with a person.

**Establish, then write down**

- Whether SDF is content for an external service to gate on membership of their
  guild at all. Ask plainly; do not infer consent from the absence of a policy.
- **What the Membership Screening form actually contains.** We know the gate is
  *enabled*; we do not know whether it is one checkbox or a real questionnaire.
  This is not observable from outside and it decides how much `pending` is
  worth. One member can read it in one click — do that first, it may make the
  rest of the conversation shorter.
- **Whether they intend to keep screening on, and whether they would tell us if
  it changed. This is the single most important question in this task.**
  ADR 0010 set the account-age threshold to 5 minutes (matching Stellar's own
  `verification_level: 2`), which makes the age check a speed-bump rather than a
  barrier. **Membership screening is therefore the entire abuse barrier.** If
  SDF turns it off — and Discord actively encourages exactly that, its
  Onboarding guide says *"Remove verification steps that overwhelm or lock new
  members"* — our gate silently degrades to "clicked a public invite", and we
  would not find out from an API response. Ask for a heads-up, and record
  whether we got a commitment or a shrug; the honest answer changes what
  mitigation we should pre-plan.
- Whether any role should be required beyond bare membership. Note roles come
  back as **opaque snowflakes**, so any role rule means storing their role ID as
  config and re-checking it whenever they restructure.

**Prove it against the real guild**

- Run the full issuance flow against `897514728459468821` with a real account:
  member and non-member, `pending` true and false.
- Confirm the empirical unknowns from [[0171]] behave on the production guild as
  they did on `stellar_test` — particularly the not-a-member status code, and
  whether `pending`/`flags` are present on the REST response.

**Then flip the config**

- Guild ID is per-environment SSM config per ADR 0010. Production moves from the
  test guild to `897514728459468821` as a config change, not a deploy.

## Sequencing — what actually blocks what

Written down because the four steps above look like a sequence and are not one.
The tag on this task was corrected from `blocks-launch` to `pre-launch` for the
same reason.

**Nothing here blocks building or deploying the membership check.** It runs on
`GET /users/@me/guilds/{guild.id}/member` using the *user's own* OAuth token,
with the user's consent, against a public guild ID. No bot in the guild, no
admin rights, no SDF involvement — technically this ships without SDF ever
hearing about it.

| Step | Under our control? | When |
|---|---|---|
| 3 — verify against the real guild | **Yes, entirely** | **Do this early**, alongside [[0171]]. Needs one real account that is already a member |
| 1–2 — contact SDF, establish their posture | No — their response time | Start early because it is slow, not because it blocks |
| 4 — flip the SSM guild ID | Yes | **Blocks [[0164]] and blocks launch** — see below |

**Step 4 is what [[0164]] waits on.** [[0164]] produces the Tranche 3 evidence,
against a reviewer criterion that reads *"self-service API key request flow
functional"* — and while production points at `stellar_test`, the flow is not
functional for anyone outside this team, because nobody else can join that
guild. So the order at the end of the epic is fixed:

```
… 0163 → 0170 (step 4: FLIP SSM) → 0164 (evidence, real guild)
```

The same fact bounds the launch: **until the flip, no external developer can
obtain a key at all.** The flip precedes any announcement.

**Do step 3 first and do not wait for step 1.** If `pending` or `flags` behave
differently on the real guild than on `stellar_test`, that changes the code —
and it is far cheaper to learn before the build than before the launch. It also
makes the SDF conversation better informed.

**What is genuinely a judgement call, not a technical dependency:** whether we
are willing to go live against SDF's guild without having told them. That is a
relationship decision for the epic owner, not something the API forces. The
honest framing is "we should tell them", not "we cannot proceed".

## Acceptance Criteria

- [ ] A named SDF counterpart is recorded here, or it is recorded that SDF
      declined to name one
- [ ] SDF's position on us gating on guild membership is established in writing
      and linked here
- [ ] Membership Screening form contents established and written down, with the
      date checked
- [ ] Whether screening will stay on — and whether we would be told if it
      changed — established, or explicitly recorded as unknowable
- [ ] Role requirement beyond membership decided (yes with a role ID, or no)
- [ ] Full issuance flow proven against `897514728459468821` for member,
      non-member and `pending` cases
- [ ] Production SSM guild ID switched from the test guild, and the switch
      verified in production — **before** [[0164]]'s evidence run and before
      any public announcement
- [ ] ADR 0010 updated if SDF's answers change the barrier's shape

## Notes

- **Do not hard-code an invite.** Only `stellardev` is the registered vanity
  code; the other three invites SDF publishes are personal invites created by
  individual accounts and revocable by them. One SDF-published invite
  (`hAZTTvtq`, on the Developer and Protocol Meeting page) is **already dead** —
  evidence that these links rot.
- Adjacent guilds that are **not** the target: Stellar Global
  (`761985725453303838`), Stellar Quest / "Lumenauts" (`763798356484161566`),
  and **Stellar Community Fund [Archived]** (`831188872536784947`). The archived
  SCF guild is `verification_level: 4` — SDF knows level 4 exists and chose not
  to use it on the main server. Worth raising.
- Useful leverage in the conversation: SDF's own SCF Dashboard authenticates via
  Discord OAuth (`client_id=917408694822658160`), so the pattern is not foreign
  to them. It requests `identify email connections guilds`; we deliberately
  request less.
