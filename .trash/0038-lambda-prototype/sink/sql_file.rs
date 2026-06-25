use std::path::{Path, PathBuf};

use super::{OhlcvSink, SinkError};
use crate::bucket::OhlcvRow;

/// Emits one `INSERT INTO prices.price_ohlcv ...` statement per row.
/// Production replaces this with a single `INSERT INTO prices.price_ohlcv ...
/// VALUES (...)` batched through `clickhouse::Client::insert`; the per-row
/// form here is what BE reads in the meeting to confirm the column shape.
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

impl OhlcvSink for SqlFileSink {
    async fn write(&self, rows: &[OhlcvRow]) -> Result<(), SinkError> {
        if rows.is_empty() {
            return Ok(());
        }
        tokio::fs::create_dir_all(&self.out_dir)
            .await
            .map_err(|e| SinkError::Write(e.to_string()))?;
        let path = self.out_dir.join(format!(
            "ohlcv-{}.sql",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let mut sql = String::new();
        for row in rows {
            sql.push_str(&row_to_insert(row));
            sql.push('\n');
        }
        tokio::fs::write(&path, sql)
            .await
            .map_err(|e| SinkError::Write(e.to_string()))
    }
}

fn row_to_insert(r: &OhlcvRow) -> String {
    format!(
        "INSERT INTO prices.price_ohlcv (timestamp, asset_id, granularity, quote_asset_id, source, open, high, low, close, volume_base, volume_quote, trade_count, vwap_num, vwap_den) VALUES ({ts}, '{aid}', '{gr}', '{qaid}', '{src}', {o}, {h}, {l}, {c}, {vb}, {vq}, {tc}, {vn}, {vd});",
        ts = r.key.timestamp_minute,
        aid = r.key.asset_id,
        gr = r.key.granularity,
        qaid = r.key.quote_asset_id,
        src = r.key.source,
        o = r.open,
        h = r.high,
        l = r.low,
        c = r.close,
        vb = r.volume_base,
        vq = r.volume_quote,
        tc = r.trade_count,
        vn = r.vwap_numerator,
        vd = r.vwap_denominator,
    )
}
