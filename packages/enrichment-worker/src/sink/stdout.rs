use super::{EnrichmentSink, SinkError};
use crate::enrich::EnrichedRow;

pub struct StdoutJsonSink;

impl EnrichmentSink for StdoutJsonSink {
    async fn write(&self, rows: &[EnrichedRow]) -> Result<(), SinkError> {
        for row in rows {
            let line = serde_json::to_string(row).map_err(|e| SinkError::Write(e.to_string()))?;
            println!("{line}");
        }
        Ok(())
    }
}
