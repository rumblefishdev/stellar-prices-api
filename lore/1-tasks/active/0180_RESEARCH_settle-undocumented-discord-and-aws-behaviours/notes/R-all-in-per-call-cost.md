---
title: "All-in per-call cost, and whether ADR 0010's proportionality argument survives it"
type: research
status: seed
spawned_from: notes/Q-which-undocumented-behaviours-hold.md
spawns: []
tags: [aws, cost, lambda, clickhouse, adr-0010]
links:
  - "../../../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
  - "../../../archive/0156_RESEARCH_self-service-auth-assumptions/notes/R-abuse-mitigation-options-costed.md"
history:
  - date: 2026-08-12
    status: seed
    who: akot
    note: "Created empty; this is arithmetic over existing CloudWatch metrics, not a measurement — no Discord prereq"
---

# All-in per-call cost

Covers item 9. **Not a spike** — no Discord app, no scratch guild, no scratch
usage plan. It is arithmetic over metrics the deployed `prices-api` already
emits, which is why it can run first, before any of the setup lands.

> **Status: nothing computed yet.**

## What is wrong with the current figure

Every cost figure in [[0156]] is **gateway-only**: $3.50/million requests +
$0.09/GB transfer. Compute, ClickHouse query time and storage reads are
unpriced — they were simply not in scope there.

ADR 0010 then declines paid abuse mitigations (CAPTCHA, email verification) on
a proportionality argument: a fully-drained key costs us about **$0.38**, which
is too small to justify the friction. That number is gateway-only.

**If backend cost dominates by 10×, the exposure is ~$4/key and the argument
needs revisiting.** Not necessarily reversing — but it stops being obvious, and
an ADR resting on an obviously-true number should not quietly rest on a
possibly-false one.

## How to compute it

Per request, from CloudWatch over a representative window (state the window):

| Component | Source | Rate |
|---|---|---|
| Gateway request | AWS pricing | $3.50 / million |
| Gateway transfer | response size × | $0.09 / GB |
| Lambda duration | `Duration` p50/p95 × memory | GB-second price |
| Lambda requests | invocation count | $0.20 / million |
| ClickHouse query time | query duration / read bytes | shared Hetzner box — see below |
| CloudWatch logs | ingest per invocation | $0.50 / GB |

**ClickHouse is not usage-priced.** It is a fixed monthly box (ADR 0007), so
per-call cost there is an *allocation*, not a bill. A marginal-cost reading puts
it at zero, which is misleading in one direction; dividing the whole box cost by
our request volume is misleading in the other, because **`ch-prod-01` is shared
with the BE team** — see `docs/runbooks/0136-coarse-rollup-merge-recovery.md:33`.
Allocate our share (query time is the honest divisor, ingestion and backfill
dominate the box) and state the basis explicitly next to the number.

Use the **monthly quota** as the multiplier for a fully-drained key, so the
result is directly comparable to ADR 0010's $0.38.

## Result (date: ______)

| Component | $/call | Notes |
|---|---|---|
| | | |

- **All-in per call:** →
- **Fully-drained key (× monthly quota):** →
- **Ratio to ADR 0010's $0.38:** →

## Verdict for ADR 0010

- Does the proportionality argument survive? →
- If not, what changes — the mitigations, the quota, or the ADR's reasoning? →
