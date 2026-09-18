//! `--discover-pools`: fill `prices.pool_registry` from the AMM factory events in
//! a ledger range, instead of repricing (task 0291).
//!
//! The reprice reads only the events of contracts already in the registry, so it
//! can never find a pool the registry is missing. This mode reads the factory
//! events themselves — Aquarius `add_pool`, Phoenix `create`, Soroswap
//! `new_pair` — and runs them through the same `learn_factory` the live
//! processor uses, so a row written here is the row live would have learned.
//! Only rows not already in the table are written; a re-run writes nothing.
//!
//! Needed once because the registry stopped growing when the history backfill
//! ended (2026-07-06), and the live processor did not persist what it learned
//! until task 0291. Also the check to run before a reprice that `DROP`s a
//! partition: a dry run reporting 0 new pools means the registry covers the
//! range.

use std::collections::{BTreeMap, HashMap};

use clickhouse::query::RowCursor;
use clickhouse::{Client, Row};
use prices_ingest_core::{OhlcvWriter, PoolRegistryRow, Registries, learn_factory_event};
use serde::Deserialize;
use serde_json::Value;
use tracing::info;

use crate::cli::Cli;
use crate::error::EventsBackfillError;

/// One factory-event candidate. Only the payload matters to `learn_factory`,
/// which classifies by event shape, not by emitter.
#[derive(Debug, Row, Deserialize)]
pub struct FactoryEventRow {
    pub ledger_sequence: u32,
    pub topics_xdr: String,
    pub data_xdr: String,
}

/// The factory-event read for `[start, end]`, as text.
///
/// Filters are a superset of what `learn_factory` accepts. BE fills `signature`
/// only when topic[0] is a Symbol, and two of the three factories use Strings:
/// - Aquarius `add_pool`: Symbol topics, so `signature = 'add_pool'`;
/// - Phoenix `create`/`liquidity_pool`: **String** topics, so `signature` is NULL
///   — a `signature`-only filter finds none of the 20 Phoenix pools;
/// - Soroswap `SoroswapFactory`/`new_pair`: String topics, the action in topic[1].
///
/// The NULL branch therefore matches topic values, as `learn_factory` does
/// (it reads the value whatever its type). No emitter filter — `learn_factory`
/// has none either. Measured on production 2026-09-17: 2-4 s per 320k-ledger
/// chunk.
///
/// No `FINAL`: an RMT double is the same event twice, and learning a pool twice
/// is a no-op.
pub(crate) fn factory_events_sql(start: u32, end: u32) -> String {
    format!(
        "SELECT \
            toUInt32(ledger_sequence) AS ledger_sequence, \
            topics_xdr, \
            data_xdr \
         FROM default.soroban_events \
         WHERE ledger_sequence BETWEEN {start} AND {end} \
           AND (signature IN ('add_pool', 'create') \
                OR (signature IS NULL \
                    AND (JSONExtractString(topics_xdr, 1, 'value') IN ('add_pool', 'create') \
                         OR JSONExtractString(topics_xdr, 2, 'value') = 'new_pair'))) \
         ORDER BY ledger_sequence, transaction_id, event_index"
    )
}

fn stream_factory_events(
    client: &Client,
    start: u32,
    end: u32,
) -> Result<RowCursor<FactoryEventRow>, EventsBackfillError> {
    Ok(client
        .query(&factory_events_sql(start, end))
        .fetch::<FactoryEventRow>()?)
}

/// Feed one candidate through `learn_factory`. A payload that does not parse
/// is ignored, as in the reprice: it can only cost that one event.
pub(crate) fn learn_from_row(row: &FactoryEventRow, reg: &mut Registries) {
    let topics = serde_json::from_str::<Value>(&row.topics_xdr).unwrap_or(Value::Null);
    let data = serde_json::from_str::<Value>(&row.data_xdr).unwrap_or(Value::Null);
    learn_factory_event(&topics, &data, reg);
}

fn snapshot(reg: &Registries) -> HashMap<String, PoolRegistryRow> {
    reg.to_pool_rows()
        .into_iter()
        .map(|row| (row.contract_id.clone(), row))
        .collect()
}

pub async fn execute(cli: &Cli, writer: &OhlcvWriter) -> Result<(), EventsBackfillError> {
    let mut reg = writer.load_pool_registry().await?;
    let persisted = snapshot(&reg);
    info!(
        pools = persisted.len(),
        start = cli.start,
        end = cli.end,
        dry_run = cli.dry_run,
        "discover-pools: preloaded registry; reading factory events"
    );

    let mut candidates: u64 = 0;
    let mut chunk_start = cli.start;
    loop {
        let chunk_end = chunk_start.saturating_add(cli.chunk_size - 1).min(cli.end);
        let mut cursor = stream_factory_events(writer.client(), chunk_start, chunk_end)?;
        while let Some(row) = cursor.next().await? {
            candidates += 1;
            learn_from_row(&row, &mut reg);
        }
        if chunk_end >= cli.end {
            break;
        }
        chunk_start = chunk_end + 1;
    }

    let new_rows = reg.pool_rows_unpersisted(&persisted);
    let mut per_venue: BTreeMap<&str, u64> = BTreeMap::new();
    for row in &new_rows {
        *per_venue.entry(row.venue.as_str()).or_insert(0) += 1;
        let change = if persisted.contains_key(&row.contract_id) {
            "changed"
        } else {
            "new"
        };
        info!(
            contract_id = row.contract_id,
            venue = row.venue,
            token0 = row.token0,
            token1 = row.token1,
            change,
            "discover-pools: pool not in prices.pool_registry"
        );
    }
    info!(
        candidates,
        to_write = new_rows.len(),
        ?per_venue,
        "discover-pools: factory events read"
    );

    if cli.dry_run {
        info!("discover-pools: DRY RUN — nothing written");
        return Ok(());
    }
    writer.write_pool_rows(&new_rows).await?;
    info!(written = new_rows.len(), "discover-pools: done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use extractors_core::Venue;

    use super::*;

    fn row(topics: &str, data: &str) -> FactoryEventRow {
        FactoryEventRow {
            ledger_sequence: 1,
            topics_xdr: topics.to_string(),
            data_xdr: data.to_string(),
        }
    }

    // Real `default.soroban_events` payloads, production 2026-09-17.
    const NEW_PAIR_TOPICS: &str =
        r#"[{"type":"string","value":"SoroswapFactory"},{"type":"sym","value":"new_pair"}]"#;
    const NEW_PAIR_DATA: &str = r#"{"type":"map","value":[{"key":{"type":"sym","value":"new_pairs_length"},"value":{"type":"u32","value":202}},{"key":{"type":"sym","value":"pair"},"value":{"type":"address","value":"CAZ4Z273BBAAFL5NYNQJKEMZDQBRCPKAS4GOXDUFXPSE56M4ONBJUOVD"}},{"key":{"type":"sym","value":"token_0"},"value":{"type":"address","value":"CBLLEW7HD2RWATVSMLAGWM4G3WCHSHDJ25ALP4DI6LULV5TU35N2CIZA"}},{"key":{"type":"sym","value":"token_1"},"value":{"type":"address","value":"CBY4MSZXK5L4HDMJHDXQLNLOA5MM5BIGCHQYMRG7ZAFY34UNU4UXPEJJ"}}]}"#;
    const ADD_POOL_TOPICS: &str = r#"[{"type":"sym","value":"add_pool"},{"type":"vec","value":[{"type":"address","value":"CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK"},{"type":"address","value":"CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY"}]}]"#;
    const ADD_POOL_DATA: &str = r#"{"type":"vec","value":[{"type":"address","value":"CDQ4OYM3RPLEWNZFVAJQGEYLSDMPHEZYMHVOQBKI767UWV5XV5ISAJE2"},{"type":"sym","value":"concentrated"},{"type":"bytes","value":"JPnJkcRKzzP/9fRAMcQDhdI13CEtc3noJLo9scNTcfM="},{"type":"vec","value":[{"type":"u32","value":10},{"type":"i32","value":20}]}]}"#;
    // Phoenix's topics are Strings, not Symbols — which is why BE's `signature`
    // is NULL for it (ledger 51,572,026).
    const CREATE_TOPICS: &str =
        r#"[{"type":"string","value":"create"},{"type":"string","value":"liquidity_pool"}]"#;
    const CREATE_DATA: &str =
        r#"{"type":"address","value":"CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX"}"#;

    #[test]
    fn learns_all_three_factory_shapes_from_real_payloads() {
        let mut reg = Registries::new();
        learn_from_row(&row(NEW_PAIR_TOPICS, NEW_PAIR_DATA), &mut reg);
        learn_from_row(&row(ADD_POOL_TOPICS, ADD_POOL_DATA), &mut reg);
        learn_from_row(&row(CREATE_TOPICS, CREATE_DATA), &mut reg);

        let pair = "CAZ4Z273BBAAFL5NYNQJKEMZDQBRCPKAS4GOXDUFXPSE56M4ONBJUOVD";
        assert_eq!(reg.venue.get(pair), Some(&Venue::Soroswap));
        let p = reg.soroswap.lookup(pair).expect("pair tokens learned");
        assert_eq!(
            p.token0,
            "CBLLEW7HD2RWATVSMLAGWM4G3WCHSHDJ25ALP4DI6LULV5TU35N2CIZA"
        );
        assert_eq!(
            p.token1,
            "CBY4MSZXK5L4HDMJHDXQLNLOA5MM5BIGCHQYMRG7ZAFY34UNU4UXPEJJ"
        );
        assert_eq!(
            reg.venue
                .get("CDQ4OYM3RPLEWNZFVAJQGEYLSDMPHEZYMHVOQBKI767UWV5XV5ISAJE2"),
            Some(&Venue::Aquarius)
        );
        assert_eq!(
            reg.venue
                .get("CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX"),
            Some(&Venue::Phoenix)
        );
    }

    #[test]
    fn only_pools_missing_from_the_table_are_written() {
        // The Aquarius pool is already registered; the Soroswap pair is not.
        let mut reg = Registries::new();
        learn_from_row(&row(ADD_POOL_TOPICS, ADD_POOL_DATA), &mut reg);
        let persisted = snapshot(&reg);

        learn_from_row(&row(ADD_POOL_TOPICS, ADD_POOL_DATA), &mut reg);
        learn_from_row(&row(NEW_PAIR_TOPICS, NEW_PAIR_DATA), &mut reg);

        let rows = reg.pool_rows_unpersisted(&persisted);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].contract_id,
            "CAZ4Z273BBAAFL5NYNQJKEMZDQBRCPKAS4GOXDUFXPSE56M4ONBJUOVD"
        );
        assert_eq!(rows[0].venue, "soroswap");
    }

    #[test]
    fn non_factory_and_unparseable_rows_learn_nothing() {
        let mut reg = Registries::new();
        // A Phoenix-style `create` that is not a liquidity pool (stake, etc.).
        learn_from_row(
            &row(
                r#"[{"type":"string","value":"create"},{"type":"string","value":"stake"}]"#,
                CREATE_DATA,
            ),
            &mut reg,
        );
        learn_from_row(&row("not json", "{"), &mut reg);
        assert!(reg.venue.is_empty());
    }

    #[test]
    fn the_read_matches_every_shape_learn_factory_accepts() {
        let sql = factory_events_sql(63_000_000, 63_319_999);
        assert!(sql.contains("ledger_sequence BETWEEN 63000000 AND 63319999"));
        let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(sql.contains("signature IN ('add_pool', 'create')"));
        // String-topic factories leave `signature` NULL: Phoenix's action is in
        // topic[0], Soroswap's in topic[1] (1-based indexes 1 and 2).
        assert!(sql.contains(
            "signature IS NULL AND (JSONExtractString(topics_xdr, 1, 'value') IN ('add_pool', 'create') \
             OR JSONExtractString(topics_xdr, 2, 'value') = 'new_pair')"
        ));
        // No SETTINGS — the read runs under readonly=1 (see source.rs).
        assert!(!sql.contains("SETTINGS"));
    }
}
