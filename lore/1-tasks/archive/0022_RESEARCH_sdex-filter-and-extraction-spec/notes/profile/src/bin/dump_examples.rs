//! Dump one worked example per `ClaimAtom` variant from a directory of
//! `.xdr.zst` files, plus the surrounding ledger/tx/op context. Output is
//! consumed by `notes/G-sdex-decode-and-bucket-spec.md` as the basis for
//! the per-variant worked examples.

use anyhow::Result;
use sdex_profile::{
    ClaimAtomVariant, SdexOpKind, batch_metas, claim_atom_to_json, decode_file, ledger_id,
    tx_views,
};
use std::collections::HashMap;
use std::path::PathBuf;
use stellar_xdr::curr::{
    ClaimAtom, ManageBuyOfferResult, ManageSellOfferResult, OperationResult, OperationResultTr,
    PathPaymentStrictReceiveResult, PathPaymentStrictSendResult, TransactionResultResult,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: dump-examples <dir-of-xdr-zst-files> <output-dir> [max_files]\n\
             writes <output-dir>/{{v0,order_book,liquidity_pool}}.json"
        );
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    let max_files: usize = args
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    std::fs::create_dir_all(&out_dir)?;

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

    let mut found: HashMap<ClaimAtomVariant, ()> = HashMap::new();
    let want = [
        ClaimAtomVariant::V0,
        ClaimAtomVariant::OrderBook,
        ClaimAtomVariant::LiquidityPool,
    ];

    for f in &files {
        let batch = match decode_file(f) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("decode {} failed: {e:#}", f.display());
                continue;
            }
        };
        for meta in batch_metas(&batch) {
            let (ledger_seq, close_time) = ledger_id(meta);
            for tx in tx_views(meta) {
                let op_results: &[OperationResult] = match &tx.result.result {
                    TransactionResultResult::TxSuccess(v) => v.as_slice(),
                    _ => &[],
                };
                for (op_idx, op) in op_results.iter().enumerate() {
                    let OperationResult::OpInner(tr) = op else {
                        continue;
                    };
                    for (claim_idx, (op_kind, atom)) in walk_op_inner(tr).enumerate() {
                        let variant = ClaimAtomVariant::of(atom);
                        if found.contains_key(&variant) {
                            continue;
                        }
                        let path = out_dir.join(format!(
                            "{}.json",
                            match variant {
                                ClaimAtomVariant::V0 => "v0",
                                ClaimAtomVariant::OrderBook => "order_book",
                                ClaimAtomVariant::LiquidityPool => "liquidity_pool",
                            }
                        ));
                        let v = serde_json::json!({
                            "source_file": f.to_string_lossy(),
                            "ledger_sequence": ledger_seq,
                            "close_time_unix": close_time,
                            "transaction_hash": hex::encode(tx.transaction_hash),
                            "operation_index_0based": op_idx,
                            "claim_index_0based": claim_idx,
                            "op_kind": format!("{:?}", op_kind),
                            "atom": claim_atom_to_json(atom)?,
                        });
                        std::fs::write(&path, serde_json::to_string_pretty(&v)?)?;
                        println!("wrote {}", path.display());
                        found.insert(variant, ());
                        if want.iter().all(|w| found.contains_key(w)) {
                            println!("all three variants captured; stopping");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    if want.iter().any(|w| !found.contains_key(w)) {
        let missing: Vec<_> = want.iter().filter(|w| !found.contains_key(w)).collect();
        eprintln!(
            "warning: scanned all input but did not find variants: {:?}\nfound: {:?}",
            missing,
            found.keys().collect::<Vec<_>>()
        );
    }

    Ok(())
}

fn walk_op_inner<'a>(
    tr: &'a OperationResultTr,
) -> Box<dyn Iterator<Item = (SdexOpKind, &'a ClaimAtom)> + 'a> {
    match tr {
        OperationResultTr::ManageSellOffer(ManageSellOfferResult::Success(s)) => Box::new(
            s.offers_claimed
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::ManageSellOffer, c)),
        ),
        OperationResultTr::ManageBuyOffer(ManageBuyOfferResult::Success(s)) => Box::new(
            s.offers_claimed
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::ManageBuyOffer, c)),
        ),
        OperationResultTr::CreatePassiveSellOffer(ManageSellOfferResult::Success(s)) => Box::new(
            s.offers_claimed
                .as_slice()
                .iter()
                .map(|c| (SdexOpKind::CreatePassiveSellOffer, c)),
        ),
        OperationResultTr::PathPaymentStrictReceive(PathPaymentStrictReceiveResult::Success(s)) => {
            Box::new(
                s.offers
                    .as_slice()
                    .iter()
                    .map(|c| (SdexOpKind::PathPaymentStrictReceive, c)),
            )
        }
        OperationResultTr::PathPaymentStrictSend(PathPaymentStrictSendResult::Success(s)) => {
            Box::new(
                s.offers
                    .as_slice()
                    .iter()
                    .map(|c| (SdexOpKind::PathPaymentStrictSend, c)),
            )
        }
        _ => Box::new(std::iter::empty()),
    }
}
