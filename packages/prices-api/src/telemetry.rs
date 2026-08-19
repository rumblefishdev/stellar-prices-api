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
//! # What the guard does
//!
//! [`filter_from`] takes what the operator asked for, **drops any directive
//! aimed at a credential-bearing crate**, and then pins those crates at
//! [`GUARD_LEVEL`]. Dropping first is what makes it a guard rather than a
//! default: `EnvFilter` resolves the most specific target match, so a pin of
//! `aws_smithy_runtime=info` alone would still lose to a hand-written
//! `RUST_LOG=aws_smithy_runtime::client::orchestrator=trace`. After the drop
//! there is nothing more specific left for it to lose to.
//!
//! `info` rather than `off`, because the SDK's warnings and errors are worth
//! having and neither carries a body: everything measured above is `debug` or
//! `trace`.
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
        .filter(|directive| !directive.is_empty() && !must_be_dropped(directive))
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

/// Whether one `RUST_LOG` directive has to be dropped for the pins to hold.
///
/// Two reasons, and the second one is not obvious:
///
/// 1. **It names a guarded crate** — `aws_sdk_apigateway`,
///    `aws_smithy_runtime::client::orchestrator=trace`,
///    `aws_smithy_runtime[try_op]=trace`. Anything more specific than the pin
///    would win over it, so it goes.
/// 2. **It names no target at all** — `[try_op]=trace`, `[{field=x}]=trace`.
///    This is a *dynamic span directive*, and `EnvFilter` gives those precedence
///    over static target directives for events inside a matching span: the event
///    is enabled by scope, and the `aws_smithy_runtime=info` pin never gets to
///    refuse it. Since the SDK wraps every control-plane call in spans of its
///    own (`try_op`, `try_attempt`, `deserialization`), a targetless directive is
///    a full bypass — and `try_op` is precisely what somebody debugging one call
///    would reach for. Measured, not reasoned about: with `[try_op]=trace` a
///    `trace!` on `aws_smithy_runtime::client::orchestrator` inside that span was
///    emitted in full, pin and all. `tests/telemetry_filter.rs` is that
///    measurement, kept.
///
/// A bare *level* (`trace`, `off`) is neither: it names no target because it is
/// not a target directive at all, and a global level is exactly what the pins are
/// designed to survive. It is kept, and the pins beat it by being more specific.
fn must_be_dropped(directive: &str) -> bool {
    let before_eq = directive.split('=').next().unwrap_or_default();

    // Any span/field directive at all, whatever target it carries. The narrow
    // rule — "drop it only when it names no target" — leaves
    // `aws_smithy[try_op]=trace` standing, which is a prefix match on the same
    // events by a target this guard would not recognise. Dynamic directives can
    // only ever ADD events beyond the static pins, and no deployment here needs
    // one, so the blunt rule is the statable one.
    if before_eq.contains('[') {
        return true;
    }
    let target = before_eq.trim();

    CREDENTIAL_BEARING_TARGETS.iter().any(|guarded| {
        target == *guarded
            || target
                .strip_prefix(guarded)
                .is_some_and(|r| r.starts_with("::"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EnvFilter`'s `Display` is its directive list, which is what these tests
    /// assert on — the filter has no API for asking "what would you do with a
    /// `trace` event from this target" without constructing `Metadata`, which
    /// only a macro can do.
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
    /// plane. Everything else gets louder; these three do not.
    #[test]
    fn a_global_trace_does_not_reach_the_sdk() {
        let printed = directives(&filter_from("trace"));
        assert!(printed.contains("trace"), "{printed}");
        for target in CREDENTIAL_BEARING_TARGETS {
            assert!(printed.contains(&format!("{target}=info")), "{printed}");
            assert!(!printed.contains(&format!("{target}=trace")), "{printed}");
        }
    }

    /// The case a bare pin would have lost: a directive more specific than the
    /// pin. Dropped before the pin is added, so there is nothing left to lose to
    /// — this is the assertion that fails if `filter_from` is ever simplified
    /// into "parse `RUST_LOG`, then `add_directive`".
    #[test]
    fn a_directive_aimed_straight_at_the_leak_site_is_dropped() {
        let printed = directives(&filter_from(
            "info,aws_smithy_runtime::client::orchestrator=trace",
        ));
        assert!(!printed.contains("orchestrator"), "{printed}");
        assert!(printed.contains("aws_smithy_runtime=info"), "{printed}");
    }

    /// A directive that names no target is a dynamic span filter, which outranks
    /// the pins inside a matching span — so it is dropped whatever it says.
    /// `tests/telemetry_filter.rs` shows the leak this prevents; this is the
    /// unit-level statement of the same rule.
    #[test]
    fn a_targetless_span_directive_is_dropped() {
        for hostile in ["[try_op]=trace", "[{code.namespace}]=trace", " [try_op] "] {
            let printed = directives(&filter_from(hostile));
            assert!(
                !printed.contains("try_op") && !printed.contains("code.namespace"),
                "`{hostile}` survived the guard: {printed}"
            );
        }
    }

    /// Every shape `EnvFilter` accepts for naming a target, since a guard that
    /// covers three of the four is a guard with a documented way around it.
    #[test]
    fn every_way_of_naming_a_guarded_target_is_dropped() {
        for hostile in [
            "aws_sdk_apigateway",
            "aws_sdk_apigateway=trace",
            "aws_sdk_apigateway::operation::get_api_key=trace",
            "aws_smithy_runtime[try_op]=trace",
            " aws_smithy_runtime = trace ",
        ] {
            let printed = directives(&filter_from(hostile));
            assert!(
                !printed.contains("trace"),
                "`{hostile}` survived the guard: {printed}"
            );
        }
    }

    /// And the guard is narrow: a directive for anything else is untouched, so
    /// raising the log level still works for the reason people do it.
    #[test]
    fn directives_for_other_crates_survive() {
        let printed = directives(&filter_from("prices_api=trace,hyper=debug"));
        assert!(printed.contains("prices_api=trace"), "{printed}");
        assert!(printed.contains("hyper=debug"), "{printed}");
    }
}
