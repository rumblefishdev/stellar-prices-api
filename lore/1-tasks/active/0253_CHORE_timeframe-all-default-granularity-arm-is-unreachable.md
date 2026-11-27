---
id: "0253"
title: "Timeframe::All's default_granularity arm is unreachable — and the value it would return diverges from the computed one in 2029"
type: CHORE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0119", "0246"]
tags: ["priority-low", "effort-small", "api", "read-surface", "dead-code"]
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-api/src/assets/handlers.rs"
history:
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: >
      Implemented on chore/0253_… (f6998534): default_granularity returns
      Option, None for all; resolve_granularity + 3 unit tests pin the
      computed branch. No response change. PR to follow.
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: "Activated for implementation (quick-fix queue)."
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

- [x] The reachability claim is verified workspace-wide, not asserted — every
      caller of `default_granularity` enumerated, tests included
      ✅ 2026-11-25: `grep -rn default_granularity --include=*.rs` over the whole
      repo finds the definition and one call, `handlers.rs:579`, behind the
      `!timeframe.is_all()` guard. No test, no other crate, no OpenAPI path.
      The arm was neither reachable nor tested.
- [x] A decision is recorded (delete vs. keep-and-document) with its reason
      ✅ Deleted — see Design Decisions.
- [x] A test pins that `timeframe=all` with no `?granularity` resolves through
      `finest_for_span`, so the guard cannot be removed silently
      ✅ `handlers.rs` `tests::granularity::all_takes_the_computed_grain_not_a_fixed_default`:
      `1d` at today's span, `1w` at 5000 days (the 2029-06-08 crossover).
- [x] If the arm is kept, its shadowing is stated in a comment at the arm itself,
      not only at the call site
      ✅ N/A: the arm was deleted. The doc comment on `default_granularity`
      says why `all` returns `None`.
- [x] No change to any response a caller sees today — this is a clarity and
      test-coverage change, and any behaviour change would be out of scope
      ✅ Same resolution order as before; 355/355 `prices-api` unit tests pass.

## Implementation Notes

Branch `chore/0253_timeframe-all-default-granularity-arm-is-unreachable`,
commit `f6998534`.

- `queries_ch.rs`: `Timeframe::default_granularity` returns
  `Option<Granularity>`, `None` for `All` — the same shape as
  `Timeframe::seconds()`, which already returns `None` for `all`.
- `handlers.rs`: the inline `match` with the `!timeframe.is_all()` guard is
  replaced by `resolve_granularity(requested, explicit_window, timeframe, span)`:
  the caller's grain, else the fixed default when no window was sent, else
  `finest_for_span(span, OHLCV_MAX_POINTS)`.
- Three unit tests in `handlers.rs` `tests::granularity`: `all` computed
  (incl. the 2029 step to `1w`), fixed timeframes keep their default, an
  explicit window or `?granularity` wins.
- No existing test changed. clippy and rustfmt clean.

## Design Decisions

### Emerged

1. **Delete the arm, encode it in the type.** `Option` instead of an
   `unreachable!` or a kept-and-commented arm: `all` really has no fixed
   default, so `None` states the fact and the compiler stops any caller from
   reading one. A kept arm would still name `1d`, which is wrong from
   2029-06-08; an `unreachable!` would only fail at run time.
2. **Extract `resolve_granularity`.** The handler is async and needs
   ClickHouse; a pure function lets the guard be pinned by a unit test with no
   database. The `is_all()` guard is no longer needed there; `is_all()` stays
   for the backfill note further down the handler.

## Out of scope

- Changing `OHLCV_MAX_POINTS`, the timeframe default table, or `all`'s
  self-coarsening rule — all of those are [[0119]] decisions and stand.
