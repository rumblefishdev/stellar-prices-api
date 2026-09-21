---
id: "0285"
title: "prices.pool_registry does not match what is actually trading — 72 of 178 swap-emitting contracts are unregistered, and they carry 96.7% of the swap events"
type: RESEARCH
status: completed
assignee: okarcz
related_adr: []
related_tasks: ["0282", "0101", "0096", "0097", "0099", "0080"]
tags: [layer-indexing, priority-medium, effort-medium, amm, soroswap, phoenix, ingestion, data-correctness, clickhouse]
links:
  - "../../active/0282_BUG_aquarius-live-ingestion-drops-half-its-trades/README.md"
  - "../../blocked/0101_FEATURE_live-era-amm-reprice-gap/README.md"
history:
  - date: 2026-09-15
    status: backlog
    who: okarcz
    note: >
      Found while trying to size [[0282]]'s damage for soroswap and phoenix. The
      raw-vs-stored instrument that works exactly for aquarius returned nonsense
      for them — 27,897 trades STORED against 6,942 raw — which is impossible if
      the registry described what trades. It does not: over the live era 178
      distinct contracts emit `swap` and only 106 are in `prices.pool_registry`.
      Filed as RESEARCH, not BUG: the unregistered contracts have NOT been
      classified, and the two plausible readings (pools we never index vs
      routers/aggregators reusing the event name) have opposite fixes — one is
      missing data, the other would be double-counting.
  - date: 2026-09-17
    status: active
    who: okarcz
    note: >
      Activated while 0282's live fix waits for its full-day check and 0286
      waits for its phase 1-2 rollout. 0286's phase-3 runbook now names this
      task as a precondition for the AMM side of the live-era months, so the
      reverse question (does live write candles for unregistered pools?) is
      the one that gates the most.
  - date: 2026-09-17
    status: active
    who: okarcz
    note: >
      Classified. 06F4207B is the Aquarius router (already filtered, must stay
      out). 003710B3 + 95A8E001 are a Uniswap-v3-style pool family with its own
      factory — 16,933 swaps never indexed. The other routers only repeat pool
      swaps. The stored-exceeds-raw contradiction was the instrument: the
      signature column is NULL for most string-topic events, so soroswap really
      has 51,589 swaps (43.8% lost) and phoenix ~4,194 (~12% lost). And the
      registry is stale since the backfill's 2026-07-06 end: 22 aquarius pools
      (34,684 trades) and 10 soroswap pools are missing, and live forgets them
      on every cold start — which answers 0286's precondition 9 with YES.
      Spawned 0290 (new venue) and 0291 (registry refresh).
  - date: 2026-09-21
    status: active
    who: okarcz
    note: >
      The last open measurement is answered, alongside 0282's step 8. Live
      stored NONE of the unregistered pools' trades — zero, not a fraction.
      On 2026-09-18, hour by hour, lost equals the not-yet-registered pools'
      raw count EXACTLY in all eight hours before 0291's 08:00 UTC seed, and
      stored equals the registered pools' count exactly; after the seed, zero
      loss for fifteen hours. So an unregistered pool is invisible rather than
      under-counted. Also recorded here as a denominator gotcha: raw resolves
      pool_registry as of now, so any window spanning a seed shows a phantom
      loss. 0100 rescoped the same day into the recurring sweep that would
      have caught 0290's venue without a human going looking.
  - date: 2026-09-21
    status: completed
    who: okarcz
    note: >
      CLOSED. All 7 criteria met. The last one was answered this morning
      alongside 0282's step 8: live stored NONE of the unregistered pools'
      trades — an unregistered pool is INVISIBLE, not under-counted — proven
      hour by hour against 0291's 08:00 UTC seed, where lost equals the
      not-yet-registered pools' raw count exactly in all eight hours before it
      and zero for the fifteen hours after. Criterion 7 closed the same day by
      rescoping 0100 into a recurring coverage sweep. Three follow-ups spawned
      and live: 0290, 0291, 0100. Longest-reaching finding: pool_registry is
      not fit as a raw-vs-stored denominator as-is, which 0282 and 0101 both
      depend on.
---

# The pool registry does not describe what is actually trading

## 📊 STATUS — 2026-09-21 · ✅ COMPLETED, all 7 criteria met

Findings in [notes/S-classification-2026-09-17.md](notes/S-classification-2026-09-17.md).

- ⛔ **The headline below is overstated** — it counted swaps with
  `signature = 'swap'`, which misses most Soroswap and Phoenix swaps (string
  topics leave `signature` NULL).
- **Router `06F4207B`** = Aquarius router → already ignored, keep it that way.
- **Pool family `003710B3` / `95A8E001`** = Uniswap-v3-style venue we do not
  index → 16,933 swaps missing → [[0290]].
- **Registry stale since 2026-07-06** → 22 Aquarius + 10 Soroswap pools
  missing, dropped by live after cold starts → [[0291]]. Must be fixed before
  [[0286]] phase 3 reaches 2026-07.
- ✅ **ANSWERED 2026-09-21: live stored NONE of them — zero, not a fraction.**
  Measured on 2026-09-18, the day [[0291]] seeded them at 08:00 UTC. Split by
  hour and by `pool_registry.updated_at`, `lost` equals the unregistered pools'
  raw trade count **exactly** in all eight hours before the write (63, 114,
  262, 834, 53, 36, 143, 58) and `stored` equals the registered pools' count
  exactly — then zero loss for the fifteen hours after. So an unregistered pool
  is not under-counted, it is **invisible**, and the moment it is registered it
  is complete. Full table in [[0282]]'s runbook step 8.

## Summary

`prices.pool_registry` is the set of AMM pools we believe exist. Measured on
production 2026-09-15 over the live era (ledgers 63,494,982 → 64,439,313,
2026-07-16 → 09-15), it does **not** match the set of contracts actually
emitting swap events, and the mismatch is not marginal — **96.7% of all `swap`
events in the window come from contracts the registry has never heard of.**

This is **not** [[0282]]. 0282 is about candle writes replacing each other after
extraction. This is about swaps that may never reach extraction at all. It was
found *because* 0282's sizing work needed a raw-vs-stored number for soroswap
and phoenix and could not get a coherent one.

⚠️ **Filed as RESEARCH deliberately.** The unregistered contracts are
unclassified, and the two readings have opposite consequences:

- **pools we do not index** → we are silently missing their trades entirely,
  and the estate is short by more than 0282 accounts for;
- **routers / aggregators** that emit `swap` while the underlying pool emits its
  own event → adding them would **double-count** every trade they touch.

Do not act on the counts below until they are classified.

## Evidence — production, `dev_read`, 2026-09-15

### The instrument fails in a way that proves the premise

The comparison that reproduces [[0282]]'s aquarius numbers to the row returns an
impossible result for the other two venues:

| venue | raw (registry-joined) | stored `trade_count` | reading |
| --- | --- | --- | --- |
| aquarius | 659,365 | 346,915 | coherent — 47.4% lost |
| soroswap | 6,942 | **27,897** | ⛔ stored 4x the raw |
| phoenix | 2,960 | **3,602** | ⛔ stored exceeds raw |

Stored cannot exceed raw. Either the registry-joined raw is an undercount, or
the live path writes candles for pools the registry does not contain — and both
of those are this task.

### Registered vs actually trading

| venue | registered | registered AND traded in the window |
| --- | --- | --- |
| aquarius | 488 | 221 |
| soroswap | 221 | 99 |
| phoenix | 19 | 7 |

(Registered-but-idle is normal and not a problem. The problem is the other
direction.)

### 🔑 The other direction: who emits `swap`

```
contracts emitting `swap`, live era:   178
  ... of which in pool_registry:       106   (99 soroswap + 7 phoenix)
  ... unregistered:                     72

swap events, live era:             296,413
  ... from registered contracts:     9,902   (3.3%)
  ... from unregistered contracts: 286,511   (96.7%)
```

### Grouped by contract code — this is what makes it classifiable

`wasm_hash` separates "a pool family" from "one busy contract" cleanly:

| wasm_hash (prefix) | contracts | swap events | registered as | reads like |
| --- | --- | --- | --- | --- |
| `06F4207B` | **2** | **258,809** | — | ⚠️ 2 contracts, 87% of all swaps — a **router/aggregator**, almost certainly not a pool |
| `003710B3` | **46** | 16,214 | — | ⚠️ 46 contracts sharing one wasm — reads like a **pool family we do not index at all** |
| `18051456` | 105 | 6,983 | soroswap | ✅ soroswap's pool wasm |
| `4EDD745F` | 1 | 5,792 | — | single busy contract |
| `4C3DB3EB` | 1 | 3,109 | — | single busy contract |
| `167AB414` | 6 | 2,400 | phoenix | ✅ phoenix pool wasm |
| `9677074A` | 1 | 1,727 | — | single busy contract |
| `95A8E001` | 4 | 720 | — | small family |
| `F74D87D7` | 1 | 560 | phoenix | ✅ second phoenix wasm version |

🔑 **The shape does most of the work.** The two dominant unregistered groups are
qualitatively different: `06F4207B` is 2 contracts doing 258k swaps (a routing
contract's signature — enormous volume, no pool count), while `003710B3` is 46
contracts sharing a code hash (a pool family's signature). They almost certainly
need opposite treatment.

## Implementation

- **Classify `06F4207B` first** — it is 87% of the volume and the answer decides
  whether this task is about missing data or about avoiding double-counting.
  Read the contract's interface (`default.wasm_interface_metadata`,
  `soroban_contract_metadata`) and inspect a sample event's topics against the
  known venue shapes ([[soroban-events-gotchas]] — swap has 3 shapes, and
  soroswap's action is in `topic[1]`, not `topic[0]`).
- **Then `003710B3` (46 contracts).** If it is a pool family, identify the venue
  and decide whether it belongs in `pool_registry` — that is a new extractor, so
  scope it as its own task rather than growing this one.
- **Establish how the registry is populated today and why it drifts.** Soroswap
  and phoenix are factory-discovered ([[amm-historical-pool-discovery-gap]],
  [[amm-live-pool-registry-preload-gap]]); aquarius is seeded. A registry that
  only grows at deploy time will always trail a live chain.
- **Check the reverse direction too** — whether the live path writes candles for
  pools absent from `pool_registry`, which is one of the two explanations for
  stored-exceeds-raw above. The live processor builds its own registry view;
  if it is *broader* than the table, then the table is the wrong instrument for
  every measurement anyone makes with it, including [[0101]]'s.
- ⚠️ **All 488 aquarius and all 19 phoenix registry rows have empty
  `token0`/`token1`** (only soroswap's 221 are populated). Recorded in [[0282]]
  as not causing that defect — the tokens ride in the event topics — but it is
  more evidence the table is not maintained as a source of truth.

## Acceptance Criteria

- [x] `06F4207B` (2 contracts, 258,809 swaps) is classified: pool, router, or
      something else — with the evidence that settles it. → **Aquarius router**
      (interface + 87.6% co-occurrence with registered Aquarius `trade`s, the
      rest with unregistered Aquarius pools).
- [x] `003710B3` (46 contracts, 16,214 swaps) is classified the same way. →
      **Uniswap-v3-style pool family** (constructor + event shape), factory
      `CD3KRKGD…GLYF`; second code version `95A8E001`. Venue name still unknown.
- [x] The remaining unregistered emitters are classified or explicitly dismissed
      as immaterial, with their event counts. → three routers (100%
      co-occurrence with pool swaps), the rest ≤ 36 events each. See the note.
- [x] A statement of whether we are **missing trades**, at risk of
      **double-counting**, or neither — and how many trades that is worth. →
      **missing:** 16,933 v3-style swaps + 34,684 Aquarius trades / 267 Soroswap
      swaps from unregistered pools (at risk after cold starts); **no
      double-counting today.**
- [x] The stored-exceeds-raw contradiction for soroswap and phoenix is explained.
      → the `signature` column, not extra pools. Corrected: soroswap 51,589 raw
      vs 28,980 stored (43.8% lost), phoenix ~4,194 vs 3,675 (~12%).
- [x] It is recorded whether `prices.pool_registry` is fit to be used as the
      denominator in raw-vs-stored measurements, since [[0282]] and [[0101]]
      both rely on it. → **not as-is**: stale since 2026-07-06, and
      `signature` cannot count string-topic venues.
- [x] Any follow-up work (a new venue extractor, a registry-refresh mechanism)
      is spawned as its own task rather than absorbed here. → [[0290]],
      [[0291]] spawned 2026-09-17; [[0100]] rescoped 2026-09-21 from a one-time
      triage into the recurring coverage sweep that would have caught [[0290]]'s
      venue on its own. ✅ The measurement this was held open for was taken
      2026-09-21 — see the STATUS bullet: live stored **none** of the
      unregistered pools' trades.
      🔑 **A gotcha this task should own**, since it is about the registry as a
      denominator: `raw` resolves `pool_registry` as of *now*, so a raw-vs-stored
      window that spans a seed reports a phantom loss for its pre-seed hours.
      That is what 09-18's 6.6% is. Measure after a seed, or pin the registry to
      the measured day.

## Implementation Notes

A RESEARCH task, so what it produced is answers, not code. All seven criteria
are met; the classification is in
[notes/S-classification-2026-09-17.md](notes/S-classification-2026-09-17.md).

What it settled, in the order it mattered:

1. **The headline was wrong, and why** — 96.7% counted only
   `signature = 'swap'`, and string-topic venues leave `signature` NULL. The
   corrected figures are soroswap 51,589 raw vs 28,980 stored (43.8% lost) and
   phoenix ~4,194 vs 3,675 (~12%).
2. **Every unregistered emitter is classified** — the Aquarius router
   (`06F4207B`), a whole unindexed venue (`003710B3`/`95A8E001`), three more
   routers, and a tail of ≤ 36 events each.
3. **`pool_registry` is NOT fit as a raw-vs-stored denominator as-is** — stale
   since 2026-07-06, and `signature` cannot count string-topic venues. This is
   the finding with the longest reach: [[0282]] and [[0101]] both lean on it.
4. **An unregistered pool is INVISIBLE, not under-counted** (2026-09-21). Live
   stores *none* of its trades. Proven hour by hour against [[0291]]'s seed —
   see the STATUS bullet.

## Design Decisions

### From Plan

1. **Filed as RESEARCH, not BUG.** The two readings — pools we never index vs
   routers double-counting — have opposite fixes, so classifying had to come
   before any repair. That judgement held: the answer was *both*, and each half
   went to a different task.

### Emerged

2. **The stored-exceeds-raw contradiction was treated as an instrument, not an
   error.** 27,897 stored against 6,942 raw is impossible, so rather than
   discard it the impossibility was used to find the defect in the measurement
   — the `signature` column. That is what corrected the headline.
3. **Follow-ups were spawned rather than absorbed** ([[0290]], [[0291]]), and
   the task then stayed open only for the one measurement it could not take
   until the live fix had run. Keeping it open for a single question, instead
   of closing with a caveat, is what made criterion 7 answerable at all.
4. **[[0100]] was rescoped rather than left as filed** (2026-09-21). Its
   one-time triage of 144 emitters became a recurring coverage sweep, because
   this task is the second time the same question was asked from scratch — and
   0100's own probe had already found the answer in July.

## Issues Encountered

- **`dev_read`'s hourly 2 TiB quota** was exhausted in under an hour by
  whole-era scans that parse `topics_xdr` (`Code: 201 QUOTA_EXCEEDED`, resets
  on the clock hour); single queries also cap at 30 s. Chunk by ~320k ledgers
  and filter on `signature`/`contract_id` before parsing JSON.
- **`signature` is NULL for string-topic events**, which is what made the
  original headline wrong. Any venue-spanning count must not filter on it.
- **A registry seed inside the measured window fabricates loss.** `raw`
  resolves `pool_registry` as of *now*, so hours before a seed report the
  newly-seeded pools as lost. Found on 2026-09-18; see the criterion 7 note.

## Future Work

All spawned; nothing left as prose.

- [[0290]] — index the unindexed venue (SushiSwap V3). Blocked on [[0286]].
- [[0291]] — make the live processor maintain the registry. Blocked on [[0286]].
- [[0100]] — rescoped 2026-09-21 into the recurring coverage sweep that would
  have surfaced 0290's venue without anyone going looking.
- ⏳ **Not spawned, and deliberately:** [[0282]]'s "a swap from a pool the live
  path cannot resolve leaves no trace" criterion. It is 0291's
  `UnregisteredPoolEvents`, which is built and alarmed but not yet deployed —
  so it belongs to 0291's AC 3, not to a new task.

## Notes

- ⛔ **Nothing here changes [[0282]]'s aquarius numbers.** Aquarius is measured
  through `signature = 'trade'` on 488 registered contracts and its raw counts
  reproduce 0282's own table exactly. This task is about soroswap, phoenix, and
  whatever the unregistered contracts turn out to be.
- ⚠️ **[[0101]] should not run until at least the first two criteria are
  answered.** It reprices soroswap and phoenix over this same live era using
  `pool_registry` as its pool set, so if that set is wrong its output is wrong
  in the same direction — and its acceptance criteria are stated in terms of
  counts drawn from it.
- The signatures differ by venue and it matters for every query here: aquarius
  emits `trade`, soroswap and phoenix emit `swap`. A single-signature filter
  silently returns one venue's data.
