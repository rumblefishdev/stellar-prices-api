---
title: "G: BE agreement record — responses to the Cluster A-D brief"
type: generation
status: developing
spawned_from: ./G-be-conversation-brief.md
spawns: ["0047"]
tags: [generation, agreement-record, cross-team, block-explorer, hetzner, clickhouse]
links:
  - "./G-be-conversation-brief.md"
  - "../README.md"
  - "../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../active/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md"
history:
  - date: 2026-05-19
    status: developing
    who: okarcz
    note: >
      First batch of BE responses captured against the brief's
      §7 outcome-tracking table. 10 of 13 asks have yes/no
      answers; 3 carry caveats (B6 spawns task 0047; B7 and D12
      need further iteration). ADR 0007 stays `proposed` until
      the three open items resolve.
---

# G: BE agreement record — responses to the Cluster A-D brief

## 0. Context

This record captures BE's responses to the brief in
[`G-be-conversation-brief.md`](./G-be-conversation-brief.md). It is
the artifact ADR 0007 will reference when it transitions
`proposed → accepted`.

**Status:** First-batch responses received 2026-05-19. Three items
still open (B6, B7, D12). Recording the current state here so the
implementation tasks can plan against the answered subset while the
open items resolve.

---

## 1. Outcome table

| Cluster | Ask | Outcome | Note |
|---|---|---|---|
| A | 1. Separate `prices` DB + dedicated user | **yes** | |
| A | 2. SNS topic between S3 and Lambdas | **yes** | |
| A | 3. Announcement-not-approval DDL norm | **yes** | |
| B | 4. Hardware specs + monthly cost | **accepted** | Accepted on the basis of task 0046's empirical calculation; cost estimation flows from there. |
| B | 5. Caddy `max_keepalive_conns` headroom | **yes** | |
| B | 6. BE ADR 0006 retention confirmed | **TBD — spawns task 0047** | Big risk: can shared Hetzner CH+Caddy API serve combined BE + prices-api request volume fast enough? Verify before committing. |
| B | 7. Daily `BACKUP DATABASE prices` Borg target | **yes** | Open question: do we need daily, or is a longer cadence (e.g. weekly) sufficient given the <1 GB/yr footprint? To revisit. |
| B | 8. Daily RPO acceptable (heads-up) | **yes** | |
| C | 9. Per-env client certs via BE script | **yes** | |
| C | 10. 1-year manual rotation cadence | **yes** | |
| C | 11. Revocation = CA rotation | **yes** | |
| D | 12. Cost-share number | **TBD** | More estimation needed for combined DB size projection and Hetzner storage-cost basis (BE side + prices side together). Pending. |
| D | 13. Money-movement mechanism | **yes** | |

**Summary:** 10 yes, 0 no, 3 TBD. The architectural shape (Cluster A
all-yes) is locked in. Auth (Cluster C all-yes) is locked in. Two of
five Cluster B asks are locked in; one spawns a follow-up; one needs
a cadence call. Cluster D money is half-locked; the number is still
open.

---

## 2. What this changes

### 2.1 Locked-in commitments

The 10 yes-answered asks unblock the **architectural commitment** in
ADR 0007 conceptually, but the ADR stays in `proposed` status until
the 3 TBDs resolve (specifically B6, since a "no" or
"with-conditions" answer there would force the sidecar-CH fallback).

| Commitment | What unblocks |
|---|---|
| Separate `prices` DB + user | Task 0011 rewrite can spec the CDK migration applier + Secrets Manager certs against a concrete DB target |
| SNS topic on S3 bucket | Task 0038 rewrite can spec the Lambda subscription shape |
| DDL announcement norm | Schema-applier task (0044 §5.3) can ship as prices-api-owned tooling |
| Caddy keepalive headroom | Task 0038 + 0039 can plan their batched HTTP write rate without re-tuning Caddy |
| Per-env certs + 1y rotation + CA-revocation | Task 0011 rewrite can spec the Secrets Manager mTLS material shape + the CloudWatch NotAfter alarm |
| Daily Borg backup + RPO | Operational runbook can plan against daily RPO; no extra tooling needed on prices-api side |
| Money mechanism (yes) | When the number lands (D12 TBD), the flow is already agreed |

### 2.2 Spawned follow-up: task 0047

**Why:** The Cluster B6 concern is the only "no, with verification
needed" answer. BE flagged the risk that the shared Hetzner CH host
behind a single Caddy:443 endpoint may not absorb the combined
read/write load of both tenants under realistic peak conditions. The
empirical task 0046 measured storage and row volume; it did **not**
measure connection-layer throughput, concurrent query load on CH,
or MV-chain CPU contention.

Task 0047 will measure throughput under simulated combined load and
confirm (or revise) the architecture. Until 0047 closes, ADR 0007
stays `proposed` and the implementation tasks stay blocked.

### 2.3 Open question: B7 — backup cadence

BE accepted the `BACKUP DATABASE prices` Borg target ask (yes), but
flagged a follow-up: given the empirical <1 GB/yr footprint, is daily
Borg actually needed, or would weekly suffice? The cost difference is
trivial; the RPO difference (1 day vs. 7 days of replay) matters more.

**Recommendation (not yet agreed with BE):** keep daily. Argument:
prices-api's replay-from-S3 path takes ~minutes per day of mainnet
data; recovering 7 days of lost OHLCV via replay is operationally
unpleasant. Daily Borg is the cheap safety net. Revisit if BE pushes
back further.

### 2.4 Open question: D12 — cost-share number

BE wants more estimation before committing to the number. Specifically
the user note flags "more estimation needed for db size and hetzner
storage costs" — likely meaning BE wants a measured (not extrapolated)
fraction once their `default.*` data lands on the production box, and
possibly a re-validated Hetzner tier-pricing reference.

**Path forward:** wait until BE's CH is live on the production box for
~1-2 weeks, query `system.parts` for BE's actual storage footprint,
re-compute the empirical share against measured (not estimated)
denominators. Until then, the brief's stance (open at ~1-2% / $1-2/env
flat, re-open clause at 10× scale) holds as the proposal.

---

## 3. Risks introduced by the responses

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Task 0047 finds shared CH+Caddy can't sustain combined load | Medium | High | Fallback to Option 4 sidecar CH (per task 0044 I-note §4); ADR 0007 would shift to that path |
| Cost-share number lands materially higher than the empirical 1-2% pro-rata | Low | Low | Brief's flat-fee ceiling ($5/env) absorbs most realistic outcomes; >$10/env triggers fallback re-eval (per brief §3.4) |
| BE flips B7 to weekly Borg and we accept | Low | Low | Replay path covers the gap; ~minutes per day to recompute OHLCV from S3 |

---

## 4. State transitions triggered by this record

When this record lands on develop, the following should follow:

1. **Spawn task 0047** — `cross-tenant-throughput-verification-on-shared-hetzner-ch` (RESEARCH). Backlog initially; activates when BE's CH is live enough to load-test against.
2. **Task 0045 stays blocked** — re-target the `by:` from `[0046]` to `[0047]` once 0047 lands in backlog. (0046 is closed.)
3. **ADR 0007 stays `proposed`** — gate on 0047 closing successfully (i.e. throughput verified).
4. **Brief's §7 outcome-tracking table** — superseded by this record. The brief is the sent artifact; this is the response.

---

## 5. Verbatim BE responses (for traceability)

The original outcomes as supplied by the team contact (preserved
verbatim for the historical record — paraphrased only where the
matrix cells were shorter than the user's prose):

> | Cluster | Ask | Outcome | Note |
> |---|---|---|---|
> | A | 1. Separate `prices` DB + dedicated user | yes | |
> | A | 2. SNS topic between S3 and Lambdas | yes | |
> | A | 3. Announcement-not-approval DDL norm | yes | |
> | B | 4. Hardware specs + monthly cost | — | accepted as task 0046 calculated and money estimated cost |
> | B | 5. Caddy `max_keepalive_conns` headroom | yes | |
> | B | 6. BE ADR 0006 retention confirmed | TBD | the big risk is if the Hetzner API will be able to cover all API requests from both BE and prices-api projects. Create new RESEARCH task to verify if the API is fast enough |
> | B | 7. Daily `BACKUP DATABASE prices` Borg target | yes | to consider if daily target is needed or can be longer period eg 1 week |
> | B | 8. Daily RPO acceptable (heads-up) | yes | |
> | C | 9. Per-env client certs via BE script | yes | |
> | C | 10. 1-year manual rotation cadence | yes | |
> | C | 11. Revocation = CA rotation | yes | |
> | D | 12. Cost-share number | tdb | more estimation need to be done for db size and hetzner storage costs |
> | D | 13. Money-movement mechanism | yes | |
