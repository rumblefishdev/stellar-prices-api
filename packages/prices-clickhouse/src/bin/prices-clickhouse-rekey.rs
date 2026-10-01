//! Task 0139 migration tool (see `prices_clickhouse::rekey`).
//!
//!     prices-clickhouse-rekey <command> [--database DB] [--execute]
//!         [--mtls-domain D --mtls-cert PEM --mtls-key PEM --mtls-ca PEM]
//!
//! Commands and their flags, in window order:
//!
//! - `preflight`; `capture [--rewrite-db]`; `map`; `create`;
//!   `fill|check [--table T] [--partition P] [--source S --target T]`;
//! - `alter-assets [--mutation-timeout SECS]`; `swap [--check-only]`;
//!   `recreate-mvs --source prod-text|generator`;
//!   `rollback [--force-lose-post-swap-rows]`;
//! - `verify`; `gap-backfill [--to 'YYYY-MM-DD HH:MM:SS']`;
//!   `gap-verify [--set window|next-day|period-close]`, or
//!   `gap-verify [--schema pre0139] --from T --to T`.
//!
//! `--rewrite-db` (rehearsal in a scratch `--database`) captures the MVs and
//! views of `prices` rewritten into the scratch database.
//!
//! Without `--execute` nothing is written: the SQL is printed. `preflight`
//! only reads. `--database` defaults to `prices` (a rehearsal names a scratch
//! database). `fill` and `check` default to all eleven copied tables.
//!
//! Connection: loopback via `CLICKHOUSE_URL`, `CLICKHOUSE_USER`,
//! `CLICKHOUSE_PASSWORD` (the password never goes in argv), or mTLS with
//! `--mtls-*` PEM paths on a build with `--features aws-mtls`.
//!
//! `async_insert` never applies to `INSERT … SELECT`, so every copy is
//! synchronous on prod's `async_insert=1` profile.
//!
//! Exit 0 on success, 1 on a refusal, a failed gate or an error.

use std::time::Duration;

use prices_clickhouse::rekey::gap::Postcheck;
use prices_clickhouse::rekey::swap::{FORCE_LOSE, MvSource};
use prices_clickhouse::rekey::{COPIED_TABLES, Fill, Rekey, RekeyError};

const COMMANDS: [&str; 13] = [
    "preflight",
    "capture",
    "map",
    "create",
    "fill",
    "check",
    "alter-assets",
    "swap",
    "recreate-mvs",
    "rollback",
    "verify",
    "gap-backfill",
    "gap-verify",
];
use prices_clickhouse::{Config, client, with_readable_errors};

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    command: String,
    database: String,
    execute: bool,
    table: Option<String>,
    partition: Option<String>,
    source: Option<String>,
    target: Option<String>,
    mtls: Option<[String; 4]>,
    rewrite_db: bool,
    check_only: bool,
    mv_source: Option<MvSource>,
    force_lose: bool,
    mutation_timeout: u64,
    gap_from: Option<String>,
    gap_to: Option<String>,
    gap_set: Option<Postcheck>,
    pre0139: bool,
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        database: "prices".into(),
        mutation_timeout: 1800,
        ..Args::default()
    };
    let mut mtls: [Option<String>; 4] = Default::default();
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--execute" => a.execute = true,
            "--database" => a.database = value()?,
            "--table" => a.table = Some(value()?),
            "--partition" => a.partition = Some(value()?),
            "--source" if a.command == "recreate-mvs" => {
                a.mv_source = Some(match value()?.as_str() {
                    "prod-text" => MvSource::ProdText,
                    "generator" => MvSource::Generator,
                    v => return Err(format!("--source {v}: prod-text or generator")),
                })
            }
            "--source" => a.source = Some(value()?),
            "--target" => a.target = Some(value()?),
            "--mtls-domain" => mtls[0] = Some(value()?),
            "--mtls-cert" => mtls[1] = Some(value()?),
            "--mtls-key" => mtls[2] = Some(value()?),
            "--mtls-ca" => mtls[3] = Some(value()?),
            "--from" => a.gap_from = Some(value()?),
            "--to" => a.gap_to = Some(value()?),
            "--set" => {
                a.gap_set = match Postcheck::parse(&value()?) {
                    Some(Postcheck::Month) | None => {
                        return Err("--set: window, next-day or period-close".into());
                    }
                    set => set,
                }
            }
            "--schema" => match value()?.as_str() {
                "pre0139" => a.pre0139 = true,
                v => return Err(format!("--schema {v}: only pre0139")),
            },
            "--rewrite-db" => a.rewrite_db = true,
            "--check-only" => a.check_only = true,
            FORCE_LOSE => a.force_lose = true,
            "--mutation-timeout" => {
                a.mutation_timeout = value()?
                    .parse()
                    .map_err(|_| "--mutation-timeout takes seconds".to_string())?
            }
            c if a.command.is_empty() && COMMANDS.contains(&c) => a.command = c.to_string(),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    if a.command.is_empty() {
        return Err(format!("no command ({})", COMMANDS.join(", ")));
    }
    if !a
        .database
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || a.database.is_empty()
    {
        return Err(format!(
            "--database {} is not a bare identifier",
            a.database
        ));
    }
    if a.gap_from.is_some() && a.gap_to.is_none() {
        return Err("--from needs --to".into());
    }
    if a.gap_set.is_some() && a.gap_from.is_some() {
        return Err("--set or --from/--to, not both".into());
    }
    if a.command == "recreate-mvs" && a.mv_source.is_none() {
        return Err("recreate-mvs needs --source prod-text or --source generator".into());
    }
    if a.source.is_some() != a.target.is_some() {
        return Err("--source and --target go together".into());
    }
    if a.table.is_some() && a.source.is_some() {
        return Err("--table or --source/--target, not both".into());
    }
    if let Some(t) = &a.table
        && !COPIED_TABLES.contains(&t.as_str())
    {
        return Err(format!("--table {t} is not one of the copied tables"));
    }
    match mtls {
        [None, None, None, None] => {}
        [Some(d), Some(c), Some(k), Some(ca)] => a.mtls = Some([d, c, k, ca]),
        _ => return Err("--mtls-domain, --mtls-cert, --mtls-key, --mtls-ca go together".into()),
    }
    Ok(a)
}

fn fills(a: &Args) -> Vec<Fill> {
    let one = |mut f: Fill| {
        f.partition = a.partition.clone();
        f
    };
    match (&a.table, &a.source, &a.target) {
        (_, Some(s), Some(t)) => vec![one(Fill {
            source: s.clone(),
            target: t.clone(),
            ..Fill::default()
        })],
        (Some(t), _, _) => vec![one(Fill::table(t))],
        _ => COPIED_TABLES.iter().map(|t| one(Fill::table(t))).collect(),
    }
}

fn connect(a: &Args) -> Result<clickhouse::Client, String> {
    match &a.mtls {
        None => {
            let mut cfg = Config::from_env();
            cfg.database = a.database.clone();
            Ok(with_readable_errors(client(&cfg)))
        }
        #[cfg(feature = "aws-mtls")]
        Some([domain, cert, key, ca]) => prices_clickhouse::mtls::client_with_mtls_from_paths(
            domain,
            std::path::Path::new(cert),
            std::path::Path::new(key),
            std::path::Path::new(ca),
            &a.database,
        )
        .map_err(|e| e.to_string()),
        #[cfg(not(feature = "aws-mtls"))]
        Some(_) => Err("--mtls-* needs a build with --features aws-mtls".into()),
    }
}

async fn run(a: &Args) -> Result<Vec<String>, RekeyError> {
    let r = Rekey::new(
        connect(a).map_err(RekeyError::Refused)?,
        &a.database,
        a.execute,
    );
    match a.command.as_str() {
        "preflight" => r.preflight().await,
        "map" => Ok(vec![r.map().await?]),
        "create" => r.create().await.map(|()| vec!["created".into()]),
        "fill" => {
            let mut out = Vec::new();
            for f in fills(a) {
                let rep = r.fill(&f).await?;
                out.push(format!(
                    "{} -> {}: copied {}, unchanged {}",
                    f.source, f.target, rep.copied, rep.skipped
                ));
            }
            Ok(out)
        }
        "check" => r.check(&fills(a)).await,
        "capture" => r.capture(a.rewrite_db.then_some("prices")).await,
        "alter-assets" => {
            r.alter_assets(Duration::from_secs(a.mutation_timeout))
                .await
        }
        "swap" => r.swap(a.check_only).await,
        "recreate-mvs" => {
            r.recreate_mvs(a.mv_source.expect("parse requires it"))
                .await
        }
        "rollback" => r.rollback(a.force_lose).await,
        "verify" => r.verify().await,
        "gap-backfill" => r.gap_backfill(a.gap_to.as_deref()).await,
        "gap-verify" => {
            let bounds = a.gap_from.as_deref().zip(a.gap_to.as_deref());
            let set = a.gap_set.unwrap_or(Postcheck::Window);
            r.gap_verify(a.pre0139, bounds, set).await
        }
        _ => unreachable!("parse admits only COMMANDS"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("usage error: {e}");
            std::process::exit(1);
        }
    };
    let reads_only = ["preflight", "verify", "gap-verify"].contains(&a.command.as_str());
    if !a.execute && !reads_only && !a.check_only {
        eprintln!("dry run: nothing is written (add --execute)");
    }
    match run(&a).await {
        Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &str) -> Result<Args, String> {
        parse(
            &args
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn dry_run_is_the_default_and_the_database_is_prices() {
        let a = p("fill --table price_ohlcv_1d --partition 202401").unwrap();
        assert!(!a.execute);
        assert_eq!(a.database, "prices");
        let f = fills(&a);
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].source.as_str(), f[0].target.as_str()),
            ("price_ohlcv_1d", "price_ohlcv_1d__new")
        );
        assert_eq!(f[0].partition.as_deref(), Some("202401"));
        assert_eq!(fills(&p("check").unwrap()).len(), 11);
        let generic = p("fill --source bak --target bak__new --execute --database x").unwrap();
        assert!(generic.execute);
        assert_eq!(fills(&generic)[0].target, "bak__new");
    }

    #[test]
    fn bad_arguments_are_refused() {
        for bad in [
            "",
            "verify-everything",
            "map map",
            "recreate-mvs",
            "recreate-mvs --source text",
            "map --database x;y",
            "alter-assets --mutation-timeout soon",
            "gap-verify --schema current",
            "gap-verify --from 2026-10-01",
            "gap-verify --set month",
            "gap-verify --set daily",
            "gap-verify --set next-day --from a --to b",
            "fill --source a",
            "fill --table assets",
            "fill --table price_ohlcv_1m --source a --target b",
            "map --mtls-domain d",
            "map --database",
        ] {
            assert!(p(bad).is_err(), "`{bad}` should be refused");
        }
        assert!(
            p("map --mtls-domain d --mtls-cert c --mtls-key k --mtls-ca a")
                .unwrap()
                .mtls
                .is_some()
        );
    }

    /// The window runbook runs every command through its `rk` helper.
    #[test]
    fn the_runbook_runs_every_command() {
        let runbook = include_str!("../../../../docs/runbooks/0139-asset-id-migration.md");
        for c in COMMANDS {
            assert!(
                runbook.contains(&format!("rk {c}")),
                "`rk {c}` is not in the runbook"
            );
        }
    }

    #[test]
    fn window_commands_take_their_flags() {
        let a = p("recreate-mvs --source prod-text --execute").unwrap();
        assert_eq!(a.mv_source, Some(MvSource::ProdText));
        assert_eq!(
            p("recreate-mvs --source generator").unwrap().mv_source,
            Some(MvSource::Generator)
        );
        assert!(
            p("rollback --force-lose-post-swap-rows")
                .unwrap()
                .force_lose
        );
        assert!(p("swap --check-only").unwrap().check_only);
        assert_eq!(
            p("gap-verify --set period-close").unwrap().gap_set,
            Some(Postcheck::PeriodClose)
        );
        assert_eq!(p("gap-verify").unwrap().gap_set, None);
        assert!(
            p("capture --rewrite-db --database prices_r0139")
                .unwrap()
                .rewrite_db
        );
        assert_eq!(p("alter-assets").unwrap().mutation_timeout, 1800);
        assert_eq!(
            p("alter-assets --mutation-timeout 60")
                .unwrap()
                .mutation_timeout,
            60
        );
    }
}
