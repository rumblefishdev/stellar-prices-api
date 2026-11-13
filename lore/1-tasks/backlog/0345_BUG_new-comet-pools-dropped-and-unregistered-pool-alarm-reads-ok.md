---
id: "0345"
title: "Two new Comet pools have dropped trades since 2026-11-10 while unregistered-pool reads OK — and the alarm sends an ALARM/OK pair per dropped trade"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0334", "0337", "0338", "0300", "0291", "0100", "0323"]
tags: [layer-ingest, layer-infra, priority-high, effort-medium, observability, alarms, pool-registry, comet]
links:
  - "../../../packages/prices-ingest-core/src/static_pools.rs"
  - "../../../packages/prices-ledger-processor/src/reconcile.rs"
  - "../../../packages/prices-ledger-processor/src/metrics.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
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
---

# New Comet pools dropped, and unregistered-pool has the wrong shape

## Summary

A. On 2026-11-10 at 14:17 UTC the ledger-processor dropped Comet trades from two
pools that are not in `STATIC_POOLS`, and no task covers them. This is the third
time: [[0334]] (from 2026-10-07) and [[0337]] (2026-10-23..26) had the same cause.
Comet pools have no factory event, so they are registered by hand, because
[[0337]] deferred automatic wasm-hash registration.

B. `prices-production-ledger-processor-unregistered-pool` sends an ALARM and an
OK message for every dropped trade, and then reads OK while the pool is still
unregistered. Its description covers only the factory paths.

C. `prices-production-coverage-sweep-unclassified` has been in ALARM since
2026-11-09 05:18 UTC over six other unclassified swap emitters that no task names.

## Evidence (2026-11-13)

**A. Dropped trades.** `/aws/lambda/prices-production-ledger-processor`, stream
`2026/11/10/…[$LATEST]228cf8bb…`, `2026-11-10T14:17:06.081944Z` WARN
`"dropped trades from pools missing from prices.pool_registry (task 0291)"`,
`unregistered_pool_events {"comet": 2}`, contracts
`CBFKHCZDHYOWGO6QQJ4MCDYGSNSVC7IZ5SSMLV5LWV5CJUJMT5AGLKI6` and
`CBGQHIZUV7SSZQ3MCMN7NETTDU3Z5MCQOGHJJTRD7UTQ7ZVXUGP7AKJJ`.
- `packages/prices-ingest-core/src/static_pools.rs:35` lists five Comet pools
  (CAS3FL6T…, CASYAKK3…, CDQMIKIH…, CA5HX3D4…, CCNDNFA7…). Neither new pool is
  among them.
- `git grep` for both addresses on every local and `origin/*` ref: no hits.
- [[0337]]:70-71: "No wasm-hash auto-registration for now … Revisit if Comet
  pools keep appearing." They keep appearing.

**B. Alarm shape.** Live `describe-alarms`: `Prices/Ingest UnregisteredPoolEvents`,
Sum, period 300 s, 1 of 1, `>= 1`, `notBreaching`, OK action set
(`observability-stack.ts:1471-1497` on develop).
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

1. **A.** Confirm both contracts run a Comet wasm already in `STATIC_POOLS`, add
   them, deploy, and reprice the dropped minutes from 2026-11-10 onwards, as
   [[0338]] did for [[0334]] and [[0337]]. Then decide on wasm-hash
   auto-registration, which [[0337]] said to revisit if Comet pools kept
   appearing. Three occurrences in five weeks meet that condition. Record the
   decision here.
2. **B.** Reshape the alarm so it stays firing while drops continue: Sum, period
   1 h, 1 of 24, `notBreaching`. It then clears after 24 clean hours. The
   window's three incidents become about 3 ALARM/OK pairs instead of about 61 + 62
   messages. Rewrite the description to cover Comet/`STATIC_POOLS` next to the
   factory path, under the 1024-character guard.
3. **C.** Triage the six emitters with `docs/runbooks/0100-coverage-sweep-triage.md`
   (index, allowlist with a reason, or a task of its own), so the sweep can
   clear on its next clean run ([[0323]]).

## Acceptance Criteria

- [ ] Both Comet pools are in `STATIC_POOLS` and the dropped minutes are repriced.
- [ ] Decision on wasm-hash auto-registration recorded here.
- [ ] `ledger-processor-unregistered-pool` stays ALARM for 24 h after the last
      drop. Its description names the Comet fix. A synth test pins the shape.
- [ ] Each of the six 2026-11-09 emitters is classified, allowlisted with a
      reason, or has its own task. `coverage-sweep-unclassified` returns to OK.
- [ ] The alarm change is agreed with the other teams on the shared account
      before it is deployed.
