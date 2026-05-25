//! Find and dump the canonical pure-SDEX trade case: a
//! `ManageSellOffer` / `ManageBuyOffer` / `CreatePassiveSellOffer`
//! result with **multiple** `offers_claimed` (book-walking trade).

use anyhow::Result;
use sdex_profile::{
    SdexOpKind, batch_metas, claim_atom_to_json, decode_file, ledger_id, tx_views,
};
use std::path::PathBuf;
use stellar_xdr::curr::{
    ManageBuyOfferResult, ManageSellOfferResult, OperationResult, OperationResultTr,
    TransactionResultResult,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: dump-canonical <dir-of-xdr-zst-files> <output-dir> [min_claims=2]");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    let min_claims: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);

    std::fs::create_dir_all(&out_dir)?;

    let files: Vec<PathBuf> = walkdir::WalkDir::new(&dir)
        .min_depth(1)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_owned())
        .filter(|p| p.to_string_lossy().ends_with(".xdr.zst"))
        .collect();

    for f in &files {
        let batch = match decode_file(f) {
            Ok(b) => b,
            Err(_) => continue,
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
                    let (op_kind, claims): (SdexOpKind, &[stellar_xdr::curr::ClaimAtom]) = match tr {
                        OperationResultTr::ManageSellOffer(ManageSellOfferResult::Success(s)) => {
                            (SdexOpKind::ManageSellOffer, s.offers_claimed.as_slice())
                        }
                        OperationResultTr::ManageBuyOffer(ManageBuyOfferResult::Success(s)) => {
                            (SdexOpKind::ManageBuyOffer, s.offers_claimed.as_slice())
                        }
                        OperationResultTr::CreatePassiveSellOffer(
                            ManageSellOfferResult::Success(s),
                        ) => (
                            SdexOpKind::CreatePassiveSellOffer,
                            s.offers_claimed.as_slice(),
                        ),
                        _ => continue,
                    };
                    if claims.len() < min_claims {
                        continue;
                    }
                    let atoms_json: Vec<_> = claims
                        .iter()
                        .enumerate()
                        .map(|(i, atom)| {
                            Ok::<_, anyhow::Error>(serde_json::json!({
                                "claim_index_0based": i,
                                "atom": claim_atom_to_json(atom)?,
                            }))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let v = serde_json::json!({
                        "source_file": f.to_string_lossy(),
                        "ledger_sequence": ledger_seq,
                        "close_time_unix": close_time,
                        "transaction_hash": hex::encode(tx.transaction_hash),
                        "operation_index_0based": op_idx,
                        "op_kind": format!("{:?}", op_kind),
                        "claim_count": claims.len(),
                        "claims": atoms_json,
                    });
                    let path = out_dir.join("manage_offer_multi_claim.json");
                    std::fs::write(&path, serde_json::to_string_pretty(&v)?)?;
                    println!(
                        "wrote {} ({} claims, op_kind {:?})",
                        path.display(),
                        claims.len(),
                        op_kind
                    );
                    return Ok(());
                }
            }
        }
    }

    eprintln!("no manage-offer with >= {min_claims} claims found in input");
    Ok(())
}
