//! The log filter, and the one thing it will not let `RUST_LOG` do.
//!
//! # Why this is not just `EnvFilter::from_default_env()`
//!
//! `portal/keys/gateway.rs` keeps a key value inside [`KeyValue`], which has no
//! `Display`, no `Serialize` and a `Debug` that prints `<redacted>` — so no
//! format string in *our* code can leak one. That guarantee stops at the edge of
//! this workspace. The AWS SDK holds the same value in its own types on the way
//! to us, and it logs them:
//!
//! - `aws_smithy_runtime::client::orchestrator` emits
//!   `trace!(output_or_error = ?…)` after every deserialization, which prints
//!   `GetApiKeyOutput { …, value: Some("<the key>"), … }`.
//! - the same crate's `log_response_body` emits the **raw HTTP response body**
//!   at `trace`. It has a redaction branch, but it is conditional on the
//!   operation being marked `@sensitive` in the service model, and
//!   `aws-sdk-apigateway` marks nothing sensitive (zero occurrences of
//!   `SensitiveOutput` in the crate). So the branch that would have saved us
//!   never runs.
//!
//! Neither is reachable at `info`, which is what `ComputeStack` sets — but
//! `RUST_LOG` is an environment variable, and raising it is exactly what
//! somebody does while debugging the control plane. That is the moment this
//! module exists for: `RUST_LOG=trace` on the api-handler would write live,
//! production API keys into CloudWatch Logs, where they outlive the debugging
//! session and are readable by everyone with log access rather than by the one
//! person the key belongs to. The same applies to a local `serve` run, which
//! holds production credentials and prints to a terminal with scrollback.
//!
//! # What the guard does, and why it is an allowlist
//!
//! [`filter_from`] keeps only the directives it **recognises as safe** — a bare
//! level, or a target inside [`OUR_TARGETS`] — drops everything else, and then
//! pins the credential-bearing crates at [`GUARD_LEVEL`] so a bare level cannot
//! reach them either.
//!
//! It was a blocklist first, and it was walked past twice:
//!
//! - `RUST_LOG='[try_op]=trace'` — a directive with no target at all. `EnvFilter`
//!   gives dynamic span scopes precedence over static target directives, so the
//!   pin never got to refuse the event.
//! - `RUST_LOG='aws_smithy_runtime:=trace'` — **one trailing colon**. Targets
//!   match by `starts_with`, the directive parser accepts any character in a
//!   target, and `StaticDirective` orders by target length, so a 19-character
//!   target outranked the 18-character pin for
//!   `aws_smithy_runtime::client::orchestrator`.
//!
//! Both were closed by naming one more shape. The third time, the shape changes
//! instead: a blocklist has to anticipate every way of writing a target that
//! prefix-matches something dangerous, and this parser accepts more shapes than
//! anyone reviewing it will enumerate. An allowlist inverts the failure mode —
//! a form nobody thought about is **refused** rather than admitted — and it is
//! the difference between a property that must be re-measured after every
//! `tracing-subscriber` bump and one that can simply be stated.
//!
//! The cost is real and accepted: raising the log level for a third-party crate
//! — `hyper`, `rustls`, `reqwest` — now needs a code change rather than an
//! environment variable. For a function that holds production credentials, that
//! is the right side of the trade, and `prices_api` itself (which is where our
//! own diagnostics live) stays freely adjustable.
//!
//! `info` rather than `off` for the pins, because the SDK's warnings and errors
//! are worth having and neither carries a body: everything measured above is
//! `debug` or `trace`.
//!
//! [`KeyValue`]: crate::portal::keys::gateway::KeyValue

use tracing_subscriber::EnvFilter;

/// Crates whose `debug`/`trace` output can carry an API key value.
///
/// The membership rule is narrow on purpose: a crate belongs here when it can
/// print a **decoded control-plane response** — the SDK's own orchestrator, its
/// runtime API, and the generated API Gateway client. `hyper` and `rustls` are
/// deliberately absent; they move bytes and log byte counts, and widening this
/// list on a hunch would silence diagnostics that have nothing to do with
/// credentials.
pub const CREDENTIAL_BEARING_TARGETS: &[&str] = &[
    "aws_smithy_runtime",
    "aws_smithy_runtime_api",
    "aws_sdk_apigateway",
];

/// The ceiling those crates are held to, whatever `RUST_LOG` says.
const GUARD_LEVEL: &str = "info";

/// The only crates a `RUST_LOG` directive may name.
///
/// Ours, and nothing else. A directive for anything outside this list is dropped
/// rather than examined for danger — see the module docs for why the guard is an
/// allowlist and not a longer blocklist.
pub const OUR_TARGETS: &[&str] = &["prices_api", "prices_clickhouse"];

/// Levels `EnvFilter` accepts as a whole directive, i.e. with no target.
///
/// A bare level is the one targetless form that is kept, because it is not a
/// target directive at all: it sets a global default that the pins then outrank
/// for the crates that matter. `EnvFilter` also accepts `0`-`5`; those are here
/// so that `RUST_LOG=5` keeps meaning what the operator intended rather than
/// being silently dropped to `info`.
const BARE_LEVELS: &[&str] = &[
    "off", "error", "warn", "info", "debug", "trace", "0", "1", "2", "3", "4", "5",
];

/// What both binaries log at: `RUST_LOG` if set, `info` if not, guarded.
pub fn env_filter() -> EnvFilter {
    filter_from(&std::env::var("RUST_LOG").unwrap_or_default())
}

/// [`env_filter`] with the environment read out of the way.
///
/// Separate so the guard is decidable by a unit test on a string somebody typed,
/// rather than by a test that mutates the process environment — which is
/// `unsafe` in this edition and races every other test in the binary.
pub fn filter_from(requested: &str) -> EnvFilter {
    let kept: Vec<&str> = requested
        .split(',')
        .map(str::trim)
        .filter(|directive| !directive.is_empty() && is_permitted(directive))
        .collect();

    // An empty result is not "log nothing": it is either an unset `RUST_LOG` or
    // one that consisted entirely of directives this guard removed, and in both
    // cases the deployment still wants its own logs at `info`.
    let base = if kept.is_empty() {
        "info".to_string()
    } else {
        kept.join(",")
    };

    let mut filter = EnvFilter::new(base);
    for target in CREDENTIAL_BEARING_TARGETS {
        filter = filter.add_directive(
            format!("{target}={GUARD_LEVEL}")
                .parse()
                .expect("a `<target>=<level>` directive built from constants always parses"),
        );
    }
    filter
}

/// Whether one `RUST_LOG` directive is one this deployment will honour.
///
/// Three shapes survive, and nothing else:
///
/// 1. **A bare level** — `trace`, `off`, `3`. Sets a global default; the pins
///    added afterwards outrank it for the credential-bearing crates, which is
///    the whole reason they are pinned by target rather than by level.
/// 2. **A target inside [`OUR_TARGETS`]**, with or without a level —
///    `prices_api=trace`, `prices_api::portal::keys=debug`. Matching is
///    `starts_with`, exactly as `EnvFilter` matches, so anything a kept
///    directive can enable is a target of ours.
/// 3. Nothing else. Not `hyper=debug`, not `aws_smithy_runtime=info` (the pins
///    supply that themselves), not a span or field directive of any kind —
///    `prices_api[req]=trace` goes too, because a dynamic scope is the one form
///    that can outrank a static pin and no deployment here needs one.
///
/// Written as "what is allowed" rather than "what is dangerous" deliberately: a
/// shape nobody anticipated lands in case 3 and is dropped, instead of landing
/// outside a blocklist and being honoured. Two bypasses got in through the
/// blocklist version — see the module docs.
fn is_permitted(directive: &str) -> bool {
    // A dynamic scope directive can enable events by span or field regardless of
    // target, so it is refused before anything else is considered.
    if directive.contains('[') || directive.contains(']') {
        return false;
    }

    let (target, level) = match directive.split_once('=') {
        Some((target, level)) => (target.trim(), Some(level.trim())),
        None => (directive.trim(), None),
    };

    // Case 1: a bare level, written on its own.
    if level.is_none() && is_bare_level(target) {
        return true;
    }

    // A `target=` with nothing after it, or a level `EnvFilter` will not parse,
    // is not a directive this deployment has any reason to honour.
    if let Some(level) = level
        && !is_bare_level(level)
    {
        return false;
    }

    // Case 2: one of ours. `starts_with`, matching how `EnvFilter` resolves a
    // target, so a kept directive cannot reach past our own crates.
    OUR_TARGETS.iter().any(|ours| target.starts_with(ours))
}

/// Whether `s` is a level `EnvFilter` would parse, case-insensitively.
fn is_bare_level(s: &str) -> bool {
    BARE_LEVELS
        .iter()
        .any(|level| s.eq_ignore_ascii_case(level))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EnvFilter`'s `Display` is its directive list, which is what these tests
    /// assert on — the filter has no API for asking "what would you do with a
    /// `trace` event from this target" without constructing `Metadata`, which
    /// only a macro can do. `tests/telemetry_filter.rs` asks that question the
    /// only way it can be asked: by emitting the events.
    fn directives(filter: &EnvFilter) -> String {
        filter.to_string()
    }

    #[test]
    fn an_unset_rust_log_is_info_plus_the_pins() {
        // Every other test here loops over the list, so an empty list would
        // make all of them pass while guarding nothing. Asserted once, here.
        assert!(!CREDENTIAL_BEARING_TARGETS.is_empty());

        let printed = directives(&filter_from(""));
        assert!(printed.contains("info"), "{printed}");
        for target in CREDENTIAL_BEARING_TARGETS {
            assert!(printed.contains(&format!("{target}=info")), "{printed}");
        }
    }

    /// The realistic case: somebody raises the whole thing to debug the control
    /// plane. Everything of ours gets louder; the pinned crates do not.
    #[test]
    fn a_global_trace_survives_and_the_pins_outrank_it() {
        let printed = directives(&filter_from("trace"));
        assert!(printed.contains("trace"), "{printed}");
        for target in CREDENTIAL_BEARING_TARGETS {
            assert!(printed.contains(&format!("{target}=info")), "{printed}");
            assert!(!printed.contains(&format!("{target}=trace")), "{printed}");
        }
    }

    /// Our own crates stay adjustable, which is the point of an allowlist rather
    /// than a blanket refusal: the guard is about somebody else's log lines.
    #[test]
    fn our_own_targets_survive() {
        for ours in [
            "prices_api=trace",
            "prices_api::portal::keys=debug",
            "prices_clickhouse=warn",
        ] {
            let printed = directives(&filter_from(ours));
            assert!(
                printed.contains("prices_"),
                "`{ours}` was dropped: {printed}"
            );
        }

        // Case-sensitively, because that is how `EnvFilter` matches targets and
        // how Rust module paths are spelled: `PRICES_API` would enable nothing
        // even if it were kept, so it is refused like any other unrecognised
        // target rather than quietly honoured.
        let shouting = directives(&filter_from("PRICES_API=TRACE"));
        assert!(!shouting.contains("PRICES_API"), "{shouting}");
    }

    /// Everything that is not ours and not a bare level, including both shapes
    /// that walked past the blocklist this replaced.
    ///
    /// The single trailing colon is the one that matters most: `EnvFilter`
    /// matches targets by `starts_with` and orders directives by target length,
    /// so `aws_smithy_runtime:` is longer than the `aws_smithy_runtime` pin and
    /// beat it for `aws_smithy_runtime::client::orchestrator`.
    #[test]
    fn nothing_else_survives() {
        for hostile in [
            "aws_smithy_runtime=trace",
            "aws_smithy_runtime:=trace",
            "aws_smithy_runtime::=trace",
            "aws_smithy_runtime::client::orchestrator=trace",
            "aws_smithy=trace",
            "aws=trace",
            "aws_sdk_apigateway",
            "hyper=debug",
            "[try_op]=trace",
            "aws_smithy[try_op]=trace",
            "prices_api[req]=trace",
            "[{code.namespace}]=trace",
        ] {
            let printed = directives(&filter_from(hostile));
            assert!(
                !printed.contains("trace") && !printed.contains("debug"),
                "`{hostile}` survived the guard: {printed}"
            );
        }
    }

    /// A dropped directive must not take the rest of the line with it, and must
    /// not leave a filter that logs nothing.
    #[test]
    fn a_mixed_line_keeps_the_half_that_is_allowed() {
        let printed = directives(&filter_from("hyper=debug,prices_api=trace,[try_op]=trace"));
        assert!(printed.contains("prices_api=trace"), "{printed}");
        assert!(!printed.contains("hyper"), "{printed}");
        assert!(!printed.contains("try_op"), "{printed}");
    }

    /// A line consisting entirely of refused directives falls back to `info`
    /// rather than to silence — an operator who typed something we would not
    /// honour still gets the logs the deployment normally produces.
    #[test]
    fn a_wholly_refused_line_still_logs_at_info() {
        let printed = directives(&filter_from("hyper=debug,[try_op]=trace"));
        assert!(printed.contains("info"), "{printed}");
    }
}
