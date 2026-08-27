---
id: "0191"
title: "Replace my key — revoke now, re-issue next quota period (merged with 0192)"
type: FEATURE
status: completed
related_adr: ["0010"]
related_tasks: ["0183", "0157", "0160", "0180", "0187", "0189", "0190", "0192", "0193", "0221", "0164"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, security, slice-8, slice-9]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/notes/R-apigw-namequery-quota-and-disable.md"
history:
  - date: "2026-08-13"
    status: backlog
    who: akot
    note: >
      Eighth slice, the last of [[0160]]'s four operations, and the new home of
      [[0180]] item 7 (the quota rollover instant). Placed after the dashboard
      because a user who cannot see their key does not need to replace it.
  - date: "2026-08-21"
    status: active
    who: akot
    note: >
      Activated on `develop` with [[0188]] and [[0189]] both merged (#227,
      #230), so the reconciler, the read-only `/key` route, the usage cache
      and `eligibility::decide` are all in place to build on. Step 0's DAY
      rollover measurement runs alongside the implementation; the MONTH
      confirmation cannot happen before 1 September 2026 and is recorded as
      an open limit, not assumed.
  - date: "2026-08-21"
    status: active
    who: akot
    note: >
      **Model reversed by Adam after seeing it live.** The 2026-08-07 "swap"
      (delete old + issue new in one operation, capped once per period) was
      built and shipped to PR #238, then replaced the same day: "Replace my
      key" is a **revocation** — the key is deactivated immediately, nothing
      is issued, and a new key can be issued only from the next quota period.
      Disable rather than delete, so the disabled key's `lastUpdatedDate` is
      the revocation record the re-issue cap reads (no registry). This
      absorbs [[0192]]; the `action=rework` OAuth round-trip is gone (revoke
      is session-only — a leak must be killable while Discord is down), and
      one IAM grant is added (`apigateway:PATCH` on `/apikeys/*`).
  - date: "2026-08-21"
    status: active
    who: akot
    note: >
      **Merged with [[0192]]** at Adam's request — the reversal above made
      "Replace my key" exactly 0192's rule, so two tasks described one
      feature. 0192 is archived as `superseded`; its rule, the three measured
      properties it was designed around, and its unmet criterion (the ~25 s
      data-plane propagation window must be said out loud) move here. Title
      and tags widened (`slice-9`, `security`); file name kept because the
      branch and PR #238 carry it.
  - date: "2026-08-24"
    status: active
    who: akot
    note: >
      **Step 0's `DAY` proxy abandoned, deliberately, and the scratch stack
      torn down.** Two runs died silently (13 and 21 August; 183 samples, all
      `429`, the second dead ~9 h before the midnight it existed to observe).
      Dropped rather than run a third time because the economics inverted: the
      proxy existed to avoid waiting for 1 September, which is now 8 days out,
      and the real `MONTH` answer comes off production from the `GetUsage`
      reset warn in `keys/gateway.rs`. The criterion it served — stop calling
      the boundary AWS-documented — was already met by the wording work on
      2026-08-21, and nothing in the build depends on the instant. Scratch
      key/plan/API deleted 07:45Z (the script's own `teardown` reported
      success while leaving the plan alive; defect recorded in the script).
      Dead poll logs moved to `.trash/`. One AC left: the 1 September
      confirmation.
  - date: "2026-08-25"
    status: completed
    who: claude
    note: >
      **Completed.** Shipped as PR #238 (approved, all three CI checks green
      on the merge commit): "Replace my key" deactivates every key under the
      caller's name with one `UpdateApiKey(enabled=false)` and issues nothing;
      the reveal answers `key_revoked` with the date; the re-issue is an
      ordinary round-trip that `cap::decide` refuses until the 1st of the next
      month, 00:00 UTC — our rule, one definition in `portal/period.rs`, read
      by four call sites. Absorbed [[0192]]. Built twice: the 2026-08-07 swap,
      reversed by Adam the day it went live, then the revoke — both on the
      record. Four review rounds (audit, the audit's own review, karczuRF's
      review of #238, one self-found finding) closed 40 numbered decisions.
      Tests: 690 Rust across the workspace, portal 95, 0 failed; `fmt`,
      `clippy -p prices-api` (0 warnings), `--features lambda`, `nx format:check`,
      `synth-production` and the three `openapi:*` checks green. One IAM grant
      added (`apigateway:PATCH` on `/apikeys/*`, tag-scoped in its own sid).
      One acceptance criterion deferred, not dropped: the `MONTH` rollover
      confirmation needs 1 September 2026, spawned as [[0221]].
  - date: "2026-08-27"
    status: completed
    who: akot
    note: >
      Amendment, written from [[0193]]: the phrase that arms the modal is
      `regenerate-key`, not `delete-key` — decision 41 below. Status
      unchanged; nothing else in this task is reopened. ADR 0010 §8 and
      [[0164]]'s checklist re-pointed in the same change.
---

# Rework — a new key, once a period

## Summary

**Story:** *as a developer whose key has leaked, I can kill it immediately —
knowing that I will not have a working key again until the next quota period.*

> **Reversed 2026-08-21.** The paragraph below is the 2026-08-07 design; it was
> implemented, then reversed by Adam on seeing it live. **Current rule:**
> "Replace my key" **deactivates** the current key now (`UpdateApiKey
> enabled=false`) and issues nothing; "Get my API key" is refused until the
> 1st of the next month (00:00 UTC, our period rule), naming the date. The
> disabled key is the record of the revocation. See Implementation Notes.

~~An atomic swap: the old key is deleted and a new one issued in the same
operation, so the user is never without a working key. The cap blocks the *next*
rework, not the replacement.~~

## The revoke rule (from [[0192]])

**Revoking does not reset, consume or bypass the cap. It kills a key and
issues nothing.** A user who was issued a key on 3 August and revokes on the
4th is keyless until 1 September. Settled by Adam on 2026-08-13; not a default
to re-derive.

- The cap exists so a burnt quota cannot be escaped by minting a fresh
  `apiKeyId` with a clean counter (quota is per `(usagePlanId, apiKeyId)`). If
  revoke handed out a replacement, "revoke" would be the button pressed on the
  20th of a heavy month.
- Being keyless is the correct cost: the same as not using the leaked key,
  minus the risk of someone else using it. So the confirmation must say it is
  destructive to the user's own access, with the actual date.

Three properties measured under [[0180]] item 8 (2026-08-12) that the build
rests on:

| Measured | Consequence |
| --- | --- |
| `UpdateApiKey(enabled=false)` **preserves** the usage counter (drained → disabled → re-enabled → still `429`) | revocation is not itself a quota reset, so it needed no cross-key tracking |
| A disabled key is `403`, **byte-identical to no key** | the portal cannot infer a revocation from the gateway; the disabled key's own record (`lastUpdatedDate`) is what the reveal and the cap read |
| Disable/enable take **~25 s** to reach the data plane | a `200` from the revoke reports the control plane, not reality — the window must be stated, not hidden |

What 0192 planned and this merge did differently: it specified `DeleteApiKey`
plus a persisted revocation record in ClickHouse (append-only table, a write
grant from BE, a writer mTLS bundle on the Lambda). **Disabling instead of
deleting makes the key its own record**, so none of that storage work exists.
The ClickHouse sketch stays in the archived 0192 file for the day a record is
needed that a key cannot carry.

## Step 0 — measure the rollover (was [[0180]] item 7)

**The rule we render is ours, not AWS's.** AWS documents neither the quota reset
instant nor its timezone; the only statement anywhere is an example caption,
*"creates a usage plan that resets at the beginning of the month"*, and `offset`
is a **request count**, not a time shift. This is ADR 0010's correction #2 and it
is still open.

A `DAY`-period scratch plan was the proxy for the instant and the timezone:
drain the key, poll across a UTC midnight, record when `429` becomes `200`.
**Abandoned 2026-08-24 and the scratch stack torn down** — see the status block
below. `item7-quota-rollover.sh` stays in the archived
`0180_RESEARCH_.../measurement/` with its defects recorded at the top of the
file.

**The honest limit that decided it: this could never be fully settled before
1 September 2026**, the next real `MONTH` rollover — the `DAY` proxy was always
evidence, not proof. It was *enough* only because the criterion is to stop
presenting "00:00 UTC on the 1st" as AWS-documented, not to prove AWS's
implementation; and that criterion is met by the wording alone, which is why
losing the proxy costs nothing. The `MONTH` confirmation on 1 September stands.

**If anything here is ever re-run:** `UpdateUsagePlan` is throttled to **1
request per 20 seconds per account, non-adjustable**, and the whole control
plane shares a **10 rps / burst 40** budget with our deploys. A careless loop
slows CI for everyone.

> **Status 2026-08-24 — the `DAY` proxy is abandoned, deliberately.** Two runs
> were attempted and both died silently: 2026-08-13 (three samples, dead two
> minutes in, and the archived note kept saying "running" for a week) and
> 2026-08-21 (183 samples, 11:51Z → 15:03Z, dead ~9 h before the UTC midnight it
> existed to observe). Nothing was ever measured; nothing is invented below.
>
> Dropped rather than attempted a third time because **the economics inverted**.
> The proxy existed to avoid waiting for 1 September — 19 days away when it was
> designed, 8 days away now. A third run needs the script's defects fixed *and*
> credentials that outlive a 26 h window, to produce what this task already
> calls "evidence, not proof".
>
> **What replaces it:** the real `MONTH` rollover on 1 September, read off
> production by the warn `keys/gateway.rs` already emits when `GetUsage` reports
> a reset inside the queried period. Real data, no scratch stack, no credentials
> marathon. That is the last open AC.
>
> Nothing in the build waits on the answer — the cap is **our** rule, one
> definition in `portal/period.rs`, and a different AWS instant changes the
> dashboard label, not the cap. The wording half of the criterion (stop calling
> the boundary AWS-documented) was done 2026-08-21 and stands.
>
> **Teardown, 2026-08-24 07:45Z:** key `2ke0ixjy7h`, usage plan `ox7pv0` and
> REST API `9utcrbmoc6` deleted; the account greps clean of `lore0180*` and
> `pricing-api-free-production` (`71t9im`) is untouched. Noted because the
> script's `teardown` **reported success while leaving the plan alive** — it
> deletes the plan before the REST API that still references its stage, and
> every delete is `|| true`. The plan was removed by hand once the API was gone.

| Question | Result | Date |
| --- | --- | --- |
| `DAY` reset instant and timezone | **not measured — abandoned 2026-08-24** after two silently-dead runs; superseded by the 1 September `MONTH` observation on production | 2026-08-24 |
| Calendar-aligned or creation-anchored? | **not measured** — dropped with the proxy; the question stays open and costs nothing, since the cap is ours | 2026-08-24 |
| `GetUsage` agreeing with enforcement at the boundary? | **not measured** — dropped with the proxy | 2026-08-24 |
| Inference for `MONTH` | **cannot be settled before 1 September 2026** — recorded as an open limit, not assumed. Whatever `DAY` shows is evidence, not proof | — |
| Text restated as our rule? | **done** — [[0157]] (dated correction), the epic doc, `manual-api-key-tier.md`, `portal/period.rs`, the usage panel's copy, the rework copy | 2026-08-21 |

## Implementation

> **As built (2026-08-21, after the reversal):**
>
> - `POST /api-tokens/api/key/rework` — session-authorized **revoke**:
>   `UpdateApiKey(enabled=false)` on every key under the caller's name; answers
>   `{revoked, next_eligible_at, revoked_at}`; idempotent; `404 no_key` with
>   nothing to revoke. `POST`-only + `SameSite=Lax` is the CSRF guard.
> - `GET /key` on a revoked key → `404 key_revoked` + `details.next_eligible_at`
>   — the value is never revealed again, and the page shows the date instead
>   of the issue link.
> - The **issue** path enforces the cap: all keys under the name disabled →
>   `cap::decide(latest lastUpdatedDate, Period::now())`; capped →
>   `?issue=capped&next_eligible_at=YYYY-MM-DD`, nothing written; allowed →
>   the disabled records are deleted and the ordinary create/attach runs.
> - Frontend: the dialog says the key is deactivated immediately AND that no
>   new key is issued until the next period; `delete-key` arms; one `POST`,
>   disabled on submit; the revoked state renders the date.
> - IAM: `apigateway:PATCH` on `/apikeys/*` (the seventh grant). No
>   `UpdateUsagePlan`, no `UpdateUsage`.
>
> The bullets below are the superseded 2026-08-07 spec, kept for the record.

- ~~`POST /api-tokens/api/key/rework`. Delete the old key, `CreateApiKey` +
  `CreateUsagePlanKey` for the new one, return the new value.~~
- **The cap:** allowed only when the current key's creation instant falls
  **before** the current quota period start (1st of the month, 00:00 UTC). Since
  a rework deletes and re-creates, the surviving key's `createdDate` is that
  instant — no stored timestamp is needed unless [[0190]] is built, in which case
  `coalesce(last_rotated_at, created_at)` is the same value from the table.
  Worked example from the 2026-08-07 meeting: reworked on 3 August → next rework
  available 1 September.
- **Why the fallback to creation time is not defensive.** Gating on a rotation
  timestamp alone leaves it null for every fresh key: a user could take a key on
  1 August, spend the whole quota, and rework on 2 August into a clean counter,
  because quota is scoped to `(usagePlanId, apiKeyId)`. Any key acquired inside
  the current period was created inside it, so it can never be reworked inside
  it. One key per period, one quota.
- **Refusal is `409`, not `429`** — `429` implies "retry shortly" when the wait
  can be weeks. Body is the existing `ErrorEnvelope`
  (`packages/prices-api/src/common/errors.rs`) with a new canonical code and
  `next_eligible_at` in `details`; the envelope already has the slot.
- **Requires a fresh eligibility proof** — membership only, never the account age
  ([[0189]] table). An account old enough once is old enough forever.
- **Frontend:** the action opens a modal stating plainly that the current key is
  deleted and stops working immediately, so anything using it breaks on confirm.
  Confirm stays **disabled until the user types `delete-key`**, and is disabled
  again on submit so a double-click cannot fire two reworks. The refusal path
  renders `next_eligible_at` — for a key reworked on 3 August, "1 September" —
  not a generic error.
- Unstyled, like every screen before [[0193]]. The modal's *wording* is this
  task's and does not get re-decided later.

## Acceptance Criteria

*(Rewritten 2026-08-21 for the revoke-now / re-issue-next-period model. The
original swap-model criteria were all green at PR #238's first commit; they
are replaced, not re-counted.)*

- [x] **Ships closed.** With `PORTAL_ENABLED=false` ([[0183]]) the revoke is an
      empty `404` with a valid session and zero control-plane calls
      (`revoke_is_an_empty_404_while_the_portal_is_closed`)
- [x] **The boundary is no longer presented as AWS-documented** — [[0157]],
      the epic, `manual-api-key-tier.md`, `portal/period.rs` and the portal
      copy all state it as our rule (2026-08-21). The `DAY`-proxy measurement
      that would have backed it with evidence is **deliberately abandoned
      2026-08-24** — two runs died silently and the real `MONTH` answer is 8
      days away on production — with the scratch stack torn down. Reasoning,
      teardown and replacement in Step 0
- [x] "Replace my key" disables the key in one `UpdateApiKey` and issues
      nothing — no Discord call, nothing created
      (`revoke_disables_the_key_in_one_call_and_issues_nothing`); every key
      under the name, not only the current one; idempotent; a failed disable
      is a `502`, never a false "revoked"
- [x] The revoked value is never revealed again; the reveal answers
      `key_revoked` with the date
      (`a_revoked_key_is_never_revealed_again_and_the_reveal_names_the_date`).
      The data-plane `403` is by construction (disabled key = `403`, measured
      under 0180 item 8) and the live `curl` in `README.md` §3c
- [x] A new key cannot be issued in the period of the revocation: the issue
      round-trip passes eligibility and still lands on
      `?issue=capped&next_eligible_at=…` with nothing written
      (`an_issue_after_a_revoke_in_the_same_period_is_capped_with_the_date`)
- [x] Revoked on the 3rd → refused until the 1st, issued once it has passed,
      old record deleted, new key created and attached — tested with literal
      dates in `keys/cap.rs` and relative to the real calendar over HTTP
      (`revoked_on_the_3rd_refuses_until_the_1st_and_issues_once_it_has_passed`,
      `the_full_cycle_issue_revoke_wait_reissue`)
- [x] A session cookie can revoke **its own** key and nothing else — `POST`
      only (`GET` is `405`), unauthenticated is `401` before AWS, another
      user's key and a console lookalike are untouched
      (`a_session_can_only_revoke_its_own_key`)
- [x] Membership is still re-proved on the re-issue (it is an issue), and a
      non-member is told to rejoin before being told to wait
      (`membership_is_still_checked_before_the_cap`)
- [x] The modal states the key is deactivated immediately and that no new key
      is issued until the next period; confirm is gated on typing
      `delete-key` and cannot double-fire; the revoked state renders the date
      (12 frontend tests)
- [x] **(from 0192)** The revoke response and the dialog do not claim
      immediacy the data plane does not have: the dialog says the key stops
      working "within about half a minute" and to treat it as live until
      then; the revoked state renders the API's `revoked_at` as a UTC
      instant ("21 August 2026, 12:00 UTC") and repeats the window anchored
      on it; the word "immediately" is asserted absent. The button reads
      "Deactivate my key". Tests: the dialog wording spec and
      `revokes with one POST … renders the revoked state with the date`;
      the backend test is renamed
      `revoke_disables_the_key_in_one_call_and_issues_nothing` so its name
      does not claim what the mock cannot show
- [x] **(from 0192)** Revocation works while Discord is unreachable — the
      route is session-only and the revoke test makes zero Discord calls
- [x] **(from 0192)** Revocation does not reset the quota counter — measured
      under 0180 item 8, not assumed; the re-issue is a new key and is what
      the cap governs
- [x] **(from 0192)** The choice of `UpdateApiKey(enabled=false)` over
      `DeleteApiKey` is recorded with its reasoning (decision #17; the
      reverse of 0192's draft, for the reason given there)
- [ ] **(deferred to [[0221]])** `MONTH` confirmation on 1 September 2026 —
      dated note, not performed, because the next real rollover is after this
      task closes: on/after 2026-09-01 look for `summarize_days`' `quota reset
      inside the queried period` warn in the api-handler log, or re-run the
      `DAY` proxy script against a `MONTH` scratch plan drained on 31 August.
      Nothing in the build waits on it — the cap is our rule, one definition in
      `portal/period.rs`, and a different AWS instant changes the dashboard
      label, not the cap

## Notes

- A user who reworks on the last day of a period gets a fresh counter and a
  period reset a day later. Not an exploit — the reset was coming anyway — but
  written down so it is not re-raised as one.
- If the measured AWS rollover instant differs from ours, the dashboard renders
  our date and the counter does its own thing. A UX wrinkle, not a correctness
  bug: the cap is ours to define.
- ~~The rework cap is why a leaked key cannot be invalidated until the 1st. That
  gap is [[0192]], and it is no longer blocked.~~ Closed by the reversal: the
  action *is* the revocation, and [[0192]] is merged into this task.

## Implementation Notes

Built on `develop` with [[0188]] and [[0189]] merged, in the existing axum
router (ADR 0008), mirroring [[0189]]'s shape: the round-trip is the proof,
the callback completes the action, every outcome is a redirect to a literal.

**New backend:**

- `portal/period.rs` (~150 lines) — **the** quota period: calendar month,
  UTC. Extracted from `usage/mod.rs` (its `current_period` and the four tests
  moved here unchanged in meaning) so the dashboard's label and the rework
  cap cannot disagree. Adds `start_secs()` (what a `createdDate` is compared
  against), `next_start_ymd()` and `resets_at()` — the same instant.
- `portal/keys/cap.rs` (~190 lines) — the pure cap: `decide(created_at,
  &Period) -> Allowed | Capped { next_eligible_at (RFC 3339),
  next_eligible_date (YYYY-MM-DD) }`. Strictly *before* the period start;
  an undated key is capped, not waved through. The 3 August → 1 September
  example is a test with those literal dates.
- `portal/auth/rework.rs` (~280 lines) — `complete_rework`: guild id → member
  (token borrowed) → identity (token consumed) → session → **membership
  only** (`eligibility::membership`) → budget → `keys::rework_for` → one of
  eight `?rework=` landings, declared in one place.
- `keys/mod.rs`: `POST /key/rework` (the read-only pre-check: `200
  {eligible:true}` / `409 rework_capped` + `details.next_eligible_at` / `404
  no_key`), `rework_for` + `swap` (list → cap → create → attach → delete every
  old key → read back; a failed delete rolls the replacement back),
  `ReworkOutcome`.

**Changed backend:** `state_token.rs` (`Action::Rework`; `revoke` is now the
"arrives early" example), `eligibility.rs` (`membership()` extracted from
`decide`, which now calls it — one `pending` table for both paths, with a
test that they agree on every shape), `auth/mod.rs` (login refusal for both
re-auth actions, `prompt=none` on both, the cancelled/denied/exchange/dispatch
arms), `issue.rs` (`ISSUE_BUDGET`/`RECONCILE_FLOOR` shared; `IssueDeps` docs),
`usage/mod.rs` (`UsageCache::invalidate` — evict *everything* for the caller,
because after a rework the cached numbers describe a key that no longer
exists), `portal/mod.rs` (module).

**Frontend:** `api/portal.ts` (`reworkUrl`, `checkRework`, `navigateTo`
seam), `app.tsx` (`ReplaceKey` dialog, eight `?rework=` landings,
`describeNextEligible`, the settling rule extended to `rework=ok`, one
`useOneShotParams` over both landing families).

**No infra change.** The swap is five control-plane calls the role already
holds; no `UpdateUsagePlan`, no `UpdateUsage`, no new parameter. CI check 5
(the `apigateway:` grant shape) passes unchanged.

**Tests: 85 covering this task** (workspace 558 → 655 Rust, 0 failed; portal
70 → 93).

| where | count | covers |
| --- | --- | --- |
| `period.rs` | 5 | the four moved period tests + `start_secs` |
| `cap.rs` | 6 | 3 Aug → 1 Sep both halves, issued-inside-period, the boundary second, December, undated, URL-safe date |
| `state_token.rs` / `eligibility.rs` / `rework.rs` / `auth/mod.rs` | +6 | rework pair round-trip + cross-action mismatch, `parse`, membership/decide agreement, landing literals, `prompt=none` on both re-auth actions |
| `keys/mod.rs` | +1 | the pre-check path's depth |
| `tests/portal_rework.rs` (new) | 31 | ships closed; the swap and its **write ordering**; duplicates; two simultaneous reworks; the cap on pre-check and round-trip; the worked example relative to the calendar; issued-this-period; session-alone unreachability; left-the-guild + key keeps working; 429/5xx/401/403 → unknown; `pending` absent; age not re-checked; fresh token; `prompt=none`; drifted grant; cancelled/denied; exchange/identity failures; unwired deployment; control-plane down / failed delete rollback / vanished replacement / missing plan / deadline — each leaving the old key; usage-cache eviction; re-auth identity; `no-store` |
| `tests/portal_auth.rs` | 1 reshaped | `revoke` is now the unimplemented action |
| `app.spec.tsx` | 23 new | the dialog (opens, not navigates; POST pre-check; wording; arming on the exact phrase; once-and-only-once navigation; capped renders the date and never arms; failed pre-check; cancel) and the eight landings + settling + URL cleanup |

Verified: `cargo fmt --all --check`, `cargo clippy -p prices-api --all-targets`
(0 warnings), `cargo test --workspace` (655 passed, 0 failed), `cargo check -p
prices-api --features lambda`, `nx run-many -t lint typecheck build test -p
portal`, `nx format:check --all`, `make -C infra synth-production`, `npm run
openapi:lint`, `openapi:verify-routes`, `openapi:verify-servers`.

## Issues Encountered

- **The 0180 item-7 measurement had never run.** The archived note said
  "running since 2026-08-13 08:10Z"; the log holds three samples and the
  poller output ends at the poll start. Found while reading the data to carry
  it in, not while reading the note. Corrected in the note with a date; the
  result table now lives in this task so it cannot happen twice.
- **The AWS SSO session was expired for the whole build**, so the re-run
  could not be started from this session. Nothing about the implementation
  depends on it — recorded honestly in Step 0 and the AC rather than waited
  on.
- **`window.location` is unforgeable under jsdom 22** — neither `delete`
  nor `defineProperty` can replace it — so the "navigates once, never
  twice" property of the confirm could not be tested against
  `location.assign` directly. A one-line `navigateTo` seam in `api/portal.ts`
  is what the spec mocks; everything else in that module stays real.
- **A mock `createdDate` frozen at 1 800 000 000 (2027)** would have made
  every cap test pass for the wrong reason (a future date is always "inside
  the period"). The mock now stamps the current time like the service, with
  an override for tests that need a date.

**Broken/modified tests** (intentional): `portal_auth.rs`'s
`login_refuses_an_action_it_does_not_implement` now uses `revoke` — `rework`
is implemented; `state_token.rs`'s "an action this build does not know"
examples likewise. `usage/mod.rs`'s four period tests moved to `period.rs`.
No behaviour of an existing route changed.

## Design Decisions

### From Plan

1. **Rework is an OAuth round-trip (`action=rework`), and the callback
   completes it** — ADR 0010 §8 and [[0189]]'s per-action table: membership
   is re-proved against a fresh token, age is never re-checked. The `409` +
   `next_eligible_at` envelope the task asks for is the **pre-check**
   (`POST /key/rework`), which the dialog calls on open; the callback
   decides the cap again, authoritatively, before any key moves.
2. **Cap = `createdDate < period start`**, strictly, with the period the
   calendar month in UTC — our rule, defined once. An undated key is capped.
3. **Create and attach first, delete after.** Never keyless; the ordering is
   a tested property of the write sequence.
4. **`409`, not `429`**; `next_eligible_at` in `details`, RFC 3339.
5. **The modal's wording** — the old key dies immediately, `delete-key` arms,
   disabled on submit — and the refusal renders a calendar date.

### Emerged

6. **`POST /key/rework` is read-only by construction, like `/key`.** It lists
   and compares; it cannot create, attach or delete. A `GET` on it is a `405`.
   This keeps "rework is unreachable with a session cookie alone" structural:
   the only session-reachable route cannot swap anything.
7. **Membership before the cap.** A departed member is told to rejoin before
   being told to wait — the membership answer is already in hand and the cap
   needs a listing. Both are refusals; the order decides which fixable thing
   is heard first.
8. **Every old key under the name is deleted, not only the winner.** A
   duplicate left by an earlier double-submit is a working credential the
   visitor was just told had died.
9. **A failed delete rolls the replacement back** (best effort) and lands
   `failed`: the visitor holds exactly the key they had, not two keys of which
   the page reveals the older.
10. **A replacement that vanishes before attach, or after the old key is
    gone, is `failed`, loudly** — not healed by creating a third key inside
    a budget already mostly spent. The heal is one issue round-trip. Two
    simultaneous reworks (two tabs) may leave two keys, both capped, which the
    next issue's reconciler converges; accepted rather than serialised on a
    control plane with no lock.
11. **`UsageCache::invalidate` evicts everything for the caller**, where the
    issue path evicts only `NoKey`: after a rework the cached numbers are not
    stale, they are about a key that no longer exists. An in-flight real
    answer for the old key can still land for one TTL, dated by `as_of`.
12. **`prompt=none` joins the rework** by one variant in the condition, as
    [[0189]] decision #23 said it would; sign-in still never sends it.
13. **The `Period` extraction was forced, not optional.** Two copies of "the
    1st, 00:00 UTC" were a label problem; with the cap they became a
    correctness problem (a dashboard saying "resets on the 1st" beside a cap
    counting from a different instant). One module, two readers.
14. **The worked example is tested twice**: with the literal 2026-08-03 →
    2026-09-01 dates in the pure cap, and relative to the real calendar over
    HTTP (the 3rd of this month vs the 3rd of last) — so the HTTP suite is
    valid on every day of the year without a clock seam in production code.
15. **`next_eligible_at` travels in the URL as `YYYY-MM-DD`** (digits and
    dashes by construction from a `NaiveDate`) and in JSON as RFC 3339; the
    page renders both as "1 September 2026" **in UTC** — rendered in the
    viewer's zone, "1 September 00:00 UTC" reads as 31 August west of
    Greenwich.
16. **The refusal banners all say the existing key keeps working.** A
    refused rework leaves the visitor with exactly what they had; saying so
    is the difference between a refusal and a scare.

## Future Work

One task spawned ([[0221]]); every other follow-up already has an owner:

- **The `MONTH` confirmation** → [[0221]], spawned on completion, dated
  on/after 2026-09-01. The `DAY` proxy re-run is **not** carried over: it was
  abandoned on 2026-08-24 and its scratch stack torn down (Step 0).
- The live `403`/`200` curl pair → the deploy + [[0164]]'s evidence pass.
- Styling of the dialog and the eight landings → [[0193]] (wording not
  re-decided).
- ~~Revoke → [[0192]]~~ — merged into this task on 2026-08-21; nothing left
  to hand over.
- The now-seven-call surface and the per-load cost → [[0194]].

## Implementation Notes — 2026-08-21, after the reversal

The swap shipped first (the notes above), was seen live by Adam, and was
reversed the same afternoon. What changed, file by file:

- **Removed:** `auth/rework.rs`, `Action::Rework`, the eight `?rework=`
  landings, `reworkUrl`/`checkRework`/`navigateTo` on the frontend, the swap
  and its 31 tests.
- **`keys/naming.rs`:** `KeyRecord` gains `enabled` and `last_updated_at`;
  `current_key()` — the earliest *enabled* key, else the earliest record —
  is what the reveal and the revoke act on.
- **`keys/gateway.rs`:** `disable()` (`UpdateApiKey`, one `replace /enabled
  false`); `list_named`/`create` fill the two new fields. Seven calls now.
- **`keys/cap.rs`:** the same rule, re-pointed: `decide(revoked_at, period)`
  — strictly before the period start, undated → capped.
- **`keys/mod.rs`:** `POST /key/rework` is the revoke (`disable_all`); the
  reveal answers `key_revoked`; `attempt()` grows step 1b — all keys disabled
  → cap → `Attempt::Capped`, or delete the records and fall into the create.
  `IssueOutcome::Capped` → `issue.rs` lands `?issue=capped&next_eligible_at=`.
- **Frontend:** `revokeKey()`, `PortalKeyRevoked` on `fetchKey`, the
  `ReplaceKey` dialog re-worded (deactivated now, no new key until the next
  period), the `revoked` view with the date and a deferred issue link,
  `?issue=capped` landing.
- **Infra:** `PortalReadDisableAndDeleteOwnApiKeys` — `apigateway:PATCH`
  added on `/apikeys/*`, reasoning at the grant. Synth and CI check 5 green.
- **Docs:** epic decision struck through with the reversal dated; runbook §7;
  README §3c; [[0192]] absorbed.

**Tests:** `tests/portal_rework.rs` rewritten — 19 over HTTP (revoke
immediate/total/idempotent/own-key-only/502-never-false; cap on the issue path
with the worked example relative to the calendar; latest revocation governs;
live-beside-revoked; full cycle; cache eviction; `no-store`); `cap.rs` 6;
`naming.rs` +1; `app.spec.tsx` 12 for the dialog and the revoked/capped
states. Workspace 640 Rust, portal 88, 0 failed.

### Design Decisions — emerged in the reversal

17. **Disable, not delete.** Both propagate in ~25 s; only disabling leaves
    the revocation record (`lastUpdatedDate`) that the re-issue cap needs,
    and it costs one grant. The revoked value is never revealed again.
18. **Revoke is session-only.** It issues nothing and is destructive only to
    the caller's own access; a leak must be killable while Discord is down.
    `POST`-only + `SameSite=Lax` is the CSRF stance, re-derived as
    `auth/mod.rs` requires.
19. **The cap moved to the issue path.** A revoked user's "Get my API key"
    is an ordinary issue round-trip (membership + age re-proved) that the
    reconciler refuses at step 1b; the `409`-with-date of the old model is now
    `?issue=capped&next_eligible_at=` on the landing and `404 key_revoked`
    with `details.next_eligible_at` on the reveal.
20. **The latest revocation governs**, so a stale duplicate revoked last month
    cannot reopen a door closed this month.
21. **The 2026-08-07 decision is struck through, not deleted**, in the epic
    and here: both the decision and its reversal are on the record.

## Review round — 2026-08-21 audit (four lenses + two measurements)

Two measurements made on the spot, because the findings depended on them:

| measured | result |
| --- | --- |
| does `UpdateApiKey(enabled=false)` bump `lastUpdatedDate`? | **yes** (14:46:49 → 14:47:56 on a scratch key) — the cap has something to stand on |
| does a no-op patch, or a `description` patch, bump it? | **yes, both** — so the code must never re-patch a disabled key (it does not), and a console edit of a `discord-*` key extends its owner's cap |

Fixed in this round (A1–A5, B7, B8 of the audit list):

22. **One selector, one cap instant, four readers.** `naming::current_key`
    (earliest live, else earliest record) and the new
    `naming::revocation_instant` (the **latest** revocation; undated poisons
    it) are what the reveal, the revoke, the issue path **and the usage
    route** read. Before: usage ignored `enabled`, the reveal capped on the
    earliest record's date while the issue capped on the latest — two
    revocation records from different months made the page offer an issue
    the round-trip refused. Usage now also answers `no_key` once a
    revocation's period has rolled, as the reveal does.
    (`the_reveal_and_the_issue_agree_on_the_cap_with_mixed_period_revocations`,
    `usage_follows_the_same_key_as_the_reveal`)
23. **The re-listing after a post-roll create excludes the records just
    deleted and anything disabled.** `GetApiKeys` is eventually consistent;
    a phantom earlier-created record would win, 404 on attach, and spend the
    single retry — leaving the new key created and unattached.
    (`a_stale_listing_after_the_roll_does_not_rank_the_deleted_record`; the
    mock gained a sticky `list_resurrects_deleted`)
24. **A create is started only with ≥ 4 s of the deadline left**
    (`CREATE_FLOOR`), else `Attempt::OutOfTime` → `?issue=failed` with
    nothing written; and **`CreateApiKey` is sent without SDK retries** — no
    idempotency token, so a retried request whose first try landed is a
    duplicate. (`a_lost_create_response_is_not_retried_into_duplicates`; the
    mock gained `fail_next_create_after_creating`)
25. **`POST /key/rework` requires the portal's own request marker**
    (`X-Requested-With: stellar-prices-portal`) and refuses
    `Sec-Fetch-Site: cross-site`/`same-site` — `403 cross_site_request`,
    before the session is read. `SameSite=Lax` alone is site-scoped, and
    after the custom-domain cutover ([[0195]]) a sibling host's form `POST`
    would carry the cookie and cost the victim their key for a month.
    (`a_revoke_without_the_same_origin_markers_is_refused_before_anything_is_read`)
26. **IAM `/apikeys/*` is tag-scoped** (`aws:ResourceTag/ManagedBy =
    prices-portal`) on `GET`/`PATCH`/`DELETE`. The per-key `GET` with
    `includeValue=true` was the real account-wide exposure, not the listing;
    `PATCH` could rename a partner key into a portal name. Accepted change:
    a hand-made exact-name key is no longer adopted (`502`, left alone).
    Synth and CI check 5 green; the stale "NOT here: PATCH" paragraph is
    gone.
27. **Frontend:** the dashboard learns of an in-page revoke — the usage
    section drops "your key is new", re-asks (the backend evicted its
    cache), and says "deactivated" rather than "issue one above" when AWS
    has no row; an unparseable `next_eligible_at` keeps the issue link
    hidden (the safe direction); the revoke `fetch` carries the marker
    header.

Still open from the audit, deliberately: B6 (per-caller cache on the reveal
/ dedup of `GetApiKeys` across `/key` and `/usage` — the control-plane
budget), B9 ([[0205]] deploy), B10/B11 (CI allow-list; `timeoutSeconds` vs
the Rust budgets), C (dialog focus/Escape), D (ADR 0010 correction #3, epic
`:278-410`, [[0164]]'s plan, grant counts, stale comments), E (poller gap +
`429` body logging).

## Review round — 2026-08-21 code review of the audit round

Eight findings against the audit round's own diff; all eight closed.

28. **An undated record no longer poisons the cap instant.**
    `naming::revocation_instant` skipped to `None` on a single record without
    `lastUpdatedDate`, and `None` is capped — but `next_eligible_at` is then
    recomputed from the *current* period on every read, so the date rolled
    forward every month and the owner was locked out permanently, with no
    support action short of deleting the record. It is now the max over the
    dated records; `None` only when nothing under the name is datable.
    Skipping can only under-cap, and only in a shape AWS does not produce;
    the lockout was permanent.
    (`the_revocation_instant_is_the_latest_dated_one`,
    `an_undated_duplicate_does_not_lock_the_owner_out_forever` over HTTP; the
    mock's `last_updated_at` became `Option` with a `Store::undate`)
29. **The revoke stopped being a fifth reader of the cap.** It answered
    `next_eligible_at: period.resets_at()` directly, so an idempotent revoke
    of a key revoked two periods ago said "next month" while the reveal said
    `no_key` and the round-trip would have issued — a stale tab or a
    double-submit cost the visitor a month that was not owed. It now goes
    through `cap::decide` like the other four readers, and answers *now* when
    the period has rolled.
    (`an_idempotent_revoke_after_the_period_rolled_says_a_key_is_due_now`)
30. **`revoked_at` is nullable rather than an invented epoch.**
    `unwrap_or(0)` rendered as "deactivated on 1 January 1970"; the field is
    `Option<String>` in the JSON, and the page renders the revocation with no
    instant instead. `describeUtcInstant` returns `null` for a missing or
    unparseable value — its old fallbacks were "just now" (a claim about a
    revocation that may be weeks old) and, via `describeNextEligible`,
    "deactivated on the start of the next quota period", the *next-eligible*
    phrase presented as the revocation instant.
    (`renders an undated revocation without inventing an instant`)
31. **The propagation window is present-tense only while it is open.** The
    revoked view renders on every page load through the reveal, so "it stops
    working within about half a minute — until then treat it as live" was
    telling the owner of a key that died last week to keep treating it as
    live. Past tense beyond `PROPAGATION_FRESH_MS` (5 min); the measured
    window is still named, only the tense changes.
    (`states the propagation window in the past tense for an old revocation`)
32. **`RECONCILE_FLOOR` (2s) below `CREATE_FLOOR` (4s) is recorded as
    deliberate, not reconciled away.** Between the two only an *adoption* can
    end in a key — a create is refused, because one cut off by the deadline
    leaves an enabled unattached key. Raising the floor would save one
    `GetApiKeys` on a doomed first press and cost every returning press its
    recovery. Both constants now document the gap and a `const` assertion
    fails if the ordering is ever inverted.
- **Docs:** `README.md` §3c no longer says "deactivates the key
  immediately" — the claim this round removed from the dialog and asserts
  absent in the spec; it states the ~25 s window instead. `api/portal.ts`'s
  `revokeKey` doc no longer credits `POST` + `SameSite` as the CSRF guard,
  which the backend explicitly rejects: the marker header is.

**Tests:** 648 Rust across the workspace (+3 for this round), portal 93 (+3), 0 failed. `cargo fmt --check`,
`clippy --all-targets` (0 warnings), `cargo check --features lambda`,
`nx run-many -t lint typecheck build test -p portal`, `nx format:check --all`
all green.

---

## Review round — 2026-08-24, karczuRF's code review of PR #238

Seven findings, all verified against the branch before acting; five were real
as written, two needed correcting first. What that verification changed:

- **Finding 2's stated invariant was the wrong way round.** `app.tsx`'s dialog
  doc guarantees *"revoked" is never shown for a key that still works* — the
  failure copy breaks the **opposite** direction (claiming a key is still
  active when a disable may have landed). The defect is real; only the reason
  given for it was.
- **Finding 6 overstated the user-facing hole.** The usage panel does render
  with nothing marking the key dead, but the key section above it renders
  `key-revoked` on the same screen. The defect is the comment, which points a
  future reader at copy that is not in that branch.
- **Two things the review missed**, both found while checking it: the `PATCH`
  paragraph in `compute-stack.ts` still said the tag condition was "available
  to task 0194" while limit 3 twenty lines above said 0191 had written it; and
  `disable_all` dated the revocation from **this process's clock** while every
  later `cap::decide` reads AWS's `lastUpdatedDate` — invisible except across
  00:00 UTC on the 1st, where the two fall in different quota periods and the
  page promises a replacement a month early.

### Decisions

33. **The tag condition goes on `PATCH` alone, in its own statement — `GET`
    and `DELETE` go back to 0187's unconditioned grant.** The condition was
    written across all three verbs once `PATCH` joined the statement. That is
    a behaviour change to two shipped code paths inside a feature slice, and
    0187's own comment forbade it in as many words ("Do NOT put the condition
    on `GET`: adopting a console-created key is a documented requirement of
    this slice"). The new verb is still born narrow — nothing depends on
    `PATCH` being account-wide — and narrowing the other two stays **task
    0194's**, which owns the IAM audit and can verify against the deployed
    stack. The separate `PortalDisableOwnApiKeys` sid is deliberate: a
    condition on a shared statement silently reaches every action in it.

    Also corrected in that comment: "so it is no longer adopted" was wrong.
    Adoption is by NAME (`exact_matches` + `current_key`), not by tag — an
    untagged console key is still listed, ranked and attached, and only then
    `AccessDenied`s on the value read. The outcome the comment described was
    right; the mechanism was not.

34. **The post-roll cleanup logs and steps over a failed delete, like the
    loser sweep does.** `attempt()`'s `gateway.delete(&dead.id).await?` was the
    one place the "housekeeping must not withhold the key the request is for"
    rule was not applied — and it is the worse place for it, because that
    branch runs on *every* press once the name holds nothing but revoked keys.
    One undeletable record meant `?issue=failed` forever with no in-product
    recovery. Records that fail now stay in `revoked` for the end-of-function
    sweep to retry instead of being cleared.

35. **A revocation is `Done`, `Partial` or an error — three outcomes, not
    two.** Propagating the first failed disable answered `502`, and `502` copy
    has to describe a state the page cannot know. Now: at least one disable
    landed and none failed → `Done`; some landed and some failed → `Partial`,
    which reaches the page as `partial: true` and renders "one of your keys
    could not be deactivated, a duplicate may still work" — never a plain
    "revoked", which is the claim the dialog's docs forbid making about a key
    that still answers. Nothing landed and something failed → the error, and
    the `502` stands, because there "we could not deactivate it" is true.

    The `502` copy no longer says the key "is still active" either. A `502` is
    either a refusal with nothing written or a lost response on a patch that
    landed; the page says the deactivation was not *confirmed* and tells the
    visitor to reload.

36. **Every enabled key racing away is `NoKey`, not a dated `Done`.** All
    disables answering `NotFound` left no disabled record in the account, so
    "deactivated, next eligible on the 1st" was contradicted by the very next
    issue press, which finds the name empty and creates a key outright.

37. **The revocation instant comes off the `UpdateApiKey` response.**
    `Gateway::disable` returns `Disable::Applied(Option<u64>)` carrying the
    patched key's `lastUpdatedDate` — the same byte `cap::decide` compares
    against the period on every later read — instead of `bool` plus a local
    `SystemTime::now()`. Two clocks, one of them ours, cannot straddle a period
    boundary if only one of them is ever consulted. The `Disable` enum follows
    `Attachment`'s precedent in the same module: two outcomes, the second not
    an error.

38. **`revocation_instant`'s rustdoc gets its own body back.** An edit had
    concatenated `current_key`'s doc block onto it, leaving `current_key`
    undocumented and `revocation_instant` opening with a selection rule that is
    not its own.

39. **`rework_round_trip` is deleted.** Left from the abandoned swap design and
    unreachable: `Action::parse("rework")` returns `None` (asserted in
    `state_token.rs`), so `/auth/login?action=rework` answers `400` and the
    helper's `303` assertion could only panic. `round_trip`'s doc now records
    why there is no rework variant.

**Tests:** +4 Rust (`a_partial_revocation_is_reported_as_partial`,
`a_revocation_that_raced_away_is_no_key_not_a_phantom_record`,
`the_revocation_instant_comes_from_the_control_plane_not_our_clock`,
`an_undeletable_revoked_record_does_not_block_the_re_issue`) and +2 portal
(partial warning renders; clean revocation renders none). Two mock knobs added
for them: `fail_disable_of` (the per-id twin of `fail_disables`) and
`disable_stamps_at`. The existing failed-revoke spec was renamed and now
asserts the copy does NOT claim the key is still active. `cargo test -p
prices-api` 0 failed, `clippy --all-targets -D warnings` clean, portal 95/95,
`tsc --noEmit` on `infra` and `web/portal` clean, prettier clean.

40. **A partial revocation renders neither cap sentence.** Found on a re-read
    of the round above, not in the review: the revoked view still rendered
    "You can generate a new key from <date>. Until then you do not have a
    working key." for a `Partial`, and both halves are false there — the
    duplicate that refused to be disabled IS a working key, and the issue path
    adopts it rather than refusing (`a_partial_revocation_is_reported_as_partial`
    asserts exactly that: `?issue=ok`, nothing minted). The backend still
    computes a cap for the answer, because the shape is shared; the page is
    what must not repeat it. The warning carries the only instruction that
    applies.

**Not changed:** the five "checked and cleared" items in the review all hold —
`into_service_error` was re-verified against the resolved
`aws-smithy-runtime-api` 1.12.3 source (the non-`ServiceError` arm builds an
unhandled error, it does not panic).

## Amendment — 2026-08-27, via [[0193]]: the arming phrase is `regenerate-key`

Decision 5 and the spec above say the confirm stays disabled **until the
user types `delete-key`**. Since 2026-08-25 the dashboard control has said
**Regenerate**, the dialog's heading is "Regenerate API key?" and its button
"Regenerate" (the 0193 frame), and on 2026-08-26 Adam changed the phrase to
follow the button. [[0193]] found the change during its review round and,
under its own rule ("fix it in the owning task rather than quietly here"),
records it here rather than in the styling task.

41. **The arming phrase is `regenerate-key`.** A dialog headed "Regenerate"
    that demands the word `delete` asks the visitor to agree to a different
    sentence from the one they just read; the phrase follows the button so the
    two say the same thing. **What does not change:** the REASON for a typed
    phrase — this is destructive, it must not be reachable by one stray click
    — and everything else decision 5 pins: the old key dies immediately, no
    replacement is issued now, confirm disabled on submit, the refusal
    renders a calendar date. The FAQ on the landing page is a second place
    that copy lives (`web/portal/src/landing/Faq.tsx`); a future change to the
    wording has two targets.

    Re-pointed in the same change: ADR 0010 §8 ("the `regenerate-key` modal"),
    [[0164]]'s quiet-failure check and acceptance criterion (a tester following
    the old checklist would type `delete-key`, see confirm stay disabled, and
    file the dialog as broken), and [[0193]]'s own context line. Code:
    `REWORK_CONFIRM_PHRASE` in `web/portal/src/app/app.tsx`, with the spec
    `keeps confirm disabled until the visitor types regenerate-key`.
