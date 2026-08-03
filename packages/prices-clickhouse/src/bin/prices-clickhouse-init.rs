//! Apply the prices ClickHouse schema to a target instance.
//!
//! Idempotent, but NOT uniformly `CREATE … IF NOT EXISTS` (task 0134). The init
//! tables are `IF NOT EXISTS` — they must never be recreated. The views are
//! `CREATE OR REPLACE`, because `IF NOT EXISTS` silently fails to redefine a
//! view that already exists; re-running this binary therefore RE-APPLIES every
//! view definition rather than skipping it. Applies the init tables, seeds the
//! canonical `backfill_progress` streams, and applies the read-surface views
//! (`prices.price_usd_series`, `prices.usd_reference`); the refreshable rollup
//! MVs are opt-in (`--rollups`).
//!
//! ⚠️ REQUIRES A PRIVILEGED USER. `CREATE OR REPLACE VIEW` needs a `DROP VIEW`
//! grant unconditionally (even when the view does not exist), and `init.sql`
//! opens with `CREATE DATABASE IF NOT EXISTS`. The scoped production users
//! (`prices_writer` / `prices_reader`) hold no DDL grants at all and cannot run
//! this — on ch-prod-01 schema DDL is an operator action as the container's
//! `default` user over the loopback native port. See the header of
//! `schema/views.sql` for the measured grants and the reasoning.
//! Used by local dev / CI: `cargo run -p prices-clickhouse --bin prices-clickhouse-init`.
//!
//! Reads `CLICKHOUSE_URL`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`,
//! `CLICKHOUSE_DATABASE` from the environment (local-dev defaults otherwise).
//!
//! Flags (via args):
//!   --rollups   also apply schema/rollups.sql (production refreshable MVs;
//!               needs ClickHouse ≥ 23.12)

use prices_clickhouse::{
    Config, ROLLUPS_SQL, VIEWS_SQL, apply_init_sql, apply_seed, apply_sql, client,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let with_rollups = std::env::args().any(|a| a == "--rollups");

    // Apply from the always-present `default` database: init.sql issues
    // `CREATE DATABASE IF NOT EXISTS prices` and fully qualifies every object,
    // so the connection must not be scoped to `prices` (which may not exist yet
    // — e.g. after a DROP DATABASE). Overrides CLICKHOUSE_DATABASE.
    let mut cfg = Config::from_env();
    cfg.database = "default".to_string();
    tracing::info!(
        url = %cfg.url,
        user = %cfg.user,
        database = %cfg.database,
        with_rollups,
        "applying prices ClickHouse schema"
    );

    let client = client(&cfg);
    apply_init_sql(&client).await?;
    tracing::info!("tables applied");

    // Seed the canonical backfill streams. Idempotent (NOT IN guard), so this is
    // safe to run on every apply and never clobbers live progress.
    apply_seed(&client).await?;
    tracing::info!("backfill_progress seeded");

    // Read-surface views depend on the tables above; plain views, no CH-version
    // constraint, so always applied (unlike the opt-in refreshable rollup MVs).
    apply_sql(&client, VIEWS_SQL).await?;
    tracing::info!("read-surface views applied");

    if with_rollups {
        apply_sql(&client, ROLLUPS_SQL).await?;
        tracing::info!("rollup MV chain applied");
    }

    tracing::info!("schema applied successfully");
    Ok(())
}
