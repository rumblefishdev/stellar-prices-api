//! One enrichment pass — the orchestrator that wires
//! `CandidateSource` → `enrich_one` (with oracle lookup) →
//! `EnrichmentSink`. Caps work at `max_batches × batch_size` rows
//! per invocation; remaining candidates roll over to the next pass.
//!
//! Idempotency: the read-side `volume_quote_usd = 0` filter
//! (enforced upstream of `CandidateSource`) means re-running this
//! pass on already-enriched rows is a no-op. The prototype
//! `JsonlCandidateSource` reads fixtures that the operator
//! pre-filtered; production reads `prices.price_ohlcv FINAL WHERE
//! volume_quote_usd = 0`.

use tracing::{info, warn};

use crate::candidates::{CandidateError, CandidateSource};
use crate::enrich::{EnrichError, EnrichedRow, Outcome, enrich_one};
use crate::oracle::OraclePriceLookup;
use crate::sink::{EnrichmentSink, SinkError};

#[derive(Debug, thiserror::Error)]
pub enum PassError {
    #[error("candidates: {0}")]
    Candidates(#[from] CandidateError),
    #[error("enrich: {0}")]
    Enrich(#[from] EnrichError),
    #[error("sink: {0}")]
    Sink(#[from] SinkError),
}

#[derive(Debug, Clone, Default)]
pub struct PassStats {
    pub batches: u64,
    pub candidates_seen: u64,
    pub rows_enriched: u64,
    pub oracle_misses: u64,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_pass<C, O, S>(
    candidates: &mut C,
    oracle: &O,
    sink: &S,
    oracle_name: &str,
    window_s: u32,
    batch_size: usize,
    max_batches: usize,
    now_unix_seconds: i64,
) -> Result<PassStats, PassError>
where
    C: CandidateSource + Send,
    O: OraclePriceLookup + Sync,
    S: EnrichmentSink + Sync,
{
    let mut stats = PassStats::default();

    for _ in 0..max_batches {
        let batch = candidates.next_batch(batch_size).await?;
        if batch.is_empty() {
            info!(
                batches = stats.batches,
                "candidate source exhausted — pass complete"
            );
            break;
        }
        stats.batches += 1;
        stats.candidates_seen += batch.len() as u64;

        let mut enriched: Vec<EnrichedRow> = Vec::with_capacity(batch.len());
        for candidate in &batch {
            match enrich_one(candidate, oracle, oracle_name, window_s, now_unix_seconds).await? {
                Outcome::Enriched(row) => {
                    enriched.push(*row);
                    stats.rows_enriched += 1;
                }
                Outcome::OracleMiss {
                    quote_asset_id,
                    oracle_name,
                } => {
                    warn!(
                        %quote_asset_id,
                        %oracle_name,
                        "oracle miss — row stays at volume_quote_usd = 0 for next pass"
                    );
                    stats.oracle_misses += 1;
                }
            }
        }

        if !enriched.is_empty() {
            sink.write(&enriched).await?;
        }
        info!(
            batch = stats.batches,
            enriched = enriched.len(),
            "batch persisted"
        );
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::*;
    use crate::candidates::JsonlCandidateSource;
    use crate::enrich::Candidate;
    use crate::oracle::{InMemoryOracleLookup, OracleEntry};

    struct CaptureSink {
        rows: Mutex<Vec<EnrichedRow>>,
    }

    impl EnrichmentSink for CaptureSink {
        async fn write(&self, rows: &[EnrichedRow]) -> Result<(), SinkError> {
            self.rows.lock().unwrap().extend_from_slice(rows);
            Ok(())
        }
    }

    fn candidate(quote: &str, vq: &str, ts: i64) -> Candidate {
        Candidate {
            timestamp: ts,
            asset_id: "CXLM".into(),
            granularity: "1m".into(),
            quote_asset_id: quote.into(),
            source: "phoenix".into(),
            volume_base: Decimal::from_str("1000").unwrap(),
            volume_quote: Decimal::from_str(vq).unwrap(),
            open: Decimal::from_str("1.25").unwrap(),
            high: Decimal::from_str("1.26").unwrap(),
            low: Decimal::from_str("1.24").unwrap(),
            close: Decimal::from_str("1.255").unwrap(),
            trade_count: 1,
            vwap_numerator: Decimal::from_str("1250").unwrap(),
            vwap_denominator: Decimal::from_str("1000").unwrap(),
        }
    }

    fn oracle(asset: &str, ts: i64, price: &str) -> OracleEntry {
        OracleEntry {
            asset_id: asset.into(),
            oracle_name: "reflector".into(),
            timestamp: ts,
            price_usd: Decimal::from_str(price).unwrap(),
        }
    }

    #[tokio::test]
    async fn enriches_hits_skips_misses() {
        let mut src = JsonlCandidateSource::from_vec(vec![
            candidate("CUSDC", "100", 1_700_000_000), // hit → 100 * 1.0 = 100
            candidate("CUSDC", "200", 1_700_000_100), // hit → 200 * 1.0 = 200
            candidate("CAQUA", "50", 1_700_000_200),  // miss
        ]);
        let oracle =
            InMemoryOracleLookup::from_entries(vec![oracle("CUSDC", 1_700_000_000, "1.0")]);
        let sink = CaptureSink {
            rows: Mutex::new(Vec::new()),
        };

        let stats = run_pass(
            &mut src,
            &oracle,
            &sink,
            "reflector",
            300,
            10,
            5,
            1_700_000_500,
        )
        .await
        .unwrap();

        assert_eq!(stats.candidates_seen, 3);
        assert_eq!(stats.rows_enriched, 2);
        assert_eq!(stats.oracle_misses, 1);

        let written = sink.rows.lock().unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(
            written[0].volume_quote_usd,
            Decimal::from_str("100").unwrap()
        );
        assert_eq!(
            written[1].volume_quote_usd,
            Decimal::from_str("200").unwrap()
        );
    }

    #[tokio::test]
    async fn exhausting_source_stops_loop_early() {
        let mut src = JsonlCandidateSource::from_vec(vec![candidate("CUSDC", "10", 1_700_000_000)]);
        let oracle =
            InMemoryOracleLookup::from_entries(vec![oracle("CUSDC", 1_700_000_000, "1.0")]);
        let sink = CaptureSink {
            rows: Mutex::new(Vec::new()),
        };

        let stats = run_pass(
            &mut src,
            &oracle,
            &sink,
            "reflector",
            300,
            10,
            5,
            1_700_000_500,
        )
        .await
        .unwrap();

        assert_eq!(stats.batches, 1);
        assert_eq!(stats.candidates_seen, 1);
    }

    #[tokio::test]
    async fn max_batches_caps_work() {
        // 25 candidates, batch_size=10, max_batches=2 → only 20 processed.
        let many: Vec<Candidate> = (0..25)
            .map(|i| candidate("CUSDC", "1", 1_700_000_000 + i as i64))
            .collect();
        let mut src = JsonlCandidateSource::from_vec(many);
        let oracle =
            InMemoryOracleLookup::from_entries(vec![oracle("CUSDC", 1_700_000_000, "1.0")]);
        let sink = CaptureSink {
            rows: Mutex::new(Vec::new()),
        };

        let stats = run_pass(
            &mut src,
            &oracle,
            &sink,
            "reflector",
            300,
            10,
            2,
            1_700_001_000,
        )
        .await
        .unwrap();

        assert_eq!(stats.batches, 2);
        assert_eq!(stats.candidates_seen, 20);
    }
}
