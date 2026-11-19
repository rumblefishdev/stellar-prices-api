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

- [ ] At `RUST_LOG=trace`, no captured line names `discord-<id>-key` or the
      account's id, on the portal's routes and the hub's
- [ ] Whether the Lambda's trace output carries `x-amz-security-token` is
      checked and written down
- [ ] `telemetry.rs`'s doc states the widened rule
