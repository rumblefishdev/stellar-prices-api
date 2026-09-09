---
id: "0253"
title: "Timeframe::All's default_granularity arm is unreachable — and the value it would return diverges from the computed one in 2029"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0119", "0246"]
tags: ["priority-low", "effort-small", "api", "read-surface", "dead-code"]
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-api/src/assets/handlers.rs"
history:
  - date: 2026-09-02
    status: backlog
    who: okarcz
    note: >
      Spawned from a read of the /ohlcv granularity defaults while explaining
      the endpoint. `Timeframe::default_granularity` has an arm for `All`, and
      its only caller guards that arm away. Filed to CONFIRM the finding against
      the code and the tests rather than to act on it — the claim is "this arm
      cannot execute", and an unreachable-code claim is exactly the kind that is
      wrong for a reason the reader did not think of.
---

# `Timeframe::All => Granularity::D1` cannot be reached by its only caller

## Summary

`Timeframe::default_granularity` (`queries_ch.rs:552`) maps every timeframe to
the granularity `/ohlcv` uses when `?granularity` is omitted. It has six arms.
**The `All` arm appears to be dead**: the function has exactly one call site,
and that call site excludes `All` in its own match guard.

Nothing is broken today. This task is to **verify the claim, then decide**
whether the arm is deleted or the guard is what should change — and to record
which, because the two branches disagree about `all` from mid-2029 onward.

## Context

`/ohlcv` resolves an omitted granularity in two branches (`handlers.rs:472-476`):

```rust
let granularity = match p.granularity {
    Some(g) => g,
    None if !explicit_window && !timeframe.is_all() => timeframe.default_granularity(),
    None => Granularity::finest_for_span(span, OHLCV_MAX_POINTS),
};
```

The `!timeframe.is_all()` guard sends `timeframe=all` to `finest_for_span`
instead — deliberately, per the comment above it: `all`'s span grows with time,
so it must **self-coarsen** rather than hit a cliff at a fixed grain.

`grep` finds `default_granularity` in exactly two places: its definition and
that one call. So the `All` arm has no route to execution.

## Why it has never surfaced — the two values agree, for now

`finest_for_span` picks the finest grain whose inclusive point count stays under
`OHLCV_MAX_POINTS` (5000, `handlers.rs:21`). From `STELLAR_GENESIS_EPOCH`
(`queries_ch.rs:585`, 2015-09-30) to today that is:

| grain | points |
|---|---|
| `1h` | 95,768 |
| `4h` | 23,942 |
| **`1d`** | **3,991** ✅ first under 5000 |
| `1w` | 571 |

So `all` computes to `1d` — **the same value the dead arm names.** The
coincidence is why no test and no consumer has ever noticed.

⚠️ **It stops being a coincidence on 2029-06-08**, when the genesis span crosses
5000 days and `finest_for_span` steps to `1w`. From then on the dead arm and the
live path would return different grains. That is not a future bug — the arm is
still unreachable — but it means "they agree, so it doesn't matter" is only true
for the next ~2.75 years, and a future reader who deletes the guard instead of
the arm gets a silent behaviour change with a three-year fuse.

## Implementation

- **Confirm the reachability claim first.** `default_granularity` is `pub`; check
  for callers outside `prices-api` (workspace-wide, not just this crate), in
  tests, and in any doc/OpenAPI generation path. A `pub` fn on a `pub enum` can
  be exercised by a test that pins the table without going through the handler —
  if such a test exists, the arm is *tested* but still not *reachable in
  service*, and the write-up must say which.
- Decide, and record the reason:
  - **delete the arm** and make the function total over the five reachable
    timeframes (needs a type or an `unreachable!`/`debug_assert` — say which and
    why), or
  - **keep it** as the documented intent for `all` and add a comment stating it
    is deliberately shadowed by the guard, plus a test pinning that
    `finest_for_span` — not this table — is what answers `all`.
- Either way, add the missing test: **nothing currently pins that
  `timeframe=all` takes the computed branch.** That is the actual gap; the dead
  arm is only the symptom that exposed it.
- If the arm is kept, consider pinning the 2029 divergence explicitly so the
  crossover is discovered by a failing test rather than by a consumer.

## Acceptance Criteria

- [ ] The reachability claim is verified workspace-wide, not asserted — every
      caller of `default_granularity` enumerated, tests included
- [ ] A decision is recorded (delete vs. keep-and-document) with its reason
- [ ] A test pins that `timeframe=all` with no `?granularity` resolves through
      `finest_for_span`, so the guard cannot be removed silently
- [ ] If the arm is kept, its shadowing is stated in a comment at the arm itself,
      not only at the call site
- [ ] No change to any response a caller sees today — this is a clarity and
      test-coverage change, and any behaviour change would be out of scope

## Out of scope

- Changing `OHLCV_MAX_POINTS`, the timeframe default table, or `all`'s
  self-coarsening rule — all of those are [[0119]] decisions and stand.
