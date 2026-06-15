//! Apply the prices ClickHouse schema to a target instance.
//!
//! Idempotent — every statement is `CREATE … IF NOT EXISTS`. Applies the init
//! tables and the read-surface views (`prices.price_usd_series`,
//! `prices.usd_reference`); the refreshable rollup MVs are opt-in (`--rollups`).
//! Used by local dev / CI: `cargo run -p prices-clickhouse --bin prices-clickhouse-init`.
//!
//! Reads `CLICKHOUSE_URL`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`,
//! `CLICKHOUSE_DATABASE` from the environment (local-dev defaults otherwise).
//!
//! Flags (via args):
//!   --rollups   also apply schema/rollups.sql (production refreshable MVs;
//!               needs ClickHouse ≥ 23.12)

use prices_clickhouse::{Config, ROLLUPS_SQL, VIEWS_SQL, apply_init_sql, apply_sql, client};

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
