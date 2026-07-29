//! Task 0132: the live processor writes only the assets a run newly discovered,
//! not the whole registry, on every reconcile. Exercised at the sink boundary
//! with the in-memory `CountingSink` — no fixtures, no ClickHouse — so it runs
//! everywhere (unlike the fixture-gated reconcile e2e tests).

use std::sync::atomic::Ordering;

use prices_ingest_core::{AssetIdentity, AssetRegistry};
use prices_ledger_processor::sink::{CandleSink, CountingSink};

#[tokio::test]
async fn write_new_assets_writes_nothing_when_no_new_assets() {
    // Cold start loaded one existing asset; a reconcile that discovers no new
    // asset must write ZERO rows — this is the steady-state case that used to
    // re-emit all ~200k rows every run (the 9,413× amplification).
    let reg = AssetRegistry::from_existing(vec![(1, AssetIdentity::Native)]);
    let watermark = reg.watermark();
    let sink = CountingSink::default();

    sink.write_new_assets(&reg, watermark).await.unwrap();

    assert_eq!(
        sink.assets.load(Ordering::Relaxed),
        0,
        "no new assets discovered → no asset rows written"
    );
}

#[tokio::test]
async fn write_new_assets_writes_only_the_newly_discovered_asset() {
    let mut reg = AssetRegistry::from_existing(vec![
        (1, AssetIdentity::Native),
        (2, AssetIdentity::Contract("CEXISTING".to_string())),
    ]);
    // Watermark captured before the run, as the reconcile loop does.
    let watermark = reg.watermark();
    let sink = CountingSink::default();

    // The run interns one brand-new asset (plus re-sees known ones — no-ops).
    reg.get_or_assign(&AssetIdentity::Native);
    reg.get_or_assign(&AssetIdentity::Contract("CNEW".to_string()));

    sink.write_new_assets(&reg, watermark).await.unwrap();

    assert_eq!(
        sink.assets.load(Ordering::Relaxed),
        1,
        "only the one newly-interned asset is written, not the whole registry"
    );
}
