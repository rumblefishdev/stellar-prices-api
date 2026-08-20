---
id: "0189"
title: "Eligibility gate — Stellar Discord membership and minimum account age before a key is issued"
type: FEATURE
status: active
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
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Sixth slice, the second half of [[0159]], and the new home of [[0180]]
      items 1–5. Those five measurements were a task-shaped blocker in front of
      the entire epic; they are actually the first hour of this one task, and
      nothing before this slice depends on them.
  - date: 2026-08-20
    status: active
    who: akot
    note: >
      Activated. Branch cut from [[0188]]'s (not yet merged; this slice edits
      the same portal files). Step 0's five measurements remain operator-owned
      prerequisites — the code is written to the documented safe rules and the
      result tables stay empty until they are run.
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

> **Status 2026-08-20 — items 1–5 deferred to the operator (Adam), tables
> deliberately left empty.** The archived result tables were checked before
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
  | `/prices/{env}/discord-guild-id` | `stellar_test` while building, `897514728459468821` after [[0179]] |
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
- [ ] **(deferred to the operator — the prerequisites are Adam's manual work
      and the archived tables were found empty, see Step 0's status note)**
      Items 1–5 measured against `stellar_test` and a screening-off scratch
      guild, results written down with the date
- [ ] **(deferred with the above; capture reminder written into the runbook's
      scope step, where the browser flow is already open)** Consent screen
      captured with and without `guilds.members.read`
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
  reconciler → one of five literal redirects
  (`?issue=ok|not_member|too_young&wait_secs=N|unknown|failed`).

**Changed backend:** `state_token.rs` (`Action::Issue`), `discord.rs`
(`SCOPE` pair, set-equality grant check, `guild_member` + the pure
`classify_member_response`), `auth/mod.rs` (the callback's issue branch after
the exchange; unwired-issue refusal at login), `keys/mod.rs` (**the route is
read-only**: reveal = list→filter→rank→read, `no_key` envelope; the
create-capable reconciler survives as `issue_for`, reachable only from the
callback; `Outcome` no longer carries the key value), `config.rs` +
`main.rs`/`serve.rs` (`load_portal_eligibility` with the cold-start probe),
`portal/mod.rs` (wiring).

**Tests: 74 covering this task** (workspace 497 → 551, frontend 39 → 52).

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
