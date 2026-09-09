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

use chrono::{DateTime, FixedOffset, NaiveTime, Timelike};
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

/// The scale of `usd_rate.usd_rate` — `Decimal(38, 14)`. A rate with MORE
/// fractional places than this is refused (review IN-05): ClickHouse would
/// otherwise decide what to do with the fifteenth place, and a loader whose
/// whole point is the exact decimal must not leave rounding to the server.
pub const RATE_SCALE: u32 = 14;

/// The lower magnitude bound on an accepted rate (review round 2, IN-06).
///
/// [`RATE_SCALE`] bounds the number from below — how FINE it may be — and
/// `rate <= 0` bounds it at the origin, but nothing bounded the integer part:
/// `Decimal(38, 14)` leaves twenty-four integer digits, and a value with more
/// would be rendered into `toDecimal128('…', 14)` and left for the SERVER to
/// decide about. That is the same "let ClickHouse decide" that `RateTooPrecise`
/// exists to prevent, at the other end of the number.
///
/// The band is deliberately TIGHT rather than merely representable: this tool
/// loads one thing, a USD/USDC series. USDC's worst measured hour in five years
/// is 0.8833 (2023-03-11 07:00) and its highest 1.0102, so a value outside
/// [0.5, 1.5] does not mean "an unusual day" — it means the file is not the
/// composed USDC series. A rate of 2 000 would otherwise load silently and
/// publish a two-thousand-dollar stablecoin.
pub const RATE_MIN: &str = "0.5";

/// The upper magnitude bound. See [`RATE_MIN`].
pub const RATE_MAX: &str = "1.5";

/// The grain of a composed CSV, and the only thing that differs between the two
/// files task 0265 produced (Adam, 2026-09-09).
///
/// Both files carry the SAME ten columns and land under the SAME
/// [`PROMOTED_METHOD`]; the only difference the loader cares about is which
/// instants it will accept — a day start, or any full hour.
///
/// ⚠️ THE TWO GRAINS SHARE THE MIDNIGHT KEY, AND THEIR VALUES DIFFER. A daily
/// row stamped `2023-03-11 00:00` carries the whole DAY's close (0.96812); the
/// hourly row at that same instant carries the 00:00 HOUR's close
/// (0.99503491). `prices.usd_rate` is a ReplacingMergeTree keyed on
/// (identity, timestamp, method), so the two are ONE key and the higher
/// `version` wins — i.e. whichever grain was loaded LAST. On the versioned
/// files that is 1 980 of 2 049 shared midnights.
///
/// That is why the runbook loads DAILY FIRST and HOURLY SECOND, and it is not
/// a compromise: with hourly rows present, `price_usd_series` (daily) argMaxes
/// over all twenty-four hours of the day and lands on the 23:00 row, whose
/// close IS the daily close for all 2 049 days (asserted in
/// `composed_usdc_csv.rs`). Loading hourly last therefore leaves the daily
/// surface unchanged and makes the hourly surface right, which the reverse
/// order does not.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Grain {
    /// One row per UTC day, stamped at 00:00:00 — `composed_usdc_usd_1d.csv`.
    #[default]
    Daily,
    /// One row per UTC hour, stamped at mm:ss = 00:00 — `composed_usdc_usd_1h.csv`.
    Hourly,
}

impl Grain {
    /// The word the refusals and the plan report use.
    pub fn label(self) -> &'static str {
        match self {
            Grain::Daily => "daily",
            Grain::Hourly => "hourly",
        }
    }

    /// The bucket a row of this grain is stamped at the start of.
    pub fn bucket(self) -> &'static str {
        match self {
            Grain::Daily => "day",
            Grain::Hourly => "hour",
        }
    }

    /// How a refusal names the only instant this grain accepts.
    pub fn required_instant(self) -> &'static str {
        match self {
            Grain::Daily => "00:00:00 UTC",
            Grain::Hourly => "a full hour UTC (mm:ss = 00:00)",
        }
    }
}

/// 2023-03-11 00:00:00 UTC — the day USDC actually depegged, and the falsifier
/// for this whole task. ONE definition (review IN-04), the same discipline as
/// the oracle epoch: the bin's plan output, the unit tests and the versioned-CSV
/// test all read this constant rather than restating the number.
pub const DEPEG_DAY_START_S: u32 = 1_678_492_800;

/// 2023-03-11 **07:00:00** UTC — the hour of the trough, and the hourly file's
/// falsifier. The daily row for that day closes at 0.96812 because the peg had
/// largely recovered by 23:00; the 07:00 hour closes at **0.8833**, and a
/// consumer asking for `granularity=1h` on the depeg day must be able to SEE
/// that. It is the single strongest argument for loading the hourly grain at
/// all, so it gets a constant rather than a literal, the same discipline as the
/// day above and the oracle epoch.
pub const DEPEG_HOUR_S: u32 = DEPEG_DAY_START_S + 7 * 3600;

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
        "line {line} has timestamp '{value}', which is not a UTC {bucket} START ({why}). Every \
         row of a {grain} series MUST be stamped {required}. Task 0268 resolves a candle's rate \
         at the BUCKET END with a strict `rts < bend`, so a row stamped anywhere later in the \
         {bucket} resolves every bucket to the PREVIOUS {bucket} — and it fails nowhere: the \
         numbers are simply off by one {bucket}, for ever. Fix the export; do not shift the \
         timestamps by hand."
    )]
    TimestampNotBucketStart {
        line: usize,
        value: String,
        grain: &'static str,
        bucket: &'static str,
        required: &'static str,
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
        "the ClickHouse server reports timezone '{got}', not 'UTC'. Every day and hour \
         boundary this loader, the views and the enrichment tiers compute is a server-local \
         boundary (the candle tables are stamped by unzoned toStartOfInterval), so on a \
         non-UTC server the imported rows would land on the wrong day. Nothing was written. \
         Set the server timezone to UTC (or run against a UTC replica) and retry."
    )]
    ServerNotUtc { got: String },

    #[error(
        "line {line} has close '{value}', outside the accepted band [{min}, {max}]. The scale \
         check bounds how FINE a rate may be; this bounds how BIG it may be, which nothing did \
         before (review round 2, IN-06). Decimal(38, 14) leaves twenty-four integer digits, so a \
         wildly wrong number would render into toDecimal128 and let the SERVER decide what to do \
         with it. This tool loads ONE thing — a USD/USDC series whose worst measured hour in five \
         years is 0.8833 and whose highest is 1.0102 — so a value outside this band does not mean \
         an unusual day, it means the file is not the composed USDC series. Check the file before \
         you widen the band."
    )]
    RateOutOfBand {
        line: usize,
        value: String,
        min: String,
        max: String,
    },

    #[error(
        "line {line} has close '{value}', which is not a decimal number. The column is \
         Decimal(38, 14) and this loader never routes a rate through a float, so an \
         unparsable value cannot be approximated — fix the file."
    )]
    UnparsableRate { line: usize, value: String },

    #[error(
        "line {line} has close '{value}' with {scale} fractional places; the column is \
         Decimal(38, {max}) and this loader never rounds. A rate finer than the column would \
         be rounded by the SERVER, silently, and a rate wrong in the last place looks exactly \
         like one that is right. The composer writes at most 8 places — this file is not its \
         output, or the composer changed."
    )]
    RateTooPrecise {
        line: usize,
        value: String,
        scale: u32,
        max: u32,
    },

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
/// `path_hint` is how the refusal names the file — the bin passes the real
/// path (review IN-03), so "`/x/y.csv` has no data rows" tells the operator
/// which of two files they pointed the tool at.
///
/// `grain` decides which instants are accepted and NOTHING else: both composed
/// files carry the same ten columns, the same vocabulary and the same
/// refusals, and both land under the same [`PROMOTED_METHOD`]. Passing
/// [`Grain::Daily`] against the hourly file refuses at its second data row
/// (01:00 is not a day start), which is the desired direction — a grain
/// mismatch is an operator running the wrong command, not something to guess
/// about.
///
/// Hand-rolled on purpose: the file is machine-generated with ten fixed columns
/// and no quoted fields, so a CSV crate would be a new package in the lockfile
/// for thirty lines of splitting. [`LoadError::QuotedField`] is the guard for
/// the day that stops being true — see the threat register entry T-0267-SC.
pub fn parse_csv(
    path_hint: &str,
    text: &str,
    grain: Grain,
) -> Result<Vec<ExternalRateRow>, LoadError> {
    let mut numbered = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .filter(|(_, l)| !l.trim().is_empty());

    let (_, header) = numbered.next().ok_or_else(|| LoadError::NoDataRows {
        path_hint: path_hint.to_string(),
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

        let ts = parse_bucket_start(line, f[0], grain)?;
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
        if rate.scale() > RATE_SCALE {
            return Err(LoadError::RateTooPrecise {
                line,
                value: f[4].to_string(),
                scale: rate.scale(),
                max: RATE_SCALE,
            });
        }
        // Review round 2, IN-06 — the magnitude bound. Deliberately AFTER the
        // scale check so a too-precise rate still reports the more specific
        // refusal, and before anything is pushed, so nothing partial is built.
        let (min, max) = rate_band();
        if rate < min || rate > max {
            return Err(LoadError::RateOutOfBand {
                line,
                value: f[4].to_string(),
                min: min.to_string(),
                max: max.to_string(),
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
            path_hint: path_hint.to_string(),
        });
    }
    Ok(rows)
}

/// The accepted rate band as decimals, parsed from the two string constants so
/// the constants stay greppable and the parse cannot silently disagree with
/// them.
fn rate_band() -> (Decimal, Decimal) {
    (
        Decimal::from_str_exact(RATE_MIN).expect("RATE_MIN is a decimal literal"),
        Decimal::from_str_exact(RATE_MAX).expect("RATE_MAX is a decimal literal"),
    )
}

/// `YYYY-MM-DD HH:MM:SS±HH:MM` → unix seconds, refusing anything that is not
/// the START of a bucket of `grain`. Every refusal is the SAME error with a
/// different `why`, because they are one rule: the instant must be the start of
/// its own UTC bucket.
///
/// ⚠️ The UTC-offset check is not redundant with the time-of-day check at
/// either grain. `2023-03-11 00:00:00+02:00` has a time-of-day of 00:00:00 and
/// is not midnight UTC; `2023-03-11 07:00:00+05:30` is a full hour locally and
/// is 01:30 UTC, which is not a full hour at all. Refusing the offset first
/// means the remaining checks can reason in UTC alone.
fn parse_bucket_start(line: usize, raw: &str, grain: Grain) -> Result<u32, LoadError> {
    let refuse = |why: &'static str| LoadError::TimestampNotBucketStart {
        line,
        value: raw.to_string(),
        grain: grain.label(),
        bucket: grain.bucket(),
        required: grain.required_instant(),
        why,
    };

    let dt: DateTime<FixedOffset> = DateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%:z")
        .map_err(|_| refuse("it is not `YYYY-MM-DD HH:MM:SS+00:00`"))?;
    if dt.offset().local_minus_utc() != 0 {
        return Err(refuse(
            "its UTC offset is not zero, so the wall-clock time here is not the UTC one",
        ));
    }
    let t = dt.naive_utc().time();
    match grain {
        Grain::Daily => {
            if t != NaiveTime::MIN {
                return Err(refuse("its time-of-day is not 00:00:00"));
            }
        }
        Grain::Hourly => {
            if t.minute() != 0 || t.second() != 0 {
                return Err(refuse("its minutes and seconds are not both zero"));
            }
        }
    }
    let secs = dt.timestamp();
    u32::try_from(secs).map_err(|_| refuse("it is outside the range `DateTime` can store"))
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
///
/// ⚠️ The promote RE-APPLIES the epoch bound (review IN-02). [`partition`] is
/// what keeps post-epoch rows out of the staging set today, but the promote is
/// a separate write from a separate run, possibly of an older binary or over a
/// staging set some other file produced — and it is the one write whose rows
/// the read path serves. Decision F puts the boundary in code; this is the last
/// place the code can hold it.
/// The query the loader runs BEFORE any write; its single-row answer goes
/// through [`check_server_timezone`].
pub const SERVER_TIMEZONE_SQL: &str = "SELECT timezone()";

/// Review round 3, CR-04: every day/hour boundary in this repo is computed in
/// the SERVER's timezone (the candle tables are stamped by unzoned
/// `toStartOfInterval`), so pinning `'UTC'` on one operand of a comparison
/// only displaces the defect. The one gate that retires the whole class is
/// refusing to write unless the server itself is UTC. Pure so CI can test it.
pub fn check_server_timezone(reported: &str) -> Result<(), LoadError> {
    match reported.trim() {
        "UTC" | "Etc/UTC" => Ok(()),
        other => Err(LoadError::ServerNotUtc {
            got: other.to_string(),
        }),
    }
}

pub fn promote_statement(database: &str, version: u64, code: &str, issuer: &str) -> String {
    format!(
        "INSERT INTO {database}.usd_rate ({USD_RATE_COLUMNS}) \
         SELECT asset_kind, asset_code, issuer_address, contract_address, timestamp, usd_rate, \
         '{promoted}', reference_asset, quality, hops, {version} \
         FROM {database}.usd_rate FINAL \
         WHERE asset_kind = '{kind}' AND asset_code = '{code}' AND issuer_address = '{issuer}' \
         AND contract_address = '' AND method = '{shadow}' \
         AND timestamp < toDateTime({epoch})",
        epoch = USDC_ORACLE_EPOCH_S,
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

    /// Every test below that does not say otherwise is about the DAILY file, so
    /// the grain is defaulted here rather than repeated forty times. The local
    /// item shadows the glob-imported `super::parse_csv`; `parse_at` reaches the
    /// real one when a test cares which grain it is exercising.
    fn parse_csv(path: &str, text: &str) -> Result<Vec<ExternalRateRow>, LoadError> {
        super::parse_csv(path, text, Grain::Daily)
    }

    fn parse_at(grain: Grain, text: &str) -> Result<Vec<ExternalRateRow>, LoadError> {
        super::parse_csv("fixture.csv", text, grain)
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
        let err = parse_csv(
            "fixture.csv",
            "ts,close\n2023-03-11 00:00:00+00:00,0.96812\n",
        )
        .unwrap_err();
        let msg = err.to_string();
        for col in EXPECTED_HEADER {
            assert!(msg.contains(col), "the refusal must name `{col}`: {msg}");
        }
    }

    #[test]
    fn a_file_with_a_header_and_no_data_rows_is_refused_as_empty() {
        let err = parse_csv("fixture.csv", &csv(&[])).unwrap_err();
        assert!(err.to_string().contains("no data rows"), "got {err}");
    }

    /// Review IN-03: the refusal names the FILE it was given, not "the file".
    /// An operator with two candidate CSVs learns which one was empty.
    #[test]
    fn an_empty_file_refusal_names_the_path_it_was_given() {
        let header_only = csv(&[]);
        for (path, text) in [("/srv/a/composed.csv", ""), ("b.csv", header_only.as_str())] {
            let err = parse_csv(path, text).unwrap_err();
            let msg = err.to_string();
            assert!(msg.starts_with(path), "must open with the path: {msg}");
            assert!(!msg.contains("the file has"), "{msg}");
        }
    }

    #[test]
    fn a_completely_empty_file_is_refused_as_empty() {
        assert!(parse_csv("fixture.csv", "").is_err());
        assert!(parse_csv("fixture.csv", "   \n\n").is_err());
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
        let err = parse_csv(
            "fixture.csv",
            &csv(&[
                "2023-03-11 23:00:00+00:00,0.995,0.995,0.88,0.96812,243,chainlink,measured,4.1,1",
            ]),
        )
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
        let err = parse_csv(
            "fixture.csv",
            &csv(&[
                "2023-03-11 00:00:00+01:00,0.995,0.995,0.88,0.96812,243,chainlink,measured,4.1,1",
            ]),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("+01:00"), "{msg}");
        assert!(msg.contains("00:00:00 UTC"), "{msg}");
    }

    #[test]
    fn a_repeated_timestamp_is_refused_naming_the_repeat() {
        let err = parse_csv("fixture.csv", &csv(&[DEPEG, DEPEG])).unwrap_err();
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
            let err = parse_csv("fixture.csv", &csv(&[&line])).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains(bad),
                "must name the offending rate `{bad}`: {msg}"
            );
        }
    }

    /// Review IN-05: the column is `Decimal(38, 14)`; a fifteenth place would
    /// be rounded by the server, silently. Fourteen places are exactly the
    /// column and pass; the composer's own eight pass.
    #[test]
    fn a_rate_finer_than_the_column_scale_is_refused_and_one_at_the_scale_is_not() {
        let line = |rate: &str| {
            format!("2023-03-11 00:00:00+00:00,0.99,0.99,0.88,{rate},243,chainlink,measured,4.1,1")
        };
        let err = parse_csv("fixture.csv", &csv(&[&line("0.968120000000001")])).unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(
                err,
                LoadError::RateTooPrecise {
                    scale: 15,
                    max: 14,
                    ..
                }
            ),
            "{msg}"
        );
        assert!(msg.contains("15 fractional places"), "{msg}");
        assert!(msg.contains("Decimal(38, 14)"), "{msg}");

        let ok = parse_csv("fixture.csv", &csv(&[&line("0.96812000000001")])).unwrap();
        assert_eq!(
            ok[0].rate.scale(),
            14,
            "exactly the column's scale is accepted"
        );
        assert_eq!(
            render_rate(&ok[0].rate),
            "toDecimal128('0.96812000000001', 14)"
        );
    }

    /// 🔴 Review round 2, IN-06 — the magnitude bound. `RATE_SCALE` says how
    /// FINE a rate may be and `rate <= 0` pins the origin; nothing said how BIG
    /// it may be, and `Decimal(38, 14)` leaves twenty-four integer digits for a
    /// wrong file to fill. The band is tight on purpose: this tool loads a
    /// USD/USDC series, whose extremes over five years are 0.8833 and 1.0102.
    #[test]
    fn a_rate_outside_the_stablecoin_band_is_refused_at_both_ends() {
        let line = |rate: &str| {
            format!("2023-03-11 00:00:00+00:00,0.99,0.99,0.88,{rate},243,chainlink,measured,4.1,1")
        };
        for bad in ["0.49999", "1.50001", "2", "1000000", "0.000001"] {
            let err = parse_csv("fixture.csv", &csv(&[&line(bad)])).unwrap_err();
            let msg = err.to_string();
            assert!(
                matches!(err, LoadError::RateOutOfBand { .. }),
                "`{bad}` must be refused on MAGNITUDE, not on scale or sign: {msg}"
            );
            assert!(msg.contains(bad), "must name the offending rate: {msg}");
            assert!(msg.contains(RATE_MIN) && msg.contains(RATE_MAX), "{msg}");
        }

        // The bounds themselves are INCLUSIVE, and the real extremes of the
        // composed series sit comfortably inside them — a band that refused the
        // depeg trough would be worse than no band at all.
        for ok in [RATE_MIN, RATE_MAX, "0.8833", "1.0101707", "0.96812"] {
            let rows = parse_csv("fixture.csv", &csv(&[&line(ok)]))
                .unwrap_or_else(|e| panic!("`{ok}` must be accepted: {e}"));
            assert_eq!(rows.len(), 1);
        }

        // Ordering: a too-precise rate reports the more specific refusal even
        // though it is also inside the band, and a non-positive one still
        // reports the sign rather than the band.
        assert!(matches!(
            parse_csv("fixture.csv", &csv(&[&line("0.968120000000001")])).unwrap_err(),
            LoadError::RateTooPrecise { .. }
        ));
        assert!(matches!(
            parse_csv("fixture.csv", &csv(&[&line("0")])).unwrap_err(),
            LoadError::NonPositiveRate { .. }
        ));
    }

    // ---- the hourly grain (Adam, 2026-09-09) --------------------------------

    /// At `Grain::Hourly` every FULL hour is accepted and nothing else is. The
    /// rule is minutes and seconds both zero, in UTC — not "a round-looking
    /// wall clock", which is why the offset is checked first.
    #[test]
    fn the_hourly_grain_accepts_every_full_hour_and_refuses_the_rest() {
        let line = |ts: &str| format!("{ts},0.99,0.99,0.88,0.96812,243,chainlink,measured,4.1,1");
        for good in [
            "2023-03-11 00:00:00+00:00",
            "2023-03-11 07:00:00+00:00",
            "2023-03-11 23:00:00+00:00",
        ] {
            let rows = parse_at(Grain::Hourly, &csv(&[&line(good)]))
                .unwrap_or_else(|e| panic!("`{good}` is a full hour UTC: {e}"));
            assert_eq!(rows.len(), 1);
        }
        for bad in [
            "2023-03-11 07:30:00+00:00",
            "2023-03-11 07:00:30+00:00",
            "2023-03-11 07:00:01+00:00",
        ] {
            let err = parse_at(Grain::Hourly, &csv(&[&line(bad)])).unwrap_err();
            let msg = err.to_string();
            assert!(
                matches!(err, LoadError::TimestampNotBucketStart { .. }),
                "{msg}"
            );
            assert!(msg.contains("hourly"), "the refusal names the grain: {msg}");
            assert!(msg.contains("mm:ss = 00:00"), "{msg}");
            assert!(
                msg.contains("PREVIOUS hour"),
                "the refusal must give task 0182's REASON in the reader's own                  units: {msg}"
            );
        }
    }

    /// ⚠️ A non-zero UTC offset is refused at hourly grain too, and it is not
    /// redundant with the minute check: `07:00:00+05:30` is a full hour on the
    /// wall clock and 01:30 UTC, which is not a full hour at all.
    #[test]
    fn an_offset_timestamp_is_refused_at_hourly_grain_even_when_it_looks_round() {
        for bad in ["2023-03-11 07:00:00+05:30", "2023-03-11 07:00:00+02:00"] {
            let err = parse_at(
                Grain::Hourly,
                &csv(&[&format!(
                    "{bad},0.99,0.99,0.88,0.96812,243,chainlink,measured,4.1,1"
                )]),
            )
            .unwrap_err();
            assert!(err.to_string().contains("UTC offset is not zero"), "{err}");
        }
    }

    /// The grain changes WHICH INSTANTS are accepted and nothing else: the six
    /// other refusals, the epoch partition and the rendered statements are the
    /// same code on both paths. Asserted rather than assumed, because "same
    /// otherwise" is exactly the kind of claim that quietly stops being true.
    #[test]
    fn the_hourly_grain_shares_every_other_refusal_and_the_epoch_partition() {
        let at = |ts: &str, tail: &str| format!("{ts},0.99,0.99,0.88,{tail}");
        let hour = "2023-03-11 07:00:00+00:00";

        for (row, want) in [
            (
                at(hour, "0.96812,243,binance,measured,4.1,1"),
                "UnknownSource",
            ),
            (
                at(hour, "0.96812,243,chainlink,guessed,4.1,1"),
                "UnknownQuality",
            ),
            (
                at(hour, "-1,243,chainlink,measured,4.1,1"),
                "NonPositiveRate",
            ),
            (at(hour, "9,243,chainlink,measured,4.1,1"), "RateOutOfBand"),
        ] {
            let err = parse_at(Grain::Hourly, &csv(&[&row])).unwrap_err();
            assert_eq!(
                format!("{err:?}").split(' ').next().unwrap(),
                want,
                "hourly must refuse exactly as daily does: {err}"
            );
        }

        // Duplicates are refused at the HOUR, not merely at the day — two rows
        // for the same hour would collapse non-deterministically in the RMT.
        let dup = at(hour, "0.96812,243,chainlink,measured,4.1,1");
        assert!(matches!(
            parse_at(Grain::Hourly, &csv(&[&dup, &dup])).unwrap_err(),
            LoadError::DuplicateTimestamp { .. }
        ));

        // Decision F, unchanged: the boundary is the shared constant and it is
        // STRICT, so the epoch hour itself is skipped and the hour before is
        // loaded.
        let stamp = |ts: u32| {
            let d = DateTime::from_timestamp(i64::from(ts), 0).unwrap();
            at(
                &d.format("%Y-%m-%d %H:%M:%S+00:00").to_string(),
                "0.96812,243,chainlink,measured,4.1,1",
            )
        };
        let plan = partition(
            parse_at(
                Grain::Hourly,
                &csv(&[
                    &stamp(USDC_ORACLE_EPOCH_S - 3600),
                    &stamp(USDC_ORACLE_EPOCH_S),
                    &stamp(USDC_ORACLE_EPOCH_S + 3600),
                ]),
            )
            .unwrap(),
        );
        assert_eq!(plan.parsed, 3);
        assert_eq!(plan.loadable.len(), 1);
        assert_eq!(plan.loadable[0].ts, USDC_ORACLE_EPOCH_S - 3600);
        assert_eq!(plan.skipped_at_or_above_epoch, 2);
    }

    /// The hourly falsifier's constant, derived through the same parser the
    /// loader uses — the same discipline as the day constant above.
    #[test]
    fn the_depeg_hour_constant_is_2023_03_11_0700_utc() {
        assert_eq!(
            parse_bucket_start(1, "2023-03-11 07:00:00+00:00", Grain::Hourly).unwrap(),
            DEPEG_HOUR_S
        );
        assert_eq!(DEPEG_HOUR_S, DEPEG_DAY_START_S + 7 * 3600);
        const { assert!(DEPEG_HOUR_S < USDC_ORACLE_EPOCH_S) };
    }

    /// Both grains write the SAME words. The shadow/promote split, the method
    /// vocabulary and the rendered column list are grain-independent, which is
    /// what lets one `--promote` cover a load of both files.
    #[test]
    fn both_grains_render_the_same_statement_shape() {
        let daily = parse_at(Grain::Daily, &csv(&[DEPEG])).unwrap();
        let hourly = parse_at(
            Grain::Hourly,
            &csv(&["2023-03-11 07:00:00+00:00,0.90,0.90,0.88,0.8833,26,chainlink,measured,,0.0"]),
        )
        .unwrap();
        let render = |rows: &[ExternalRateRow]| {
            insert_statements("prices", SHADOW_METHOD, 7, "USDC", USDC_ISSUER, rows).join("\n")
        };
        let (d, h) = (render(&daily), render(&hourly));
        for stmt in [&d, &h] {
            assert!(stmt.contains(USD_RATE_COLUMNS), "{stmt}");
            assert!(stmt.contains(&format!("'{SHADOW_METHOD}'")), "{stmt}");
            assert!(!stmt.contains(&format!("'{PROMOTED_METHOD}',")), "{stmt}");
        }
        assert!(
            d.contains(&format!("toDateTime({DEPEG_DAY_START_S})")),
            "{d}"
        );
        assert!(h.contains(&format!("toDateTime({DEPEG_HOUR_S})")), "{h}");
        assert!(h.contains("toDecimal128('0.8833', 14)"), "{h}");
    }

    #[test]
    fn an_unknown_source_is_refused_naming_both_accepted_values() {
        let err = parse_csv(
            "fixture.csv",
            &csv(&[
                "2023-03-11 00:00:00+00:00,0.995,0.995,0.88,0.96812,243,coinbase,measured,4.1,1",
            ]),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("coinbase"), "{msg}");
        for ok in ACCEPTED_SOURCES {
            assert!(msg.contains(ok), "must name `{ok}`: {msg}");
        }
    }

    #[test]
    fn an_unknown_quality_is_refused_naming_all_three_accepted_values() {
        let err = parse_csv(
            "fixture.csv",
            &csv(&[
                "2023-03-11 00:00:00+00:00,0.995,0.995,0.88,0.96812,243,chainlink,guessed,4.1,1",
            ]),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("guessed"), "{msg}");
        for ok in ACCEPTED_QUALITIES {
            assert!(msg.contains(ok), "must name `{ok}`: {msg}");
        }
    }

    #[test]
    fn a_row_with_the_wrong_field_count_is_refused_with_its_line_number() {
        let err =
            parse_csv("fixture.csv", &csv(&["2023-03-11 00:00:00+00:00,0.96812"])).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "{msg}");
    }

    /// The hand-rolled splitter handles no quoting, so it must REFUSE a quoted
    /// field rather than mis-split it. The composed CSV has none (verified over
    /// all 2049 rows); this is the guard for the day that stops being true.
    #[test]
    fn a_quoted_field_is_refused_rather_than_mis_split() {
        let err = parse_csv("fixture.csv", &csv(&[
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
        let rows = parse_csv(
            "fixture.csv",
            &csv(&[
                DEPEG,
                "2026-03-12 00:00:00+00:00,0.9998,0.9998,0.9997,0.9998,2,chainlink,measured,1.1,2",
            ]),
        )
        .expect("a post-epoch row is not a malformed file");
        assert_eq!(rows.len(), 2, "both rows must PARSE");

        let plan = partition(rows);
        assert_eq!(plan.parsed, 2);
        assert_eq!(plan.loadable.len(), 1, "only the pre-epoch row is loadable");
        assert_eq!(plan.skipped_at_or_above_epoch, 1);
        assert_eq!(
            plan.loadable[0].ts, DEPEG_DAY_START_S,
            "2023-03-11 00:00 UTC"
        );
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

    /// Review IN-04: the falsifier instant is defined ONCE, and it is the
    /// instant its name claims. Derived through the same parser the loader
    /// uses on the file, so the constant and the parse rule cannot disagree.
    #[test]
    fn the_depeg_day_start_constant_is_2023_03_11_midnight_utc() {
        let rows = parse_csv("fixture.csv", &csv(&[DEPEG])).unwrap();
        assert_eq!(rows[0].ts, DEPEG_DAY_START_S);
        assert_eq!(
            parse_bucket_start(1, "2023-03-11 00:00:00+00:00", Grain::Daily).unwrap(),
            DEPEG_DAY_START_S
        );
        const { assert!(DEPEG_DAY_START_S < USDC_ORACLE_EPOCH_S) };
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
        let rows = parse_csv("fixture.csv", &csv(&[DEPEG])).unwrap();
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

        // Review IN-02: the promote re-applies the epoch bound itself — it is
        // the one write the read path serves, and `partition` runs in a
        // different invocation, possibly of a different binary.
        assert!(
            s.contains(&format!(
                "AND timestamp < toDateTime({USDC_ORACLE_EPOCH_S})"
            )),
            "the promote must carry the epoch bound: {s}"
        );
        assert_eq!(
            s.matches("toDateTime(").count(),
            1,
            "and exactly one bound, on the shared constant: {s}"
        );

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
    // ---- the runbook's one hand-typed epoch --------------------------------

    /// The operator runbook hand-types the oracle epoch ONCE, as a client
    /// `param`, and every query in it reads `{epoch:UInt32}`. This pins that
    /// single literal to the constant.
    ///
    /// A mirror of `ch_enrich`'s
    /// `the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant`,
    /// and it exists for the same reason: a drifted literal produces a
    /// verification query over the WRONG window that reports a clean count — a
    /// green all-clear over the one boundary the whole load depends on. It lives
    /// beside the loader rather than beside 0268's tier because it pins the
    /// loader's runbook, and it is in this task's LAST commit because
    /// `include_str!` of a file that does not yet exist does not compile.
    #[test]
    fn the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant() {
        const RUNBOOK: &str = include_str!("../../../docs/runbooks/load-external-usdc-rate.md");
        let epoch = USDC_ORACLE_EPOCH_S.to_string();

        assert_eq!(
            RUNBOOK.matches(&epoch).count(),
            1,
            "the epoch must appear exactly once, as `SET param_epoch`"
        );
        assert!(
            RUNBOOK.contains(&format!("SET param_epoch = {epoch}")),
            "the one occurrence must BE the client parameter, not prose"
        );
        assert!(
            RUNBOOK.matches("toDateTime({epoch:UInt32})").count() >= 1,
            "at least one verification query must read the parameter rather \
             than a second copy of the number"
        );

        // No other 2026-era ten-digit epoch may sneak in beside it — with ONE
        // allowance, spelled out rather than hand-waved: the hourly dry-run
        // table quotes the loadable SPAN, whose upper end is the last full hour
        // strictly below the epoch. That is a different number answering a
        // different question, it is what the tool prints, and it is DERIVED
        // here so it cannot drift away from the constant either.
        let last_loadable_hour = (USDC_ORACLE_EPOCH_S - 3600).to_string();
        let stray: Vec<&str> = RUNBOOK
            .split(|c: char| !c.is_ascii_digit())
            .filter(|w| {
                w.len() == 10 && w.starts_with("177") && *w != epoch && *w != last_loadable_hour
            })
            .collect();
        assert!(
            stray.is_empty(),
            "a second hand-typed 2026 epoch in the runbook: {stray:?}"
        );
        assert!(
            RUNBOOK.contains(&last_loadable_hour),
            "the hourly dry-run table must quote the loadable span's upper end, \
             and it is the epoch minus one hour"
        );

        // The figures the runbook gates the operator on must be the ones the
        // tool actually produces. A table that drifted from the code would stop
        // an operator on a correct run, or — worse — wave through a wrong one.
        for figure in [
            // the daily pass
            "2049",
            "1872",
            "177",
            "0.96812",
            // the hourly pass (Adam, 2026-09-09)
            "49176",
            "44918",
            "4258",
            "597",
            "44321",
            "0.8833",
            "0.99503491",
        ] {
            assert!(
                RUNBOOK.contains(figure),
                "the dry-run gate must state `{figure}`"
            );
        }
        // Both grains, both flags, and the order that resolves the shared
        // midnight key — the one thing an operator can get wrong here that no
        // query afterwards will tell them about.
        assert!(RUNBOOK.contains("--grain daily"), "the daily flag");
        assert!(RUNBOOK.contains("--grain hourly"), "the hourly flag");
        assert!(
            RUNBOOK.contains("DAILY FIRST and HOURLY SECOND"),
            "the load order, stated as an order"
        );
        // And the words the operator has to distinguish.
        assert!(RUNBOOK.contains(SHADOW_METHOD), "the staging word");
        assert!(
            RUNBOOK.contains("ADDS a key"),
            "the additive-promote warning"
        );
    }

    #[test]
    fn the_server_timezone_gate_accepts_only_utc() {
        assert!(check_server_timezone("UTC").is_ok());
        assert!(check_server_timezone("Etc/UTC\n").is_ok());
        for tz in ["Europe/Warsaw", "CET", "", "utc"] {
            let err = check_server_timezone(tz).unwrap_err();
            assert!(matches!(err, LoadError::ServerNotUtc { .. }), "{tz}: {err}");
            assert!(err.to_string().contains("Nothing was written"), "{err}");
        }
        assert_eq!(SERVER_TIMEZONE_SQL, "SELECT timezone()");
    }
}
