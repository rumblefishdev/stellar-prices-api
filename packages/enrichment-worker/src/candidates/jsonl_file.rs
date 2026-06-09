use std::path::Path;

use super::{CandidateError, CandidateSource};
use crate::enrich::Candidate;

/// Eagerly loads all candidates from a JSONL file at construction
/// time, then serves them in batches. Re-reading from the start is
/// not supported (the cursor only advances forward); to re-run a
/// pass, construct a new source.
pub struct JsonlCandidateSource {
    candidates: Vec<Candidate>,
    cursor: usize,
}

impl JsonlCandidateSource {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, CandidateError> {
        let bytes = tokio::fs::read(path.as_ref())
            .await
            .map_err(|e| CandidateError::Backend(e.to_string()))?;
        let mut candidates = Vec::new();
        for line in bytes.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let c: Candidate =
                serde_json::from_slice(line).map_err(|e| CandidateError::Backend(e.to_string()))?;
            candidates.push(c);
        }
        Ok(Self {
            candidates,
            cursor: 0,
        })
    }

    pub fn from_vec(candidates: Vec<Candidate>) -> Self {
        Self {
            candidates,
            cursor: 0,
        }
    }
}

impl CandidateSource for JsonlCandidateSource {
    async fn next_batch(&mut self, limit: usize) -> Result<Vec<Candidate>, CandidateError> {
        let end = (self.cursor + limit).min(self.candidates.len());
        let batch = self.candidates[self.cursor..end].to_vec();
        self.cursor = end;
        Ok(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use tempfile::tempdir;

    fn sample(seq: i64) -> Candidate {
        Candidate {
            timestamp: seq,
            asset_id: format!("C{seq}"),
            granularity: "1m".into(),
            quote_asset_id: "CUSDC".into(),
            source: "phoenix".into(),
            volume_base: Decimal::from_str("1").unwrap(),
            volume_quote: Decimal::from_str("1").unwrap(),
            open: Decimal::from_str("1").unwrap(),
            high: Decimal::from_str("1").unwrap(),
            low: Decimal::from_str("1").unwrap(),
            close: Decimal::from_str("1").unwrap(),
            trade_count: 1,
            vwap_numerator: Decimal::from_str("1").unwrap(),
            vwap_denominator: Decimal::from_str("1").unwrap(),
        }
    }

    #[tokio::test]
    async fn batches_in_order_and_drains() {
        let mut src = JsonlCandidateSource::from_vec(vec![sample(1), sample(2), sample(3)]);
        let b1 = src.next_batch(2).await.unwrap();
        assert_eq!(b1.len(), 2);
        assert_eq!(b1[0].timestamp, 1);
        let b2 = src.next_batch(2).await.unwrap();
        assert_eq!(b2.len(), 1);
        assert_eq!(b2[0].timestamp, 3);
        let b3 = src.next_batch(2).await.unwrap();
        assert!(b3.is_empty());
    }

    #[tokio::test]
    async fn jsonl_file_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.jsonl");
        let mut content = String::new();
        for c in [sample(1), sample(2)] {
            content.push_str(&serde_json::to_string(&c).unwrap());
            content.push('\n');
        }
        tokio::fs::write(&path, content).await.unwrap();
        let mut src = JsonlCandidateSource::open(&path).await.unwrap();
        let batch = src.next_batch(10).await.unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].timestamp, 1);
    }
}
