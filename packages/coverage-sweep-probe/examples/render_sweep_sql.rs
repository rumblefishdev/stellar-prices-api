//! Print the production sweep statement (task 0100) to stdout.
//!
//! For the manual, read-only back-test (docs/runbooks/0100-coverage-sweep-triage.md §5):
//!
//! ```text
//! cargo run -q -p coverage-sweep-probe --example render_sweep_sql > query.sql   # scratch dir, never committed
//! curl -sS --cert … --key … \
//!   "https://<ch-domain>/?param_lo=61926675&param_hi=62147853&default_format=TSVWithNames" \
//!   --data-binary @query.sql
//! ```
//!
//! It runs as the read-only `dev_read`: the trailing `SETTINGS join_use_nulls = 0`
//! equals that user's current value, and read-only mode refuses only a CHANGE
//! (checked 2026-09-22; `= 1` fails with Code 164). If it ever returns 164,
//! drop that line for the manual run — the query's `ifNull` keeps the same rows.
//!
//! The bounds are the server-side typed parameters `{lo:Int64}`/`{hi:Int64}`,
//! passed as `param_lo`/`param_hi`. The SQL does not apply the allow-list (the
//! probe subtracts it in Rust), so allow-listed families appear in the output.
//! The rendered text stays out of the repo: no `.sql` file is committed.

use coverage_sweep_probe::sweep::sweep_sql;
use coverage_sweep_probe::{BE_DATABASE, PRICES_DATABASE};

fn main() {
    match sweep_sql(BE_DATABASE, PRICES_DATABASE) {
        Ok(sql) => println!("{sql}"),
        Err(e) => {
            eprintln!("render_sweep_sql: {e}");
            std::process::exit(1);
        }
    }
}
