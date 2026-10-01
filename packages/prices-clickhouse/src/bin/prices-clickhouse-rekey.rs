//! Task 0139 migration tool, part 1 (see `prices_clickhouse::rekey`).
//!
//!     prices-clickhouse-rekey <preflight|map|create|fill|check> [--database DB]
//!         [--execute] [--table T] [--partition P] [--source S --target T]
//!         [--mtls-domain D --mtls-cert PEM --mtls-key PEM --mtls-ca PEM]
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

use prices_clickhouse::rekey::{COPIED_TABLES, Fill, Rekey, RekeyError};
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
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        database: "prices".into(),
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
            "--source" => a.source = Some(value()?),
            "--target" => a.target = Some(value()?),
            "--mtls-domain" => mtls[0] = Some(value()?),
            "--mtls-cert" => mtls[1] = Some(value()?),
            "--mtls-key" => mtls[2] = Some(value()?),
            "--mtls-ca" => mtls[3] = Some(value()?),
            "preflight" | "map" | "create" | "fill" | "check" if a.command.is_empty() => {
                a.command = arg.clone()
            }
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    if a.command.is_empty() {
        return Err("no command (preflight, map, create, fill, check)".into());
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
        _ => unreachable!("parse admits only the five commands"),
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
    if !a.execute && a.command != "preflight" {
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
            "swap",
            "map map",
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
}
