---
id: "0192"
title: "Revoke a leaked key — kills it now, but the once-per-period cap still governs the replacement"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0183", "0160", "0180", "0187", "0191", "0193"]
tags: [layer-backend, priority-medium, effort-small, milestone-M3, epic-self-service-onboarding, api-gateway, security, slice-9]
milestone: 3
links:
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/notes/R-apigw-namequery-quota-and-disable.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Ninth slice. Filed as its own task because it stopped being blocked:
      [[0180]] item 8 measured that `UpdateApiKey(enabled=false)` **preserves**
      usage counters, so revocation cannot become a free quota reset. That was
      the whole reason the 2026-08-07 meeting deferred it — the deferral is now
      a scheduling choice, not a correctness one.
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Rule settled by Adam rather than left open: **revocation does not earn a
      replacement.** Rework on the 3rd, then revoke, and the next key is still
      only issuable on the 1st. Revoke is a kill switch, not a second door to
      the same room — which makes the honesty of the confirmation screen the
      main design work in this slice.
---

# Revoke a leaked key

> **Absorbed by [[0191]] on 2026-08-21.** "Replace my key" became exactly this
> task's rule — `UpdateApiKey(enabled=false)` now, nothing issued, re-issue from
> the next period — so there is no separate revoke to build. The one difference
> from the text below: **disable, not delete**, because the disabled key's
> `lastUpdatedDate` is the revocation record the cap reads. To be closed as
> superseded when 0191 completes.

## Summary

**Story:** *as a developer whose key leaked, I can kill it immediately — knowing
that I will not have a working key again until my next rework is due.*

[[0191]] caps rework at once per quota period, which is right for quota
protection and wrong for a leak: a key that leaks on the 3rd stays live until the
1st. Revocation closes that hole without opening another one.

## The rule

**Revoking does not reset, consume or bypass the rework cap. It removes a key and
issues nothing.** A user who reworked on 3 August and then revokes is keyless
until 1 September, exactly as if they had never revoked.

This is the decision, not a default to be re-derived at implementation time. The
reasoning:

- The cap exists so a burnt quota cannot be escaped by minting a fresh
  `apiKeyId`, which gets a clean counter — quota is scoped to
  `(usagePlanId, apiKeyId)`. If revoke handed out a replacement, "revoke" would
  be the button people press on the 20th of a heavy month, and the cap would be
  decorative.
- The 2026-08-12 measurement removes the *other* half of the worry — disabling
  preserves the counter, so revoking is not itself a reset — but it says nothing
  about re-issuance. Re-issuance is a new key with a clean counter no matter what
  preceded it, so the cap has to govern it.
- Being keyless is a real cost to the user, and it is the correct one: it is the
  same cost they would pay by simply not using the leaked key, minus the risk of
  someone else using it.

Consequence this slice must carry rather than hide: **revocation is destructive
to the user's own access, and the confirmation has to say so in those words**,
with the actual date.

## Context

The 2026-08-07 meeting deferred revocation and the epic records it as a known
gap. The blocker was an unknown: if disabling a key reset its usage counter,
revocation would be a free quota reset. ADR 0010 recorded that as an assumption,
not a fact — the delete-then-create argument does **not** transfer, because that
one works only because `CreateApiKey` mints a new `id`, while `enabled=false`
keeps the `id` and `value` in place.

**Measured 2026-08-12: counters are preserved.** A key drained to its quota,
disabled and re-enabled was still at its quota — the next request came back
`429`, not `200`.

Three further properties from that measurement, which this task must design
around rather than merely note:

- **A disabled key is `403 Forbidden`, byte-identical to no key.** The gateway
  cannot explain a revocation to its owner, so whatever the portal renders has to
  come from our own record of it. Same shape as the "could not verify" vs "not a
  member" distinction in [[0189]].
- **Revocation takes ~25 s to reach the data plane** (measured by polling at 5 s
  intervals; re-enable took the same). An endpoint returning `204` the moment
  `UpdateApiKey` succeeds is reporting the control plane, not reality. For a
  leak, that window is the entire point.
- **`GetUsage` is not read-after-write**, so [[0188]]'s dashboard trails a
  revocation.

## Implementation

- `POST /api-tokens/api/key/revoke`.
- **Delete rather than disable.** Both take ~25 s to propagate, so disabling buys
  no speed; deleting is unambiguous, frees the `discord-<userId>-key` name for
  the eventual re-issue, and leaves no dormant credential in the account. The
  measurement that unblocked this slice is about counters, not about which call
  to use — record that we chose `DeleteApiKey` and why, so the `enabled=false`
  research does not read as a recommendation.
- **Whatever we do, the response must not imply immediacy** while the data plane
  takes ~25 s to catch up. Say the window out loud.
- **Record the revocation ourselves** — timestamp and the key id — because the
  gateway's `403` cannot be distinguished from "no key". This is the first thing
  in the epic that genuinely needs durable state of our own, and it is therefore
  the strongest of the arguments in [[0190]]: revisit that task's build/cancel
  decision when this slice starts.
- **The cap is read from the same source [[0191]] reads.** After a revoke there
  is no surviving key, so `createdDate` is gone with it — a revoked user's next
  eligible date cannot be recomputed from API Gateway. Either persist it with the
  revocation record above, or accept that revoke resets the cap, which the rule
  above forbids. **Persist it.**
- **IAM:** `apigateway:DELETE` on `/apikeys/{id}` — already granted by [[0187]]
  for the reconciler, so no new statement unless we end up disabling instead.
- **No fresh eligibility proof.** A user must be able to kill a leaked key while
  Discord is down, and the session already scopes the action to their own key.
  Deliberate exception to [[0189]]'s table; write it into that table's comment.
- **Frontend:** the action opens a confirmation that states, in this order, that
  the key stops working within about half a minute, that **no replacement is
  issued**, and the date on which one becomes available — the same
  `next_eligible_at` [[0191]] renders. Confirm gated the way [[0191]]'s modal is.
  Afterwards the dashboard shows a persistent "revoked on <date>, next key
  available <date>" state sourced from our record, never inferred from a `403`.

## Storage — substrate decided, shape and access are this task's

Groundwork done by [[0190]] on 2026-08-20 so this slice does not start it from
zero. [[0190]] cancelled the *registry*; it did not cancel *storage*. This is
the first slice that needs any, and the decisions split cleanly into one that is
already made and three that are not.

### Decided: ClickHouse, per ADR 0007

ADR 0007 is explicit — the prices-api data sink is BE's Hetzner ClickHouse, not
a store of our own. That governs here too. Adding DynamoDB (or anything else in
the AWS account, where this project currently has **no** other store) for one
small table would break that decision for the smallest possible reason. So the
substrate is ClickHouse, and the open questions are shape and access.

### Undecided #1 — the shape, and the one shape that is forbidden

**Do not reuse [[0158]]/[[0190]]'s schema.** It is
`ReplacingMergeTree(updated_at) ORDER BY discord_user_id` — one row per user,
replaced on every write — so the next issue overwrites the revocation row and
silently resets the cap this task exists to enforce. That is the trap [[0190]]
was cancelled over; copying the shape reintroduces it.

What this slice needs instead is **append-only**: a revocation event, keyed so
that nothing an issue writes can overwrite it. Sketch, not a specification —
this task decides the columns:

```sql
-- one ROW PER EVENT, never replaced. The cap reads the latest event per user.
ENGINE = MergeTree ORDER BY (discord_user_id, revoked_at)
```

Two ClickHouse properties this has to survive, both already true of the tables
this repo runs:

- **merges are asynchronous**, so a read straight after a write can miss it. For
  `backfill_progress` and `current_prices` that is lag and it is fine; here it
  gates whether somebody may take a key, so it is **correctness**. Decide
  deliberately whether the read tolerates it or the write must be confirmed;
- **there is no conditional insert**, so the table cannot enforce uniqueness.
  Same conclusion [[0190]] reached: the invariant stays in [[0187]]'s
  reconciler, and the table is a record, never an arbiter.

### Undecided #2 — write access does not exist yet, and is not ours to grant

This is the part with **cross-team lead time**, so raise it early rather than
discovering it mid-slice. Measured on `ch-prod-01` 2026-07-30 and recorded in
`packages/prices-clickhouse/schema/views.sql`:

| Fact | Consequence for this task |
| --- | --- |
| The api-handler reads as **`prices_reader`**, which holds **SELECT only** (`compute-stack.ts` 219-220) | The portal Lambda has never written a row to ClickHouse. It cannot write this one |
| `prices_reader` / `prices_writer` grants are **XML-managed in BE's `services.xml`** and **cannot be SQL-GRANTed by us** | Write access is a **request to BE**, not a change we can merge |
| Schema DDL on `ch-prod-01` is an **operator action** as the container's `default` user over the loopback native port, bypassing Caddy and mTLS | Creating the table is a runbook step, not `cdk deploy`. Same ownership split as the mTLS material and the eligibility parameters |
| "Requesting a broad DDL grant for the ingestion writer was **considered and rejected**" (task 0134) | Do not re-propose that shortcut. Ask for the narrow grant this table needs |

So the access work is roughly: agree the grant with BE → operator applies the
DDL → a **writer** mTLS bundle reaches the api-handler (today it carries only
the reader bundle) → the handler gains an insert path. **The last one deserves
its own look**: it puts write credentials on an internet-facing Lambda that
currently cannot write anything. Scope the CH user to `INSERT` on exactly this
table.

### Undecided #3 — whether the record is worth its cost at all

[[0190]]'s standing question applies to this slice too, and this task's own
notes already say revoke is *optional for Tranche 3*. If the grant negotiation
turns out to be slow, the documented fallback is the one already written here:
say plainly in [[0163]]'s quickstart that a leaked key waits for the 1st. That
is a worse product and a cheaper slice — a real trade, to be made with numbers
rather than by drift.

## Acceptance Criteria

- [ ] **Ships closed.** With `PORTAL_ENABLED=false` ([[0183]]) this slice's
      routes return an empty `404`; with it on, they behave normally — every
      deploy goes straight to production
- [ ] Revoking makes the key stop working, and the response does not claim it is
      instant when it takes ~25 s
- [ ] After revoking, issuing a new key is refused with `409` and
      `next_eligible_at` until the period rolls over — reworked on 3 August,
      revoked on 4 August, no key until 1 September
- [ ] That refusal is verified against a user who has *never* reworked as well:
      revoke on the 3rd of the period they were issued in also waits
- [ ] The confirmation screen states that no replacement is issued, and names the
      date, **before** the user confirms
- [ ] The dashboard shows the revoked state and the next eligible date from our
      own record, not inferred from a `403`
- [ ] The next-eligible date survives the key's deletion — it is persisted, not
      recomputed from a `createdDate` that no longer exists
- [ ] Revocation does not reset the quota counter — verified, not assumed
- [ ] Revocation works while Discord is unreachable
- [ ] The choice of `DeleteApiKey` over `UpdateApiKey(enabled=false)` is recorded
      with its reasoning

## Notes

- Also worth having near this code: **`UpdateUsage`** (`op:replace` on
  `/remaining`) moves the quota counter directly without touching the key. It is
  the right tool for a manual "more quota this month" and a different operation
  from both rework and revoke.
- Optional for Tranche 3. If it slips, the epic still ships — but the "my key
  leaked" answer stays "wait until the 1st and stop using it", and that should be
  said out loud in the quickstart ([[0163]]) rather than discovered.
- This slice is where the epic stops being able to avoid durable state. Do not
  build [[0190]] *for* it reflexively — a single small record is not the same as
  the full registry — but do make that call here rather than by accident.
- **[[0190]] was decided and CANCELLED on 2026-08-20, so that call is now this
  slice's alone** — see *Storage* above, which [[0190]] wrote so this slice does
  not restart the question. Substrate is settled (ClickHouse, ADR 0007); shape,
  write access and cost-vs-fallback are open and belong here. The measurement
  behind the cancellation is in `docs/epics/self-service-onboarding.md`.
