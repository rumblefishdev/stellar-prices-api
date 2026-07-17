//! Orchestration for the events-sourced AMM reprice (task 0097).
//!
//! Preload the seeded `prices.pool_registry` + `prices.assets` (same startup as
//! the live/backfill paths), then walk `[start, end]` in chunks: **stream** each
//! chunk's AMM events from `default.soroban_events`, group by ledger, feed them
//! through the shared [`process_soroban_event_rows`] seam, accumulate 1-minute
//! candles per source, and write them to `prices.price_ohlcv_1m`. Writes are
//! ReplacingMergeTree-idempotent, so a re-run over any range only replaces.
//!
//! The [`CandleAccumulator`]s live for the **whole run**, not per chunk, and are
//! drained with `flush_older_than(current_minute)` as ledgers advance. A minute
//! that straddles a chunk boundary therefore stays open until it is truly
//! complete, so it is written once (summed) rather than as two partial candles
//! the ReplacingMergeTree would collapse to an undercount.

use std::collections::HashMap;
use std::time::Instant;

use clickhouse::Client;
use serde_json::Value;
use tracing::{info, warn};

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, DEFAULT_BACKOFF_MS, LedgerSoroban, OhlcvCandle, OhlcvWriter,
    RawSorobanEvent, Registries, UnresolvedPool, UnresolvedPoolSwap, process_soroban_event_rows,
    retry_with_backoff,
};

use crate::cli::Cli;
use crate::error::EventsBackfillError;
use crate::source::{count_unregistered_amm_emitters, resolve_contract_ids, stream_chunk};

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

/// Run the shared seam over one ledger's events, merge the resulting ticks into
/// the **run-level** per-source accumulators, then move every candle for a minute
/// strictly older than this ledger's minute out of the accumulators and into the
/// per-source write buffers. Synchronous (no I/O): buffered candles are written
/// later, in batches, at chunk end.
///
/// Because events arrive in ledger (hence minute) order and the accumulators
/// persist across chunks, only the current minute stays open — so a minute split
/// across two read chunks is summed into one candle, never written as two
/// RMT-colliding partials.
#[allow(clippy::too_many_arguments)]
fn accumulate_ledger(
    ledger: u32,
    closed_at: i64,
    events: &[RawSorobanEvent],
    reg: &mut Registries,
    assets: &mut AssetRegistry,
    accumulators: &mut HashMap<&'static str, CandleAccumulator>,
    buffers: &mut HashMap<&'static str, Vec<OhlcvCandle>>,
    ticks_by_source: &mut HashMap<&'static str, u64>,
    raw_unresolved: &mut Vec<UnresolvedPoolSwap>,
) {
    let mut out = LedgerSoroban::default();
    process_soroban_event_rows(ledger, closed_at, events, reg, assets, &mut out);
    for (source, tick) in &out.amm_ticks {
        accumulators.entry(source).or_default().merge(tick);
        *ticks_by_source.entry(source).or_default() += 1;
    }
    raw_unresolved.extend(out.unresolved);

    // Minute-START timestamp of this ledger — the same key the accumulator buckets
    // on (`(closed_at/60)*60`), NOT the minute index; flush_older_than compares
    // against the bucket key directly.
    let current_minute = (closed_at as u32 / 60) * 60;
    let sources: Vec<&'static str> = accumulators.keys().copied().collect();
    for source in sources {
        let candles = accumulators
            .get_mut(source)
            .expect("source present")
            .flush_older_than(current_minute);
        if !candles.is_empty() {
            buffers.entry(source).or_default().extend(candles);
        }
    }
}

/// Write one source's buffered candles as a single batch. Newly-minted asset ids
/// are persisted FIRST (so a candle never references an asset_id absent from
/// `prices.assets`) and ONLY when the registry actually grew since the last asset
/// write — collapsing what would otherwise be a full-registry write on every
/// batch down to a handful over a run. Dry-run counts without writing.
async fn emit_buffer(
    writer: &OhlcvWriter,
    dry_run: bool,
    candles: &[OhlcvCandle],
    source: &str,
    assets: &AssetRegistry,
    assets_written: &mut usize,
    total_candles: &mut u64,
) -> Result<(), EventsBackfillError> {
    if candles.is_empty() {
        return Ok(());
    }
    *total_candles += candles.len() as u64;
    if dry_run {
        return Ok(());
    }
    let asset_count = assets.assets().count();
    if asset_count > *assets_written {
        retry_write(|| async { writer.write_assets(assets).await }).await?;
        *assets_written = asset_count;
    }
    retry_write(|| async { writer.write_candles(candles, source).await }).await?;
    Ok(())
}

fn build_client(cli: &Cli) -> Client {
    let mut client = Client::default()
        .with_url(&cli.clickhouse_url)
        .with_user(&cli.clickhouse_user);
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
    if cli.chunk_size == 0 {
        return Err(EventsBackfillError::InvalidChunkSize);
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
    if contract_ids.is_empty() {
        return Err(EventsBackfillError::NoResolvedContracts(pool_strkeys.len()));
    }
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

    // Run-level state (persists across chunks): one accumulator per source, one
    // write buffer per source, and the count of assets already written to
    // prices.assets (seeded with the preload so the first mint is what triggers a
    // write). `flush_older_than` keeps the current minute open across chunk
    // boundaries; the buffers are drained (written) once per chunk.
    let mut accumulators: HashMap<&'static str, CandleAccumulator> = HashMap::new();
    let mut buffers: HashMap<&'static str, Vec<OhlcvCandle>> = HashMap::new();
    let mut assets_written: usize = assets.assets().count();

    let mut chunk_start = cli.start;
    loop {
        let chunk_end = chunk_start.saturating_add(cli.chunk_size - 1).min(cli.end);

        // Stream the chunk's rows one at a time (bounded memory). Rows are ordered
        // (ledger, tx, event_index); accumulate a ledger's events until the ledger
        // changes, then run the seam on the completed ledger.
        let mut cursor = stream_chunk(writer.client(), &contract_ids, chunk_start, chunk_end)?;
        let mut cur_ledger: Option<u32> = None;
        let mut cur_closed_at: i64 = 0;
        let mut cur_missing = false;
        let mut cur_events: Vec<RawSorobanEvent> = Vec::new();
        let mut last_key: Option<(i64, i64, i16)> = None;

        while let Some(r) = cursor.next().await? {
            total_events += 1;

            if cur_ledger != Some(r.ledger_sequence) {
                // Ledger boundary: finish the previous (complete) ledger.
                if let Some(l) = cur_ledger
                    && !cur_missing
                {
                    accumulate_ledger(
                        l,
                        cur_closed_at,
                        &cur_events,
                        &mut reg,
                        &mut assets,
                        &mut accumulators,
                        &mut buffers,
                        &mut ticks_by_source,
                        &mut raw_unresolved,
                    );
                }
                cur_ledger = Some(r.ledger_sequence);
                cur_closed_at = r.closed_at;
                cur_events.clear();
                last_key = None;
                // closed_at == 0 is the LEFT-JOIN sentinel for a ledger absent from
                // default.ledgers (no real ledger closed at unix 0): skip its events
                // (can't bucket without a close time) but count them, surfacing the
                // gap the earlier INNER JOIN dropped silently.
                cur_missing = r.closed_at == 0;
                if cur_missing {
                    ledgers_missing_close += 1;
                }
            }

            if cur_missing {
                events_missing_close += 1;
                continue;
            }

            let key = (r.contract_id, r.transaction_id, r.event_index);
            if last_key == Some(key) {
                continue; // adjacent RMT double of the same event
            }
            last_key = Some(key);
            let Some(strkey) = id_map.get(&r.contract_id) else {
                continue; // contract not in the resolved set (shouldn't happen)
            };
            // topics_xdr / data_xdr are typed-JSON strings; a parse miss can only
            // drop one event, never abort the run.
            let topics = serde_json::from_str::<Value>(&r.topics_xdr).unwrap_or(Value::Null);
            let data = serde_json::from_str::<Value>(&r.data_xdr).unwrap_or(Value::Null);
            cur_events.push(RawSorobanEvent {
                contract_id: strkey.clone(),
                transaction_id: r.transaction_id.to_string(),
                ledger_sequence: r.ledger_sequence,
                event_index: r.event_index as u32,
                topics,
                data,
            });
        }

        // Finish the chunk's last ledger (complete — chunks are ledger-aligned).
        if let Some(l) = cur_ledger
            && !cur_missing
        {
            accumulate_ledger(
                l,
                cur_closed_at,
                &cur_events,
                &mut reg,
                &mut assets,
                &mut accumulators,
                &mut buffers,
                &mut ticks_by_source,
                &mut raw_unresolved,
            );
        }

        // Drain the chunk's completed-minute candles in one batch per source. The
        // open minute stays in the accumulators for the next chunk.
        let sources: Vec<&'static str> = buffers.keys().copied().collect();
        for source in sources {
            let batch = buffers.get_mut(source).expect("source present");
            emit_buffer(
                &writer,
                cli.dry_run,
                batch,
                source,
                &assets,
                &mut assets_written,
                &mut total_candles,
            )
            .await?;
            batch.clear();
        }
        info!(chunk_start, chunk_end, "chunk streamed");

        if chunk_end >= cli.end {
            break;
        }
        chunk_start = chunk_end + 1;
    }

    // Final drain: the trailing open minute(s) flush_older_than kept back.
    {
        let sources: Vec<&'static str> = accumulators.keys().copied().collect();
        for source in sources {
            let candles = accumulators
                .get_mut(source)
                .expect("source present")
                .flush_all();
            emit_buffer(
                &writer,
                cli.dry_run,
                &candles,
                source,
                &assets,
                &mut assets_written,
                &mut total_candles,
            )
            .await?;
        }
    }

    // Dropped-swap pools: reads are filtered to venue-known contracts, so the
    // only path here is a pool seeded without its pair tokens. Aggregated in BOTH
    // modes — these swaps are read but produce NO tick, so they are the
    // difference between the raw swap count in `soroban_events` and the tick
    // counts below. A dry-run that hides them looks like a clean run while
    // silently dropping a third of the range's swaps, and the operator has no way
    // to reconcile against an expected swap count. Only the WRITE is gated.
    let unresolved = aggregate_unresolved(&raw_unresolved, &reg);
    let unresolved_genuine: Vec<_> = unresolved
        .iter()
        .filter(|u| u.still_unresolved == 1)
        .collect();
    let dropped_swaps: u64 = unresolved_genuine.iter().map(|u| u.swap_count).sum();
    if !cli.dry_run && !unresolved.is_empty() {
        retry_write(|| async { writer.write_unresolved_pools(&unresolved).await }).await?;
        warn!(
            contracts = unresolved.len(),
            genuine = unresolved_genuine.len(),
            "recorded unresolved AMM pools to prices.unresolved_pools"
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
    // Print BEFORE the advisory probe: the reprice totals are this run's primary
    // output and must never be hostage to an optional extra scan. (They were —
    // the probe's `?` aborted a completed 12.9M-ledger dry-run and discarded
    // every tick count with it.)
    print_summary(
        &ticks_by_source,
        total_events,
        total_candles,
        cli.dry_run,
        unresolved_genuine.len(),
        dropped_swaps,
    );
    info!(
        elapsed_s = run_start.elapsed().as_secs(),
        "reprice complete"
    );

    if cli.dry_run {
        match count_unregistered_amm_emitters(
            writer.client(),
            &contract_ids,
            cli.start,
            cli.end,
            cli.chunk_size,
        )
        .await
        {
            Ok((contracts, events)) if contracts > 0 => warn!(
                contracts,
                events,
                "dry-run coverage probe: contracts OUTSIDE the seeded registry emitted \
                 swap/trade-shaped events in range — prices.pool_registry may be incomplete \
                 (heuristic; may include non-AMM emitters — verify before the write run)"
            ),
            Ok(_) => info!(
                "dry-run coverage probe: no swap/trade-shaped events from unregistered \
                 contracts in range"
            ),
            // Advisory only — never fail the run over it. The reprice totals are
            // already printed above and remain valid; only registry-completeness
            // assurance is lost.
            Err(e) => warn!(
                error = %e,
                "dry-run coverage probe FAILED — advisory only, reprice totals above are \
                 unaffected; registry completeness was NOT verified for this range"
            ),
        }
    }

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
    unresolved_contracts: usize,
    dropped_swaps: u64,
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
    // Always printed, including the 0 case: "0 dropped" is the statement that
    // lets an operator trust `ticks == swaps in soroban_events`. Silence would
    // be indistinguishable from "not measured".
    println!("unresolved pools:          {unresolved_contracts}");
    println!("swaps dropped (no tick):   {dropped_swaps}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use extractors_core::Venue;
    use serde_json::json;

    const POOL: &str = "CDBBBNMCWRMWEIFHUD5BXBCRTW6QM33ZEXIOBGKKQNDSH3WEF7WVBGMI";
    const T0: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
    const T1: &str = "CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK";

    fn soroswap_swap(ledger: u32, closed_at: i64) -> RawSorobanEvent {
        // A SoroswapPair swap in the typed-JSON shape BE persists — the same
        // priceable fixture used by the prices-ingest-core seam tests.
        let _ = closed_at;
        RawSorobanEvent {
            contract_id: POOL.to_string(),
            transaction_id: format!("tx-{ledger}"),
            ledger_sequence: ledger,
            event_index: 0,
            topics: json!([
                {"type":"string","value":"SoroswapPair"},
                {"type":"sym","value":"swap"}
            ]),
            data: json!({"type":"map","value":[
                {"key":{"type":"sym","value":"amount_0_in"},"value":{"type":"i128","value":"1000000"}},
                {"key":{"type":"sym","value":"amount_0_out"},"value":{"type":"i128","value":"0"}},
                {"key":{"type":"sym","value":"amount_1_in"},"value":{"type":"i128","value":"0"}},
                {"key":{"type":"sym","value":"amount_1_out"},"value":{"type":"i128","value":"914145"}}
            ]}),
        }
    }

    /// The boundary-undercount guard: two ledgers in the SAME minute processed by
    /// separate `accumulate_ledger` calls (as they would be if a chunk boundary
    /// fell between them) must sum into ONE candle, and that minute must stay in
    /// the accumulator (unwritten) until a later minute closes it. A per-chunk
    /// `flush_all` would instead emit two partial candles for the minute, which
    /// the ReplacingMergeTree collapses to one (higher version wins) — an
    /// undercount. Here `trade_count == 2` proves both swaps landed in one candle.
    #[test]
    fn minute_split_across_calls_is_summed_once_not_undercounted() {
        // Two ledgers in minute M (…000, …030), a third in minute M+1 (…060).
        const MIN_M_A: i64 = 1_700_000_000;
        const MIN_M_B: i64 = 1_700_000_030;
        const MIN_M_PLUS_1: i64 = 1_700_000_060;

        let mut reg = Registries::new();
        reg.venue.insert(POOL.to_string(), Venue::Soroswap);
        reg.soroswap
            .register(POOL.to_string(), T0.to_string(), T1.to_string());
        let mut assets = AssetRegistry::from_existing(vec![]);
        let mut accs: HashMap<&'static str, CandleAccumulator> = HashMap::new();
        let mut buffers: HashMap<&'static str, Vec<OhlcvCandle>> = HashMap::new();
        let mut ticks: HashMap<&'static str, u64> = HashMap::new();
        let mut unresolved = Vec::new();

        // A local fn (not a closure) so each call's mutable borrows release before
        // the intermediate assertions read `buffers`.
        #[allow(clippy::too_many_arguments)]
        fn step(
            ledger: u32,
            closed_at: i64,
            reg: &mut Registries,
            assets: &mut AssetRegistry,
            accs: &mut HashMap<&'static str, CandleAccumulator>,
            buffers: &mut HashMap<&'static str, Vec<OhlcvCandle>>,
            ticks: &mut HashMap<&'static str, u64>,
            unresolved: &mut Vec<UnresolvedPoolSwap>,
        ) {
            accumulate_ledger(
                ledger,
                closed_at,
                &[soroswap_swap(ledger, closed_at)],
                reg,
                assets,
                accs,
                buffers,
                ticks,
                unresolved,
            );
        }

        // Ledger in minute M — nothing flushed yet (M is the current minute).
        step(
            100,
            MIN_M_A,
            &mut reg,
            &mut assets,
            &mut accs,
            &mut buffers,
            &mut ticks,
            &mut unresolved,
        );
        assert!(
            buffers.get("soroswap").is_none_or(|b| b.is_empty()),
            "minute M must stay open after the first ledger (no premature/partial write)"
        );

        // Second ledger, still minute M — merges into the same open candle.
        step(
            101,
            MIN_M_B,
            &mut reg,
            &mut assets,
            &mut accs,
            &mut buffers,
            &mut ticks,
            &mut unresolved,
        );
        assert!(
            buffers.get("soroswap").is_none_or(|b| b.is_empty()),
            "minute M must remain open across the (simulated) chunk boundary, not flushed partially"
        );

        // Ledger in minute M+1 — advances the clock, closing minute M.
        step(
            102,
            MIN_M_PLUS_1,
            &mut reg,
            &mut assets,
            &mut accs,
            &mut buffers,
            &mut ticks,
            &mut unresolved,
        );
        let flushed = buffers
            .get("soroswap")
            .expect("minute M closed and buffered");
        assert_eq!(
            flushed.len(),
            1,
            "the two same-minute ledgers must produce ONE candle, not two partials"
        );
        assert_eq!(
            flushed[0].trade_count, 2,
            "both swaps must be summed into minute M's candle (an undercount would show 1)"
        );
    }
}
