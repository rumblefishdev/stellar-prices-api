//! Profile pass: scan N Galexie `.xdr.zst` files and report decode-time +
//! trade-bearing-op density. Output is consumed by
//! `notes/G-sdex-filter-strategy.md`.

use anyhow::{Context, Result};
use sdex_profile::{LedgerClaimStats, batch_metas, decode_file};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: profile <dir-of-xdr-zst-files> [max_files]\n\
             prints per-ledger timing + claim-atom density to stdout."
        );
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let max_files: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    let files: Vec<PathBuf> = walkdir::WalkDir::new(&dir)
        .min_depth(1)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_owned())
        .filter(|p| p.to_string_lossy().ends_with(".xdr.zst"))
        .take(max_files)
        .collect();

    eprintln!("scanning {} files from {}", files.len(), dir.display());

    let mut total_decode = Duration::ZERO;
    let mut total_walk = Duration::ZERO;
    let mut total_bytes_compressed: u64 = 0;
    let mut ledgers_scanned: u64 = 0;
    let mut ledgers_trade_bearing: u64 = 0;
    let mut total_claims: u64 = 0;
    let mut total_atom_v0: u64 = 0;
    let mut total_atom_order_book: u64 = 0;
    let mut total_atom_liquidity_pool: u64 = 0;
    let mut total_ops: u64 = 0;
    let mut total_trade_bearing_ops: u64 = 0;
    let mut total_txs_success: u64 = 0;
    let mut total_txs_failed: u64 = 0;
    let mut claims_per_trade_bearing_ledger: BTreeMap<u32, u32> = BTreeMap::new();

    for (i, f) in files.iter().enumerate() {
        let bytes = std::fs::read(f).with_context(|| format!("read {}", f.display()))?;
        total_bytes_compressed += bytes.len() as u64;
        let t_decode = Instant::now();
        let batch = match decode_file(f) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("decode error {}: {e:#}", f.display());
                continue;
            }
        };
        total_decode += t_decode.elapsed();

        let metas = batch_metas(&batch);
        for meta in metas {
            ledgers_scanned += 1;
            let t_walk = Instant::now();
            let stats = LedgerClaimStats::from_meta(meta);
            total_walk += t_walk.elapsed();

            total_ops += stats.total_ops as u64;
            total_trade_bearing_ops += stats.trade_bearing_ops as u64;
            total_txs_success += stats.successful_txs as u64;
            total_txs_failed += stats.failed_txs as u64;

            if stats.is_trade_bearing() {
                ledgers_trade_bearing += 1;
                *claims_per_trade_bearing_ledger.entry(stats.total).or_insert(0) += 1;
            }
            total_claims += stats.total as u64;
            total_atom_v0 += stats.v0 as u64;
            total_atom_order_book += stats.order_book as u64;
            total_atom_liquidity_pool += stats.liquidity_pool as u64;
        }

        if (i + 1) % 500 == 0 {
            eprintln!(
                "  progress {}/{} files; {} ledgers; {} trade-bearing",
                i + 1,
                files.len(),
                ledgers_scanned,
                ledgers_trade_bearing
            );
        }
    }

    let pct = |num: u64, den: u64| -> f64 {
        if den == 0 {
            0.0
        } else {
            100.0 * num as f64 / den as f64
        }
    };

    let median_claims = {
        let mut all: Vec<u32> = claims_per_trade_bearing_ledger
            .iter()
            .flat_map(|(&k, &n)| std::iter::repeat(k).take(n as usize))
            .collect();
        all.sort_unstable();
        if all.is_empty() {
            0
        } else {
            all[all.len() / 2]
        }
    };
    let mean_claims_per_trade_bearing = if ledgers_trade_bearing == 0 {
        0.0
    } else {
        total_claims as f64 / ledgers_trade_bearing as f64
    };
    let p95_claims = {
        let mut all: Vec<u32> = claims_per_trade_bearing_ledger
            .iter()
            .flat_map(|(&k, &n)| std::iter::repeat(k).take(n as usize))
            .collect();
        all.sort_unstable();
        if all.is_empty() {
            0
        } else {
            all[(all.len() as f64 * 0.95) as usize]
        }
    };
    let max_claims = claims_per_trade_bearing_ledger
        .keys()
        .rev()
        .next()
        .copied()
        .unwrap_or(0);

    println!("# SDEX profile run\n");
    println!("## Input");
    println!("- files scanned: {}", files.len());
    println!("- ledgers decoded: {}", ledgers_scanned);
    println!(
        "- compressed bytes read: {} ({:.2} MiB)",
        total_bytes_compressed,
        total_bytes_compressed as f64 / (1024.0 * 1024.0)
    );
    println!();
    println!("## Timing (single-threaded)");
    println!(
        "- decompress + decode: {:.2}s total, {:.2}ms/ledger mean, {:.0} ledgers/s",
        total_decode.as_secs_f64(),
        1000.0 * total_decode.as_secs_f64() / ledgers_scanned.max(1) as f64,
        ledgers_scanned as f64 / total_decode.as_secs_f64().max(1e-9),
    );
    println!(
        "- claim-atom walk (post-decode): {:.2}s total, {:.3}ms/ledger mean",
        total_walk.as_secs_f64(),
        1000.0 * total_walk.as_secs_f64() / ledgers_scanned.max(1) as f64,
    );
    println!(
        "- total wall: {:.2}s ({:.0} ledgers/s end-to-end)",
        (total_decode + total_walk).as_secs_f64(),
        ledgers_scanned as f64 / (total_decode + total_walk).as_secs_f64().max(1e-9),
    );
    println!();
    println!("## Trade-bearing density");
    println!(
        "- trade-bearing ledgers: {} / {} ({:.2}%)",
        ledgers_trade_bearing,
        ledgers_scanned,
        pct(ledgers_trade_bearing, ledgers_scanned)
    );
    println!(
        "- total claim-atoms: {} (mean {:.2}/trade-bearing-ledger, median {}, p95 {}, max {})",
        total_claims, mean_claims_per_trade_bearing, median_claims, p95_claims, max_claims
    );
    println!(
        "- variant breakdown: V0 = {}, ORDER_BOOK = {}, LIQUIDITY_POOL = {}",
        total_atom_v0, total_atom_order_book, total_atom_liquidity_pool
    );
    println!(
        "- variant share: V0 {:.2}% / OB {:.2}% / LP {:.2}%",
        pct(total_atom_v0, total_claims),
        pct(total_atom_order_book, total_claims),
        pct(total_atom_liquidity_pool, total_claims)
    );
    println!();
    println!("## Op-level");
    println!(
        "- total ops in successful txs: {} (trade-bearing: {}, {:.4}%)",
        total_ops,
        total_trade_bearing_ops,
        pct(total_trade_bearing_ops, total_ops)
    );
    println!(
        "- transactions: {} successful, {} failed ({:.2}% failure rate)",
        total_txs_success,
        total_txs_failed,
        pct(total_txs_failed, total_txs_success + total_txs_failed)
    );

    Ok(())
}
