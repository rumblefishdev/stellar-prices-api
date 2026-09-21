---
id: "0100"
title: "A recurring coverage sweep over unregistered swap emitters — the only layer that catches a venue we have never seen"
type: FEATURE
status: active
assignee: akot
related_adr: []
related_tasks: ["0097", "0079", "0078", "0285", "0290", "0291"]
tags: [layer-indexing, priority-high, effort-medium, amm, clickhouse, pool-registry, coverage, observability]
links:
  - "../../../packages/events-backfill/src/source.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: "2026-07-17"
    status: backlog
    who: okarcz
    note: >
      Spawned from 0097 future work. 0097's dry-run coverage probe found 144
      contracts OUTSIDE prices.pool_registry emitting 2.25M swap/trade-shaped
      events over [50457424, 63352611]. Confirmed NOT Soroswap (every
      SoroswapPair event in range comes from a registered pool). Unknown
      whether they are unregistered aquarius/phoenix pools (real volume we are
      losing) or non-AMM contracts (correctly ignored).
  - date: "2026-09-21"
    status: backlog
    who: okarcz
    note: >
      RESCOPED from a one-time triage of 144 contracts into a RECURRING swept
      alarm, and raised to priority-high. The reason is [[0290]]: SushiSwap V3
      traded 88,689 swaps from 2026-01 that never reached a candle, and it was
      almost certainly inside this task's own 144. The probe fired in July and
      the triage sat in backlog for two months — so the defect this task now
      fixes is that the sweep is a thing someone runs by hand during an
      unrelated investigation, not a thing that pages. [[0291]] closed the
      adjacent hole (known venue, unknown pool) with `UnregisteredPoolEvents`;
      this is the only remaining layer that can catch venue number five.
      The original 144-contract triage survives as phase 1.
  - date: "2026-09-21"
    status: active
    who: akot
    note: >
      Activated. Sweep query validated on production (dev_read) over the 14
      days to ledger 64,541,178: 9.6 s, 46.6 GB read, 53 unregistered
      emitters in 18 wasm families. Decisions D1–D4 recorded below.
---

# A recurring coverage sweep over unregistered swap emitters

## Summary

We can only price a swap from a pool that is already in `prices.pool_registry`,
and we only learn pools from factories we have registered. A venue nobody has
registered is therefore invisible — not dropped with an error, just absent.
This task makes the "what is trading that we cannot account for?" question a
**recurring, alarmed job** instead of a probe someone runs by hand.

## Context

### The three layers, and which one is missing

| # | Hole | Caught by | State |
| --- | --- | --- | --- |
| 1 | Known venue, known pool | the extractors | ✅ works |
| 2 | Known venue, **unknown pool** | `UnregisteredPoolEvents` metric + alarm | ✅ [[0291]], live |
| 3 | **Unknown venue entirely** | nothing | ⛔ this task |

Layer 2 works by recognising event *shapes* — `unregistered_pool_venue`
(`packages/prices-ingest-core/src/soroban.rs:639`) matches `"trade"` + two
addresses as Aquarius, `"swap"` + `amount0`/`amount1` as SushiSwap, and so on.
It cannot be extended to cover layer 3, because **a matcher cannot be written
for a shape nobody has seen yet.** That is pinned deliberately by the test
`routers_and_unindexed_venues_are_not_counted_as_unregistered_pools`
(`soroban.rs:1291`) — an unindexed venue is *by design* not counted there.

So layer 3 has to be answered from the outside in: not "does this event look
like something I know?" but "here is every contract emitting trade-shaped
events — which ones can I not account for?"

### This has already fired once, and nobody was listening

- **2026-07-17** — 0097's dry-run probe reported **144 contracts outside
  `prices.pool_registry` emitting 2,252,506 swap/trade-shaped events** over
  `[50457424, 63352611]`. Filed as this task. Never triaged.
- **2026-09-15** — [[0285]] asked the same question from scratch and found
  **72 of 178 swap-emitting contracts unregistered** over the live era.
- **2026-09-18** — [[0290]] classified one of them as **SushiSwap V3**:
  88,689 swaps since 2026-01, 99 pools, a whole venue we had never heard of.

The detection existed in July. The follow-through did not. **That gap, not the
missing matcher, is the root cause** — and a one-time triage would leave it
exactly where it was.

### Why the output has to stay small

144 rows is a number nobody opens. The sweep is only sustainable if its
residual is a handful, which means the *known-ignorable* set has to be an
explicit, committed artefact rather than knowledge living in a matcher and in
task notes:

- the Aquarius router (`06F4207B`) emits a `swap` summary wrapping the
  pool-level `trade` (task 0087) — correctly ignored;
- SushiSwap's two routers (`CDMIM23W…`, `CAUF4DFY…`) emit the *identical*
  `[Symbol("swap")]` topic as its pools and differ only in the data
  ([[0290]]) — indexing them would double-count;
- aggregators and non-AMM contracts that merely reuse the names `swap` /
  `trade`.

## Implementation

### Phase 1 — clear the standing backlog (the original 0100)

- Resolve every unregistered emitter to a strkey via
  `default.soroban_contracts` and classify: event shape, signature, topic
  envelope, first/last ledger, event count, resolved token pair where the
  shape carries one.
- Decide per class — genuine pool (→ seed + reprice its range),
  known-ignorable (→ into the allow-list below), or non-AMM (→ documented so
  the next run does not re-investigate it).
- Expect SushiSwap's 99 pools to fall out here as already-solved by [[0290]];
  what matters is what is left after them.

### Phase 2 — the allow-list as a committed artefact

- A checked-in list of routers / aggregators / known non-AMM emitters, each
  with the reason it is ignored and the task that established it.
- The sweep subtracts it. Anything not in the registry and not in the
  allow-list is, by construction, **unclassified** — a short list a person can
  read in minutes.

### Phase 3 — make it recur, and make it page

- Run the sweep on a schedule (weekly is enough — the venue traded for four
  months before we noticed) over a trailing ledger window.
- Cast the net **wide** on purpose: `swap` / `trade` / `SoroswapPair`
  signatures plus amount-shaped event data. Over-matching is the point; the
  allow-list is what makes over-matching cheap.
- Publish two datapoints — unclassified *contracts* and their *event volume* —
  and alarm on them. Volume is the one that matters: one new contract with
  30k swaps a month is the SushiSwap signature; twenty contracts with three
  events each are noise.
- Emit only when non-zero, `>= 1` over `NOT_BREACHING`, matching how
  `UnregisteredPoolEvents` is wired (`observability-stack.ts:1225`) so the
  alarm shape is consistent with the rest of the ingest path.

### Phase 4 — give the residual an owner

- A non-zero residual is a weekly triage item with a named owner, not a
  dashboard nobody opens. Without this the task re-creates its own two-month
  delay.

## Decisions (2026-09-21)

- **D1** — own crate `packages/coverage-sweep-probe`, own Lambda and
  EventBridge rule (not a module in `rollup-freshness-probe`: 15-min
  schedule, 1-min timeout).
- **D2** — weekly, over a trailing 14-day ledger window (~46 GB read,
  ~10 s per run, measured).
- **D3** — alarm on `UnclassifiedSwapEvents >= 1`, `NOT_BREACHING`,
  published only when non-zero; the evaluation period covers the weekly
  cadence.
- **D4** — SushiSwap V3 pools sit on the allow-list temporarily, keyed by
  wasm (`003710b3…`, `95a8e001…`) with `until = "0290"`; a wasm-wide entry
  is allowed only with `until`. Its two routers are permanent entries.
- The sweep **never registers anything** — it reports; a contract leaves
  the residual only by a human adding it to the registry or the allow-list.
- Filter on `topics_xdr` topic[0]/topic[1] of type `sym`/`string`, never
  on `signature` ([[0285]]); an Address strkey can spell `SWAP`.

## Acceptance Criteria

- [ ] Every currently unregistered swap-shaped emitter is classified, and the
      classification is recorded here.
- [ ] Any genuine AMM pools found are seeded into `pool_registry` and their
      ranges repriced.
- [ ] A committed allow-list of known-ignorable emitters exists, each entry
      carrying its reason and originating task.
- [ ] The sweep runs on a schedule and publishes unclassified contract count +
      event volume as metrics.
- [ ] An alarm fires on a non-zero unclassified volume, and is proven by a
      deliberate test (e.g. removing a known pool from the registry).
- [ ] Measured baselines established for aquarius and phoenix swap counts (the
      equivalent of Soroswap's 536,319), so their tick counts are verifiable.
- [ ] A back-test: run the sweep over the 2026-04 window and confirm it would
      have surfaced SushiSwap V3 as unclassified with its real volume.
- [ ] The residual has a named owner and a stated cadence.
