//! Report drift between `schema/rollups.sql` and a target's live rollup MVs.
//!
//!     cargo run -p prices-clickhouse --bin prices-clickhouse-drift
//!     cargo run -p prices-clickhouse --bin prices-clickhouse-drift -- --verbose
//!
//! **Read-only.** Issues `SELECT`s against `system.tables` and the
//! `formatQuerySingleLine` function; creates, alters and drops nothing. Unlike
//! `prices-clickhouse-init` it therefore needs no DDL grants — a reader account
//! that can see `system.tables` is enough, so this can be run against ch-prod-01
//! without the privileged operator path.
//!
//! Why it exists (task 0142): the rollup MVs are declared `IF NOT EXISTS`, which
//! does not redefine an object that already exists. Editing a body and
//! re-applying the file changes nothing on a provisioned target **and reports
//! success**. This turns that silent no-op into a loud one.
//!
//! Exit codes:
//!   0  every declared MV is present, matches the file, and is APPEND mode
//!   1  drift, a missing MV, a replace-mode MV, or the check could not run
//!
//! Reads `CLICKHOUSE_URL`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`,
//! `CLICKHOUSE_DATABASE` from the environment (local-dev defaults otherwise).

use prices_clickhouse::{
    Config, client,
    drift::{MvStatus, check_rollup_drift},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let verbose = std::env::args().any(|a| a == "--verbose");

    let cfg = Config::from_env();
    tracing::info!(
        url = %cfg.url,
        user = %cfg.user,
        database = %cfg.database,
        "checking rollup MV drift (read-only)"
    );

    let reports = check_rollup_drift(&client(&cfg), &cfg.database).await?;

    // A report that lists nothing is indistinguishable from a clean one at a
    // glance, and this tool exists precisely because a false all-clear is the
    // failure mode. The file declares six.
    if reports.is_empty() {
        println!("FAIL  the check produced no reports at all — nothing was compared");
        std::process::exit(1);
    }

    let mut attention = 0usize;
    for report in &reports {
        match &report.status {
            MvStatus::InSync => println!("ok       {}", report.name),
            MvStatus::Missing => println!(
                "MISSING  {} — declared in rollups.sql, absent on the target; \
                 this tier is not rolling up",
                report.name
            ),
            MvStatus::Unparseable(rendering) => {
                println!(
                    "UNKNOWN  {} — exists, but its definition is not a shape this \
                     check can read (re-created without REFRESH?)",
                    report.name
                );
                println!("             live: {rendering}");
            }
            MvStatus::Undeclared => println!(
                "EXTRA    {} — not declared in rollups.sql, but writes into a table \
                 the declared MVs own; two writers into one ReplacingMergeTree",
                report.name
            ),
            MvStatus::Drifted(differences) => {
                println!(
                    "DRIFT    {} — differs: {}; re-applying rollups.sql will NOT fix this",
                    report.name,
                    differences
                        .iter()
                        .map(|d| d.field.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                for d in differences {
                    println!("           {}", d.field.as_str());
                    if verbose {
                        println!("             declared: {}", d.declared);
                        println!("             live:     {}", d.live);
                    } else {
                        // Excerpt around the FIRST divergence, not the start of
                        // the string. A rollup body is ~700 characters that
                        // agree for the first several hundred, so truncating
                        // from the head prints two identical lines and tells the
                        // operator nothing about what actually changed.
                        let at = first_difference(&d.declared, &d.live);
                        println!("             declared: {}", excerpt(&d.declared, at));
                        println!("             live:     {}", excerpt(&d.live, at));
                    }
                }
            }
        }

        // Reported independently of drift: an in-sync file is no defence if what
        // both sides hold is replace mode. A refreshable MV without APPEND
        // atomically replaces its whole target on every refresh, which against
        // these bounded windows deletes pre-rolled history — the task 0090 data
        // loss that task 0095 fixed.
        if let Some(live) = &report.live
            && !live.is_append()
        {
            println!(
                "CRITICAL {} is NOT in APPEND mode (refresh: {}) — every refresh \
                 replaces the whole of {} with just its window",
                report.name, live.refresh, live.target
            );
        }

        if report.needs_attention() {
            attention += 1;
        }
    }

    if attention == 0 {
        // The second clause is the part the file alone cannot support: walking
        // rollups.sql finds only what it declares, so without the sweep this
        // line would read as a whole-chain all-clear while claiming only that
        // the six named objects are fine.
        println!(
            "\n{} rollup MVs in sync with rollups.sql, and nothing undeclared \
             writes into their targets",
            reports.len()
        );
        return Ok(());
    }

    println!(
        "\n{attention} of {} rollup MVs need attention — see \
         docs/runbooks/0142-rollup-mv-reapply.md",
        reports.len()
    );
    if !verbose {
        // The excerpts centre on the FIRST divergence only; a definition can
        // differ in more than one place.
        println!("re-run with --verbose for the full definitions");
    }
    std::process::exit(1);
}

/// Character index where two renderings first diverge; the shorter length when
/// one is a prefix of the other.
fn first_difference(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.chars().count().min(b.chars().count()))
}

/// A window of `s` centred on `at`, elided at each end it cuts. Keeps the
/// default output scannable while still showing the divergence; `--verbose`
/// prints the full renderings.
fn excerpt(s: &str, at: usize) -> String {
    const BEFORE: usize = 40;
    const AFTER: usize = 120;

    let total = s.chars().count();
    if total <= BEFORE + AFTER {
        return s.to_string();
    }

    let start = at.saturating_sub(BEFORE);
    let end = (start + BEFORE + AFTER).min(total);

    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(s.chars().skip(start).take(end - start));
    if end < total {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_difference_finds_the_divergence_and_handles_a_prefix() {
        assert_eq!(first_difference("abcdef", "abcXef"), 3);
        assert_eq!(first_difference("abc", "abcdef"), 3);
        assert_eq!(first_difference("abc", "abc"), 3);
    }

    /// The whole point of the excerpt: two long strings that agree for hundreds
    /// of characters must print DIFFERENT windows, or the report shows the
    /// operator two identical lines.
    #[test]
    fn the_excerpt_is_centred_on_the_divergence_not_on_the_start() {
        let declared = format!(
            "{}argMaxIf(close_usd, x, close_usd > 0){}",
            "a".repeat(400),
            "z".repeat(400)
        );
        let live = format!("{}argMax(close_usd, x){}", "a".repeat(400), "z".repeat(400));

        let at = first_difference(&declared, &live);
        let (d, l) = (excerpt(&declared, at), excerpt(&live, at));

        assert_ne!(d, l, "the two excerpts must not be identical");
        assert!(d.contains("argMaxIf(close_usd"), "got: {d}");
        assert!(l.contains("argMax(close_usd"), "got: {l}");
        assert!(
            d.starts_with('…') && d.ends_with('…'),
            "elisions missing: {d}"
        );
    }

    #[test]
    fn a_short_rendering_is_printed_whole() {
        assert_eq!(excerpt("EVERY 1 DAY APPEND", 12), "EVERY 1 DAY APPEND");
    }
}
