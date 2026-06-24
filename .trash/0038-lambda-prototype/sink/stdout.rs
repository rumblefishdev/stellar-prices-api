use super::{OhlcvSink, SinkError};
use crate::bucket::OhlcvRow;

pub struct StdoutJsonSink;

impl OhlcvSink for StdoutJsonSink {
    async fn write(&self, rows: &[OhlcvRow]) -> Result<(), SinkError> {
        for row in rows {
            let line = serde_json::to_string(row).map_err(|e| SinkError::Write(e.to_string()))?;
            println!("{line}");
        }
        Ok(())
    }
}
