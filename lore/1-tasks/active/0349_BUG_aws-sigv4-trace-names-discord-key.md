---
id: "0349"
title: "Hold aws_sigv4 to info in the log guard: at trace it prints discord-<id>-key, and likely the role's session token"
type: BUG
status: active
related_adr: []
related_tasks: ["0348"]
tags: [portal, logging, security, priority-low, effort-small]
links:
  - "../../../packages/prices-api/src/telemetry.rs"
history:
  - date: "2026-11-18"
    status: backlog
    who: akot
    note: "Spawned from 0348 future work."
  - date: "2026-11-18"
    status: active
    who: akot
    note: "Activated: on its own branch, fix/0349_aws-sigv4-trace-names-discord-key."
---

# Hold aws_sigv4 to info in the log guard

## Summary

At `RUST_LOG=trace`, `aws_sigv4::http_request::sign` prints every request it
signs: `signing request request=SignableRequest { … uri: …/apikeys?…&name=discord-<id>-key … }`
and the whole `canonical_request`, query string and signed headers included.
So a trace log names the account behind every key lookup (a Discord id is the
account, ADR 0010 §1). `aws_sigv4` is not in `telemetry::CREDENTIAL_BEARING_TARGETS`,
so the guard lets it through. Found by [[0348]]'s log test
(`tests/portal_hub_logs.rs`), which for that reason asserts "no Discord id" on
this crate's lines only. It has always been true for the portal's own routes
too.

Production runs at `info` (`RUST_LOG` set by `compute-stack.ts`), so nothing
is logged today. The risk is the day someone raises it to find a bug.

## Context

- The guard holds the crates that can print a decoded control-plane response
  (an API key's value) to `info`, whatever `RUST_LOG` says
  (`telemetry.rs`, module docs). Its membership rule is "can carry an API key
  value"; `aws_sigv4` carries a Discord id instead, so adding it widens the
  rule to personal data. Say so in the doc.
- **Likely worse in the Lambda:** its credentials are the role's, with a
  session token, and SigV4 signs `x-amz-security-token` as a header, so the
  `canonical_request` line would carry the session token itself. Not seen:
  the tests sign with static credentials and no token. Check it before
  deciding the level.

## Implementation

- Add `aws_sigv4` to `CREDENTIAL_BEARING_TARGETS` (and widen the doc's rule).
- `tests/telemetry_filter.rs`: `aws_sigv4` held to `info` at `RUST_LOG=trace`.
- `tests/portal_hub_logs.rs` and `tests/portal_keys_logs.rs`: assert no
  Discord id anywhere in the captured text, not only on `prices_api` lines.
- Check whether any other crate under the guard's global `trace` prints the
  request URI or headers (`aws_smithy_http_client`, `hyper_util`).

## Acceptance Criteria

- [x] At `RUST_LOG=trace`, no captured line names `discord-<id>-key` or the
      account's id, on the portal's routes and the hub's. Portal:
      `tests/portal_keys_logs.rs` searches the whole capture for the id
      (failed before the fix, on four `aws_sigv4` lines). Hub:
      `tests/portal_hub_logs.rs` searches the whole capture for both
      accounts' ids, widened here once 0348 merged (2026-11-19): with the
      guard as `develop` had it, it fails on a Discord id; with the fix it
      passes.
- [x] Whether the Lambda's trace output carries `x-amz-security-token` is
      checked and written down: it does not carry the token's value (below).
- [x] `telemetry.rs`'s doc states the widened rule: credentials **or personal
      data**, with the line each member was found printing.

## The session token: not printed

`tests/telemetry_sigv4.rs` signs a `GetApiKeys?name=discord-<id>-key` lookup
with the control-plane client and credentials that carry a session token, as
the role's do in the Lambda (`session_token_mode: Include`), against a
loopback port nobody listens on (signed, then refused at connect; no AWS
call), and captures at `trace` twice:

- **Guard off** (bare `EnvFilter`): no crate prints the token's value.
  `aws_sigv4` 1.4.5 prints the request before signing (the token is not on it
  yet), its signing params (the identity is `Credentials`' `Debug`: access key
  id, `secret_access_key: "** redacted **"`, no token field), and the
  canonical request with `x-amz-security-token:** redacted (length=25) **`.
  `aws_smithy_runtime`'s `transmitting request` (already held) redacts
  `authorization` and `x-amz-security-token` the same way. What does reach
  the log: the Discord id, the access key id (`ASIA…`, four times) and the
  token's length.
- **Guard on** (`filter_from("trace")`): no `aws_sigv4` line, no Discord id,
  no token.

1.4.5 is the only `aws-sigv4` this repo's lockfile has had (`develop` and
`master` today). 1.4.2, seen in the local cargo registry, logged
`canonical_request = %creq` unredacted, token included, so the suspicion held
for older versions. `aws_sigv4` has only `trace` lines, so `info` costs
nothing.

## Implementation Notes

- `telemetry::CREDENTIAL_BEARING_TARGETS`: `aws_sigv4` and
  `aws_smithy_http_client` added. The constant's doc gives the rule and each
  member's line; the module doc says the guard holds personal data too.
- What the portal's trace capture held before the fix (146 lines): `hyper_util`
  (pool, connect, client), `aws_sigv4` (14), `aws_smithy_http_client`
  (connector timing), `axum::serve` (the mock's) and `prices_api`. The four
  lines naming the id were all `aws_sigv4`'s: the `signing request` URI and
  the canonical request's query, once per `GetApiKeys` lookup.
- Other crates checked, in their sources at the locked versions:
  `aws_smithy_http_client` prints the request URI at `debug` when it adds a
  proxy's credentials (held, below); `hyper_util` logs pool keys (scheme and
  host) and connect addresses, never a path or query; `hyper` 1 is built
  without its `tracing` feature, so it logs nothing; `h2` leaves a frame's
  headers and data out of its `Debug`; `aws_runtime` and `aws_config` log
  configuration and `Credentials`' `Debug` (no secret, no token); `reqwest`
  (Discord's calls, through `log`) logs scheme and host only, and Discord's
  paths name `@me` and the public guild id, not the account.
- Tests: `tests/portal_keys_logs.rs` (no Discord id anywhere in the capture),
  `tests/telemetry_filter.rs` (the signer's and the proxy line's events, with
  a Discord-id canary, and five more hostile directives), a new
  `tests/telemetry_sigv4.rs` (the session token, above), and two more refused
  directives in `telemetry.rs`'s `nothing_else_survives`. The three binaries
  failed at `3f343fbf` and pass with the fix; with `telemetry.rs` put back as
  it was, all three fail again.
- Checks, 2026-11-18: `cargo fmt --check`; `clippy -D warnings` on
  `prices-api` (all targets, and `--features lambda`) and on the pre-push
  hook's extractor crates; `CLICKHOUSE_URL=http://127.0.0.1:9 cargo test -p
  prices-api`: 565 passed, 0 failed, 83 ignored (the new binary's one test
  included).

## Design Decisions

### From Plan

1. **`aws_sigv4` held to `info`, and the rule widened to personal data.**
2. **The log tests search the whole capture**, not only `prices_api`'s lines.

### Emerged

3. **`aws_smithy_http_client` held too.** It prints the request URI, so
   `discord-<id>-key`, at `debug` when a proxy with credentials is set, and
   `BehaviorVersion::latest()` reads the proxy from the environment
   (`HTTPS_PROXY`), so a local `serve` behind one would log it. Read in its
   source (`client.rs`, 1.1.13), not seen in a capture: the tests use no
   proxy, so `telemetry_filter.rs` emits that line itself.
4. **The constant keeps its name.** `CREDENTIAL_BEARING_TARGETS` now holds
   personal data too; its doc says so, and a rename would touch every test
   for nothing.
5. **The session-token test signs against a closed loopback port, retries
   off,** rather than the mock gateway: signing comes before connect, so a
   dispatch failure proves the request was signed, with no mock and no AWS.
6. **The guard-off "no token" assertion stays in the test.** An SDK bump that
   starts printing the token would fail CI though the guard still holds it
   (asserted next): deliberate, so this finding is re-checked rather than
   silently going stale.
7. **The hub's log test is widened here**, since 0348 merged first
   (2026-11-19): it searches `text`, not only `prices_api`'s lines, for
   `USER_ID` and `OTHER_ID`, and no longer says the SDK's lines are the
   guard's business. It passes only with this fix, so it could land only
   with whichever of 0348 and 0349 merged second.
