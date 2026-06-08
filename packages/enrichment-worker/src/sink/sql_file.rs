use std::path::{Path, PathBuf};

use super::{EnrichmentSink, SinkError};
use crate::enrich::EnrichedRow;

/// Emits an `INSERT INTO prices.price_ohlcv (...) VALUES (...)`
/// statement per invocation, batching all rows into one VALUES list.
/// Includes the `_inserted_at` version column so BE can see the
/// ReplacingMergeTree pattern (G-note Part A.2 Option 2).
pub struct SqlFileSink {
    out_dir: PathBuf,
}

impl SqlFileSink {
    pub fn new(out_dir: impl AsRef<Path>) -> Self {
        Self {
            out_dir: out_dir.as_ref().to_path_buf(),
        }
    }
}

impl EnrichmentSink for SqlFileSink {
    async fn write(&self, rows: &[EnrichedRow]) -> Result<(), SinkError> {
        if rows.is_empty() {
            return Ok(());
        }
        tokio::fs::create_dir_all(&self.out_dir)
            .await
            .map_err(|e| SinkError::Write(e.to_string()))?;
        let path = self.out_dir.join(format!(
            "enriched-{}.sql",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let mut sql = String::from(
            "INSERT INTO prices.price_ohlcv (timestamp, asset_id, granularity, quote_asset_id, source, open, high, low, close, volume_base, volume_quote, volume_quote_usd, trade_count, vwap_numerator, vwap_denominator, _inserted_at) VALUES\n",
        );
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                sql.push_str(",\n");
            }
            sql.push_str(&row_values(row));
        }
        sql.push_str(";\n");
        tokio::fs::write(&path, sql)
            .await
            .map_err(|e| SinkError::Write(e.to_string()))
    }
}

fn row_values(r: &EnrichedRow) -> String {
    format!(
        "({ts}, '{aid}', '{gr}', '{qaid}', '{src}', {o}, {h}, {l}, {c}, {vb}, {vq}, {vqu}, {tc}, {vn}, {vd}, {iat})",
        ts = r.timestamp,
        aid = r.asset_id,
        gr = r.granularity,
        qaid = r.quote_asset_id,
        src = r.source,
        o = r.open,
        h = r.high,
        l = r.low,
        c = r.close,
        vb = r.volume_base,
        vq = r.volume_quote,
        vqu = r.volume_quote_usd,
        tc = r.trade_count,
        vn = r.vwap_numerator,
        vd = r.vwap_denominator,
        iat = r.inserted_at_unix_seconds,
    )
}
