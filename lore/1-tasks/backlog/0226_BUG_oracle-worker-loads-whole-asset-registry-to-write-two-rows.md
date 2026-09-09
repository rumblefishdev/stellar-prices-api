---
id: "0226"
title: "The oracle worker loads all 620,615 assets into memory to write 2 rows, sits at its 256 MB ceiling and OOMs several times a day"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0223", "0222", "0167", "0112", "0132"]
tags: [layer-infra, priority-low, effort-medium, oracle, lambda, memory, observability, ops]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/writer.rs"
  - "../../../packages/oracle-worker/src/main.rs"
  - "../../../infra/src/lib/stacks/eventbridge-stack.ts"
  - "../../../infra/src/lib/lambda-baseline.ts"
history:
  - date: 2026-08-26
    status: backlog
    who: okarcz
    note: >
      Found by investigating two Slack pages on 2026-08-26 06:33/06:38 UTC from
      `prices-production-oracle-errors`. Cause is `Runtime.OutOfMemory` — the
      function is provisioned at 256 MB and peaks at exactly 256 MB. NOT new:
      it has clipped the ceiling on every day sampled back to 2026-08-14, at
      roughly 1-7 failures a day.
      ⚠️ The obvious remedy — bump `memorySize` to 512 — was costed (~$0.22/mo,
      negligible) and **deliberately declined as the fix**. It buys headroom
      against a design that will consume it again; the registry only grows. This
      task is the design question, not the bump. Raising memory as a temporary
      measure is still available if the noise becomes intolerable before this is
      scheduled, but it is a stopgap and must be recorded as one.
  - date: 2026-08-27
    status: backlog
    who: okarcz
    note: >
      ⚠️ Scope input from [[0227]]'s prod verification, and it may change this
      task's answer. This task's premise is that the POLL write is **wholly
      redundant** — but with 0227 deployed the two writers now collide on the
      `ReplacingMergeTree` key (`oracle_prices` has no version column, both use
      `oracle_name = 'reflector'`, and both derive the same 5-minute
      observation), so the surviving row is arbitrary and `raw_data` no longer
      discriminates them. Measured within minutes of the deploy: the same key
      read as EVENT at 14:52 and as POLL shortly after, and a three-hour census
      finds NO key where both survive. On top of that the two arms may disagree
      on price — over 24 h on asset 3, the event writer ranges
      0.999900009999-1.00019049063001 with 22/285 exactly `1`, while both
      surviving poll rows are exactly `1`. n = 2, so this is a question, not a
      finding; but if it holds, "redundant" becomes "conflicting" and the value
      `usd_rate` snapshots is a coin flip. Settle it with the distribution query
      recorded in 0227 once a few dozen poll rows have survived.
---

# The oracle worker reads the entire asset registry on every run

## Summary

`prices-production-oracle` runs every 5 minutes and writes **2 rows**. To do it,
it pulls **all 620,615 rows** of `prices.assets` into memory. That puts peak
usage at the function's entire 256 MB allocation, and a few times a day a run
tips over and dies with `Runtime.OutOfMemory`.

The oracle is non-critical by design — it degrades to last-known value — so the
data impact is small. The cost is a steady drip of failures and Slack pages for a
condition nobody is acting on.

## Evidence — measured 2026-08-26

The failing run and the one before it, from `/aws/lambda/prices-production-oracle`:

```
06:27:30  loaded asset registry from ClickHouse   existing_assets=620615
06:27:36  oracle-worker run complete   queried=2  written=2  rates_snapshotted=1
06:27    REPORT  Duration: 7629 ms   Max Memory Used: 252 MB / 256 MB     ← survived

06:32:30  loaded asset registry from ClickHouse   existing_assets=620615
06:32    REPORT  Duration: 10705 ms  Max Memory Used: 256 MB / 256 MB
                 Status: error   Error Type: Runtime.OutOfMemory          ← died
```

🔑 **620,615 rows loaded to write 2.** The successful run cleared the limit by
4 MB. There is no headroom, and the run that failed had done nothing else yet —
it never reached `run complete`.

### It has been at the ceiling for at least two weeks

`Max Memory Used` sampled from one hour of `REPORT` lines per day:

| day | runs sampled | memory used |
|---|---|---|
| 2026-08-14 | 36 | 236 – **256** MB |
| 2026-08-18 | 13 | 234 – **256** MB |
| 2026-08-22 | 12 | 187 – 248 MB |
| 2026-08-26 | 13 | 199 – **256** MB |

Daily `Errors`, 2026-08-15 → 08-26: `5, 5, 4, 6, 3, 3, 6, 7, 3, 1, 4, 1` —
roughly flat, ~1.4% of invocations (4 errors / 284 invocations in 24 h).

⚠️ 2026-08-13 and 08-14 show **134** and **267** errors. Those coincide with the
Hetzner disk-full incident and are almost certainly a different cause; they were
**not** individually verified. Do not read them as part of this trend.

7-day baseline: **2,043 invocations**, average duration **7.71 s**, maximum
**61.1 s** against a 120 s timeout.

## The mechanism

`packages/prices-ingest-core/src/writer.rs:73` — `load_assets()`:

```sql
SELECT asset_id, asset_code, issuer_address, contract_address FROM prices.assets
```

Unbounded, `fetch_all`, materialised into a `Vec<(u32, AssetIdentity)>` with an
owned `String` per code/issuer/contract. `oracle-worker/src/main.rs:24` builds an
`OhlcvWriter` and the load happens on the run path — the log line appears in both
of the separate invocations above, so it is at minimum recurring, not paid once
at cold start.

The purpose is legitimate: reuse surrogate `asset_id`s rather than reassigning
them. The defect is the **granularity** — a run that touches 2 identities reads
all 620,615.

⚠️ **`load_assets()` is shared code.** Confirm which other callers depend on the
whole-registry form before narrowing it; the ledger-processor and the backfills
have very different access patterns and a change here is not oracle-local. Same
class of trap as [[0132]]'s re-emit amplification — cheap per call, ruinous in
aggregate.

## Why the memory bump was declined as the fix

Costed properly so the decision is on record rather than on instinct. ARM64,
eu-central-1, 8,755 invocations/month at 7.71 s average:

| | 256 MB | 512 MB |
|---|---|---|
| GB-seconds/month | ~16,900 | ~33,700 |
| compute cost | ~$0.22/mo | ~$0.45/mo |

**Delta ~$0.22/month** — and pessimistic, since Lambda scales CPU with memory
(0.14 → 0.29 vCPU) so any CPU-bound portion shortens.

🔑 **It was not declined on cost. It was declined because it is not a fix.** The
registry grows monotonically; 512 MB postpones the same failure. The operator's
call, 2026-08-26: do not spend headroom to hide a design problem.

## Implementation

- Establish first whether the load is **per-invocation** or **per-cold-start**.
  Both observed log lines sit in different invocations, which bounds it from
  below but does not settle it. This changes the remedy's shape, so measure it
  before choosing.
- Options to cost:
  1. **Look up only what the run needs.** The oracle resolves a handful of
     identities; a keyed query beats a full scan. Most direct, but touches shared
     code — see the caller warning above.
  2. **A narrower loader for small consumers**, leaving `load_assets()` intact
     for the backfills that genuinely need the whole map.
  3. **Cache the map across invocations** in the warm container. Reduces
     frequency, not peak — the OOM happens *during* the load, so this may not
     help at all. Verify against the per-invocation question above.
- ⚠️ Whatever ships, the verification is **`Max Memory Used` on real
  invocations**, not a local benchmark. The whole point is behaviour at 620k rows
  and that number is production's.

## ⚠️ Deploy hazard — this is the expensive part, not the change

`memorySize` lives in `eventbridge-stack.ts:415`. CDK declares
`prices-production-cleanup` as **ENABLED** while production has it **DISABLED**,
so any EventBridge deploy can silently re-enable it — and cleanup running during
a backfill deletes output as fast as it is written. That cost 5 days once.

🔑 `aws events describe-rule --name prices-production-cleanup` **before and
after** every deploy of this stack. A code-only change to the worker crate does
not carry this risk; a `memorySize` change does.

## 🔴 Finding that belonged to [[0223]] — corrected there, same PR

Checking whether the oracle's alarm noise was 0223's problem exposed a scope
error in 0223 itself. The oracle's `-errors` alarm is not built where 0223 said
`-errors` alarms are built.

✅ **Applied to 0223 on 2026-08-26**, in the PR that spawned this task. Summary,
scope bullets and acceptance criteria corrected there; full detail lives in 0223
under "Where these alarms actually live" rather than being duplicated here.

The short version: **three** builders, not one — `addWorkerHealthAlarms` builds
no `-errors` alarm at all, `createWorkerLambda` (`lambda-baseline.ts:309`) builds
9 of them at `1/1`, and `ledger-processor-errors` is hand-rolled a third time.
0223 as originally scoped would have fixed the duration half and left every
`-errors` alarm untouched.

⚠️ **Same trap [[0222]] hit** with the hand-rolled `ledger-processor-no-invocations`
— here it recurs twice over. Nothing further is owed to 0223 from this task.

## Out of scope

- **Raising `memorySize`.** Explicitly declined as the fix; see above. Available
  as a stopgap only, and only if recorded as one.
- The alarm's `1/1` sensitivity and its two-messages-per-blip behaviour. Both
  messages are deliberate ([[0112]] wired the OK action so a recovered worker
  does not fall silent), and the noise likely disappears once the OOM does. If it
  does not, that is a separate question.
- [[0223]]'s framing decision itself — only the scope gap above is recorded here.

## Acceptance Criteria

- [ ] Whether the registry load is per-invocation or per-cold-start is
      **measured**, not inferred from the two log lines above.
- [ ] The remedy reduces **peak** memory on a real production invocation, with
      `Max Memory Used` recorded before and after.
- [ ] Every other caller of `load_assets()` is enumerated, and the change is
      shown not to alter their behaviour — or the narrowing is confined to a new
      entry point.
- [ ] `prices-production-oracle-errors` shows **zero** `Runtime.OutOfMemory` for
      a full week after the change.
- [ ] If the deploy touches `eventbridge-stack.ts`, `describe-rule` output for
      `prices-production-cleanup` is recorded **before and after**.

## Notes

Found from a live page rather than an audit. The two Slack messages on
2026-08-26 (`ALARM` 06:33:31, `OK` 06:38:31) were one event: a single failed run,
self-recovered by the next 5-minute invocation.

⚠️ The alarm behaved **correctly** here. It is not an instance of [[0222]]'s or
[[0223]]'s failure modes — it detected a real error and cleared when the error
stopped. Recorded because "the alarm was right" is the easiest thing to lose when
the surrounding pages are about alarms being wrong.
