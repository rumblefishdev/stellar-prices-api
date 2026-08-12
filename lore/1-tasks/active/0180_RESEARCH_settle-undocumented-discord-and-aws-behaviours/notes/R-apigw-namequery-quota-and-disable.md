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

**Result (date: ______):**

| Query | Keys returned | Interpretation |
|---|---|---|
| exact full name | | |
| strict prefix | | |
| substring (not a prefix) | | |
| case-differing | | |

**Action either way:** keep the client-side exact-match filter and document it
as **load-bearing, not defence in depth**, so nobody later "simplifies" it away.

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

**Result (date: ______):**

| Step | `used` | `remaining` |
|---|---|---|
| after consuming N | | |
| while disabled | | |
| after re-enable | | |

**Verdict:** preserved / frozen / zeroed →

---

## Consequences for other tasks

- [[0158]] — reconciler reasoning, exact-match filter documented as load-bearing
- [[0160]] — same, plus "Open" item on revocation now costable
- [[0157]] + `docs/runbooks/manual-api-key-tier.md` — quota-period wording, and
  whether `--name-query` returns as a server-side prefilter
