---
id: "0345"
title: "Five pools missing from the registry dropped trades in November (three Aquarius since 2026-11-14, two Comet on 2026-11-10); unregistered-pool flaps per trade; six unclassified swap emitters"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0334", "0337", "0338", "0300", "0291", "0100", "0323", "0242", "0080"]
tags: [layer-ingest, layer-infra, priority-high, effort-medium, observability, alarms, pool-registry, comet]
links:
  - "../../../packages/prices-ingest-core/src/static_pools.rs"
  - "../../../packages/prices-ledger-processor/src/reconcile.rs"
  - "../../../packages/prices-ledger-processor/src/metrics.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
  - "../../../docs/runbooks/events-sourced-amm-reprice.md"
  - "https://github.com/rumblefishdev/stellar-x402/pull/186"
  - "https://github.com/rumblefishdev/soroban-block-explorer/pull/733"
history:
  - date: 2026-11-13
    status: backlog
    who: stkrolikiewicz
    note: >
      From the 2026-11-13 read-only CloudWatch review. Re-checked on
      origin/develop (66724665), live CloudWatch and the ledger-processor log the
      same day. Correction to the review: the coverage-sweep alarm (in ALARM since
      2026-11-09) is NOT about these two pools; it lists six other contracts, also
      untracked (part C).
  - date: 2026-11-17
    status: backlog
    who: stkrolikiewicz
    note: >
      After okarcz's review of PR #420. Re-read the ledger-processor log from
      2026-11-01: besides the two Comet pools (2 events, one WARN on 11-10), three
      Aquarius pools have dropped 830 events since 2026-11-14 19:31 UTC, and the
      alarm is in ALARM again. Part A covers all five; the reprice step follows
      0338. Part C's criterion no longer waits on 0323's latch.
---

# Pools missing from the registry, and unregistered-pool has the wrong shape

## Summary

A. Five pools missing from `prices.pool_registry` dropped trades in November,
and no task covers them:
- **Three Aquarius pools, 830 events since 2026-11-14 19:31 UTC**, still dropping
  on 11-17. Aquarius pools are learned from factory events, so these came by a
  path the processor never saw, or are a pool type it skips.
- **Two Comet pools, 2 events in one minute on 2026-11-10 14:17 UTC**, none since.
  This is the third time for Comet: [[0334]] (from 2026-10-07) and [[0337]]
  (2026-10-23..26) had the same cause. Comet pools have no factory event, so they
  are registered by hand, because [[0337]] deferred automatic wasm-hash
  registration.

B. `prices-production-ledger-processor-unregistered-pool` sends an ALARM and an
OK message for every burst of dropped trades, and reads OK between them while the
pool is still unregistered: 68 ALARM and 67 OK in the 30 days to 2026-11-17. Its
description covers only the factory paths.

C. `prices-production-coverage-sweep-unclassified` has been in ALARM since
2026-11-09 05:18 UTC over six other unclassified swap emitters that no task names.

## Evidence (2026-11-13, re-read 2026-11-17)

**A. Dropped trades.** Logs Insights on `/aws/lambda/prices-production-ledger-processor`,
2026-11-01 00:00 to 2026-11-17 17:05 UTC, WARN
`"dropped trades from pools missing from prices.pool_registry (task 0291)"`: 656
lines, one per invocation that dropped something:

| contract | venue | WARN lines | first | last |
|---|---|---|---|---|
| `CDJJA3CJRZHMKKW4XRRRIT7GVIL4C4AMS4E2OJ7ZHUL5IUYBAOX7H4XZ` | aquarius | 617 | 11-14 19:31 | 11-17 17:02 |
| `CDQ4LAKS4HZI2KBAMBY46SULO4V4JVOO2C7GMUGIZBX34BGYLYS5KLT7` | aquarius | 26 | 11-17 10:48 | 11-17 16:54 |
| `CDLXYTF3FA26BDZUFX7PGAIFG5VCGOEQZN4LRCB5Z5OUAKFHOEQUZH5T` | aquarius | 24 | 11-17 03:17 | 11-17 15:59 |
| `CBFKHCZDHYOWGO6QQJ4MCDYGSNSVC7IZ5SSMLV5LWV5CJUJMT5AGLKI6` | comet | 1 | 11-10 14:17 | 11-10 14:17 |
| `CBGQHIZUV7SSZQ3MCMN7NETTDU3Z5MCQOGHJJTRD7UTQ7ZVXUGP7AKJJ` | comet | 1 | 11-10 14:17 | 11-10 14:17 |

In all: `aquarius` 830 events, `comet` 2 (the one WARN of 11-10, `{"comet": 2}`).

- Not checked: whether the three Aquarius pools are `concentrated`, which the seed
  skips on purpose (`docs/runbooks/seed-pool-registry.md:49`, pending [[0080]]),
  or a new factory or event shape (the alarm's description).
- `packages/prices-ingest-core/src/static_pools.rs:35` lists five Comet pools
  (CAS3FL6T…, CASYAKK3…, CDQMIKIH…, CA5HX3D4…, CCNDNFA7…). Neither new pool is
  among them.
- `git grep` for the two Comet addresses on every local and `origin/*` ref: no hits.
- [[0337]]:70-71: "No wasm-hash auto-registration for now … Revisit if Comet
  pools keep appearing." They keep appearing.

**B. Alarm shape.** Live `describe-alarms`: `Prices/Ingest UnregisteredPoolEvents`,
Sum, period 300 s, 1 of 1, `>= 1`, `notBreaching`, OK action set
(`observability-stack.ts:1471-1497` on develop).
- Re-read 2026-11-17: in ALARM since 16:55 UTC, **68 ALARM + 67 OK** in the
  30 days, 95 of the 135 transitions on 11-14..17 (the Aquarius pools).
- `describe-alarm-history`, 30-day retention, re-read 2026-11-13 (window from
  2026-10-14 14:31 UTC): **61 ALARM + 62 OK**. The review saw 63 ALARM + 64 OK
  from 13:20 UTC; the window has rolled since. Three bursts:
  - 51 between 10-14 14:41 and 10-19 07:21 UTC ([[0334]], fixed in PR #405),
  - 9 between 10-23 19:00 and 10-26 01:41 UTC ([[0337]]),
  - 1 at 11-10 14:18 UTC (untracked, part A). The alarm returned to OK at 14:33
    while both pools were still unregistered.
- The description (`observability-stack.ts:1476-1477`) speaks only of "an
  Aquarius, Soroswap or Phoenix pool" and prescribes `events-backfill
  --discover-pools`. The counter also counts Comet `POOL/swap`
  (`packages/prices-ledger-processor/src/metrics.rs:57`), and every firing in
  the window was Comet. The fix for Comet is a `STATIC_POOLS` entry, not discovery.

**C. Coverage sweep.** Live `describe-alarms`: `coverage-sweep-unclassified` in
ALARM since 2026-11-09 05:18 UTC (`TreatMissingData ignore`; the sweep runs
`cron(17 5 ? * MON *)`). The 2026-11-09 05:17:21 UTC run
(`/aws/lambda/prices-production-coverage-sweep-probe`) reports `allowlisted 15,
unclassified 6, unclassified_events 1019`:

| contract | wasm (prefix) | events | top_action |
|---|---|---|---|
| CCJQWK4R42GSP42YASHZ2CK4NAD2QYVNIK2S5DG3UST7OIFX5ECN2WP6 | b0266a69 | 985 | `swapped` |
| CDG3X6GSOC6YJEGSJU4ZWTQHKM2JO3G64J456RCBVJGNFTOFGQODSMLT | d4affd49 | 17 | `swap_executed` |
| CBDQRFSKLR5535Y5MHHF25DUSKJHIB54EXLRTNZVDEI24JW3FIA5AUZX | d4affd49 | 12 | `swap_executed` |
| CCCUDVUXYBBY4EST4NXFWFQWNTZZCK5XSUVEJJDHZJMWEIGCQ6ELLYUQ | 2aaffc33 | 2 | `swap_in_loop` |
| CCA56KMCEEAK2SGMHNU7TYJUO7GP5KSDTTFB6ZS2NQY7VQ3SKWLMTEB4 | f550cc18 | 2 | `swap_exact` |
| CDTG4XSKBFJQCSPXEAYXEOUW5HE7BMIHYIXT4X5HJMHGDEJC5ZPT34XM | 1a458fe9 | 1 | `swap_key_set` |

None of the six is in the repo on any ref. The two Comet pools from part A
dropped trades a day after this run, so the next sweep is the first that can
show them.

## Implementation

1. **A, Aquarius.** Find each pool's `add_pool` event in BE's
   `default.soroban_events`. A `concentrated` pool is skipped on purpose until
   [[0080]], so record it and keep it out of part B's alarm. Otherwise run
   `events-backfill --discover-pools` over the range that holds its factory event
   (`docs/runbooks/seed-pool-registry.md`), and find out why the live processor
   didn't learn it.
2. **A, Comet.** Confirm both contracts run a Comet wasm already in `STATIC_POOLS`,
   add them and deploy. Then decide on wasm-hash auto-registration, which [[0337]]
   said to revisit if Comet pools kept appearing. Three occurrences in five weeks
   meet that condition. Record the decision here.
3. **A, reprice.** For each pool registered in steps 1-2, reprice from its first
   drop with `docs/runbooks/events-sourced-amm-reprice.md`, as [[0338]] did for
   [[0334]] and [[0337]], and with what it learned:
   - build a **new** `events-backfill` from develop: an older binary doesn't hold
     the new pools and reports a false `to_write=0`;
   - an enriched row sits at `version + 1`, above what the reprice writes, so a
     reprice fills only empty minutes. Count what comes back, as 0338 did;
   - a new pool can trade a SAC before anything interns it, which leaves an orphan
     asset id. The reprice doesn't heal that; add any such id to [[0242]]'s
     residual list.
4. **B.** Reshape the alarm so it stays firing while drops continue: Sum, period
   1 h, 1 of 24, `notBreaching`. It then clears after 24 clean hours. The
   window's incidents become one ALARM/OK pair each instead of 68 + 67
   messages. Rewrite the description to cover Comet/`STATIC_POOLS` next to the
   factory path, under the 1024-character guard.
5. **C.** Triage the six emitters with `docs/runbooks/0100-coverage-sweep-triage.md`
   (index, allowlist with a reason, or a task of its own). The alarm itself clears
   only when [[0323]]'s latch and the next Monday sweep allow it, which is outside
   this task.

## Acceptance Criteria

- [ ] Each of the three Aquarius pools is in `prices.pool_registry`, or is
      recorded here as `concentrated` and pending [[0080]], with the cause of the
      miss noted.
- [ ] Both Comet pools are in `STATIC_POOLS`.
- [ ] The dropped minutes of every registered pool are repriced, with the count
      of swaps that came back, and any orphan asset id is on [[0242]]'s list.
- [ ] Decision on wasm-hash auto-registration recorded here.
- [ ] `ledger-processor-unregistered-pool` stays ALARM for 24 h after the last
      drop. Its description names the Comet fix. A synth test pins the shape.
- [ ] Each of the six 2026-11-09 emitters is classified, allowlisted with a
      reason, or has its own task, and the sweep's next run lists none of them.
- [ ] The alarm change is agreed with the other teams on the shared account
      before it is deployed, in stellar-x402#186 and soroban-block-explorer#733.
