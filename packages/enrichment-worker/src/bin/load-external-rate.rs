//! Operator entrypoint for task 0267's external USD-rate loader.
//!
//! Loads task 0265's composed USDC/USD history into `prices.usd_rate` as
//! measured, IMPORTED rows, so `/v1/assets/USDC:GA5Z…/ohlcv` stops publishing a
//! literal 1.0 labelled `peg` for every pre-2026-03-11 bucket the series covers
//! — and reports `0.96812` on 2023-03-11, the day USDC actually depegged.
//!
//! ⚠️ This binary is a THIN DRIVER. Every decision — parsing, the six refusals,
//! the epoch partition, the rendered statements — lives in
//! `enrichment_worker::external_rate`, which compiles in the default build and
//! is unit-tested in CI. This file needs `--features aws-mtls` for the Hetzner
//! transport, and Cargo SILENTLY SKIPS a `[[bin]]` whose `required-features` are
//! unmet, so anything living here would be invisible to `cargo test
//! --workspace`. Keep it that way: clap wiring, the transport match, and
//! `.execute()`.
//!
//! Runbook: `docs/runbooks/load-external-usdc-rate.md`. The order there is not
//! advisory — dry run, shadow, three verification queries, promote.
//!
//!   # 1. dry run (writes nothing, prints the figures the runbook gates on):
//!   cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
//!     --transport hetzner --dry-run --grain daily \
//!     lore/1-tasks/archive/0265_FEATURE_price-usdc-from-measurement-not-the-peg/data/composed_usdc_usd_1d.csv
//!
//!   # 2. shadow load (method = 'external-candidate'; nothing reads it yet):
//!   cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
//!     --transport hetzner --grain daily <csv>
//!
//!   # 2b. then the HOURLY file, SECOND — it must win the shared midnight key:
//!   cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
//!     --transport hetzner --grain hourly <csv_1h>
//!
//!   # 3. promote, AFTER the runbook's verification queries pass:
//!   cargo run -p enrichment-worker --features aws-mtls --bin load-external-rate -- \
//!     --transport hetzner --promote <csv>
//!
//! mTLS env (same as coarse-repair / sdex-backfill): CH_DOMAIN, MTLS_CERT_PATH,
//! MTLS_KEY_PATH, MTLS_CA_PATH.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use enrichment_worker::external_rate::{
    DEPEG_DAY_START_S, DEPEG_HOUR_S, Grain, LoadPlan, PROMOTED_METHOD, SERVER_TIMEZONE_SQL,
    SHADOW_METHOD, check_identity, check_server_timezone, insert_statements, parse_csv, partition,
    promote_statement,
};
use prices_clickhouse::{USDC_ISSUER, USDC_ORACLE_EPOCH_S};
use tracing::info;

/// clap's view of [`Grain`]. A separate enum so the lib module stays free of a
/// clap dependency — the same split as the rest of this binary.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum GrainArg {
    /// `composed_usdc_usd_1d.csv` — one row per UTC day, stamped 00:00:00.
    Daily,
    /// `composed_usdc_usd_1h.csv` — one row per UTC hour, stamped mm:ss = 00:00.
    Hourly,
}

impl From<GrainArg> for Grain {
    fn from(g: GrainArg) -> Self {
        match g {
            GrainArg::Daily => Grain::Daily,
            GrainArg::Hourly => Grain::Hourly,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Transport {
    /// Plain HTTP to a local ClickHouse (testing).
    Local,
    /// Direct mTLS to the Hetzner prod CH (needs `--features aws-mtls`).
    Hetzner,
}

#[derive(Parser, Debug)]
#[command(
    name = "load-external-rate",
    about = "Load a composed external USD/asset series into prices.usd_rate (task 0267)"
)]
struct Args {
    /// The composed CSV to load (task 0265's artefact, read 1:1). Not needed
    /// with `--promote`, which rewrites the staged rows already in the table
    /// and parses nothing (review round 3, CR-03).
    #[arg(required_unless_present = "promote")]
    csv: Option<PathBuf>,

    /// Which composed file this is. `daily` accepts ONLY 00:00:00 UTC stamps;
    /// `hourly` accepts any full hour (mm:ss = 00:00). Everything else — the
    /// six refusals, the epoch partition, the shadow/promote split, the `method`
    /// values — is identical.
    ///
    /// ⚠️ Defaults to `daily`, and the default is not symmetric: every midnight
    /// is also a full hour, so the DAILY file parses at `--grain hourly` too.
    /// Only the hourly file is refused at the wrong grain (at its 01:00 row).
    /// Name the flag beside the file, as the runbook does.
    ///
    /// ⚠️ THE TWO GRAINS SHARE EVERY MIDNIGHT KEY AND CARRY DIFFERENT VALUES
    /// THERE — the daily row is the DAY's close, the hourly row the 00:00
    /// HOUR's. `usd_rate` is a ReplacingMergeTree keyed on
    /// (identity, timestamp, method), so the later `version` wins. Load DAILY
    /// FIRST and HOURLY SECOND: `price_usd_series` argMaxes to the 23:00 row,
    /// whose close is the daily close, so the daily surface is unaffected —
    /// while the hourly surfaces need the hour's own number at 00:00.
    #[arg(long, value_enum, default_value_t = GrainArg::Daily)]
    grain: GrainArg,

    /// `local` (plain HTTP) or `hetzner` (mTLS direct-write).
    #[arg(long, value_enum, default_value_t = Transport::Local)]
    transport: Transport,

    /// ClickHouse HTTP URL for `--transport local`.
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://localhost:8123")]
    clickhouse_url: String,

    /// Caddy host fronting Hetzner CH, for `--transport hetzner`.
    #[arg(long, env = "CH_DOMAIN")]
    ch_domain: Option<String>,

    /// Target CH database.
    #[arg(long, env = "CH_DATABASE", default_value = "prices")]
    database: String,

    #[arg(long, env = "MTLS_CERT_PATH")]
    mtls_cert_path: Option<PathBuf>,
    #[arg(long, env = "MTLS_KEY_PATH")]
    mtls_key_path: Option<PathBuf>,
    #[arg(long, env = "MTLS_CA_PATH")]
    mtls_ca_path: Option<PathBuf>,

    /// Asset code the series describes. Defaults to canonical USDC, and the
    /// identity gate refuses everything else — a different asset needs its own
    /// composed series and its own review, not this flag.
    #[arg(long, default_value = "USDC")]
    asset_code: String,

    /// Issuer the series describes. Defaults to Circle's own issuance. Asset
    /// codes are not unique on Stellar; this is the task 0173 gate.
    #[arg(long, default_value = USDC_ISSUER)]
    issuer: String,

    /// Preview: parse, validate, partition and print the counts plus the
    /// 2023-03-11 row. Writes nothing and opens no write path.
    #[arg(long)]
    dry_run: bool,

    /// Write the rows under the PRE-PROMOTION staging method
    /// (`external-candidate`). ON by default, and the only mode that writes
    /// rows from the CSV. No read predicate anywhere names that word, so staged
    /// rows are inert until `--promote` runs — that is what makes the runbook's
    /// three verification queries a real gate rather than a formality.
    #[arg(long, default_value_t = true)]
    shadow: bool,

    /// Promote the already-staged rows to `external`, which is the word every
    /// consumer reads.
    ///
    /// ⚠️ ADDITIVE. `method` is part of `usd_rate`'s ORDER BY, so the staged
    /// rows and the promoted rows are DIFFERENT ReplacingMergeTree keys and BOTH
    /// SURVIVE. This adds a key; it does not move one, and it deletes nothing.
    /// The staged rows stay behind, inert and negligible in size. Re-running the
    /// promote is idempotent only because RMT dedups the identical promoted key
    /// on the higher version.
    ///
    /// Reads no rows from the CSV — the CSV is still required, and still
    /// validated, because a promote against a file that no longer parses is an
    /// operator running the wrong command.
    #[arg(long, conflicts_with = "shadow")]
    promote: bool,
}

fn print_plan(plan: &LoadPlan, grain: Grain, dry_run: bool) {
    println!("\n=== load-external-rate plan ===");
    println!("{:>28}  {}", "grain", grain.label());
    println!("{:>28}  {}", "rows parsed", plan.parsed);
    println!("{:>28}  {}", "loadable (below epoch)", plan.loadable.len());
    println!(
        "{:>28}  {}   (epoch = {} — task 0268's USDC_ORACLE_EPOCH_S)",
        "skipped (at/above epoch)", plan.skipped_at_or_above_epoch, USDC_ORACLE_EPOCH_S
    );
    for (q, n) in &plan.by_quality {
        println!("{:>28}  {}", format!("quality {q}"), n);
    }
    for (s, n) in &plan.by_source {
        println!("{:>28}  {}", format!("source {s}"), n);
    }
    if let Some((first, last)) = plan.loadable_span() {
        println!("{:>28}  {} .. {}", "loadable span (unix)", first, last);
    }

    // The falsifier, per grain. Printed by name because it is what the runbook
    // gates on: if this row is absent or reads 1.0, do not load.
    //
    // At daily grain that is 2023-03-11 itself (0.96812, the day's close). At
    // hourly grain the DAY's close is a poor falsifier — the peg had largely
    // recovered by 23:00 — so the trough hour 07:00 (0.8833) is printed too. It
    // is the number a daily-only load cannot show at `granularity=1h`, and
    // therefore the whole argument for this grain.
    let show = |label: &str, ts: u32| match plan.loadable.iter().find(|r| r.ts == ts) {
        Some(r) => println!("{:>28}  {} ({}, {})", label, r.rate, r.source, r.quality),
        None => println!(
            "{:>28}  ABSENT — this is what the whole task exists for. STOP.",
            label
        ),
    };
    show("2023-03-11 00:00 close", DEPEG_DAY_START_S);
    if grain == Grain::Hourly {
        show("2023-03-11 07:00 close", DEPEG_HOUR_S);
        show("2023-03-11 23:00 close", DEPEG_DAY_START_S + 23 * 3600);
    }
    if dry_run {
        println!("[DRY RUN — nothing written]");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let args = Args::parse();

    // ⚠️ `--shadow` DEFAULTS to true, and clap's `conflicts_with` only fires on
    // an EXPLICITLY supplied argument — a defaulted value is not "present" for
    // conflict purposes. That is deliberate here: it lets a bare `--promote`
    // work while still refusing the contradictory `--shadow --promote`.
    //
    // The promote/dry-run pair is not covered by that, and is refused here,
    // BEFORE a connection is opened: a promote has nothing to preview, so the
    // combination can only be an operator reaching for the wrong step.
    if args.promote && args.dry_run {
        return Err(
            "--promote and --dry-run are mutually exclusive: a promote has \
             nothing to preview. Run the dry run against the CSV first, then the \
             shadow load, then the runbook's three verification queries, and only \
             then --promote."
                .into(),
        );
    }

    // ⚠️ Every refusal is rendered through Display (`to_string`) before it
    // becomes a `Box<dyn Error>`. Returning the `LoadError` directly compiles
    // and looks right, but `main`'s Result reports its error via DEBUG, so the
    // operator would get `ForeignIdentity { code: "USDT", … }` instead of the
    // sentence explaining what to do — which is the entire value of these
    // messages. Same shape as coarse-repair's `format!(…).into()` refusals.
    //
    // The task 0173 ticker→issuer gate, in code.
    check_identity(&args.asset_code, &args.issuer).map_err(|e| e.to_string())?;

    let grain: Grain = args.grain.into();
    // A promote parses nothing: it rewrites the staged rows already in the
    // table, whichever grain wrote them. Parsing the CSV here at the default
    // daily grain made `--promote <csv_1h>` abort on the file's 01:00 row
    // before the promote branch was reached (review round 3, CR-03).
    let plan = match (&args.csv, args.promote) {
        (Some(csv), false) => {
            let text = std::fs::read_to_string(csv)
                .map_err(|e| format!("could not read {}: {e}", csv.display()))?;
            let plan = partition(
                parse_csv(&csv.display().to_string(), &text, grain).map_err(|e| e.to_string())?,
            );
            print_plan(&plan, grain, args.dry_run);
            Some(plan)
        }
        (_, true) => None,
        (None, false) => return Err("a CSV path is required unless --promote is given".into()),
    };

    if args.dry_run {
        let plan = plan.as_ref().expect("dry run always parses");
        info!(
            parsed = plan.parsed,
            loadable = plan.loadable.len(),
            skipped = plan.skipped_at_or_above_epoch,
            grain = grain.label(),
            "dry run complete — nothing written"
        );
        return Ok(());
    }

    // ONE version for the whole run, resolved here and logged. Every row of one
    // run must carry the same value, or a re-run's rows become indistinguishable
    // from the first run's and `version` stops answering "which import wrote
    // this".
    let version = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let mode = if args.promote { "promote" } else { "shadow" };
    info!(
        version,
        mode,
        grain = grain.label(),
        database = %args.database,
        transport = ?args.transport,
        rows = plan.as_ref().map_or(0, |p| p.loadable.len()),
        "load-external-rate starting"
    );

    let statements: Vec<String> = if args.promote {
        vec![promote_statement(
            &args.database,
            version,
            &args.asset_code,
            &args.issuer,
        )]
    } else {
        insert_statements(
            &args.database,
            SHADOW_METHOD,
            version,
            &args.asset_code,
            &args.issuer,
            &plan.as_ref().expect("shadow always parses").loadable,
        )
    };
    if statements.is_empty() {
        return Err(
            "nothing to write: the loadable set is empty. Either the file \
             holds only days at or above the oracle epoch, or it is not the \
             composed series."
                .into(),
        );
    }

    let client = match args.transport {
        Transport::Local => clickhouse::Client::default()
            .with_url(&args.clickhouse_url)
            .with_database(&args.database),
        Transport::Hetzner => {
            let domain = args
                .ch_domain
                .as_deref()
                .ok_or("--transport hetzner requires --ch-domain / CH_DOMAIN")?;
            let cert = args
                .mtls_cert_path
                .as_deref()
                .ok_or("--transport hetzner requires --mtls-cert-path / MTLS_CERT_PATH")?;
            let key = args
                .mtls_key_path
                .as_deref()
                .ok_or("--transport hetzner requires --mtls-key-path / MTLS_KEY_PATH")?;
            let ca = args
                .mtls_ca_path
                .as_deref()
                .ok_or("--transport hetzner requires --mtls-ca-path / MTLS_CA_PATH")?;
            prices_clickhouse::mtls::client_with_mtls_from_paths(
                domain,
                cert,
                key,
                ca,
                &args.database,
            )?
        }
    };

    // The one gate that retires the whole timezone class (review round 3,
    // CR-04): every boundary this repo computes is server-local, so nothing
    // is written unless the server itself is UTC.
    let tz: String = client.query(SERVER_TIMEZONE_SQL).fetch_one().await?;
    check_server_timezone(&tz).map_err(|e| e.to_string())?;
    info!(timezone = %tz, "server timezone verified");

    let total = statements.len();
    for (i, stmt) in statements.iter().enumerate() {
        client.query(stmt).execute().await?;
        info!(statement = i + 1, of = total, "written");
    }

    if args.promote {
        println!(
            "\npromoted the staged rows to method = '{PROMOTED_METHOD}' at version {version}.\n\
             The '{SHADOW_METHOD}' rows are still there and still inert — `method` is part of \
             the sorting key, so this ADDED a key rather than moving one. Nothing was deleted.\n\
             Next: apply the schema and the views, deploy the API, then run the 2023-03-11 \
             curl in docs/runbooks/load-external-usdc-rate.md."
        );
    } else {
        println!(
            "\nstaged {} row(s) as method = '{SHADOW_METHOD}' at version {version}.\n\
             NOTHING READS THAT WORD YET. Run the three verification queries in \
             docs/runbooks/load-external-usdc-rate.md before --promote.",
            plan.as_ref().map_or(0, |p| p.loadable.len())
        );
    }

    Ok(())
}
