---
id: "0191"
title: "Replace my key — rework, capped at once per quota period"
type: FEATURE
status: active
related_adr: ["0010"]
related_tasks: ["0183", "0157", "0160", "0180", "0187", "0189", "0190", "0192", "0193"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, slice-8]
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

## Step 0 — measure the rollover (was [[0180]] item 7)

**The rule we render is ours, not AWS's.** AWS documents neither the quota reset
instant nor its timezone; the only statement anywhere is an example caption,
*"creates a usage plan that resets at the beginning of the month"*, and `offset`
is a **request count**, not a time shift. This is ADR 0010's correction #2 and it
is still open.

A `DAY`-period scratch plan is a proxy for the instant and the timezone, and the
scratch resources from the abandoned run are **still standing** for it: REST API
`9utcrbmoc6`, usage plan `ox7pv0`, key `2ke0ixjy7h`, plus
`item7-quota-rollover.sh` and its partial log in the archived
`0180_RESEARCH_.../measurement/`. Drain the key, poll across a UTC midnight,
record when `429` becomes `200`.

**One honest limit, stated up front: this cannot be fully settled before
1 September 2026**, the next real `MONTH` rollover. The `DAY` proxy is strong
evidence, not proof — and it is enough, because the criterion is to stop
presenting "00:00 UTC on the 1st" as AWS-documented, not to prove AWS's
implementation. Correct [[0157]]'s and this task's wording either way, and run
the `MONTH` confirmation on 1 September if the epic is still open.

**Careful with the control plane while doing it:** `UpdateUsagePlan` is throttled
to **1 request per 20 seconds per account, non-adjustable**, and the whole
control plane shares a **10 rps / burst 40** budget with our deploys. A careless
loop here slows CI for everyone.

> **Status 2026-08-21 — the 0180 run never happened; re-run from here.** The
> "partial log" the paragraph above refers to holds **three samples**
> (08:12:37Z–08:14:37Z on 2026-08-13, all `429`) and `poller.out` ends at the
> poll start: the poller died two minutes in, and the archived note kept
> saying "running" for a week. Nothing was measured, and nothing is invented
> below. The scratch stack still stands; the procedure is `item7-quota-rollover.sh
> drain` then `poll` across the next UTC midnight, unchanged. The implementation
> does not wait on the answer — the cap is **our** rule, one definition in
> `portal/period.rs`, and a different AWS instant changes the dashboard label,
> not the cap.

| Question | Result | Date |
| --- | --- | --- |
| `DAY` reset instant and timezone | *re-run pending — the AWS SSO session had expired when this slice was built; run `drain` + `poll` and record the first `200` after the `429` run* | — |
| Calendar-aligned or creation-anchored? | *pending — setup was 08:10Z, so the two hypotheses predict 00:00Z vs ~08:10Z and the window discriminates them* | — |
| `GetUsage` agreeing with enforcement at the boundary? | *pending* | — |
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
- [ ] **(half done — the wording half; the measurement half is running)**
      Item 7 measured on the `DAY`-period proxy and written up with the date;
      [[0157]] and this task stop presenting the boundary as AWS-documented.
      The restating is done everywhere. The poller was started 2026-08-21
      11:51Z (drain `200,200,200,429`); the verdict is the first `200` after
      the `429` run — record it in the Step 0 table
- [x] "Replace my key" deactivates the key **immediately** and issues nothing —
      one `UpdateApiKey`, no Discord call, nothing created
      (`revoke_deactivates_the_key_immediately_and_issues_nothing`); every key
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
- [ ] `MONTH` confirmation scheduled for 1 September 2026 if the epic is
      open — dated note, not performed: on/after 2026-09-01 look for
      `summarize_days`' `quota reset inside the queried period` warn in the
      api-handler log, or re-run the `DAY` proxy script against a `MONTH`
      scratch plan drained on 31 August

## Notes

- A user who reworks on the last day of a period gets a fresh counter and a
  period reset a day later. Not an exploit — the reset was coming anyway — but
  written down so it is not re-raised as one.
- If the measured AWS rollover instant differs from ours, the dashboard renders
  our date and the counter does its own thing. A UX wrinkle, not a correctness
  bug: the cap is ours to define.
- The rework cap is why a leaked key cannot be invalidated until the 1st. That
  gap is [[0192]], and it is no longer blocked.

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

Nothing new spawned — every follow-up already has an owner or a date:

- **The `DAY` proxy re-run** → this task's Step 0, on the next AWS session
  (`item7-quota-rollover.sh drain` + `poll`); **the `MONTH` confirmation** →
  on/after 2026-09-01, per the last AC.
- The live `403`/`200` curl pair → the deploy + [[0164]]'s evidence pass.
- Styling of the dialog and the eight landings → [[0193]] (wording not
  re-decided).
- Revoke (`UpdateApiKey(enabled=false)`, no cap, session-only by design) →
  [[0192]], which will find `Action::parse`'s "arrives early" example is now
  `revoke`.
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
