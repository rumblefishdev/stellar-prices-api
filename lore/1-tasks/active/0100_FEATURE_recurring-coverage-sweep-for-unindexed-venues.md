---
id: "0100"
title: "A recurring coverage sweep over unregistered swap emitters — the only layer that catches a venue we have never seen"
type: FEATURE
status: active
assignee: akot
related_adr: []
related_tasks: ["0097", "0079", "0078", "0285", "0290", "0291", "0300"]
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
  - date: "2026-09-21"
    status: active
    who: akot
    note: >
      Implemented on feat/0100_recurring-coverage-sweep-for-unindexed-venues,
      PR #332 (8 commits, not merged, not deployed). D5 settled: option A,
      the probe runs as prices_writer. prices_writer got Code 497 on
      default.* in the morning; BE added SELECT on default.soroban_events /
      default.soroban_contracts the same afternoon (0477-style in-place
      users.d edit) and every read-only check passed (default.transactions
      still 497). A local dry run of the probe's code against production gave
      13 unclassified contracts / 911 events; the 2026-04 back-test surfaced
      SushiSwap V3 (32 contracts, 3,125 events) and the Soroswap factory, now
      allow-listed. Rule shipped enabled (coverageSweepEnabled=true).
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Phase 1 started: the 13 unclassified emitters of the 14-day window to
      64,543,788 classified (section "Phase 1 classification"). One real
      missing venue — the Comet BLND/USDC pool (Blend backstop, 53,092 swaps
      since 2024-05-02), spawned as [[0300]] and allow-listed temporarily by
      wasm until 0300; 12 routers, aggregators and non-AMM contracts
      allow-listed permanently (PR #332).
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
- **D5 — identity: option A.** The probe uses the existing ingestion
  identity (`prices_writer`); BE granted it `SELECT` on
  `default.soroban_events` and `default.soroban_contracts` only (not
  `default.*`). Chosen over a dedicated read-only user (option B: new cert,
  secret, CN-map entry) for a smaller change; the cost is that every
  ingestion Lambda can now read those two BE tables and the scan shares the
  `prices_write` quota. A dedicated identity is a later clean-up under
  [[0258]] — only the secret name in infra changes.
- **Rule switch.** `coverageSweepEnabled` in `infra/envs/production.json`
  (`true` since the grants were verified) disables the rule durably;
  `aws events disable-rule` is undone by the next EventBridge deploy.

## Status (2026-09-21)

- Code: PR #332 — crate `packages/coverage-sweep-probe`, allow-list
  (6 contract entries incl. the Soroswap factory, 2 SushiSwap V3 wasm
  entries `until = "0290"`), weekly rule Mon 05:17 UTC, alarms
  `prices-production-coverage-sweep-unclassified` and `-probe-errors`,
  runbook `docs/runbooks/0100-coverage-sweep-triage.md`.
- BE grants: live and verified 2026-09-21 (runbook §4.2).
- Not yet done: merge, deploy (EventBridge + Observability, runbook §4.4),
  first run. After the phase-1 allow-list (PR #332, `e4bae2a`) the current
  window reads 0 unclassified, so the first run publishes nothing; the AC5
  proof is a synthetic `UnclassifiedSwapEvents = 1` datapoint (runbook §4.5).
- Phase-1 residual (current 14 days): classified 2026-09-22, see below.
  Still to classify: the April-only candidates `Swap` (`d4b4976b`, 462),
  `SwappedToVUsd` (`a757a1ed`, 355), 2× `tokens_swapped_event`, and a few
  1–2-event emitters (runbook §5).

## Phase 1 classification (2026-09-22, production, `dev_read`)

Window 64,322,610–64,543,788 (the probe's own 14 days). Method as [[0285]]:
the wasm interface (`default.wasm_interface_metadata`), an event sample, and
how many of the contract's transactions in the window also hold an event of a
pool in `prices.pool_registry` ("with pool") — a wrapper around pool swaps
shows ~all, a venue of its own does not.

| Contract | wasm | Events | What it is | Evidence | Verdict |
| --- | --- | --- | --- | --- | --- |
| `CAS3FL6T…` | `8abc2891` | 727 | Comet weighted pool, BLND/USDC — Blend backstop, LP `CPAL` | `init(…weights…)`, `join_pool`/`exit_pool`, `swap_exact_amount_in`; 53,092 swaps since 2024-05-02; with pool 125/412 | **missing venue → [[0300]]**; allow-listed by wasm `until = "0300"` |
| `CC6QAV7J…` | `91060e9c` | 12 | split-route aggregator | "Execute a swap atomically (single-path or split-order)"; with pool 682/684 | router, allow-list |
| `CAYP3UWL…` | `5e0bff5a` | 23 | Soroswap Aggregator | adapters, "sets the soroswap_router"; 23/23 | router, allow-list |
| `CDETNHJC…` | `277444b5` | 15 | router with a pool whitelist | `set_pool_whitelist`, `price_protection`; 15/15 | router, allow-list |
| `CAVDUEGL…` | `c546a2d0` | 3 | fee-taking split aggregator | `swap_split`, `register_pool`, `fee_ppm`; 3/3 | router, allow-list |
| `CARVQXFP…` | `43765328` | 8 | fee-taking swap wrapper | `swap_exact_in`, `set_routers`, `set_treasury`; 6/8 (+2 Sushi) | router, allow-list |
| `CAVNAZFN…` | `3afca103` | 5 | same family, older | same interface without `set_routers`; 5/7 | router, allow-list |
| `CDE5MFAG…` | `a317f5bd` | 1 | small router | `swap`, `registry`, `fee_policy`; 1/2 | router, allow-list (low confidence) |
| `CDCNXZHY…` | `cbbb1bde` | 19 | LI.FI swap/bridge | `swap`, `bridge`, `swap_bridge`; 19/57 | aggregator, allow-list |
| `CDWTSHMD…` | `8110634b` | 63 | USDM0↔USDM1 issuer conversion | "Atomic swap between USDM0 and USDM1", mint/burn against a signed price attestation; 0/63 | not a market, allow-list |
| `CA5YJ5H2…`, `CBOSXSEZ…` | `4d6abb0d` | 17 + 17 | on-chain order book of tokenised instruments | `new_buy_order`, `best_bid_offer`, order-book phases, halts, price collar; instruments are `u64` ids, no token addresses; 0/88 | non-AMM, not priceable, allow-list |
| `CBP76I2F…` | `76e60d02` | 1 | lending protocol | `borrow`, `liquidate`, `flash_loan`; a `swap_exact` inside a liquidation; 1/1 | non-AMM, allow-list |

With these entries the current window reads **0 unclassified**.

## Acceptance Criteria

- [ ] Every currently unregistered swap-shaped emitter is classified, and the
      classification is recorded here. (The 14-day window: done 2026-09-22,
      13/13. The April-only candidates are still open.)
- [ ] Any genuine AMM pools found are seeded into `pool_registry` and their
      ranges repriced. (Comet BLND/USDC → [[0300]]; SushiSwap V3 → [[0290]].)
- [ ] A committed allow-list of known-ignorable emitters exists, each entry
      carrying its reason and originating task. (In PR #332, validated by
      tests; ticks on merge.)
- [ ] The sweep runs on a schedule and publishes unclassified contract count +
      event volume as metrics.
- [ ] An alarm fires on a non-zero unclassified volume, and is proven by a
      deliberate test (e.g. removing a known pool from the registry).
- [ ] Measured baselines established for aquarius and phoenix swap counts (the
      equivalent of Soroswap's 536,319), so their tick counts are verifiable.
- [x] A back-test: run the sweep over the 2026-04 window and confirm it would
      have surfaced SushiSwap V3 as unclassified with its real volume.
      → 2026-09-21, the probe's own code as `prices_writer`, ledgers
      61,926,675–62,147,853, SushiSwap wasm entries removed: 32 contracts /
      3,125 events unclassified, largest pool `CCR2CH4G…` (2,829) first
      (runbook §5).
- [ ] The residual has a named owner and a stated cadence. (Cadence stated in
      the runbook §1; owner deliberately `TBD` — to be agreed with the team.)
