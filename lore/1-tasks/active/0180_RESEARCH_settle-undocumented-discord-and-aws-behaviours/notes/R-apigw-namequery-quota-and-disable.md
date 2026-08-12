---
title: "API Gateway — nameQuery matching, quota rollover instant, and enabled=false"
type: research
status: seed
spawned_from: notes/Q-which-undocumented-behaviours-hold.md
spawns: []
tags: [aws, api-gateway, usage-plan, quota, api-keys, corrections]
links:
  - "../../../archive/0156_RESEARCH_self-service-auth-assumptions/notes/R-apigw-usage-plan-quota-mechanics.md"
  - "../../../../../docs/runbooks/manual-api-key-tier.md"
history:
  - date: 2026-08-12
    status: seed
    who: akot
    note: "Created empty with the three measurements to fill; two of them are corrections to already-written text"
---

# API Gateway — three behaviours we assumed were documented

Covers items 6–8. **Two of the three are corrections**, not new findings: the
text is already written into other tasks as though AWS guaranteed it.

> Run everything here against a **scratch usage plan**, never
> `pricing-api-free`. Throttling is the constraint — see the
> [runbook](G-measurement-runbook.md).

> **Status: nothing measured yet.**

---

## 6. `nameQuery` matching semantics — prefix or exact? *(correction)*

**What the docs give us.** One sentence: *"The name of queried API keys."* No
matching rule is stated. Prefix behaviour is **community knowledge, not an AWS
contract**.

**What already depends on it.** [[0158]] and [[0160]] build a reconciler and a
"prefix hazard" guard on the premise that it is a prefix match.
`docs/runbooks/manual-api-key-tier.md` reasons about it too ([[0157]]): it
dropped `--name-query` for a client-side `starts_with` on these grounds, and
explicitly invites re-adding it as a server-side prefilter. **Whatever is
measured here decides whether that invitation stands.**

**How to measure.** Create keys on a scratch plan with deliberately overlapping
names, then query each way:

```
disc-111111111111111111
disc-111111111111111111-old
disc-1111
```

Then `GetApiKeys(nameQuery=...)` with the full name, a strict prefix, a
substring, and a differing-case variant.

**Result — measured 2026-08-12**, three scratch keys in `eu-central-1`
(`lore0180-1111`, `lore0180-111111111111111111`,
`lore0180-111111111111111111-old`), created disabled and destroyed after:

| Query | Keys returned | Interpretation |
|---|---|---|
| `lore0180-111111111111111111` — an **exact key name** | that key **and** `…-old` | **not exact matching** |
| `lore0180-1111` — exact name *and* a prefix of the other two | all three | prefix |
| `lore0180-11111` — a prefix that is **nobody's name** | the two long ones | prefix, decisively |
| `lore0180` | all three | prefix |
| `0180-1111` — substring, not a prefix | none | **not** substring matching |
| `LORE0180-1111` — case-differing exact name | none | **case-sensitive** |
| `…-old-extra` — longer than any name | none | sanity check |

**Verdict: `nameQuery` is a case-sensitive prefix match on `name`.** The
community knowledge was right — but it is now measured rather than assumed, which
is the whole point of the item.

**Action either way:** keep the client-side exact-match filter and document it
as **load-bearing, not defence in depth**, so nobody later "simplifies" it away.
The measurement makes that concrete rather than precautionary: the very first row
is the failure. A query for a key's *own full name* returns a **different key**,
because [[0157]]'s rotation suffix makes `…-key-20260812T142317Z` a strict
extension of `…-key`. `--name-query` narrows; it never identifies.

**Two consequences the note did not anticipate.**

- **The server-side prefilter invitation stands.** `docs/runbooks/manual-api-key-tier.md`
  dropped `--name-query` for a client-side `starts_with` and invited re-adding it
  as a prefilter. Safe: server-side matching is prefix and client-side
  `starts_with` is prefix, both case-sensitive, so the prefilter can never drop a
  key the client-side filter would have kept. The two compose exactly. Whether it
  is *worth* re-adding is a separate question — at our key counts, no.
- **A prefilter does not exempt a reconciler from paging** ([[0158]]).
  `get-api-keys --name-query lore0180 --limit 1` returned one name **and** a
  `position` token, so `nameQuery` filters the paginated set rather than replacing
  pagination. A reconciler that reads one page and treats it as the whole
  namespace is wrong regardless of how narrow its query is.

---

## 7. Monthly quota reset instant and timezone *(correction)*

**What the docs give us.** Nothing. AWS's only statement is an example caption —
*"creates a usage plan that resets at the beginning of the month"* — with no
timezone and no instant. `offset` is a **request count**, not a time shift.

**What already claims otherwise.** [[0157]] / [[0158]] / [[0160]] all state
"1st of the month, 00:00 UTC" as if inherited from AWS.

**The measurement problem.** A real `MONTH` rollover cannot be observed before
1 September 2026 — this task is not waiting a fortnight for it. Proxy:

- create a scratch plan with period **`DAY`**, drain some quota, observe the
  instant the counter resets → gives the **timezone and the boundary instant**
- `UpdateUsage` (`op:replace` on `/remaining`) sets counters directly, so test
  states are cheap to construct

A `DAY` observation is **strong evidence, not proof**, for `MONTH`. That is
acceptable here: the AC is to stop presenting the rule as AWS-documented, not
to prove AWS's implementation. If `DAY` resets at 00:00 UTC, restate the monthly
rule as **our product decision, consistent with observed daily behaviour**.

**Result (date: ______):**

- `DAY` period reset observed at → (instant, timezone)
- Inference for `MONTH` →
- Text restated in 0157 / 0158 / 0160 as our rule? →

The worked example ("reworked 3 August → next 1 September") stands either way.
Its *justification* does not.

---

## 8. Does `UpdateApiKey(enabled=false)` preserve, freeze or zero usage counters?

**What the docs give us.** Nothing. And the delete-then-create derivation does
**not** transfer here, because the key `id` survives disabling.

**Why it matters.** This blocks **costing revocation** ([[0160]] "Open"). If
disabling resets the counter, revocation cannot be offered freely — a user could
drain a quota, get disabled, get re-enabled and start from zero.

**How to measure.** On the scratch plan: consume known quota → `GetUsage` →
`UpdateApiKey(enabled=false)` → `GetUsage` → re-enable → `GetUsage`.

**Measured 2026-08-12.** Scratch REST API `lore0180-scratch` (`jfsxfpw280`) with a
MOCK integration on `GET /`, stage `test`, plan `lore0180-scratch-plan`
(`j11m0r`). Production was never involved — the note's warning is satisfied by
construction rather than by care.

**Verdict: preserved.** Disabling neither zeroes nor rolls back the counter, and
re-enabling resumes exactly where the key left off.

Proved twice, because the reported counters turned out to lag (below) and a
`GetUsage`-only method could not have carried the verdict on its own:

*Behavioural — immune to any reporting lag.* Quota `5/DAY`, drained to
exhaustion:

| Step | request | reported `[used, remaining]` |
|---|---|---|
| 5 requests | 5 × `200` | `[5, 0]` |
| 3 more | 3 × `429` `Limit Exceeded` | `[5, 0]` |
| disable → request | `429` | `[5, 0]` |
| re-enable → request | **`429`** | `[5, 0]` |

The last row is the finding: the first request after re-enabling is still
rejected. A zeroed counter would have returned `200`. A user cannot launder a
drained quota through a disable/enable cycle.

*Counter-level, with quota raised to `1000/DAY` so the flag is the only variable:*

| Step | reported `[used, remaining]` |
|---|---|
| at the instant disabling took effect | `[7, 993]` |
| after re-enabling, one further `200` | `[8, 992]` |

**Three things measured on the way that the item did not ask for, and that matter
more operationally than the verdict itself.**

1. **A disabled key gets `403 {"message":"Forbidden"}` — byte-identical to
   sending no key at all.** So a revoked user cannot be told apart from a user who
   never had a key, by the gateway alone. [[0162]] already needs "could not
   verify" to render differently from "not a member"; this is the same problem one
   layer down, and the portal is the only place it can be fixed.

2. **Enable and disable take ~25 s to reach the data plane** — measured by polling
   every 5 s: `200` at 20 s, `403` at 25 s on disable; `403` at 20 s, `200` at 25 s
   on re-enable. Symmetric, `n = 1` each way, so treat it as "tens of seconds", not
   as 25. **Revocation is not immediate.** Anything in [[0160]] that reports a key
   as revoked the moment the API call returns is reporting the control plane, not
   reality.

3. **Quota-rejected requests do not increment `used`.** It stayed at 5 across
   three `429`s. The quota counts what it lets through, so a client hammering an
   exhausted key cannot push itself further into debt.

**On `UpdateUsage` as a test-state shortcut** — the task's Notes recommend it, and
it does work (`op:replace` on `/remaining` moved the enforcement counter and
requests started succeeding). But it left the *reported* pair inconsistent with
`limit` — `used = 5`, `remaining = 1000`, `limit = 1000` — and frozen there for
over a minute of successful traffic before reconciling to a coherent `[7, 993]`.
Fine for constructing states; do not read `GetUsage` as ground truth in the same
period you patched it.

---

## Consequences for other tasks

- [[0158]] — reconciler reasoning, exact-match filter documented as load-bearing
- [[0160]] — same, plus "Open" item on revocation now costable
- [[0157]] + `docs/runbooks/manual-api-key-tier.md` — quota-period wording, and
  whether `--name-query` returns as a server-side prefilter
