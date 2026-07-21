---
id: "0077"
title: "CI should build all 9 Lambda assets + run synth-production so a missing asset fails loudly"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0070", "0056", "0026", "0108"]
tags: [layer-ops, ci, cdk, cargo-lambda, priority-medium, effort-small]
links: []
history:
  - date: 2026-07-03
    status: backlog
    who: oski
    note: >
      Spawned from 0070. During the 0070 deploy prep, `make synth-production`
      failed with CannotFindAsset for target/lambda/backfill-freshness-probe:
      the production CDK app references NINE Lambda assets (ledger-processor,
      api-handler/prices-api, asset-discovery, cleanup, supply, oracle,
      enrichment, backfill-freshness-probe, mtls-notafter-probe) but ci.yml
      builds only SIX (missing the two 0056 probes + prices-api) and never runs
      synth-production, so the drift is invisible until an operator synths by hand.
  - date: 2026-07-20
    status: active
    who: okarcz
    note: >
      Activated from the 0108 grooming sweep, which re-verified the gap and found
      it **worse than described**: CI builds 6 crates but the "Verify Lambda
      artifacts" `expected=()` array lists only **5** — `enrichment-worker` is
      built and never checked, so its bootstrap could silently fail to package
      and still pass CI. Confirmed 9 assets are referenced by the production app
      (7 in eventbridge-stack.ts, 2 in compute-stack.ts).
---

# CI should build all 9 Lambda assets + run synth-production

## Summary

`.github/workflows/ci.yml` builds 6 Lambda bootstraps and verifies those exist,
but the production CDK app references 9. Because CI never runs `synth-production`,
a stack referencing an un-built asset passes CI and only fails when the operator
synths during a deploy (as happened in 0070, 2026-07-03). Close the gap so CI
fails loudly instead.

## Context

- Missing from the CI `cargo lambda build` list: `backfill-freshness-probe`,
  `mtls-notafter-probe` (task 0056), and `prices-api` (the api-handler).
- The "Verify Lambda artifacts" step only checks the 6 it builds.
- Root cause: the build/verify lists are hand-maintained and drifted when 0056 +
  the API handler were wired into the stacks.

## Implementation

- Add the 3 missing crates to the `cargo lambda build … --features lambda` step
  and to the `expected=(…)` verify array in `ci.yml`.
- Add a `make synth-production` (or a CI-env synth) step after the build so a
  future asset/stack mismatch fails CI directly — the single source of truth for
  "did we build everything the app needs" is a successful synth.
- Consider deriving the crate list from one place (Makefile var or a small script)
  so the build list, verify list, and CDK asset dirs can't drift again.

## Acceptance Criteria

- [x] CI builds all 9 arm64 bootstraps the production app references.
- [x] CI runs `synth-production` (or equivalent) and fails on any missing asset.
- [x] Verify array / crate list is single-sourced or otherwise drift-resistant.

## Implementation Notes

**`tools/scripts/lambda-assets.sh`** is the single source: it derives the asset
list *from the CDK source* by matching the `'../target/lambda/<name>'` string
literals, so the build list and verify list can no longer drift from what the
app references — they are the same list. Add a Lambda to a stack and CI builds
and verifies it automatically. Resolves to exactly the 9 known assets.

The crate name, bin name and asset dir name are identical for all 9 (each bin
is gated behind the crate's `lambda` feature — verified), so one name serves as
both `-p <crate>` and `target/lambda/<name>`.

**Guard split, by cost.** The heavy guard sits in the `rust` job, which only
runs on `packages/**` changes; an infra-only PR could still point
`Code.fromAsset` at a nonexistent crate. Rather than add `infra/**` to the rust
filter — which would make every infra PR pay for `cargo test --workspace` plus
9 Lambda builds — the `typescript` job got a build-free counterpart that asserts
each derived asset name resolves to a real crate. Both jobs consume the same
script.

| job | trigger | guard |
|-----|---------|-------|
| `typescript` | `infra/**`, TS | asset name → real crate (no build) |
| `rust` | `packages/**` | build all 9 → verify all 9 → `synth-production` |

## Design Decisions

### From Plan

1. **Derive rather than hand-list.** The task suggested "consider deriving the
   crate list from one place"; taken, because the alternative (adding the 3
   missing crates to two hand-kept arrays) fixes today's drift and leaves the
   mechanism that produced it intact. This is the only option where forgetting
   a crate is *impossible* rather than merely caught.

### Emerged

2. **Fail on an empty derived list.** If a refactor changes how asset dirs are
   written (template literal, shared helper, path built from a variable), the
   regex would match nothing — and an empty list makes every downstream check
   pass vacuously, silently disabling the guard. The script exits non-zero on
   zero matches with a message saying so. Tested.

3. **Match only the quoted literal**, not bare `target/lambda/...` text, so
   prose in a nearby comment cannot inject a name into the build list.

4. **Did not extend the `rust` paths filter to `infra/**`.** Covered the
   infra-only case with the cheap typescript-job check instead (see table).
   The expensive filter change buys full synth on infra-only PRs; judged not
   worth a multi-minute Rust build on every infra edit, since adding a Lambda
   in practice also adds a crate under `packages/**`, which already triggers
   the rust job. **Held after review** — the cost split survived; what changed
   is that the cheap check became strong enough to carry it (#5 below).

### Post-review (code review of PR #134, three findings fixed in-branch)

5. **The cheap check tested the wrong property.** It asked "does a crate by
   this name exist" (`grep -rqx 'name = "<x>"' --include=Cargo.toml packages/`)
   when what matters is "will this name produce `target/lambda/<name>/bootstrap`".
   `grep` is section-blind, so it matched `[[bin]]` and `[lib]` names as readily
   as `[package]` ones. Three false-pass classes, each verified against the old
   check:

   | asset name | why it cannot build | old check |
   |---|---|---|
   | `extractors-core` | pure library, no `[[bin]]` at all | passed |
   | `enrichment-cli` | a bin name, not a package — `-p` cannot resolve it | passed |
   | `prices-clickhouse` | its only bin is `prices-clickhouse-init` → wrong asset dir | passed |
   | `events-backfill` | has a bin but no `lambda` feature → wrong binary shipped | passed |

   Replaced by `tools/scripts/verify-lambda-assets.sh`, which requires all
   three of: a package by that name, a bin target of the same name, and a
   `lambda` feature on that package. Uses `cargo metadata --no-deps --offline`
   — compiles nothing, needs no network, so the check stays build-free and the
   cost split holds. The eligible set it computes is exactly the 9 CDK assets.

6. **`tools/scripts/**` was in neither paths filter.** The script both guards
   depend on was the one file CI never exercised: a PR editing only it ran no
   job at all. Added to both filters.

7. **Dropped the `$GITHUB_OUTPUT` plumbing between steps.** `lambda-assets.sh`
   refuses to emit an empty list, but that guarantee did not survive a step
   boundary — if the producing step were renamed or its `id` changed, the
   consumer's `${{ steps.… }}` would expand to empty, the loop would run zero
   iterations, and Verify would exit 0 having asserted nothing (confirmed: it
   did). Each step now reads the script directly via process substitution, and
   both Build and Verify additionally fail on a zero count. Re-running a grep is
   far cheaper than that failure mode; it also removes workflow-expression text
   from the shell body, so the guard no longer depends on the script's character
   class for shell safety.

**Not fixed here** (recorded, lower severity): the asset-name regex excludes
`_`, so a `my_worker` asset would be silently dropped from the list while the
list stays non-empty and no tripwire fires; and the derived count is logged but
never asserted against a floor, so a refactor that drops 7 of 9 literals still
passes. Both are partial-coverage variants of the empty-list case.

## Verification

Run locally against the real repo:

- `lambda-assets.sh` → exactly the 9 expected names; count asserted.
- Empty-list guard fires (exit 1) on a stub tree with no matches.
- Missing-directory guard fires (exit 1).
- Build-arg construction → 18 args, all 9 `-p` flags, correct order.
- All 9 crates confirmed to declare a `lambda` feature (a missing one would
  fail the whole `cargo lambda build` invocation).
- Crate-mapping check passes for all 9; verified a bogus name fails.
- **Post-review, against a fixture root** (real `packages/` + `Cargo.toml`, a
  scratch copy of `infra/src` with one bad literal appended): all four
  false-pass classes in the table above now exit 1, each with an error naming
  the missing bootstrap path. The old `grep` check passed all four.
- `cargo metadata --no-deps --offline` succeeds with no network and no
  registry access; eligible set == the 9 CDK assets exactly (`diff` clean).
- Vacuous-pass guard: a Verify loop over an empty list now exits 1 (it
  exited 0 before).
- `ci.yml` parses as YAML; no `${{ steps.… }}` expressions remain in any
  shell body.
- **`make -C infra synth-production` succeeds credential-free** — every SSM read
  is `valueForStringParameter` (a deploy-time CloudFormation dynamic reference);
  there are no `fromLookup` / `valueFromLookup` context lookups, which are what
  would require AWS credentials at synth time.
- **Synth fails (exit 1) on a missing asset** — proven by pointing
  `BACKFILL_FRESHNESS_PROBE_ASSET_DIR` at a nonexistent dir, which reproduces
  the 0070 `CannotFindAsset` class of failure. This is the negative case that
  matters; without it the synth step could have been passing vacuously.
