# 2026-08-10 — Research, verification, decision

## What was done

Five parallel research lines, deliberately non-overlapping so nothing was
researched twice:

| Note | Case |
|---|---|
| `R-discord-platform-verification-mechanics` | What Discord requires of a new account; verification levels; Rules Screening; Onboarding; raid/AutoMod posture |
| `R-discord-oauth-observable-signals` | What our callback can observe per scope; user/member objects; snowflake; rate limits |
| `R-stellar-discord-server-posture` | The Stellar guild as it exists; ownership; the epic's "other services do this" claim |
| `R-apigw-usage-plan-quota-mechanics` | Quota scoping, period reset, key/plan cardinality, control-plane limits |
| `R-abuse-mitigation-options-costed` | Captcha/email/quota/approval costed against fetched pricing |

Then `Q-` (root question), `S-` (synthesis and decision), ADR 0010, and updates
to 0157-0160. 17 sources archived under `sources/`.

## Verification

Every URL cited in the five notes was re-fetched and its quoted text compared
against the original before the synthesis was written. Live endpoints were
re-run independently rather than trusted:

- **Discord invite API** (`stellardev`) — re-run; guild ID, `verification_level: 2`,
  the full 39-entry `features` array, and member counts matched exactly.
- **SCF Dashboard redirect chain** — reproduced with `curl`; the four-hop chain
  and `scope=identify+email+connections+guilds&client_id=917408694822658160`
  matched exactly.
- **AWS docs** — 18 URLs, all quotes verbatim.
- **Discord support articles** — fetched through the same public Help Center
  JSON API the research used (the HTML is Cloudflare-blocked); all quotes
  verbatim.
- **Pricing pages** — all figures verbatim; every cost calculation in the
  mitigation note was recomputed independently and is correct.

Nothing was found to be fabricated. Two things were corrected or added:

- Added the exclusion that 429s with `X-RateLimit-Scope: shared` do not count
  toward Discord's 10,000-invalid-requests ban — it changes how retries should
  be bounded.
- Recorded a **genuine contradiction inside Discord's own documentation**: the
  API reference gives MEDIUM as "registered on Discord for longer than 5
  minutes"; the support article gives Medium as email "verified for longer than
  five minutes". Left unresolved and flagged for measurement rather than
  papered over.

## Decisions taken

- **Membership in the Stellar guild is required** (Adam) → scope
  `identify` + `guilds.members.read`, never `guilds`.
- **Account-age minimum on top** (Adam), threshold in SSM.
- **`stellar_test` guild for build and test** (Adam), production guild
  integration split out as [[0179]].
- **One active key per account confirmed** — and AWS quota accounting makes it
  structurally required, not just tidy.

## Surprises worth remembering

1. **The epic's barrier was invisible to its own design.** The interesting
   finding was not "is there a gate" (there is) but "would we ever see it"
   (no, under `identify`). Splitting those two questions is what made the
   research produce an answer instead of a debate.
2. **`verified` costs the `email` scope.** The cheapest account-quality signal
   Discord exposes is not free, and the epic assumed it was implicit in login.
3. **SDF's own service contradicts the epic's premise** — SCF adds social
   verification and wallet auth on top of Discord, i.e. it does not treat a
   Discord account as sufficient. This was the single most useful finding and it
   came from testing the epic's own supporting claim rather than its main one.
4. **Two "settled" facts in 0157/0158/0160 were never sourced.** Both came from
   plausible community knowledge. Worth remembering that the tasks read as
   confidently for the unsourced claims as for the sourced ones.
5. **The abuse is worth $0.38/key/month.** Costing the threat before costing the
   defences reframed the whole mitigation discussion — several options cost more
   than the damage they prevent.
   *(Measured 2026-08-12 by [[0180]] #9: **$0.55–$0.89** all-in. The $0.38 was
   gateway-only and priced in us-east-1. The reframing this entry describes is
   what mattered and it survives — the conclusion never depended on the second
   decimal place.)*

## Follow-ups spawned

- [[0179]] — contact SDF, agree production guild integration, prove it end to end
- [[0180]] — measure the seven undocumented Discord/AWS behaviours the design
  depends on, and correct the two unsourced claims in 0157/0158/0160
