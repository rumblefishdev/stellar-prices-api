use std::collections::HashMap;
use std::path::Path;

use super::{OracleEntry, OracleError, OraclePrice, OraclePriceLookup};

/// In-memory oracle lookup, indexed by `asset_id`. Each asset's
/// entries are kept sorted by timestamp ascending so the forward-fill
/// lookup is a single binary search.
pub struct InMemoryOracleLookup {
    by_asset: HashMap<String, Vec<OracleEntry>>,
}

impl InMemoryOracleLookup {
    pub fn from_entries(entries: Vec<OracleEntry>) -> Self {
        let mut by_asset: HashMap<String, Vec<OracleEntry>> = HashMap::new();
        for entry in entries {
            by_asset
                .entry(entry.asset_id.clone())
                .or_default()
                .push(entry);
        }
        for v in by_asset.values_mut() {
            v.sort_by_key(|e| e.timestamp);
        }
        Self { by_asset }
    }

    /// Load a JSONL fixture file. Each line is one `OracleEntry`.
    pub async fn load_jsonl(path: impl AsRef<Path>) -> Result<Self, OracleError> {
        let bytes = tokio::fs::read(path.as_ref())
            .await
            .map_err(|e| OracleError::Backend(e.to_string()))?;
        let mut entries = Vec::new();
        for line in bytes.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let entry: OracleEntry =
                serde_json::from_slice(line).map_err(|e| OracleError::Backend(e.to_string()))?;
            entries.push(entry);
        }
        Ok(Self::from_entries(entries))
    }
}

impl OraclePriceLookup for InMemoryOracleLookup {
    async fn lookup(
        &self,
        asset_id: &str,
        at_unix_seconds: i64,
        window_s: u32,
    ) -> Result<Option<OraclePrice>, OracleError> {
        let Some(entries) = self.by_asset.get(asset_id) else {
            return Ok(None);
        };
        let cutoff_lower = at_unix_seconds - window_s as i64;
        // Find the latest entry with timestamp <= at_unix_seconds.
        let upper = entries.partition_point(|e| e.timestamp <= at_unix_seconds);
        if upper == 0 {
            return Ok(None);
        }
        let chosen = &entries[upper - 1];
        if chosen.timestamp <= cutoff_lower {
            return Ok(None);
        }
        Ok(Some(OraclePrice {
            price_usd: chosen.price_usd,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn entry(asset: &str, ts: i64, price: &str) -> OracleEntry {
        OracleEntry {
            asset_id: asset.into(),
            oracle_name: "reflector".into(),
            timestamp: ts,
            price_usd: Decimal::from_str(price).unwrap(),
        }
    }

    #[tokio::test]
    async fn picks_newest_within_window() {
        let o = InMemoryOracleLookup::from_entries(vec![
            entry("CUSDC", 100, "1.0"),
            entry("CUSDC", 200, "1.1"),
            entry("CUSDC", 300, "1.2"),
        ]);
        let result = o.lookup("CUSDC", 250, 300).await.unwrap().unwrap();
        assert_eq!(result.price_usd, Decimal::from_str("1.1").unwrap());
    }

    #[tokio::test]
    async fn exact_timestamp_match_is_hit() {
        let o = InMemoryOracleLookup::from_entries(vec![entry("CUSDC", 200, "1.5")]);
        let result = o.lookup("CUSDC", 200, 300).await.unwrap().unwrap();
        assert_eq!(result.price_usd, Decimal::from_str("1.5").unwrap());
    }

    #[tokio::test]
    async fn entry_older_than_window_is_miss() {
        let o = InMemoryOracleLookup::from_entries(vec![entry("CUSDC", 100, "1.0")]);
        let result = o.lookup("CUSDC", 1000, 300).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn unknown_asset_is_miss() {
        let o = InMemoryOracleLookup::from_entries(Vec::new());
        let result = o.lookup("CAQUA", 100, 300).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn future_only_entries_is_miss() {
        // Candidate at t=50, but only oracle entries at t=100,200 — all in future.
        let o = InMemoryOracleLookup::from_entries(vec![
            entry("CUSDC", 100, "1.0"),
            entry("CUSDC", 200, "1.1"),
        ]);
        let result = o.lookup("CUSDC", 50, 300).await.unwrap();
        assert!(result.is_none());
    }
}
