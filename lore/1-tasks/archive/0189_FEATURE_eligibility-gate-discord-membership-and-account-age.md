---
id: "0189"
title: "Eligibility gate — Stellar Discord membership and minimum account age before a key is issued"
type: FEATURE
status: completed
related_adr: ["0010"]
related_tasks: ["0183", "0156", "0159", "0179", "0180", "0186", "0187", "0191", "0193"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, discord, auth, abuse-prevention, spike, slice-6]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../archive/0159_FEATURE_discord-oauth-sign-in.md"
  - "../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/notes/R-discord-member-endpoint-response-shape.md"
history:
  - date: "2026-08-13"
    status: backlog
    who: akot
    note: >
      Sixth slice, the second half of [[0159]], and the new home of [[0180]]
      items 1–5. Those five measurements were a task-shaped blocker in front of
      the entire epic; they are actually the first hour of this one task, and
      nothing before this slice depends on them.
  - date: "2026-08-20"
    status: active
    who: akot
    note: >
      Activated. Branch cut from [[0188]]'s (not yet merged; this slice edits
      the same portal files). Step 0's five measurements remain operator-owned
      prerequisites — the code is written to the documented safe rules and the
      result tables stay empty until they are run.
  - date: "2026-08-21"
    status: completed
    who: akot
    note: >
      Shipped in PR #230 (merged to `develop` as `99bca3a`; reviewed and
      approved by Oskar Karcz). 88 tests for the slice — 78 with the
      implementation and 10 more from the review round — leaving the
      workspace at 558 Rust tests and the portal at 61 frontend tests, 0
      failures. All twelve acceptance criteria are closed in code: the gate
      ships closed with zero Discord and zero control-plane calls, a
      non-member is refused and no key is created, a `429`/`5xx` refuses
      without claiming non-membership, `pending: undefined` fails closed and
      loudly, an account under the threshold is told how long to wait, and
      the `/key` route went fully read-only so a session cookie alone can
      cause no control-plane write at all.

      Step 0's five measurements are **not** done, and none were invented.
      Every prerequisite is operator-owned (the extended Discord app, the
      screening-off scratch guild, the non-member account), so the tables
      keep a dated deferral and the code follows the documented safe rules —
      a measurement changes at most one match arm each (Design Decisions
      #4-#6). Runbook §5's `pending_absent` log check is where item 2 now
      gets measured, from production rather than a scratch guild.

      Twelve review findings across two rounds, all real. The one that most
      deserves carrying: three separate comments asserted that Discord does
      not re-prompt for consent while the authorize URL never sent
      `prompt=none` — and once added, putting it on the *shared* URL
      threatened both first-time sign-in and this task's own scope upgrade,
      so it is issue-only (#19, #23). Second: one `is_snowflake` now guards
      the cold-start probe and the member URL together, because
      `stellar_test` — the value this task's own parameter table named —
      passed the probe and would then have refused every visitor as
      `unknown`, indefinitely.

      Still operator-owned before production: `guilds.members.read` in the
      Developer Portal (runbook §1 step 3), the two `put-parameter` seeds
      (§2a), and [[0179]] step 4 re-pointing the guild id at the real Stellar
      server. Nothing spawned — every follow-up already has an owner
      ([[0179]], [[0180]], [[0191]], [[0192]], [[0193]], [[0194]]).

      One finding raised and deliberately not fixed: S5, the symmetric dead
      end on [[0186]]'s sign-in arm (`refuse_discord` answers a `502` to a
      top-level navigation back from Discord). It is documented, pinned by
      three tests, and changing sign-in's failure semantics inside a PR about
      eligibility is a surprise a reviewer should not have to absorb.
---

# Eligibility gate — membership and account age

## Summary

**Story:** *as the operator, I want a key to be issuable only by a member of the
Stellar Discord whose account is not brand new — so the abuse barrier is real
rather than assumed.*

This is the whole of the epic's abuse story. Until it lands, [[0187]] issues a
key to anyone with a Discord account, which is acceptable on a dev distribution
and must not reach production.

## Step 0 — measure first (was [[0180]] items 1–5)

Five behaviours the design leans on are **undocumented**. Measure them before
writing the branch that depends on them, and write the results into this task
with the date — converting it to a directory with `notes/` if they run long, per
the task-size convention. Prerequisites, all owned by Adam: the Discord app
from [[0186]] extended with `guilds.members.read`, the `stellar_test` guild with
Membership Screening **on** and `verification_level: 2`, **a second scratch guild
with screening off**, and **a second account that is not a member**.

| # | Question | Why it is load-bearing |
| --- | --- | --- |
| 1 | Status code + JSON error code when the user is **not** a member | Fail closed on a `429` and legitimate users are denied; fail open on a `404` and the barrier is void. Highest stakes item in the epic |
| 2 | Is `pending` present on the REST member response? | The docs' presence guarantee is written about gateway events, not this route |
| 3 | Is `flags` populated on that response? | `BYPASSES_VERIFICATION` changes what `pending === false` means |
| 4 | What `pending` means with screening **off** | Needs the second guild — this is the comparison |
| 5 | Consent-screen copy with and without `guilds.members.read` | Friction on the one screen every user sees |

Detail and reasoning: archived
`0180_RESEARCH_.../notes/R-discord-member-endpoint-response-shape.md` and
`notes/G-measurement-runbook.md`. Do not re-derive them.

### Item 2 — measured 2026-08-27: **`pending` IS present**

| # | Result | How | Date |
| --- | --- | --- | --- |
| 2 | **Present.** The REST member response carried `pending: false`. | Local `serve` (`scripts/measure-pending-absent.sh`), guild `1536303837785362432`, account `kotryba`, one full sign-in round-trip | 2026-08-27 |

**The evidence is an absence, so the chain is written out.** The log
(`/tmp/portal-pending-absent-20260827T164107.log`) carries
`portal issued an API key key_id=smdesqkg5j created=false` at 14:42:28 and
**zero WARN or ERROR lines of any kind** — no `pending_absent`, no
"membership could not be verified", no `outcome = "unknown"`. Issuance on the
sign-in path runs only from `issue::after_sign_in`, which `auth/mod.rs` reaches
only after `match membership` falls through on `Membership::Member`, and
`eligibility::membership` returns `Member` only for `pending == Some(false)`.
So the field was present and false.

**What this closes:** risk R1's worst case — "if the field turns out never to be
sent, EVERY member is refused, indefinitely, and it looks exactly like a Discord
outage". Discord does send it on this route. That was the fear behind
[[0193]]'s review blocker (PR #249, karczuRF) and it is disproved.

**What it does not close, and must not be read as closing:**

- **This is one guild, and not the production one.** Production gates on the
  real Stellar Developers guild (`897514728459468821`, [[0179]] step 4). Re-run
  the script with `GUILD=897514728459468821` and an account that is a member.
- **Item 4 is still open and is now the interesting one.** If the guild measured
  here has Membership Screening **off**, then `pending: false` arrives without
  screening at all — which is more than item 2 asked and would settle item 4 in
  the same breath. Adam owns that server; confirming the setting (Server
  Settings → Members) turns one measurement into two. Recorded as unconfirmed
  rather than assumed.
- Items 1, 3 and 5 remain unmeasured.

> **Status 2026-08-20 — items 1–5 deferred to the operator (Adam), tables
> deliberately left empty.** *(Item 2 measured 2026-08-27 — see the table
> above. The rest of this note stands.)* The archived result tables were checked before
> implementation and are **empty placeholders** (`status: seed`, "nothing
> measured yet"); no dated results exist to carry in, and none are invented
> here. Every prerequisite is operator-owned and unmet: the Discord app
> extended with `guilds.members.read`, the screening-off scratch guild, the
> second non-member account, the consent screenshots. The procedure is ready
> to run — `0180/notes/G-measurement-runbook.md` step 3, plus the
> consent-capture reminder now written into the deploy-prep runbook §1
> step 3 — and the code below is written to the documented **safe** rules so
> that either measured outcome changes at most one match arm (see Design
> Decisions #4–#6): only a confirmed `10007`/`10004` 404 is "not a member",
> everything else refuses without accusation, and `pending` absent never
> passes. Results, when measured, go into these tables with their date.

## Implementation

- **Add `guilds.members.read` to the scope** — in the Developer Portal
  registration as well as the authorize URL; scopes "must be declared in the
  Developer Portal". Still never `guilds` (it returns every server the user is
  in, and its partial guild objects carry neither `pending` nor `joined_at`) and
  never `email`.
- **Membership check:** `GET /users/@me/guilds/{guild.id}/member`, guild id from
  SSM.
- **Three outcomes, not two: eligible, ineligible, unknown.** Only an explicit
  `10007`/`10004`-style `404` is ineligible. `401`/`403`/`429`/`5xx` is unknown —
  do not issue, and **do not tell the user they are not a member**. Step 0 item 1
  pins the exact shape.
- **`pending` is optional (`pending?`).** Treat `undefined` as a third state;
  never read absent as "cleared". And `pending === false` can mean an admin waved
  someone through via `BYPASSES_VERIFICATION`.
- **Minimum account age from the snowflake:** `(BigInt(id) >> 22n) +
  1420070400000n`. Costs no extra scope and no extra consent line — `id` is
  already in the `identify` response. Use `BigInt`; `Number` loses precision
  above 2^53. Threshold **5 minutes**, matching Stellar's own
  `verification_level: 2`.
- **Two SSM parameters, operator-seeded, read at runtime:**

  | Parameter | Value |
  | --- | --- |
  | `/prices/{env}/discord-guild-id` | the **snowflake** of the `stellar_test` guild while building, `897514728459468821` after [[0179]] — never the guild's *name*, which the cold-start probe refuses |
  | `/prices/{env}/min-account-age-minutes` | `5` |

  **Do not write `new ssm.StringParameter`.** A CloudFormation-managed parameter
  is CDK-owned, so the next `cdk deploy` silently restores the committed value —
  un-flipping production back to the test guild after [[0179]] step 4. The repo's
  precedent is explicit: `SecretsStack` deliberately does not create the secrets,
  and `compute-stack.ts` *reads* `/prices/{env}/ledger-processor/initial-cursor`
  because the operator seeds it at deploy prep. Read via the SSM SDK, **not**
  `valueForStringParameter` — the latter resolves at deploy and would make the
  threshold un-tunable without a redeploy, defeating the point.
  No IAM work needed: `lambda-baseline.ts` already grants `ssm:GetParameter*` on
  `arn:…:parameter/prices/{env}/*`.
- **Eligibility is proved per action, never carried in the session** (ADR 0010
  §8). A signed "eligible" claim would date the verdict to sign-in time and would
  not survive to a rework weeks later.

  | Path | Re-auth | Checks |
  | --- | --- | --- |
  | Sign in | — | identity only |
  | Issue a key | **yes** | membership (`pending === false`) + account age |
  | Reveal / usage | no | session only |
  | Rework ([[0191]]) | **yes** | membership only — age is never re-checked |

  `state` from [[0186]] carries the intended action, signed; the callback
  exchanges the code for a **fresh** token, checks, and only then hands off to
  the issue path. Discord does not re-prompt for consent on repeat authorisation
  of the same scopes, so the second round-trip is a redirect, not a login.
- **Frontend, plain but specific.** Both refusals are fixable by the user, and
  a generic error makes them abandon:
  - **Not a member** — name the server, link `discord.gg/stellardev` (the
    registered vanity code; the other invites SDF publishes are personal and one
    is already dead, [[0179]]), and let retry re-run the round-trip.
  - **Too young** — this is a *wait*, not a rejection. Render the time
    remaining and allow retry in place. **Not a calendar date** — that pattern is
    right for [[0191]]'s weeks-long cap and absurd for five minutes. Do not
    hard-code "5 minutes"; drive the copy from what the backend returns.
  - **"Could not verify" must render differently from "not a member."** A
    Discord outage is not an accusation the user can act on.
  - **The landing page states both prerequisites before the user authenticates.**
    Learning about the membership requirement afterwards means they authorised an
    app for nothing.

  Styling is [[0193]]'s; the wording is this task's.

## Acceptance Criteria

- [x] **Ships closed**, and this is the slice [[0194]] is waiting on before
      `PORTAL_ENABLED` may be flipped to `'true'` — the gate must pass first.
      Asserted with everything fully wired (mock Discord, mock control plane,
      eligibility settings): login-with-`action=issue` and the callback both
      answer the gate's empty `404`, with **zero** Discord and zero
      control-plane calls made on the way
      (`everything_including_issue_is_an_empty_404_while_the_portal_is_closed`)
- [x] Scope is exactly `identify` + `guilds.members.read`, in the Developer
      Portal as well as the authorize URL — the code half everywhere it lives
      (authorize URL, `discord::SCOPE`, and the granted-scope check upgraded
      to **set equality**, refusing wider *and* narrower grants); the
      registration half is the operator's Developer Portal click, spelled out
      in runbook §1 step 3 and enforced at runtime by exactly those two checks
- [x] A non-member is refused and no key is created — `?issue=not_member`,
      `create_calls == 0`, and the control plane not even listed
      (`a_non_member_is_refused_and_no_key_is_created`)
- [x] A `429` or `5xx` from Discord refuses **without** claiming
      non-membership, and renders as "try again shortly" — `?issue=unknown`
      across 429/500/503/401/403, and the frontend's could-not-verify copy
      explicitly disclaims any statement about membership
- [x] `pending === undefined` is handled explicitly and does not silently
      pass — a dedicated `Eligibility::Unknown` arm with a `pending_absent`
      warn, unit-tested, integration-tested, and marked reversible once 0180
      item 2 is measured
- [x] An account below the threshold is refused with the time remaining —
      `?issue=too_young&wait_secs=N` (ceiling seconds, digits only), rendered
      as a wait with no calendar date and no hard-coded "5 minutes"
- [x] Both SSM parameters are operator-seeded and read at runtime; changing
      `min-account-age-minutes` takes effect **without a redeploy** — sources
      are resolved **per issuance** through the Parameters & Secrets extension
      (its ~5 min cache is the only delay), probed once at cold start so a bad
      seed fails in `Init Errors`; the seeding itself is runbook §2a's
      `put-parameter`, run at deploy prep
- [x] The guild id survives a `cdk deploy` unchanged — nothing creates it
      (no `StringParameter`, no `valueForStringParameter`), and CI check 7
      **refuses** any synthesized template that would (non-vacuity proven by
      injecting one and watching it fail)
- [x] Parameter names and the seeding step are in the deploy-prep runbook,
      alongside the mTLS material (§2a, with the ownership-split table and
      the §5 precondition)
- [x] Issue is unreachable with a session cookie alone — verified by calling
      it directly with nothing else: `GET` **and** `POST /key` with a valid
      session answer `no_key` with zero creates/attaches/deletes (the route is
      read-only by construction), and a callback presented with a session but
      no signed state is a `400` before any Discord call
      (`a_session_cookie_alone_cannot_create_a_key_on_either_verb`,
      `issuing_without_a_session…` and the state-verification suite)
- [x] Reveal and usage still work, with no re-auth, for a user who has left
      the guild — session only, with the mock Discord answering 10007 and the
      test asserting **zero** member calls and zero exchanges
      (`reveal_and_usage_still_work_for_a_user_who_has_left_the_guild`)

## Notes

- The epic's non-goal, worth a code comment: a user who later leaves the server
  keeps their key. Sign-in proves membership at the moment of issuance and
  nothing afterwards.
- Production points at the real Stellar guild only after [[0179]]. Until then
  this gate is real but gated on our own test guild — which [[0164]] is explicit
  is *not* evidence of a functional flow for an outside developer.

## Implementation Notes

Built on [[0188]]'s branch (not yet in develop; this slice edits the same
portal files), in the existing axum router (ADR 0008), mirroring the module
shape of the earlier slices.

**New backend:**

- `portal/eligibility.rs` (~460 lines with docs and unit tests) — the policy
  module [[0191]] will reuse: `ParamSource` (Direct local seam / Ssm runtime
  fetch through the extension), `EligibilitySettings` (per-action resolve,
  trimmed values, typed errors), the pure `decide` with the full verdict
  table, `account_created_ms` (`(id >> 22) + 1_420_070_400_000`, `u64` — the
  task's `BigInt` note is a JavaScript-precision concern this backend proves
  irrelevant with a >2^53 test id), and the per-action table with [[0191]]'s
  and [[0192]]'s exceptions in the module docs.
- `portal/auth/issue.rs` (~330 lines) — `IssueDeps` (gateway + usage-cache +
  settings, wired by `portal::apply`, refused at `/auth/login?action=issue`
  when absent) and `complete_issue`: params → membership (token **borrowed**)
  → identity (token consumed) → session on every outcome → verdict → the key
  reconciler → one of five **verdict** redirects
  (`?issue=ok|not_member|too_young&wait_secs=N|unknown|failed`). The module
  also declares the two **pre-check** landings the callback reaches when the
  round-trip ends at Discord before any check runs — `?issue=cancelled` and
  `?issue=denied` — so every `?issue=` literal is named in one place.

**Changed backend:** `state_token.rs` (`Action::Issue`), `discord.rs`
(`SCOPE` pair, set-equality grant check, `guild_member` + the pure
`classify_member_response`), `auth/mod.rs` (the callback's issue branch after
the exchange; unwired-issue refusal at login), `keys/mod.rs` (**the route is
read-only**: reveal = list→filter→rank→read, `no_key` envelope; the
create-capable reconciler survives as `issue_for`, reachable only from the
callback; `Outcome` no longer carries the key value), `config.rs` +
`main.rs`/`serve.rs` (`load_portal_eligibility` with the cold-start probe),
`portal/mod.rs` (wiring).

**Tests: 78 covering this task** (workspace 497 → 553, frontend 39 → 56).

| where | count | covers |
| --- | --- | --- |
| `eligibility.rs` | 14 | snowflake math (documented example, >2^53 exactness), boundary/ceil, threshold-as-input, the decide table, source resolve/trim/errors |
| `discord.rs` | +5 | scope-pair, set-vs-string, member-URL snowflake guard, the 404-code table, `pending` optionality |
| `state_token.rs` / `auth/mod.rs` / `issue.rs` | +7 | Issue round-trip + mismatch, parse, redirect literals ×5 + digits-only wait |
| `tests/portal_issue.rs` (new) | 22 | ships closed, happy path (+value-free redirect), idempotence, two users, concurrency, the whole verdict table over HTTP, fresh-token+guild recording, session replacement, left-the-guild, unwired/unreadable params, failed≠unknown, and 0187's reconciler suite driven through the callback |
| `tests/portal_keys.rs` (rewritten) | 19 | the read-only invariant (zero writes on either verb, around duplicates, orphans, vanished keys), `no_key` envelope, prefix hazard, no-store, failure shapes, deadline |
| `tests/portal_auth.rs` | 31 (2 new/reshaped) | scope-pair authorize URL, wider/narrower/reordered grants, unwired-issue 503, member-endpoint-never-called-on-signin |
| `app.spec.tsx` | 52 (13 new) | prerequisites-before-auth, link-not-fetch, mount reveal, the five landing states, wait rendering + sanitising, one-shot URL cleanup (issue and signin), gate-404 honesty |
| `verify-openapi-routes.mjs` | check 7 | both parameter names exact on the handler; no template may create either parameter (both halves non-vacuity-tested) |

Verified: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`
(0 warnings in prices-api), `cargo test --workspace` (551 passed, 0 failed),
`cargo check -p prices-api --features lambda` (all seams compiled out), `nx
run-many -t lint typecheck build test`, `nx format:check --all`, `make -C
infra synth-production`, `npm run openapi:lint`, `openapi:verify-routes`
(check 7 live), `openapi:verify-servers`.

## Issues Encountered

- **0180 items 1–5 turned out to be unmeasured.** The task says "write the
  results into this task" as if they existed to carry; the archived note is
  `status: seed` with every table blank and the prerequisites (second guild,
  non-member account, extended app) unstarted. Decision taken with Adam:
  implement to the documented safe rules, leave the tables empty with a dated
  deferral, never fabricate a dated result. The refusal shapes are built so
  measurement changes at most one arm each.
- **A two-scope grant broke the whole-string scope check.** 0186 compared
  `granted_scopes().trim() != SCOPE`; RFC 6749 §3.3 makes scope an unordered
  set, so Discord echoing `guilds.members.read identify` would refuse every
  legitimate grant. Replaced with set equality — still refuses wider AND
  narrower — and pinned with a reordered-grant round-trip test.
- **Dropping `Outcome.value` forced an honest look at step 6.** With the
  issue path answering by redirect, the reconciler's key value had no reader
  left; rather than `#[allow(dead_code)]`, the value was removed from
  `Outcome` so a credential the callback cannot receive is one it cannot leak
  — the `value_of` call survives purely as the readability check.
- **`useOneShotParams` initially re-navigated on every mount and tripped
  React's `act` warning.** A `setSearchParams` whose updater returns the same
  params still navigates; guarded to strip at most once and only when a named
  param is actually present.

**Broken/modified tests** (intentional, per the read-only re-design — none are
regressions): 0187's create-path suite
(`the_first_press_creates…`, `duplicates_converge…`, `two_simultaneous…`,
pagination, adoption/attach/orphan/vanish, `a_usage_plan_that_does_not_exist…`)
moved to `portal_issue.rs` driven through the callback;
`a_key_deleted_by_hand_is_recreated_on_the_next_reveal` **flipped** to
`…answers_no_key_rather_than_recreating`; `a_winner_whose_value_never_reads…`
became a one-pass `no_key` on the reveal and a bounded `?issue=failed` on the
issue side; 0188's `issuing_a_key_evicts_a_cached_no_key` became
`a_reveal_that_finds_a_key_evicts_a_cached_no_key` (the issue-side eviction
is asserted in the issue suite's happy path); `state_token`'s "issue is
unknown" assertions now assert the opposite (with `rework` as the
still-unknown action); the frontend's press-to-issue tests became mount-fetch
and landing-state tests.

- **A code review found seven findings; all seven were real.** Verified each
  against the code at HEAD before acting — the standing rule on this repo
  after [[0185]] — and none was a false positive. Five were fixed on the spot,
  two after checking that no future task owned them (see Design Decisions
  #18–#22). The one that mattered most is the reason this entry exists: three
  separate comments asserted that Discord does not re-prompt for consent, and
  **the authorize URL never sent `prompt=none`**, so the assertion was false
  everywhere it appeared. A comment can be load-bearing documentation and
  still describe code that was never written.
- **The wildcard `apigateway:DELETE` on `/apikeys/*` was deliberately left
  alone.** It is a real weakness — "own" is enforced only in code — but
  [[0194]]'s checklist already names it verbatim, mitigation included. Fixing
  it here would have removed the audit's subject.

## Design Decisions

### From Plan

1. **The callback completes the action** (ADR 0010 §8 verbatim): eligibility
   is checked against the fresh token inside the `action=issue` callback, and
   the key is created right there via `keys::issue_for`. No eligibility fact
   is stored anywhere — not in the session, not in a proof token — so there
   is nothing to expire and nothing to replay.
2. **The `/key` route went fully read-only** — no create, **no attach, no
   delete** — making "issue is unreachable with a session cookie alone"
   structural rather than guarded: a session cookie can cause zero
   control-plane writes, which also retires 0187's `SameSite=Lax` GET-may-
   create argument outright. Costs accepted and documented at the module:
   a hand-deleted key answers `no_key` instead of resurrecting (the heal is
   one gated press), an unattached orphan reveals un-repaired (same heal),
   duplicates wait for the next issue to converge.
3. **Two operator-seeded SSM parameters, resolved per action** through the
   Parameters & Secrets extension (the repo's runtime-read mechanism — the
   task's "SSM SDK" phrasing taken as "at runtime", matching how the plan id
   already works), probed once at cold start so a bad seed is an `Init
   Errors` event, and never CloudFormation resources — CI check 7 enforces
   the never.
4. **Only Discord's own `10007`/`10004` on a 404 reads as "not a member";
   any other 404 is `unknown`.** The allowlist is one `const`; 0180 item 1's
   measurement adjusts it in one place. 10004 additionally warns with the
   guild id, because "Unknown Guild" is likelier to be our mis-seeded
   parameter than the visitor's standing.

### Emerged

5. **`pending: None` → `unknown`, loudly, reversible in one arm.** Absent is
   never "cleared" (the docs' presence guarantee is about gateway events) and
   never an accusation either — if 0180 item 2 shows the REST route simply
   omits the field, a naive not-a-member reading would have refused every
   member in the guild. The `pending_absent` warn makes the gap visible in
   CloudWatch the first time it fires.
6. **`pending: Some(false)` passes even under `BYPASSES_VERIFICATION`** — an
   admin's wave-through is the guild considering them a member, and
   second-guessing it would need the `flags` field this service deliberately
   does not read (ADR 0010: the registry stores no membership data).
7. **Five landing states, not four: `failed` ≠ `unknown`.** "Discord could
   not vouch for you" and "you are fine, our key service was not" point at
   different people; collapsing them renders an AWS incident as a doubt about
   the visitor's membership.
8. **The granted-scope check became set equality** (RFC 6749 §3.3) — order-
   independent, still refusing wider and narrower. The authorize URL keeps
   the literal pair.
9. **`pending: Some(true)` renders under `not_member`** — "join the server
   and complete its screening" is one user action, and a fourth refusal
   state would split it for no one's benefit.
10. **Membership precedence over age**: a non-member is told to join before
    being told to wait; the age check only runs on a cleared member.
11. **The session is issued/refreshed on every outcome past identity**, and a
    fresh re-auth identity replaces any existing session unconditionally —
    the key issued and the session shown can never disagree about who the
    visitor is. Even a refused visitor leaves signed in, because a
    non-member legitimately holds reveal and usage (the non-goal).
12. **The frontend fetches the key on mount** — 0187 decision 14 re-derived
    the way 0188's decision 9 re-derived it for usage: the rule existed
    because the route could create, and it no longer can. The per-load
    control-plane read joins [[0194]]'s costing pass, whose checklist already
    names per-load calls.
13. **Landing params are one-shot** (`useOneShotParams`): read once, stripped
    from the URL with `replace`, so no banner outlives the landing it
    described. Applied to `?signin=…` too, which closes 0186's open item O10
    exactly where 0186 predicted it would be closed.
14. **`wait_secs` travels in the URL, digits by type on the way out and
    sanitised on the way in** — display-only, backend-computed, clamped and
    pattern-checked before rendering; nonsense degrades to "a few minutes".
15. **`Outcome` carries no key value** — the issue path's caller is a
    redirect, and a value it cannot receive is a value that cannot leak into
    a `Location` or a log by later mistake. The reveal reads values through
    its own read-only lookup.
16. **`/auth/login?action=issue` refuses on an unwired deployment** (503,
    `keys_unconfigured` — 0187's code for the same fault) instead of starting
    a round-trip that can only end in `?issue=failed`.
17. **`KeyResponse` dropped `created`** — a read-only reveal never creates,
    so the field could only ever be `false`; one less lie to maintain, and
    the frontend type followed.

18. **The reconcile deadline is derived, not constant.** `RECONCILE_DEADLINE`
    was sized for [[0187]], where the reconciliation *was* the request. The
    issue path puts four network calls in front of it, so the worst case
    reached ~29 s against a 15 s Lambda — an API Gateway `502` instead of
    `?issue=failed`, plus a possible unattached orphan. `ISSUE_BUDGET` is
    measured from **request entry** (the exchange spends the same budget) and
    `RECONCILE_FLOOR` refuses to start work that cannot finish.
19. **`prompt=none` on the authorize URL**, asserted in both places that test
    that URL's shape. Without it every issue, every retry after a refusal and
    every future rework was a full consent screen — which would have made the
    per-action model expensive in exactly the way its own justification
    denied. The one assumption (Discord still shows the screen for a
    first-time authorisation) is written at the call site and is free to
    confirm during Step 0 item 5's capture.
20. **`?issue=cancelled` and `?issue=denied` — and they are not a sixth and
    seventh verdict.** "Five states" was about the outcomes of a *completed*
    check; these happen before one starts, and sign-in has had exactly this
    pair since [[0186]]. Issue had neither, so its callback borrowed
    sign-in's — whose banners render only in the signed-out branch an issue
    round-trip has by definition left, meaning a cancelled press landed on an
    unchanged dashboard in silence. Two, not one, for decision #7's reason:
    "you changed your mind" and "our registration is wrong" belong to
    different people.
21. **One `is_snowflake`, shared by `guild_id()` and `member_url`.** The seed
    was validated in one place and consumed in another, and only the consumer
    checked the shape — so `stellar_test`, the value this task's own parameter
    table named, passed the cold-start probe and then refused **every**
    visitor as `unknown`, indefinitely. Deliberately shape-only: no length
    floor, because a well-formed id for the wrong guild is already caught at
    runtime by `10004` and its warn, and an invented floor could refuse a
    legitimate id. The parameter table above is corrected too — it was the
    trap.
22. **The key fetch happens once per load.** `load` depended on the `onKey`
    prop, which the dashboard passes as an inline arrow, so reporting the key
    re-fired the mount effect: two `GET /key` per load, three off `?issue=ok`.
    Held in a ref, and the count is now the assertion. Separately, `?issue=ok`
    no longer renders beside "you have no API key yet" — `GetApiKeys` is
    eventually consistent, and that window offered to issue a *second* key to
    somebody who had just been given their first.

23. **`prompt=none` is for the re-authorisation round-trips only.** Decision
    #19 put it on the shared `authorize_url`, which also changed **first-time
    sign-in** — the one path that cannot carry the assumption, because it is
    where the first authorisation always happens (the issue link exists only
    inside the signed-in dashboard) and a wrong assumption there means nobody
    signs in at all. Sign-in gains nothing from it either: a first-timer has
    no consent to skip. The same reasoning covers this task's own scope
    change — an account that authorised under [[0186]]'s `identify` alone
    must be re-shown the screen to grant `guilds.members.read`, and
    suppressing it on sign-in is exactly how that grant would come back
    narrower and be refused by `scopes_match` on every attempt. On `issue`
    the assumption stops being load-bearing: by construction the app is
    already authorised with these scopes, which is Discord's documented case.
    [[0191]]'s rework joins by adding one variant to the condition.

24. **Every exit from the issue path is a redirect — the `502` is not
    reachable from it.** A landing the page can render, chosen by fault:
    `UnexpectedScope` is our registration drifting, which is the same fault
    Discord reports as `invalid_scope` and which already lands on
    `?issue=denied`; everything else is Discord not answering, which is
    `?issue=unknown`. `/auth/login?action=issue` on an unwired deployment
    joins them at `?issue=failed` instead of a `503` envelope, and logs the
    deployment fault it previously reported to nobody.

25. **`?issue=failed`'s copy no longer claims a check ran.** It is now
    reachable *before* any check (decision #24's unwired door), so "your
    eligibility checked out" would be a lie on one of its two causes. The
    replacement keeps decision #7's split verbatim — our key service, never a
    doubt about the visitor — and is true of both.

26. **`?issue=ok` reports the key's existence to the dashboard by itself.**
    The backend created the key before it redirected, so the landing is proof
    even while `GetApiKeys` has not caught up. Without it the key section
    said "your key was created" and the usage section directly below said
    "you have no API key yet — issue one above": decision #22's
    contradiction, one section down, offering a second key to somebody who
    had just been given their first.

## Review Findings

**A second round** (PR #230 review by Oskar Karcz: five findings, plus two
risks filed as prose rather than as bugs). **All five findings valid and
fixed; both risks addressed** — one in code, one as an operator procedure.
Five further problems found while re-reading the whole diff, four of them
fixed. The round cleared the security-relevant core on its own reading (scope
set-comparison, `member_url` snowflake validation, `classify_member_response`,
the snowflake epoch math, the `ISSUE_BUDGET` derivation, the read-only
`lookup`/`issue_for` split, the `Arc`-backed `UsageCache` handle, the
`Action::Issue` state round-trip, the feature gating, `useOneShotParams`).

| # | severity | what | resolution |
| --- | --- | --- | --- |
| K1 | Significant | Discord failures on an `action=issue` callback bypassed the `?issue=` design and answered a bare `502` worded "could not complete **sign-in**" — no page, no link back, about an action the visitor did not take. Likeliest trigger on this very PR: `UnexpectedScope`, if the Developer Portal registration still carries `identify` alone | Confirmed. `issue::refuse_issue_discord` + `refuse_issue_start`; see Design Decision #24. Three doors closed: the exchange, the identity read, and `login`'s unwired refusal |
| K2 | Low | `describeWait` **rejected** rather than clamped: `/^\d{1,7}$/` turned any wait past ~16 weeks into "about a few minutes" — and `min-account-age-minutes` is a `put-parameter` applied without a redeploy and validated by nothing at deploy time | Confirmed. Digits-only without a length bound, a second/minute/hour/day ladder, and a clamp at 100 years (`Number` of a long digit string is `Infinity`, which the clamp resolves). Understating a wait is the one direction that cannot be recovered from |
| K3 | Low | `?issue=ok` was guarded only against `view.state === 'none'`, so "Your key is ready." could sit directly above "Could not get your API key" | Confirmed. Guarded by naming the two states the line belongs to (`ok`, `loading`) instead of excluding one |
| K4 | Low | The settling "Check again" did not reset the view, so a retry that still found nothing produced **no visible change at all** | Confirmed. `reload()` shows the loading state first, like `Usage`'s Refresh. `ApiKey` also gained that section's in-flight canceller, so two quick presses cannot land the older answer last |
| K5 | Low | CI check 7(b) inspected `Properties.Name` only when it was a literal string, so a parameter whose name synthesizes to an `Fn::Join`/`Fn::Sub` — the natural spelling of `/prices/${env}/discord-guild-id` — walked straight past the never-CDK-owned guard | Confirmed. `resolveName` resolves `Fn::Sub` (including `${!Escaped}`), `Fn::Join` and `Ref`-family intrinsics, rendering unresolvable pieces as one NUL; a name whose **last segment** is not literal is a failure, not a skip. Suffix matching checks the following character, so `…-backup` is not a false positive — the over-broad-matcher mistake [[0188]]'s check 5b already made once. Exercised against seven synthesized shapes |
| R1 | Risk | `pending: None` → `Unknown` refuses **every** member indefinitely if Discord's REST member object omits the field, and 0180 item 2 is unmeasured | Valid, and the behaviour is kept: fail-closed is the acceptance criterion, and flipping it would be inventing the measurement. What was missing is that nothing detects it — to a visitor it is indistinguishable from a Discord outage. Runbook §5 now carries a `filter-log-events --filter-pattern pending_absent` check for the first live attempt, which **is** 0180 item 2, taken from production instead of a scratch guild |
| R2 | Risk | `prompt=none` on the shared `authorize_url` changed first-time sign-in, not just issuance | Valid, and understated: it also threatens the scope upgrade this task makes. Fixed in code — Design Decision #23 |

Five more found in a second pass over the whole diff, four fixed:

| # | what | resolution |
| --- | --- | --- |
| S1 | **The issue path's usage-cache eviction was asserted nowhere.** `portal_usage.rs`'s comment claimed the happy-path round-trip covered it; it did not, and deleting `invalidate_no_key` from `complete_issue` left the whole workspace green — resurrecting [[0188]]'s R2/C2 for a full TTL, on the one page load that follows an issue | Fixed: `a_successful_issue_evicts_the_cached_no_key`, driven through the real router so the callback and the usage route share one cache. Non-vacuity proven by deleting the call and watching it fail |
| S2 | `?issue=ok` + a listing that has not caught up: the usage section told a visitor who had just been issued a key that they had none and should "issue one above" | Fixed — Design Decision #26 |
| S3 | `?issue=failed`'s copy became false once the unwired door landed there | Fixed — Design Decision #25 |
| S4 | `/auth/login?action=issue` on an unwired deployment reported the deployment fault to **nobody** — a `503` with no log line | Fixed: `tracing::error!` naming which of the three dependencies is missing |
| S5 | **The same dead end exists on the sign-in arm**: `refuse_discord`'s `502` is answered to a top-level navigation back from Discord, leaving a first-time visitor on an API URL holding JSON | **Not fixed, deliberately.** It is [[0186]]'s code, untouched by this slice, and its "502, not 500" reasoning is documented and pinned by three tests; changing sign-in's failure semantics inside a PR about eligibility is a surprise a reviewer should not have to absorb. The fix is symmetric and one line (`?signin=failed`) — raised for a decision rather than taken |

**Tests: +10** (workspace 553 → 558, `app.spec.tsx` 56 → 61). Backend:
`?issue=denied` on a narrow *and* a wider grant, `?issue=unknown` on a failed
exchange and on a failed identity read, `prompt=none` present on issue and
absent on sign-in, the issue-path cache eviction, and the unwired login on
both a missing control plane and missing credentials. Frontend: a long wait,
an absurd wait, `?issue=ok` over a failed reveal, the retry's feedback, and
the usage section after an issue. The mock Discord grew one knob
(`start_full`'s `user_status`) so the identity read can fail on its own —
the only way to reach the callback's last Discord call.

**Verified:** `cargo fmt --all --check`, `cargo clippy -p prices-api
--all-targets` (0 warnings), `cargo test --workspace` (558 passed, 0 failed),
`cargo check -p prices-api --features lambda`, `nx run-many -t lint typecheck
build test` (portal 70 passed), `nx format:check --all`, `make -C infra
synth-production`, `npm run openapi:lint`, `openapi:verify-routes`,
`openapi:verify-servers`.

**Still required before merge, and not a code change:** this branch is cut
from [[0188]]'s at a commit six behind its tip, and two of those six touch
the same files (O1 moves the `1 req/s` literal onto `/config`; O3 rewrites
the `keyOnScreen` effect this slice's C4 also touched). The rebase is the
reviewer's own closing note. Those two lines were deliberately left alone
here so the conflict is not deepened.

## Future Work

Nothing new spawned — every follow-up already has an owner:

- **0180 items 1–5 + the consent screenshots** → the operator procedure
  (Step 0's status note; `G-measurement-runbook.md` step 3; runbook §1
  step 3's capture reminder). If measurement contradicts a safe rule, the
  reversible arms are Design Decisions #4–#6.
- The Developer Portal scope addition and the two `put-parameter` seeds →
  deploy prep (runbook §1 step 3 and §2a); [[0179]] step 4 re-points the
  guild id at the real server.
- Styling and copy layout of the five landing states → [[0193]] (which
  re-decides none of the wording).
- The throttle-classification and envelope-helper extraction notes from
  [[0188]] now have a **third** SDK call-site pattern to cover → still
  [[0192]]'s extraction.
- Rework's membership-only re-proof reuses `eligibility::decide` and the
  `Action` slot → [[0191]]; revoke's deliberate no-proof exception is
  already in the module table → [[0192]].
- The audit of the now-seven IAM statements and the per-load control-plane
  cost of mount-reveal + usage → [[0194]].
