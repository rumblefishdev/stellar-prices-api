---
id: "0170"
title: "Settle the seven undocumented Discord/AWS behaviours the onboarding design depends on"
type: RESEARCH
status: backlog
related_adr: ["0010"]
related_tasks: ["0156", "0157", "0158", "0159", "0160", "0169"]
tags: [layer-backend, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, discord, aws, api-gateway, spike, blocks-build]
milestone: 3
links:
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../active/0156_RESEARCH_self-service-auth-assumptions/notes/S-account-model-and-abuse-barrier.md"
history:
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      Spawned from 0156. Seven behaviours that 0157-0160 currently assume are
      documented, and are not. Two of them are stated in those tasks as if they
      were vendor guarantees. All are minutes-cheap to measure once the Discord
      app and `stellar_test` guild exist.
---

# Settle the undocumented behaviours before building on them

## Summary

0156 verified every source behind the onboarding design against the original
documentation. Seven behaviours the design leans on turn out to be
**undocumented**, and two of them appear in already-written tasks as though they
were vendor guarantees.

None is hard to settle. All are a short spike against the `stellar_test` guild
(ADR 0010) and a scratch AWS usage plan. The point of filing it as a task is
that they are currently invisible risks: the tasks read as though these facts
were sourced.

## Context

This is not speculative hardening. Two items are **corrections to text already
written into 0157/0158/0160**, and the rest are load-bearing branches in code
nobody has written yet — the cheapest possible moment to check them.

## Implementation

### Discord — needs the app registered and `stellar_test` configured

1. **Status code when the user is not a guild member.**
   `GET /users/@me/guilds/{guild.id}/member` documents only the success case.
   Generic `404 NOT FOUND` plus JSON error codes `10004` ("Unknown guild") and
   `10007` ("Unknown member") are all that exist.
   **Why it matters:** the membership check is a *negative* inferred from an
   undocumented error shape. Fail closed on a 429 and legitimate users are
   denied; fail open on a 404 and the check is void. Treat only an explicit
   `10007`/`10004`-style 404 as "not a member"; treat 401/403/429/5xx as
   "unknown, do not deny".

2. **Is `pending` present on that REST response?**
   The field is optional (`pending?`). The docs' presence guarantee — *"In
   `GUILD_` events, `pending` will always be included"* — is written about
   **gateway events**, not this route.
   **Why it matters:** `pending === undefined` must be handled as a third state.
   Never write `if (member.pending)` and read absent as "cleared".

3. **Is `flags` populated on that response?**
   Non-optional in the field table, which *suggests* always present — but that
   is inference. Required before any rule uses `COMPLETED_ONBOARDING` or
   `AUTOMOD_QUARANTINED_USERNAME`.

4. **What `pending` is for a guild without screening enabled.**
   Every documented statement is scoped "In guilds with Membership Screening
   enabled". Test against a scratch guild with the gate **off**.
   **Why it matters:** `pending === false` may mean "cleared the gate" or "there
   was no gate". Only the guild-level `MEMBER_VERIFICATION_GATE_ENABLED` feature
   distinguishes them — and that lives on the guild object, which
   `guilds.members.read` does not return.

5. *(Cheap, while you are there)* Screenshot the consent screen with and without
   `guilds.members.read`. Discord documents no per-scope consent copy anywhere,
   so the friction cost of the scope is currently unknown.

### AWS — needs a scratch usage plan, not the production one

6. **`nameQuery` matching semantics — prefix or exact?**
   **This is a correction.** [[0158]] and [[0160]] build a reconciler and a
   "prefix hazard" guard on the premise that it is a prefix match. AWS documents
   one sentence — *"The name of queried API keys."* — and states no matching
   rule. Prefix behaviour is community knowledge, not an AWS contract.
   **Action:** measure it, and either way keep the client-side exact-match
   filter — it is load-bearing, not defence in depth. Update the reasoning in
   both tasks so nobody later "simplifies" the filter away.

7. **The monthly quota reset instant and timezone.**
   **This is a correction.** [[0157]]/[[0158]]/[[0160]] state "1st of the month,
   00:00 UTC". AWS never says this. Its only statement is an example caption —
   *"creates a usage plan that resets at the beginning of the month"* — with no
   timezone and no instant. `offset` is a **request count**, not a time shift.
   **Action:** measure the actual rollover, then either source the rule or
   restate it in those tasks as **our** product decision rather than inherited
   AWS behaviour. The worked example ("reworked 3 August → next 1 September")
   can stand either way; its justification cannot.

8. **Does `UpdateApiKey(enabled=false)` preserve, freeze or zero usage
   counters?**
   Undocumented. The delete-then-create derivation does **not** transfer, since
   the key `id` survives. Needed before revocation ([[0160]] "Open") can be
   costed — if disabling resets the counter it cannot be offered freely.

### Cost — needed before the ADR's proportionality argument can be trusted

9. **All-in per-call backend cost.**
   Every cost figure in 0156 is **gateway-only** ($3.50/million + $0.09/GB).
   Compute, ClickHouse query time and storage reads are unpriced.
   **Why it matters:** ADR 0010 justifies declining paid mitigations on a
   $0.38/fully-drained-key exposure. If backend cost dominates by 10×, that
   becomes ~$4/key and the proportionality argument needs revisiting.

## Acceptance Criteria

- [ ] Items 1-4 measured against `stellar_test` and a screening-off scratch
      guild, results written down with the date
- [ ] Consent screen captured with and without `guilds.members.read`
- [ ] `nameQuery` semantics measured; [[0158]] and [[0160]] updated so the
      exact-match filter is documented as load-bearing rather than redundant
- [ ] Quota rollover instant measured; [[0157]]/[[0158]]/[[0160]] corrected to
      stop presenting "00:00 UTC on the 1st" as AWS-documented behaviour
- [ ] `enabled=false` effect on usage counters measured and recorded against
      [[0160]] "Open"
- [ ] All-in per-call cost established and ADR 0010's proportionality argument
      re-checked against it
- [ ] Any finding that changes ADR 0010's shape reflected in the ADR

## Notes

- Do the AWS items on a scratch usage plan. `UpdateUsagePlan` is throttled to
  **1 request every 20 seconds per account, non-adjustable**, and the whole
  control plane shares a **10 rps / burst 40** budget with our deploys — a
  careless loop here slows CI down for everyone.
- `UpdateUsage` (`op:replace` on `/remaining`) moves the quota counter directly
  without touching the key. Useful for constructing test states cheaply, and
  worth knowing about independently of this spike.
