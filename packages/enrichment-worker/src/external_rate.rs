//! Task 0267 — the pure half of `load-external-rate`: parse, validate,
//! partition and RENDER the statements that load task 0265's composed
//! USDC/USD history into `prices.usd_rate` as measured, imported rows.
//!
//! ## Why this is a lib module and not just the binary
//!
//! `ci.yml` runs `cargo check --workspace` and `cargo test --workspace` with no
//! extra features, and **Cargo silently SKIPS a `[[bin]]` whose
//! `required-features` are unmet** (the comment above the `coarse-repair` entry
//! in `Cargo.toml` says exactly this). The binary needs `aws-mtls` for the
//! Hetzner transport, so anything living inside it is invisible to CI — it
//! would neither compile nor run there. Everything that can be decided without
//! a connection therefore lives HERE, in the default build, and the bin keeps
//! only clap wiring, the transport match and `.execute()`. Same split as
//! `ledger-processor::metrics`.
//!
//! ## The decisions this file implements
//!
//! - **D-01 / Decision A** — a Rust binary with shadow / promote / dry-run
//!   modes and hard refusals expressed as code, not as runbook prose.
//! - **D-02 / Decision B** — the `quality` column on `prices.usd_rate`, filled
//!   from the CSV. Oracle rows keep the `''` default.
//! - **D-05 / Decision E** — [`prices_clickhouse::USDC_ORACLE_EPOCH_S`] is
//!   IMPORTED, never restated. It is the same constant the API's label arm and
//!   task 0268's reset bound key on; two hand-typed epochs would drift and
//!   nothing would fail loudly.
//! - **D-06 / Decision F** — the epoch is a PARTITION, not a refusal. The
//!   composed series runs to 2026-09-04 and our own oracle is primary from
//!   2026-03-11 14:00 UTC; rows at or above the epoch are correct data that
//!   this tool has no business writing, so they are counted and reported,
//!   never written and never treated as a malformed file.
//!
//! ## Shadow, then promote — and the promote ADDS a key
//!
//! [`SHADOW_METHOD`] is written first. No read predicate anywhere names it —
//! not `views.sql`, not `queries_ch::ohlcv_peg_series`, and emphatically not
//! `ch_enrich::external_sql`, which is what re-enriches hundreds of millions of
//! candles. Staged rows are therefore INERT by construction rather than by
//! anyone's discipline, and an operator can verify them against the runbook's
//! three checks before anything serves them.
//!
//! `--promote` then INSERTs the same observations under [`PROMOTED_METHOD`].
//! ⚠️ It **adds** a ReplacingMergeTree key rather than moving one: `method` is
//! part of `usd_rate`'s ORDER BY (`init.sql`), so the staged rows and the
//! promoted rows are DIFFERENT keys and both survive. Nothing is deleted — that
//! is precisely what makes the rollback in the runbook free, because reverting
//! the read path's preference restores today's behaviour without touching a
//! row. Re-running the promote is idempotent only because RMT dedups the
//! identical promoted key on the higher version.
//!
//! ⚠️ [`SHADOW_METHOD`] has [`PROMOTED_METHOD`] as a **prefix**
//! (`external-candidate` / `external`). Every reader must use an EQUALITY (or
//! an `IN` list); a `LIKE 'external%'` or `startsWith(method, 'external')`
//! would select the staged rows too. Pinned by
//! `only_an_equality_predicate_keeps_the_staged_rows_out_of_the_read_path`.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, FixedOffset, NaiveTime};
use prices_clickhouse::{USDC_ISSUER, USDC_ORACLE_EPOCH_S};
use rust_decimal::Decimal;

/// The PRE-PROMOTION staging value. See the module docs: nothing reads it, and
/// that is the whole safety of the shadow/promote split.
pub const SHADOW_METHOD: &str = "external-candidate";

/// The promoted value — the exact word `ch_enrich::external_sql` filters on and
/// the widened views admit. ⚠️ Changing it silently disconnects the loader from
/// every consumer of its rows.
pub const PROMOTED_METHOD: &str = "external";

/// `usd_rate.asset_kind` for a classic credit asset.
pub const ASSET_KIND: &str = "credit";

/// The only asset code this tool loads. The task 0173 ticker→issuer gate as
/// CODE rather than as a comment: asset codes are not unique on Stellar and
/// hundreds of issuers publish a "USDC".
pub const CANONICAL_ASSET_CODE: &str = "USDC";

/// The composer's column order, in full. A file that does not match is refused
/// before a single row is read — a positional parser over an unexpected header
/// reads the wrong column and produces plausible garbage.
pub const EXPECTED_HEADER: &[&str] = &[
    "ts",
    "open",
    "high",
    "low",
    "close",
    "n_obs",
    "source",
    "quality",
    "xcheck_spread_bps",
    "xcheck_sources",
];

/// The two series the composer draws from (task 0265).
pub const ACCEPTED_SOURCES: &[&str] = &["chainlink", "bitstamp"];

/// The composer's confidence vocabulary for a day's observation.
pub const ACCEPTED_QUALITIES: &[&str] = &["measured", "measured-disputed", "fallback"];

/// Every column of `prices.usd_rate`, in table order, including task 0267's
/// `quality`. ⚠️ This list and the `ADD COLUMN` in `init.sql` are the coupling
/// that must not drift: a mismatch makes every INSERT fail LOUDLY, which is the
/// desired direction — the alternative is a silent partial write.
pub const USD_RATE_COLUMNS: &str = "asset_kind, asset_code, issuer_address, contract_address, \
     timestamp, usd_rate, method, reference_asset, quality, hops, version";

/// Rows per INSERT statement. 1872 rows in one request against a SHARED
/// production cluster is inconsiderate at best; the load is not urgent, so it
/// is chunked. Every row survives the chunking exactly once — asserted.
pub const INSERT_CHUNK_ROWS: usize = 250;

/// The scale of `usd_rate.usd_rate` — `Decimal(38, 14)`.
const RATE_SCALE: u32 = 14;

/// One accepted day of the composed series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRateRow {
    /// 1-based line number in the source file, carried for the dry-run report.
    pub line: usize,
    /// UTC day START, as unix seconds. ⚠️ Day start, never day end: task 0268's
    /// `external_sql` resolves at the bucket END with a strict `rts < bend`, so
    /// a day-END stamp resolves every bucket to the PREVIOUS day and fails
    /// nowhere (task 0182's lesson).
    pub ts: u32,
    /// The day's close, as the exact decimal the composer wrote.
    pub rate: Decimal,
    /// The composing series (`chainlink` / `bitstamp`) — lands in
    /// `reference_asset`.
    pub source: String,
    /// The composer's confidence — lands in `quality`.
    pub quality: String,
}

/// What a run would do, computed before anything is written. This IS the
/// dry-run report.
#[derive(Debug, Clone, Default)]
pub struct LoadPlan {
    /// Data rows that parsed (the whole file — loadable plus skipped).
    pub parsed: usize,
    /// Rows strictly below [`USDC_ORACLE_EPOCH_S`], in file order.
    pub loadable: Vec<ExternalRateRow>,
    /// Rows at or above the epoch (Decision F): parsed, counted, never written.
    pub skipped_at_or_above_epoch: usize,
    /// Loadable rows per `source`.
    pub by_source: BTreeMap<String, usize>,
    /// Loadable rows per `quality`.
    pub by_quality: BTreeMap<String, usize>,
}

impl LoadPlan {
    /// The earliest and latest loadable day, or `None` if nothing is loadable.
    pub fn loadable_span(&self) -> Option<(u32, u32)> {
        let first = self.loadable.iter().map(|r| r.ts).min()?;
        let last = self.loadable.iter().map(|r| r.ts).max()?;
        Some((first, last))
    }
}

/// A refusal. Every variant names the offending value, where it is, and what to
/// do about it — a loader that says only "invalid file" costs an operator an
/// afternoon.
///
/// ⚠️ Every one of these refuses the WHOLE FILE. Nothing is partially written,
/// because a half-loaded price history is worse than none: the missing days
/// fall back to the $1 peg and are indistinguishable from days the composer
/// never covered.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LoadError {
    #[error(
        "the file's header does not match the composed series. Expected exactly, in this order: \
         {expected}. Got: {got}. This parser is POSITIONAL, so a header it does not recognise \
         would be read column-by-column into the wrong fields. Re-export from task 0265's \
         composer rather than editing the header by hand."
    )]
    HeaderMismatch { expected: String, got: String },

    #[error(
        "{path_hint} has no data rows — only a header, or nothing at all. Loading zero rows \
         would report success and change nothing, which reads exactly like a load that worked. \
         Check the file is the versioned composed CSV and not a truncated copy."
    )]
    NoDataRows { path_hint: String },

    #[error(
        "this tool loads ONE identity: asset_code '{expected_code}' issued by '{expected_issuer}' \
         (Circle's own issuance). Got code '{code}' issuer '{issuer}'. Asset codes are not unique \
         on Stellar — hundreds of issuers publish a 'USDC' — so an imported USD history attached \
         to the wrong issuer would publish one asset's price under another's name. If you mean a \
         different asset, it needs its own composed series and its own review, not this flag."
    )]
    ForeignIdentity {
        code: String,
        issuer: String,
        expected_code: String,
        expected_issuer: String,
    },

    #[error(
        "line {line} has {got} fields, expected {want}. The composed CSV is machine-generated \
         with a fixed {want}-column shape; a differing row means the file was edited or \
         truncated. Re-export it."
    )]
    FieldCount {
        line: usize,
        got: usize,
        want: usize,
    },

    #[error(
        "line {line} contains a double quote. This parser splits on commas and handles NO \
         quoting, deliberately — the composed CSV has no quoted fields across all of its rows, \
         so a quote means the file is no longer the shape this tool was written for. Refusing \
         rather than mis-splitting it."
    )]
    QuotedField { line: usize },

    #[error(
        "line {line} has timestamp '{value}', which is not a UTC day START ({why}). Every row \
         MUST be stamped 00:00:00 UTC. Task 0268 resolves a candle's rate at the BUCKET END with \
         a strict `rts < bend`, so a row stamped anywhere later in the day resolves every bucket \
         to the PREVIOUS day — and it fails nowhere: the numbers are simply off by a day, for \
         ever. Fix the export; do not shift the timestamps by hand."
    )]
    TimestampNotUtcDayStart {
        line: usize,
        value: String,
        why: &'static str,
    },

    #[error(
        "line {line} repeats timestamp '{value}', first seen on line {first_line}. \
         `prices.usd_rate` is a ReplacingMergeTree keyed on (identity, timestamp, method), so \
         two rows for one day would collapse to whichever the engine merged last — a silent, \
         non-deterministic choice between two prices. Deduplicate the file first."
    )]
    DuplicateTimestamp {
        line: usize,
        value: String,
        first_line: usize,
    },

    #[error(
        "line {line} has close '{value}'. A USD rate must be strictly positive: zero is the \
         'no value' sentinel this whole subsystem exists to avoid publishing as a price, and a \
         negative rate is not a number any composition can produce. Investigate the composer \
         output; do not clamp it here."
    )]
    NonPositiveRate { line: usize, value: String },

    #[error(
        "line {line} has close '{value}', which is not a decimal number. The column is \
         Decimal(38, 14) and this loader never routes a rate through a float, so an \
         unparsable value cannot be approximated — fix the file."
    )]
    UnparsableRate { line: usize, value: String },

    #[error(
        "line {line} has source '{value}'. Accepted: {accepted}. An unrecognised source means \
         either the composer gained a feed (which needs its own review, because `source` reaches \
         the public /ohlcv wire) or the file is not the composed series."
    )]
    UnknownSource {
        line: usize,
        value: String,
        accepted: String,
    },

    #[error(
        "line {line} has quality '{value}'. Accepted: {accepted}. `quality` reaches the public \
         wire and tells a consumer whether to trust the day, so an unrecognised word would be \
         published as meaningless provenance."
    )]
    UnknownQuality {
        line: usize,
        value: String,
        accepted: String,
    },
}

/// The task 0173 ticker→issuer gate. Refuses every identity but canonical USDC.
pub fn check_identity(code: &str, issuer: &str) -> Result<(), LoadError> {
    if code == CANONICAL_ASSET_CODE && issuer == USDC_ISSUER {
        return Ok(());
    }
    Err(LoadError::ForeignIdentity {
        code: code.to_string(),
        issuer: issuer.to_string(),
        expected_code: CANONICAL_ASSET_CODE.to_string(),
        expected_issuer: USDC_ISSUER.to_string(),
    })
}

/// Parse the composed CSV. Refuses the whole file on the first violation.
///
/// Hand-rolled on purpose: the file is machine-generated with ten fixed columns
/// and no quoted fields, so a CSV crate would be a new package in the lockfile
/// for thirty lines of splitting. [`LoadError::QuotedField`] is the guard for
/// the day that stops being true — see the threat register entry T-0267-SC.
pub fn parse_csv(text: &str) -> Result<Vec<ExternalRateRow>, LoadError> {
    let mut numbered = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .filter(|(_, l)| !l.trim().is_empty());

    let (_, header) = numbered.next().ok_or_else(|| LoadError::NoDataRows {
        path_hint: "the file".to_string(),
    })?;
    let got: Vec<&str> = header.split(',').map(str::trim).collect();
    if got != EXPECTED_HEADER {
        return Err(LoadError::HeaderMismatch {
            expected: EXPECTED_HEADER.join(","),
            got: got.join(","),
        });
    }

    let mut rows: Vec<ExternalRateRow> = Vec::new();
    let mut seen: HashMap<u32, usize> = HashMap::new();

    for (line, raw) in numbered {
        if raw.contains('"') {
            return Err(LoadError::QuotedField { line });
        }
        let f: Vec<&str> = raw.split(',').map(str::trim).collect();
        if f.len() != EXPECTED_HEADER.len() {
            return Err(LoadError::FieldCount {
                line,
                got: f.len(),
                want: EXPECTED_HEADER.len(),
            });
        }

        let ts = parse_utc_day_start(line, f[0])?;
        if let Some(first_line) = seen.insert(ts, line) {
            return Err(LoadError::DuplicateTimestamp {
                line,
                value: f[0].to_string(),
                first_line,
            });
        }

        let rate = Decimal::from_str_exact(f[4]).map_err(|_| LoadError::UnparsableRate {
            line,
            value: f[4].to_string(),
        })?;
        if rate <= Decimal::ZERO {
            return Err(LoadError::NonPositiveRate {
                line,
                value: f[4].to_string(),
            });
        }

        let source = f[6];
        if !ACCEPTED_SOURCES.contains(&source) {
            return Err(LoadError::UnknownSource {
                line,
                value: source.to_string(),
                accepted: ACCEPTED_SOURCES.join(", "),
            });
        }
        let quality = f[7];
        if !ACCEPTED_QUALITIES.contains(&quality) {
            return Err(LoadError::UnknownQuality {
                line,
                value: quality.to_string(),
                accepted: ACCEPTED_QUALITIES.join(", "),
            });
        }

        rows.push(ExternalRateRow {
            line,
            ts,
            rate,
            source: source.to_string(),
            quality: quality.to_string(),
        });
    }

    if rows.is_empty() {
        return Err(LoadError::NoDataRows {
            path_hint: "the file".to_string(),
        });
    }
    Ok(rows)
}

/// `YYYY-MM-DD HH:MM:SS±HH:MM` → unix seconds, refusing anything that is not a
/// UTC day start. Both refusals are the SAME error with a different `why`,
/// because they are one rule: the instant must be midnight UTC.
fn parse_utc_day_start(line: usize, raw: &str) -> Result<u32, LoadError> {
    let dt: DateTime<FixedOffset> =
        DateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%:z").map_err(|_| {
            LoadError::TimestampNotUtcDayStart {
                line,
                value: raw.to_string(),
                why: "it is not `YYYY-MM-DD HH:MM:SS+00:00`",
            }
        })?;
    if dt.offset().local_minus_utc() != 0 {
        return Err(LoadError::TimestampNotUtcDayStart {
            line,
            value: raw.to_string(),
            why: "its UTC offset is not zero, so 'midnight' here is not midnight UTC",
        });
    }
    if dt.naive_utc().time() != NaiveTime::MIN {
        return Err(LoadError::TimestampNotUtcDayStart {
            line,
            value: raw.to_string(),
            why: "its time-of-day is not 00:00:00",
        });
    }
    let secs = dt.timestamp();
    u32::try_from(secs).map_err(|_| LoadError::TimestampNotUtcDayStart {
        line,
        value: raw.to_string(),
        why: "it is outside the range `DateTime` can store",
    })
}

/// Decision F. Split the parsed rows at [`USDC_ORACLE_EPOCH_S`]: below is
/// loadable, at-or-above is counted and reported. Never a refusal — the
/// composed series legitimately runs past the epoch, and those days are ours to
/// measure rather than to import.
pub fn partition(rows: Vec<ExternalRateRow>) -> LoadPlan {
    let parsed = rows.len();
    let mut plan = LoadPlan {
        parsed,
        ..LoadPlan::default()
    };
    for r in rows {
        if r.ts >= USDC_ORACLE_EPOCH_S {
            plan.skipped_at_or_above_epoch += 1;
            continue;
        }
        *plan.by_source.entry(r.source.clone()).or_default() += 1;
        *plan.by_quality.entry(r.quality.clone()).or_default() += 1;
        plan.loadable.push(r);
    }
    plan
}

/// Render a rate as an EXACT decimal literal.
///
/// ⚠️ Never through `toFloat64` or an f64 intermediate. The column is
/// `Decimal(38, 14)` and the fourteenth place is the entire reason this table
/// stores a decimal at all; a float round-trip loses it silently, and a rate
/// that is wrong in the last place looks exactly like one that is right.
pub fn render_rate(rate: &Decimal) -> String {
    format!("toDecimal128('{rate}', {RATE_SCALE})")
}

/// Escape a value for a single-quoted SQL literal.
fn sql_lit(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Render the load as chunked `INSERT … VALUES` statements.
///
/// `method` is the caller's choice so the same builder renders both the shadow
/// load and (for tests and for a direct load) the promoted one; the binary only
/// ever passes [`SHADOW_METHOD`]. `version` is the run's unix time, resolved
/// ONCE by the caller so every row of one run carries the same value —
/// otherwise a re-run's rows would be indistinguishable from the first run's.
///
/// Returns an empty vector for an empty slice: an `INSERT … VALUES` with no
/// values is a syntax error, not a no-op.
pub fn insert_statements(
    database: &str,
    method: &str,
    version: u64,
    code: &str,
    issuer: &str,
    rows: &[ExternalRateRow],
) -> Vec<String> {
    rows.chunks(INSERT_CHUNK_ROWS)
        .map(|chunk| {
            let values: Vec<String> = chunk
                .iter()
                .map(|r| {
                    format!(
                        "('{kind}', '{code}', '{issuer}', '', toDateTime({ts}), {rate}, \
                         '{method}', '{source}', '{quality}', 0, {version})",
                        kind = sql_lit(ASSET_KIND),
                        code = sql_lit(code),
                        issuer = sql_lit(issuer),
                        ts = r.ts,
                        rate = render_rate(&r.rate),
                        method = sql_lit(method),
                        source = sql_lit(&r.source),
                        quality = sql_lit(&r.quality),
                    )
                })
                .collect();
            format!(
                "INSERT INTO {database}.usd_rate ({USD_RATE_COLUMNS}) VALUES {}",
                values.join(", ")
            )
        })
        .collect()
}

/// Render the promote: an `INSERT … SELECT` that re-writes the staged rows
/// under [`PROMOTED_METHOD`] at a higher version.
///
/// ⚠️ ADDITIVE. `method` is part of `usd_rate`'s sorting key, so this creates a
/// SECOND ReplacingMergeTree key and the staged rows survive untouched. It
/// contains no `DELETE`, `DROP`, `ALTER` or `TRUNCATE`, and there is nothing to
/// roll back at the row level — the rollback for this whole task is reverting
/// the read path's preference, which deletes nothing.
///
/// The full identity tuple is pinned so a promote cannot reach a neighbouring
/// identity even if one ever carries staged rows (threat T-0267-03).
pub fn promote_statement(database: &str, version: u64, code: &str, issuer: &str) -> String {
    format!(
        "INSERT INTO {database}.usd_rate ({USD_RATE_COLUMNS}) \
         SELECT asset_kind, asset_code, issuer_address, contract_address, timestamp, usd_rate, \
         '{promoted}', reference_asset, quality, hops, {version} \
         FROM {database}.usd_rate FINAL \
         WHERE asset_kind = '{kind}' AND asset_code = '{code}' AND issuer_address = '{issuer}' \
         AND contract_address = '' AND method = '{shadow}'",
        promoted = sql_lit(PROMOTED_METHOD),
        kind = sql_lit(ASSET_KIND),
        code = sql_lit(code),
        issuer = sql_lit(issuer),
        shadow = sql_lit(SHADOW_METHOD),
    )
}

/// The predicate every consumer of these rows applies — rendered here so the
/// shadow/promote guarantee is a Rust ASSERTION over a string this crate
/// produces, rather than a grep over someone else's source file that would rot
/// the moment that file moved.
///
/// ⚠️ It is an EQUALITY. See the module docs: [`SHADOW_METHOD`] has
/// [`PROMOTED_METHOD`] as a prefix, so a `LIKE`/`startsWith` form would select
/// the unverified staged rows too.
pub fn promoted_read_predicate() -> String {
    format!("method = '{PROMOTED_METHOD}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use prices_clickhouse::{USDC_ISSUER, USDC_ORACLE_EPOCH_S};
    use std::str::FromStr;

    /// The 2023-03-11 row, verbatim from the versioned composed CSV. The day
    /// USDC actually depegged, and this task's falsifier.
    const DEPEG: &str =
        "2023-03-11 00:00:00+00:00,0.99503491,0.99503491,0.88,0.96812,243,chainlink,measured,4.1,1";

    fn csv(rows: &[&str]) -> String {
        format!("{}\n{}\n", EXPECTED_HEADER.join(","), rows.join("\n"))
    }

    fn row(ts: u32, rate: &str) -> ExternalRateRow {
        ExternalRateRow {
            line: 2,
            ts,
            rate: Decimal::from_str(rate).unwrap(),
            source: "chainlink".to_string(),
            quality: "measured".to_string(),
        }
    }

    // ---- the six hard refusals (BRIEF acceptance criterion 1) ----------------

    #[test]
    fn a_header_mismatch_is_refused_naming_the_expected_column_order() {
        let err = parse_csv("ts,close\n2023-03-11 00:00:00+00:00,0.96812\n").unwrap_err();
        let msg = err.to_string();
        for col in EXPECTED_HEADER {
            assert!(msg.contains(col), "the refusal must name `{col}`: {msg}");
        }
    }

    #[test]
    fn a_file_with_a_header_and_no_data_rows_is_refused_as_empty() {
        let err = parse_csv(&csv(&[])).unwrap_err();
        assert!(err.to_string().contains("no data rows"), "got {err}");
    }

    #[test]
    fn a_completely_empty_file_is_refused_as_empty() {
        assert!(parse_csv("").is_err());
        assert!(parse_csv("   \n\n").is_err());
    }

    #[test]
    fn a_foreign_asset_code_is_refused_naming_the_only_accepted_identity() {
        let err = check_identity("USDT", USDC_ISSUER).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("USDT"), "must name what was asked for: {msg}");
        assert!(msg.contains(CANONICAL_ASSET_CODE), "{msg}");
        assert!(
            msg.contains(USDC_ISSUER),
            "must name the accepted issuer: {msg}"
        );
    }

    #[test]
    fn a_foreign_issuer_is_refused_by_the_same_gate() {
        let err = check_identity(CANONICAL_ASSET_CODE, "GNOTCIRCLE").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("GNOTCIRCLE"), "{msg}");
        assert!(msg.contains(USDC_ISSUER), "{msg}");
        check_identity(CANONICAL_ASSET_CODE, USDC_ISSUER)
            .expect("canonical USDC is the one identity this tool loads");
    }

    #[test]
    fn a_timestamp_that_is_not_midnight_is_refused_naming_the_day_start_rule() {
        let err = parse_csv(&csv(&[
            "2023-03-11 23:00:00+00:00,0.995,0.995,0.88,0.96812,243,chainlink,measured,4.1,1",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("23:00:00"),
            "must name the offending value: {msg}"
        );
        assert!(
            msg.contains("00:00:00 UTC"),
            "must name the required convention: {msg}"
        );
        assert!(
            msg.contains("PREVIOUS day"),
            "must give the REASON — task 0268 resolves at the bucket END with a \
             strict `rts < bend`, so a day-end stamp resolves every bucket to \
             the previous day and fails nowhere: {msg}"
        );
    }

    #[test]
    fn a_timestamp_with_a_non_utc_offset_is_refused_by_the_same_rule() {
        let err = parse_csv(&csv(&[
            "2023-03-11 00:00:00+01:00,0.995,0.995,0.88,0.96812,243,chainlink,measured,4.1,1",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("+01:00"), "{msg}");
        assert!(msg.contains("00:00:00 UTC"), "{msg}");
    }

    #[test]
    fn a_repeated_timestamp_is_refused_naming_the_repeat() {
        let err = parse_csv(&csv(&[DEPEG, DEPEG])).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("2023-03-11"),
            "must name the repeated timestamp: {msg}"
        );
    }

    #[test]
    fn a_non_positive_rate_is_refused() {
        for bad in ["0", "0.0", "-0.5"] {
            let line = format!(
                "2023-03-11 00:00:00+00:00,0.995,0.995,0.88,{bad},243,chainlink,measured,4.1,1"
            );
            let err = parse_csv(&csv(&[&line])).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains(bad),
                "must name the offending rate `{bad}`: {msg}"
            );
        }
    }

    #[test]
    fn an_unknown_source_is_refused_naming_both_accepted_values() {
        let err = parse_csv(&csv(&[
            "2023-03-11 00:00:00+00:00,0.995,0.995,0.88,0.96812,243,coinbase,measured,4.1,1",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("coinbase"), "{msg}");
        for ok in ACCEPTED_SOURCES {
            assert!(msg.contains(ok), "must name `{ok}`: {msg}");
        }
    }

    #[test]
    fn an_unknown_quality_is_refused_naming_all_three_accepted_values() {
        let err = parse_csv(&csv(&[
            "2023-03-11 00:00:00+00:00,0.995,0.995,0.88,0.96812,243,chainlink,guessed,4.1,1",
        ]))
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("guessed"), "{msg}");
        for ok in ACCEPTED_QUALITIES {
            assert!(msg.contains(ok), "must name `{ok}`: {msg}");
        }
    }

    #[test]
    fn a_row_with_the_wrong_field_count_is_refused_with_its_line_number() {
        let err = parse_csv(&csv(&["2023-03-11 00:00:00+00:00,0.96812"])).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "{msg}");
    }

    /// The hand-rolled splitter handles no quoting, so it must REFUSE a quoted
    /// field rather than mis-split it. The composed CSV has none (verified over
    /// all 2049 rows); this is the guard for the day that stops being true.
    #[test]
    fn a_quoted_field_is_refused_rather_than_mis_split() {
        let err = parse_csv(&csv(&[
            "2023-03-11 00:00:00+00:00,0.995,0.995,0.88,\"0.96812\",243,chainlink,measured,4.1,1",
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("quote"), "got {err}");
    }

    // ---- decision F: the epoch is a PARTITION, not a refusal ----------------

    #[test]
    fn a_row_at_or_above_the_oracle_epoch_is_parsed_and_skipped_not_refused() {
        // 2026-03-12 — the first midnight at or above the epoch
        // (2026-03-11 14:00 UTC). Parsing must SUCCEED and the row must simply
        // not be loadable: the composed series runs to 2026-09-04 and our own
        // oracle is primary from the epoch on, so these rows are correct data
        // that this tool has no business writing.
        let rows = parse_csv(&csv(&[
            DEPEG,
            "2026-03-12 00:00:00+00:00,0.9998,0.9998,0.9997,0.9998,2,chainlink,measured,1.1,2",
        ]))
        .expect("a post-epoch row is not a malformed file");
        assert_eq!(rows.len(), 2, "both rows must PARSE");

        let plan = partition(rows);
        assert_eq!(plan.parsed, 2);
        assert_eq!(plan.loadable.len(), 1, "only the pre-epoch row is loadable");
        assert_eq!(plan.skipped_at_or_above_epoch, 1);
        assert_eq!(plan.loadable[0].ts, 1_678_492_800, "2023-03-11 00:00 UTC");
    }

    #[test]
    fn the_partition_boundary_is_exactly_the_shared_epoch_constant() {
        // Boundary rows are built directly rather than through the CSV: the
        // instant one second below the epoch is 13:59:59, which the midnight
        // rule refuses at parse time. The two rules are independent and the
        // boundary belongs to `partition`.
        let plan = partition(vec![
            row(USDC_ORACLE_EPOCH_S - 1, "0.99"),
            row(USDC_ORACLE_EPOCH_S, "0.99"),
            row(USDC_ORACLE_EPOCH_S + 1, "0.99"),
        ]);
        assert_eq!(plan.loadable.len(), 1, "the bound is strictly `<`");
        assert_eq!(plan.loadable[0].ts, USDC_ORACLE_EPOCH_S - 1);
        assert_eq!(plan.skipped_at_or_above_epoch, 2);
    }

    #[test]
    fn the_plan_counts_by_source_and_by_quality_over_the_loadable_set() {
        let mut a = row(1_000, "0.99");
        a.quality = "fallback".to_string();
        a.source = "bitstamp".to_string();
        let b = row(2_000, "0.99");
        let mut post = row(USDC_ORACLE_EPOCH_S + 86_400, "0.99");
        post.quality = "fallback".to_string();

        let plan = partition(vec![a, b, post]);
        assert_eq!(plan.parsed, 3);
        assert_eq!(
            plan.by_quality.get("fallback"),
            Some(&1),
            "{:?}",
            plan.by_quality
        );
        assert_eq!(plan.by_quality.get("measured"), Some(&1));
        assert_eq!(plan.by_source.get("bitstamp"), Some(&1));
        assert_eq!(plan.by_source.get("chainlink"), Some(&1));
    }

    // ---- the method vocabulary, and why the split is safe -------------------

    #[test]
    fn the_staging_and_promoted_method_values_are_the_locked_literals() {
        assert_eq!(SHADOW_METHOD, "external-candidate");
        assert_eq!(PROMOTED_METHOD, "external");
        assert_ne!(
            SHADOW_METHOD, PROMOTED_METHOD,
            "a collision would let UNVERIFIED rows reach task 0268's \
             re-enrichment of hundreds of millions of candles"
        );
    }

    /// ⚠️ The staging word has the promoted word as a PREFIX. That is safe under
    /// an equality predicate and unsafe under a prefix match, so the read
    /// predicate must be — and stay — an equality.
    #[test]
    fn only_an_equality_predicate_keeps_the_staged_rows_out_of_the_read_path() {
        assert!(
            SHADOW_METHOD.starts_with(PROMOTED_METHOD),
            "if this ever stops being true the warning below can be relaxed"
        );

        let pred = promoted_read_predicate();
        assert_eq!(pred, format!("method = '{PROMOTED_METHOD}'"));
        assert!(
            !pred.contains("LIKE") && !pred.contains("startsWith") && !pred.contains('%'),
            "a prefix match would select `{SHADOW_METHOD}` too: {pred}"
        );
        // The predicate as a whole must not contain the staging literal.
        assert!(!pred.contains(SHADOW_METHOD), "{pred}");
    }

    /// The staging word must not appear in any read surface this repo ships.
    /// `VIEWS_SQL` is the widened read path from this task's own first commit.
    ///
    /// Asserted on the SQL with `-- …` comments STRIPPED: the word is discussed
    /// at length in those comments (that is where the shadow/promote rule is
    /// written down), and a test that could not tell prose from a predicate
    /// would have to be deleted the first time anyone documented the rule.
    #[test]
    fn no_shipped_view_reads_the_staging_method() {
        let code: String = prices_clickhouse::VIEWS_SQL
            .lines()
            .map(|l| match l.find("--") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains(&format!("'{SHADOW_METHOD}'")),
            "a view that reads `{SHADOW_METHOD}` would serve unverified rows"
        );
        assert!(
            code.contains(&format!("'{PROMOTED_METHOD}'")),
            "the promoted word MUST be readable, or the loaded history is \
             written and never served"
        );
    }

    // ---- the statements -----------------------------------------------------

    #[test]
    fn the_shadow_insert_names_every_usd_rate_column_and_stamps_the_run() {
        let rows = parse_csv(&csv(&[DEPEG])).unwrap();
        let stmts = insert_statements(
            "prices",
            SHADOW_METHOD,
            1_757_000_000,
            CANONICAL_ASSET_CODE,
            USDC_ISSUER,
            &rows,
        );
        assert_eq!(stmts.len(), 1);
        let s = &stmts[0];

        for col in [
            "asset_kind",
            "asset_code",
            "issuer_address",
            "contract_address",
            "timestamp",
            "usd_rate",
            "method",
            "reference_asset",
            "quality",
            "hops",
            "version",
        ] {
            assert!(s.contains(col), "the INSERT must name `{col}`: {s}");
        }
        assert!(s.starts_with("INSERT INTO prices.usd_rate ("), "{s}");
        assert!(s.contains(&format!("'{SHADOW_METHOD}'")), "{s}");
        assert!(!s.contains(&format!("'{PROMOTED_METHOD}',")), "{s}");

        // The rate is a DECIMAL string literal. The column is Decimal(38, 14)
        // and the 14th place is the whole point — an f64 intermediate would
        // silently round it.
        assert!(s.contains("toDecimal128('0.96812', 14)"), "{s}");
        assert!(!s.contains("toFloat64"), "{s}");

        assert!(
            s.contains("toDateTime(1678492800)"),
            "day START, 2023-03-11: {s}"
        );
        assert!(
            s.contains("'chainlink'"),
            "reference_asset carries the CSV source: {s}"
        );
        assert!(
            s.contains("'measured'"),
            "quality carries the CSV quality: {s}"
        );
        assert!(
            s.contains("1757000000"),
            "version is the caller's run time: {s}"
        );
        assert!(s.contains(&format!("'{USDC_ISSUER}'")), "{s}");
    }

    #[test]
    fn the_promote_is_an_additive_insert_select_with_no_destructive_verb() {
        let s = promote_statement("prices", 1_757_000_001, CANONICAL_ASSET_CODE, USDC_ISSUER);
        assert!(s.starts_with("INSERT INTO prices.usd_rate ("), "{s}");
        assert!(s.contains("SELECT"), "{s}");
        assert!(s.contains("FROM prices.usd_rate FINAL"), "{s}");
        assert!(
            s.contains(&format!("method = '{SHADOW_METHOD}'")),
            "reads the staged rows: {s}"
        );
        assert!(
            s.contains(&format!("'{PROMOTED_METHOD}'")),
            "writes the promoted word: {s}"
        );
        assert!(s.contains("1757000001"), "with a higher version: {s}");

        // The full identity tuple is pinned, so a promote cannot reach a
        // neighbouring identity even if one ever carries staged rows.
        for pin in [
            &format!("asset_kind = '{ASSET_KIND}'"),
            &format!("asset_code = '{CANONICAL_ASSET_CODE}'"),
            &format!("issuer_address = '{USDC_ISSUER}'"),
            &"contract_address = ''".to_string(),
        ] {
            assert!(
                s.contains(pin.as_str()),
                "the promote must pin `{pin}`: {s}"
            );
        }

        for verb in ["DELETE", "DROP", "ALTER", "TRUNCATE"] {
            assert!(
                !s.contains(verb),
                "the promote is ADDITIVE — `{verb}` has no business in it: {s}"
            );
        }
    }

    #[test]
    fn a_row_set_larger_than_the_chunk_size_spills_into_several_inserts_losing_nothing() {
        let rows: Vec<ExternalRateRow> = (0..INSERT_CHUNK_ROWS * 2 + 7)
            .map(|i| row(1_000 + i as u32 * 86_400, "0.99"))
            .collect();
        let stmts = insert_statements(
            "prices",
            SHADOW_METHOD,
            7,
            CANONICAL_ASSET_CODE,
            USDC_ISSUER,
            &rows,
        );
        assert_eq!(
            stmts.len(),
            3,
            "1872 rows in ONE statement is a large request \
             against a shared production cluster"
        );

        for r in &rows {
            let hits: usize = stmts
                .iter()
                .map(|s| s.matches(&format!("toDateTime({})", r.ts)).count())
                .sum();
            assert_eq!(hits, 1, "row at ts {} must appear exactly once", r.ts);
        }
    }

    #[test]
    fn an_empty_row_slice_produces_no_statement_at_all() {
        assert!(
            insert_statements(
                "prices",
                SHADOW_METHOD,
                1,
                CANONICAL_ASSET_CODE,
                USDC_ISSUER,
                &[]
            )
            .is_empty(),
            "an INSERT with no VALUES is a syntax error, not a no-op"
        );
    }
}
