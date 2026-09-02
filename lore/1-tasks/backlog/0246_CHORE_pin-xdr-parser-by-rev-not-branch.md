---
id: "0246"
title: "Pin the xdr-parser git dependency by rev, not branch — a bare `cargo update` silently pulls BE's develop HEAD into our event decoding"
type: CHORE
status: backlog
related_adr: []
related_tasks: []
tags: [layer-tooling, layer-infra, priority-low, effort-small, dependencies, block-explorer]
links:
  - "../../../Cargo.toml"
  - "../../../.github/workflows/ci.yml"
history:
  - date: 2026-09-01
    status: backlog
    who: stkrolikiewicz
    note: >
      Raised by Karol (BE) while reviewing how prices-api depends on
      soroban-block-explorer: "pytanie czy nie wolicie przypięcia w jakiś inny
      sposób bo cargo lock chyba zaktualizuje wam ten commit". Correct about the
      outcome, not the trigger — `cargo build` honours the lockfile, `cargo
      update` is what moves it. Filed rather than fixed inline because the
      session's active task was 0210.
---

# Pin xdr-parser by rev, not branch

## Summary

`xdr-parser` is pulled from BE's repo by **branch**, so its resolved commit is
held only by `Cargo.lock`. A bare `cargo update` — run for any unrelated reason
— re-resolves it to whatever `develop` points at that day, and the bump lands in
the same diff as the dependency someone actually meant to bump. Pin by `rev` so
moving it becomes an explicit one-line change.

## Context

[`Cargo.toml:32`](../../../Cargo.toml):

```toml
xdr-parser = { git = "https://github.com/rumblefishdev/soroban-block-explorer.git", branch = "develop" }
```

`Cargo.lock` currently resolves this to `d61b359f39994c7ef5f5bde8a0d709cf81a1026c`.

Two things are true and are easy to conflate:

- **`cargo build` / `cargo check` will not move it.** The lockfile is committed
  and authoritative; CI gets exactly `d61b359`.
- **`cargo update` will.** Without arguments it re-resolves every dependency,
  including this one, straight to BE's `develop` HEAD. Nothing in the diff says
  "we changed how ledgers are decoded" — it reads as a routine lockfile churn.

CI does not close the gap: [`ci.yml:178-180`](../../../.github/workflows/ci.yml)
runs `cargo check --workspace`, `cargo clippy`, `cargo test --workspace` with no
`--locked`, so a lockfile that has drifted from `Cargo.toml` is resolved and
compiled quietly instead of failing the build.

## Why the dependency stays

Worth recording, so nobody re-opens this as "just drop the dep". We import
three functions and three types out of a ~14k-line crate:

| Item | Used in |
|---|---|
| `decompress_zstd`, `deserialize_batch` | `prices-ingest-core/src/decode.rs:15`, `sdex-backfill/src/ingest.rs:124` and `:233` |
| `extract_events`, `types::{EventSource, ExtractedEvent}` | `prices-ingest-core/src/soroban.rs:280` |
| `ParseError` | error enums in both crates |

`prices-ingest-core` is a dependency of six crates — `prices-ledger-processor`
(the live ingest Lambda), `asset-discovery`, `events-backfill`, `oracle-worker`,
`pool-registry-seed`, `sdex-backfill` — so this is on the path of every ledger
we ingest.

The decode half is trivially replaceable: `decompress_zstd` is `zstd::decode_all`
plus a size cap, `deserialize_batch` is `LedgerCloseMetaBatch::from_xdr` with
limits, and we already depend on `stellar-xdr` 27 directly, which carries that
type. **`extract_events` is not.** It handles the V3-vs-V4 event layout split
(CAP-67 moved events into three locations under Protocol 23), tags diagnostic
against consensus events so consumers can drop the diagnostic container without
trusting the inner `type_`, and keeps `event_index` monotonic across all three
sources. That is protocol logic BE re-touches on every protocol bump. Forking it
to avoid a git dependency costs more than it saves — which is exactly why the
pin matters: an unnoticed `cargo update` can pull a CAP-67 refactor into our
event indexing.

## Implementation

One line in [`Cargo.toml:32`](../../../Cargo.toml):

```toml
xdr-parser = { git = "https://github.com/rumblefishdev/soroban-block-explorer.git", rev = "d61b359f39994c7ef5f5bde8a0d709cf81a1026c" }
```

`cargo update` cannot move a `rev`. Bumping becomes a deliberate, reviewable
one-line diff.

Optional, only if a second git dependency ever appears: add `--locked` to the
three cargo invocations in `ci.yml` so lockfile drift fails the build. Redundant
while `rev` is the only git pin.

### Declined

- **`tag = "..."`** — depends on BE cutting tags they do not cut today. Another
  team's workflow change for no gain over `rev`.
- **Vendoring / forking the crate** — see "Why the dependency stays". We would
  own protocol-tracking code we currently get for free.

## Acceptance Criteria

- [ ] `Cargo.toml` pins `xdr-parser` by `rev`, at the commit `Cargo.lock`
      already resolves to (`d61b359`) — no behaviour change in the same commit
- [ ] `cargo update` leaves the `xdr-parser` entry in `Cargo.lock` untouched
      (verify: run it on a scratch branch, diff the lockfile)
- [ ] `cargo check --workspace` and `cargo test --workspace` pass unchanged
