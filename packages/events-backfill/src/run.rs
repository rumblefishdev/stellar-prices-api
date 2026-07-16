//! Orchestration for the events-sourced AMM reprice (task 0097).
//!
//! Preload the seeded `prices.pool_registry` + `prices.assets` (same startup as
//! the live/backfill paths), then walk `[start, end]` in chunks: read each
//! chunk's AMM events from `default.soroban_events`, group by ledger, feed them
//! through the shared [`process_soroban_event_rows`] seam, accumulate 1-minute
//! candles per source, and write them to `prices.price_ohlcv_1m`. Writes are
//! ReplacingMergeTree-idempotent, so a re-run over any range only replaces.

use std::collections::HashMap;
use std::time::Instant;

use clickhouse::Client;
use serde_json::Value;
use tracing::{info, warn};

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, DEFAULT_BACKOFF_MS, LedgerSoroban, OhlcvWriter,
    RawSorobanEvent, Registries, UnresolvedPool, UnresolvedPoolSwap, process_soroban_event_rows,
    retry_with_backoff,
};

use crate::cli::Cli;
use crate::error::EventsBackfillError;
use crate::source::{count_unregistered_amm_emitters, read_chunk, resolve_contract_ids};

/// Run one idempotent `prices.*` write under the shared bounded backoff
/// (`[50, 200, 800] ms`, every error treated as transient — safe because the
/// writes are ReplacingMergeTree-idempotent). Without this a single transient
/// ClickHouse/network blip would abort the whole reprice, which (having no
/// resume cursor) then restarts from `--start`. Mirrors sdex-backfill's sink.
async fn retry_write<F, Fut, E>(op: F) -> Result<(), EventsBackfillError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
    EventsBackfillError: From<E>,
{
    retry_with_backoff(&DEFAULT_BACKOFF_MS, |_| true, op)
        .await
        .map(|_| ())
        .map_err(EventsBackfillError::from)
}

fn build_client(cli: &Cli) -> Client {
    let mut client = Client::default().with_url(&cli.clickhouse_url);
    if let Some(user) = &cli.clickhouse_user {
        client = client.with_user(user);
    }
    if let Some(password) = &cli.clickhouse_password {
        client = client.with_password(password);
    }
    client
}

pub async fn execute(cli: &Cli) -> Result<(), EventsBackfillError> {
    if cli.start > cli.end {
        return Err(EventsBackfillError::InvalidRange {
            start: cli.start,
            end: cli.end,
        });
    }

    let writer = OhlcvWriter::new(build_client(cli));
    writer.preflight().await?;
    info!("pre-flight: ClickHouse reachable");

    // Preload — identical to the live/backfill cold start so repriced candles
    // reuse existing surrogate `asset_id`s and resolve every seeded pool.
    let existing_assets = writer.load_assets().await?;
    let mut assets = AssetRegistry::from_existing(existing_assets);
    let mut reg = writer.load_pool_registry().await?;

    // Every AMM pool has a `venue` entry (the registry superset); its strkeys are
    // the exact contract set whose events we must read. We filter reads to these
    // so the extractor sees each pool's FULL event group (Phoenix emits 8
    // micro-events, all NULL-signature) — a signature/topic filter would break it.
    let pool_strkeys: Vec<String> = reg.venue.keys().cloned().collect();
    if pool_strkeys.is_empty() {
        return Err(EventsBackfillError::EmptyPoolRegistry);
    }
    let id_map = resolve_contract_ids(writer.client(), &pool_strkeys).await?;
    let contract_ids: Vec<i64> = id_map.keys().copied().collect();
    info!(
        pools = pool_strkeys.len(),
        resolved = contract_ids.len(),
        start = cli.start,
        end = cli.end,
        chunk_size = cli.chunk_size,
        dry_run = cli.dry_run,
        "preloaded registry; starting reprice"
    );

    let run_start = Instant::now();
    let mut ticks_by_source: HashMap<&'static str, u64> = HashMap::new();
    let mut total_events: u64 = 0;
    let mut total_candles: u64 = 0;
    let mut raw_unresolved: Vec<UnresolvedPoolSwap> = Vec::new();
    // Ledgers present in soroban_events but absent from default.ledgers (no close
    // time): their events are skipped but COUNTED, so the gap is visible instead
    // of a silent LEFT-JOIN drop.
    let mut ledgers_missing_close: u64 = 0;
    let mut events_missing_close: u64 = 0;

    let mut chunk_start = cli.start;
    loop {
        let chunk_end = chunk_start
            .saturating_add(cli.chunk_size.saturating_sub(1))
            .min(cli.end);

        let rows = read_chunk(writer.client(), &contract_ids, chunk_start, chunk_end).await?;
        total_events += rows.len() as u64;

        // One accumulator per source (phoenix / soroswap / aquarius); the source
        // string is applied at write time, not carried in the bucket key.
        let mut accumulators: HashMap<&'static str, CandleAccumulator> = HashMap::new();

        // Rows are ordered (ledger, tx, event_index). Walk ledger-by-ledger,
        // dropping adjacent RMT duplicates, and run the shared seam per ledger.
        let mut i = 0usize;
        while i < rows.len() {
            let ledger = rows[i].ledger_sequence;
            let closed_at = rows[i].closed_at;

            // closed_at == 0 is the LEFT-JOIN sentinel for a ledger missing from
            // default.ledgers (no real ledger closed at unix 0). We cannot bucket
            // without a close time, so skip the ledger's events but count them —
            // surfacing the gap the earlier INNER JOIN dropped silently.
            if closed_at == 0 {
                let mut skipped = 0u64;
                while i < rows.len() && rows[i].ledger_sequence == ledger {
                    skipped += 1;
                    i += 1;
                }
                ledgers_missing_close += 1;
                events_missing_close += skipped;
                continue;
            }

            let mut events: Vec<RawSorobanEvent> = Vec::new();
            let mut last_key: Option<(i64, i64, i16)> = None;

            while i < rows.len() && rows[i].ledger_sequence == ledger {
                let r = &rows[i];
                i += 1;
                let key = (r.contract_id, r.transaction_id, r.event_index);
                if last_key == Some(key) {
                    continue; // adjacent RMT double of the same event
                }
                last_key = Some(key);
                let Some(strkey) = id_map.get(&r.contract_id) else {
                    continue; // contract not in the resolved set (shouldn't happen)
                };
                // topics_xdr / data_xdr are typed-JSON strings; a parse miss can
                // only drop one event, never abort the run.
                let topics = serde_json::from_str::<Value>(&r.topics_xdr).unwrap_or(Value::Null);
                let data = serde_json::from_str::<Value>(&r.data_xdr).unwrap_or(Value::Null);
                events.push(RawSorobanEvent {
                    contract_id: strkey.clone(),
                    transaction_id: r.transaction_id.to_string(),
                    ledger_sequence: r.ledger_sequence,
                    event_index: r.event_index as u32,
                    topics,
                    data,
                });
            }

            let mut out = LedgerSoroban::default();
            process_soroban_event_rows(ledger, closed_at, &events, &mut reg, &mut assets, &mut out);
            for (source, tick) in &out.amm_ticks {
                accumulators.entry(source).or_default().merge(tick);
                *ticks_by_source.entry(source).or_default() += 1;
            }
            raw_unresolved.extend(out.unresolved);
        }

        if cli.dry_run {
            let chunk_candles: usize = accumulators.values_mut().map(|a| a.flush_all().len()).sum();
            info!(
                chunk_start,
                chunk_end,
                events = rows.len(),
                candles = chunk_candles,
                "dry-run: classified chunk (no writes)"
            );
        } else {
            // Persist newly-minted surrogate ids BEFORE the candles that
            // reference them: a crash between the two writes must never leave
            // prices.price_ohlcv_1m rows pointing at asset_ids absent from
            // prices.assets. All `assets` mints for this chunk are already in the
            // registry (done in the ledger loop above), so writing it now covers
            // every base/quote id the candles use. Idempotent (RMT).
            retry_write(|| async { writer.write_assets(&assets).await }).await?;
            for (source, acc) in accumulators.iter_mut() {
                let candles = acc.flush_all();
                if candles.is_empty() {
                    continue;
                }
                total_candles += candles.len() as u64;
                retry_write(|| async { writer.write_candles(&candles, source).await }).await?;
            }
            info!(chunk_start, chunk_end, events = rows.len(), "chunk written");
        }

        if chunk_end >= cli.end {
            break;
        }
        chunk_start = chunk_end + 1;
    }

    // Record any dropped-swap pools for operator visibility (should be empty:
    // reads are filtered to venue-known contracts, so the only path here is a
    // Soroswap pool seeded without its pair tokens).
    if !cli.dry_run && !raw_unresolved.is_empty() {
        let unresolved = aggregate_unresolved(&raw_unresolved, &reg);
        let genuine = unresolved
            .iter()
            .filter(|u| u.still_unresolved == 1)
            .count();
        retry_write(|| async { writer.write_unresolved_pools(&unresolved).await }).await?;
        warn!(
            contracts = unresolved.len(),
            genuine, "recorded unresolved AMM pools to prices.unresolved_pools"
        );
    }

    if ledgers_missing_close > 0 {
        warn!(
            ledgers_missing_close,
            events_missing_close,
            "ledgers in range are absent from default.ledgers (no close time) — their AMM \
             events were skipped, NOT repriced; verify BE's ledgers table fully covers the range"
        );
    }

    // Dry-run only: advisory registry-completeness probe. Reads are filtered to
    // seeded-registry contracts, so a pool missing from prices.pool_registry is
    // invisible to the reprice (its swaps are never even fetched). This surfaces
    // AMM-shaped activity from contracts outside the registry so the operator can
    // verify coverage before committing writes (runbook §1). Scoped to dry-run to
    // keep this extra full-range scan off the write path.
    if cli.dry_run {
        let (contracts, events) =
            count_unregistered_amm_emitters(writer.client(), &contract_ids, cli.start, cli.end)
                .await?;
        if contracts > 0 {
            warn!(
                contracts,
                events,
                "dry-run coverage probe: contracts OUTSIDE the seeded registry emitted \
                 swap/trade-shaped events in range — prices.pool_registry may be incomplete \
                 (heuristic; may include non-AMM emitters — verify before the write run)"
            );
        } else {
            info!(
                "dry-run coverage probe: no swap/trade-shaped events from unregistered \
                 contracts in range"
            );
        }
    }

    print_summary(&ticks_by_source, total_events, total_candles, cli.dry_run);
    info!(
        elapsed_s = run_start.elapsed().as_secs(),
        "reprice complete"
    );
    Ok(())
}

/// Aggregate per-ledger unresolved-swap records by contract and re-check each
/// against the final registry (`still_unresolved = 1` when a contract is absent
/// from `reg.venue` at run-end). Mirrors the sdex-backfill aggregation; tagged
/// `source = "events-backfill"` in `prices.unresolved_pools`.
fn aggregate_unresolved(raw: &[UnresolvedPoolSwap], reg: &Registries) -> Vec<UnresolvedPool> {
    struct Agg {
        first: u32,
        last: u32,
        count: u64,
        sample: String,
    }
    let mut by_contract: HashMap<&str, Agg> = HashMap::new();
    for u in raw {
        let e = by_contract.entry(&u.contract_id).or_insert(Agg {
            first: u.ledger_sequence,
            last: u.ledger_sequence,
            count: 0,
            sample: u.sample_topics.clone(),
        });
        e.first = e.first.min(u.ledger_sequence);
        e.last = e.last.max(u.ledger_sequence);
        e.count += u.swap_count as u64;
    }

    let mut out: Vec<UnresolvedPool> = by_contract
        .into_iter()
        .map(|(contract_id, a)| UnresolvedPool {
            contract_id: contract_id.to_string(),
            source: "events-backfill".to_string(),
            first_ledger: a.first,
            last_ledger: a.last,
            swap_count: a.count,
            sample_topics: a.sample,
            still_unresolved: u8::from(!reg.venue.contains_key(contract_id)),
        })
        .collect();
    out.sort_by(|a, b| {
        b.still_unresolved
            .cmp(&a.still_unresolved)
            .then_with(|| a.contract_id.cmp(&b.contract_id))
    });
    out
}

fn print_summary(
    ticks_by_source: &HashMap<&'static str, u64>,
    total_events: u64,
    total_candles: u64,
    dry_run: bool,
) {
    println!();
    println!("=== events-backfill complete ===");
    println!("events read:               {total_events}");
    let mut sources: Vec<(&&str, &u64)> = ticks_by_source.iter().collect();
    sources.sort_by_key(|(s, _)| **s);
    for (source, ticks) in sources {
        println!("{source:>10} ticks:          {ticks}");
    }
    if dry_run {
        println!("price_ohlcv_1m rows:       (dry-run — not written)");
    } else {
        println!("price_ohlcv_1m rows:       {total_candles}");
    }
}
