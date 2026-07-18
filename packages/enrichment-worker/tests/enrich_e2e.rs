use std::str::FromStr;
use std::sync::Mutex;

use enrichment_worker::candidates::JsonlCandidateSource;
use enrichment_worker::enrich::{Candidate, EnrichedRow};
use enrichment_worker::oracle::{InMemoryOracleLookup, OracleEntry};
use enrichment_worker::pass::run_pass;
use enrichment_worker::sink::{EnrichmentSink, SinkError};
use rust_decimal::Decimal;
use tempfile::tempdir;

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

fn oracle_entry(asset: &str, ts: i64, price: &str) -> OracleEntry {
    OracleEntry {
        asset_id: asset.into(),
        oracle_name: "reflector".into(),
        timestamp: ts,
        price_usd: Decimal::from_str(price).unwrap(),
    }
}

#[tokio::test]
async fn fixture_files_drive_full_pipeline() {
    let dir = tempdir().unwrap();
    let cand_path = dir.path().join("candidates.jsonl");
    let oracle_path = dir.path().join("oracle.jsonl");

    let cands = vec![
        candidate("CUSDC", "100", 1_700_000_000),
        candidate("CUSDT", "200", 1_700_000_060),
        candidate("CAQUA", "300", 1_700_000_120), // miss — no oracle for AQUA
    ];
    let oracle = vec![
        oracle_entry("CUSDC", 1_700_000_000, "1.0012"),
        oracle_entry("CUSDT", 1_700_000_000, "0.9988"),
    ];

    let mut cands_jsonl = String::new();
    for c in &cands {
        cands_jsonl.push_str(&serde_json::to_string(c).unwrap());
        cands_jsonl.push('\n');
    }
    let mut oracle_jsonl = String::new();
    for o in &oracle {
        oracle_jsonl.push_str(&serde_json::to_string(o).unwrap());
        oracle_jsonl.push('\n');
    }
    tokio::fs::write(&cand_path, cands_jsonl).await.unwrap();
    tokio::fs::write(&oracle_path, oracle_jsonl).await.unwrap();

    let mut src = JsonlCandidateSource::open(&cand_path).await.unwrap();
    let oracle_lookup = InMemoryOracleLookup::load_jsonl(&oracle_path)
        .await
        .unwrap();
    let sink = CaptureSink {
        rows: Mutex::new(Vec::new()),
    };

    let stats = run_pass(
        &mut src,
        &oracle_lookup,
        &sink,
        "reflector",
        300,
        10,
        5,
        1_700_001_000,
    )
    .await
    .unwrap();

    assert_eq!(stats.candidates_seen, 3);
    assert_eq!(stats.rows_enriched, 2);
    assert_eq!(stats.oracle_misses, 1);

    let written = sink.rows.lock().unwrap();
    assert_eq!(written.len(), 2);
    // 100 * 1.0012 = 100.12
    assert_eq!(
        written[0].volume_quote_usd,
        Decimal::from_str("100.1200").unwrap()
    );
    // 200 * 0.9988 = 199.76
    assert_eq!(
        written[1].volume_quote_usd,
        Decimal::from_str("199.7600").unwrap()
    );
    // _inserted_at is the production version column.
    assert_eq!(written[0].inserted_at_unix_seconds, 1_700_001_000);
}

#[tokio::test]
async fn idempotent_rerun_same_output() {
    // Same fixture, two passes → identical enriched output.
    let oracle =
        InMemoryOracleLookup::from_entries(vec![oracle_entry("CUSDC", 1_700_000_000, "1.0")]);

    let run = || async {
        let mut src = JsonlCandidateSource::from_vec(vec![
            candidate("CUSDC", "100", 1_700_000_000),
            candidate("CUSDC", "200", 1_700_000_060),
        ]);
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
            1_700_001_000,
        )
        .await
        .unwrap();
        let rows = sink.rows.lock().unwrap().clone();
        (stats, rows)
    };

    let (a, ra) = run().await;
    let (b, rb) = run().await;

    assert_eq!(a.rows_enriched, b.rows_enriched);
    assert_eq!(a.oracle_misses, b.oracle_misses);
    assert_eq!(ra.len(), rb.len());
    for (x, y) in ra.iter().zip(rb.iter()) {
        assert_eq!(x.volume_quote_usd, y.volume_quote_usd);
        assert_eq!(x.timestamp, y.timestamp);
    }
}
