//! Oracle worker (task 0039) — polls the Reflector SEP-40 price oracle via
//! Soroban RPC `simulateTransaction` and writes `prices.oracle_prices`.
//!
//! Non-critical (general-overview §2.2): a failed fetch logs + is skipped; the
//! oracle column simply shows the last known value. Reuses `prices-ingest-core`'s
//! `OracleSample` / `write_oracle` so rows match the event-decoded oracle path.
//!
//! v1 queries the Reflector "External CEX & DEX" oracle for the USD-pegged
//! assets that resolve to a Stellar identity (XLM + USDC/USDT), reusing the
//! event path's `reflector_key_to_identity`. Reflector prices are 14-decimal
//! `i128` (SEP-40 `decimals()`), matching `OracleSample.price_usd` and
//! `oracle_prices.price_usd Decimal(38,14)`.

use base64::Engine;
use prices_ingest_core::{
    AssetIdentity, AssetRegistry, OhlcvWriter, OracleSample, reflector_key_to_identity,
};
use stellar_xdr::{
    ContractId, Hash, HostFunction, Int128Parts, InvokeContractArgs, InvokeHostFunctionOp, Limits,
    Memo, MuxedAccount, Operation, OperationBody, Preconditions, ReadXdr, ScAddress, ScMap,
    ScSymbol, ScVal, ScVec, SequenceNumber, Transaction, TransactionEnvelope, TransactionExt,
    TransactionV1Envelope, Uint256, VecM, WriteXdr,
};

/// Reflector "External CEXs & DEXs" oracle (SEP-40), Stellar mainnet.
pub const REFLECTOR_CEX_DEX: &str = "CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN";
/// SEP-40 price decimals Reflector reports (matches `Decimal(38,14)`).
pub const REFLECTOR_DECIMALS: u32 = 14;
/// Default public Soroban RPC endpoint (overridable via `SOROBAN_RPC_URL`).
pub const DEFAULT_SOROBAN_RPC: &str = "https://mainnet.sorobanrpc.com";
/// Reflector symbols the worker polls. Each is mapped to a canonical Stellar
/// identity by the shared [`reflector_key_to_identity`]; a symbol that has no
/// Stellar identity is fetched-then-skipped (the loop's filter), so this list
/// can grow independently of the mapping.
pub const TRACKED_SYMBOLS: &[&str] = &["XLM", "USDC", "USDT"];

/// The identities whose oracle readings are snapshotted into `prices.usd_rate`
/// (task 0167). Deliberately a **subset** of [`TRACKED_SYMBOLS`].
///
/// ⚠️ **XLM is polled but NOT snapshotted here, and that is a scope boundary,
/// not an oversight.** XLM is the reference asset the *pivot* tier prices
/// everything else through, and task 0154 owns the `'pivot'` / `'pivot2'`
/// methods and the transitivity rules that go with them. Writing XLM rows here
/// would pre-empt those decisions in a table 0154 then has to live with.
///
/// ⏳ **But the 13-month expiry argument applies to XLM's history identically**,
/// and that argument is the whole reason 0167 was pulled forward. If 0154 has
/// not started before `202509` ages out (~2026-10/11), snapshotting XLM as
/// `method = 'oracle'`, `hops = 0` — which is what it factually is, no pivot
/// involved — should be reconsidered on its own merits rather than deferred by
/// default. Raised explicitly so the omission is a decision, not an accident.
pub fn peg_identities() -> Vec<AssetIdentity> {
    // Built rather than declared const: AssetIdentity::Credit holds Strings.
    // Sourced from the same consts the enrichment peg tier and views.sql use,
    // so the three cannot drift apart.
    vec![
        AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: prices_clickhouse::USDC_ISSUER.to_string(),
        },
        AssetIdentity::Credit {
            code: "USDT".to_string(),
            issuer: prices_clickhouse::USDT_ISSUER.to_string(),
        },
    ]
}

#[derive(Debug, thiserror::Error)]
pub enum OracleError {
    #[error(transparent)]
    Clickhouse(#[from] clickhouse::error::Error),
    #[error(transparent)]
    Ingest(#[from] prices_ingest_core::IngestError),
    #[error("soroban rpc: {0}")]
    Http(#[from] reqwest::Error),
    #[error("rpc json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("xdr: {0}")]
    Xdr(#[from] stellar_xdr::Error),
    #[error("strkey: {0}")]
    Strkey(#[from] stellar_strkey::DecodeError),
    #[error("base64: {0}")]
    Base64(#[from] base64::DecodeError),
}

/// A decoded `PriceData` from the oracle (price is 14-decimal `i128`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

fn sc_symbol(s: &str) -> Result<ScSymbol, OracleError> {
    Ok(ScSymbol(s.try_into()?))
}

/// SEP-40 `Asset::Other(Symbol)` ScVal — a contracttype enum is a 2-element vec
/// `[variant-symbol, value]`.
pub fn asset_other_scval(symbol: &str) -> Result<ScVal, OracleError> {
    let variant = ScVal::Symbol(sc_symbol("Other")?);
    let value = ScVal::Symbol(sc_symbol(symbol)?);
    let vec: VecM<ScVal> = vec![variant, value].try_into()?;
    Ok(ScVal::Vec(Some(ScVec(vec))))
}

/// Reconstruct an `i128` from XDR `Int128Parts`.
fn i128_from_parts(parts: &Int128Parts) -> i128 {
    ((parts.hi as i128) << 64) | (parts.lo as i128)
}

/// Parse the `lastprice` return ScVal (`Option<PriceData>`): `Void` → `None`,
/// otherwise a `PriceData` map `{price, timestamp}`.
pub fn parse_price_data(scval: &ScVal) -> Option<PriceData> {
    let ScVal::Map(Some(ScMap(entries))) = scval else {
        return None; // Void (None) or unexpected shape
    };
    let mut price = None;
    let mut timestamp = None;
    for entry in entries.iter() {
        let ScVal::Symbol(ScSymbol(key)) = &entry.key else {
            continue;
        };
        match key.to_string().as_str() {
            "price" => {
                if let ScVal::I128(parts) = &entry.val {
                    price = Some(i128_from_parts(parts));
                }
            }
            "timestamp" => {
                if let ScVal::U64(ts) = &entry.val {
                    timestamp = Some(*ts);
                }
            }
            _ => {}
        }
    }
    Some(PriceData {
        price: price?,
        timestamp: timestamp?,
    })
}

/// Build a base64 `TransactionEnvelope` invoking `func(args)` on `contract`,
/// for a read-only `simulateTransaction` (source account / fee / seq are not
/// checked by simulation).
pub fn build_simulate_envelope(
    contract: &str,
    func: &str,
    args: Vec<ScVal>,
) -> Result<String, OracleError> {
    let contract_hash = stellar_strkey::Contract::from_string(contract)?.0;
    let invoke = InvokeContractArgs {
        contract_address: ScAddress::Contract(ContractId(Hash(contract_hash))),
        function_name: sc_symbol(func)?,
        args: args.try_into()?,
    };
    let op = Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(invoke),
            auth: VecM::default(),
        }),
    };
    let tx = Transaction {
        // Simulation ignores the source account / fee / seq for a read-only call.
        source_account: MuxedAccount::Ed25519(Uint256([0u8; 32])),
        fee: 0,
        seq_num: SequenceNumber(0),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: vec![op].try_into()?,
        ext: TransactionExt::V0,
    };
    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    });
    let xdr = envelope.to_xdr(Limits::none())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(xdr))
}

#[derive(serde::Deserialize)]
struct RpcResponse {
    result: Option<RpcResult>,
}
#[derive(serde::Deserialize)]
struct RpcResult {
    #[serde(default)]
    results: Vec<RpcInvokeResult>,
    #[serde(default)]
    error: Option<String>,
}
#[derive(serde::Deserialize)]
struct RpcInvokeResult {
    xdr: String,
}

/// Call `lastprice(asset)` on the Reflector contract via Soroban RPC
/// `simulateTransaction`, returning the decoded `PriceData` (`None` if the
/// oracle has no price for the asset).
pub async fn fetch_lastprice(
    http: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
    symbol: &str,
) -> Result<Option<PriceData>, OracleError> {
    let envelope =
        build_simulate_envelope(contract, "lastprice", vec![asset_other_scval(symbol)?])?;
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateTransaction",
        "params": { "transaction": envelope },
    });
    let resp: RpcResponse = http.post(rpc_url).json(&body).send().await?.json().await?;
    let result = match resp.result {
        Some(r) => r,
        None => return Ok(None),
    };
    if let Some(err) = result.error {
        tracing::warn!(symbol, error = %err, "simulate returned error");
        return Ok(None);
    }
    let Some(first) = result.results.into_iter().next() else {
        return Ok(None);
    };
    let raw = base64::engine::general_purpose::STANDARD.decode(first.xdr)?;
    let scval = ScVal::from_xdr(&raw, Limits::none())?;
    Ok(parse_price_data(&scval))
}

/// Outcome of a [`run_oracle`] pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct OracleStats {
    pub queried: usize,
    pub written: usize,
    pub skipped: usize,
    /// Peg identities whose rates were snapshotted into `prices.usd_rate`
    /// (task 0167). Zero is not an error — see [`run_oracle`].
    pub rates_snapshotted: usize,
}

/// Poll Reflector for each tracked symbol and write the prices to
/// `oracle_prices`. Best-effort: per-symbol failures are skipped; only a CH
/// write failure fails the run.
pub async fn run_oracle(
    writer: &OhlcvWriter,
    http: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
) -> Result<OracleStats, OracleError> {
    let existing = writer.load_assets().await?;
    let mut registry = AssetRegistry::from_existing(existing);
    let known_before = registry.assets().count();
    let mut samples = Vec::new();
    let mut skipped = 0usize;

    for &symbol in TRACKED_SYMBOLS {
        // Filter out any polled symbol with no Stellar identity (shared with the
        // event-decode path, so the two oracle arms map symbols identically).
        let Some(identity) = reflector_key_to_identity(symbol) else {
            continue;
        };
        match fetch_lastprice(http, rpc_url, contract, symbol).await {
            Ok(Some(pd)) => {
                let asset_id = registry.get_or_assign(&identity);
                samples.push(OracleSample {
                    // Reflector reports millisecond timestamps; oracle_prices.timestamp
                    // is DateTime (epoch seconds), so divide by 1000 to match the
                    // event-decoded path (prices-ingest-core soroban.rs). The clamp is
                    // a backstop for the 2106 u32 ceiling, not the unit conversion.
                    timestamp: (pd.timestamp / 1000).min(u32::MAX as u64) as u32,
                    asset_id,
                    oracle_name: "reflector".to_string(),
                    price_usd: pd.price,
                    raw_data: format!("{{\"symbol\":\"{symbol}\"}}"),
                });
            }
            Ok(None) => {
                skipped += 1;
                tracing::debug!(symbol, "no Reflector price");
            }
            Err(err) => {
                skipped += 1;
                tracing::warn!(symbol, error = %err, "oracle fetch failed; skipping");
            }
        }
    }

    // Persist any newly-minted surrogate ids BEFORE the oracle rows that
    // reference them, so oracle_prices.asset_id always resolves in prices.assets
    // and never collides with an id the discovery/ingest path mints next (it
    // derives next_id from the persisted max).
    if registry.assets().count() > known_before {
        writer.write_assets(&registry).await?;
    }

    let written = samples.len();
    writer.write_oracle(&samples).await?;

    // Snapshot the peg rates into the forever-retained prices.usd_rate
    // (task 0167). This runs AFTER write_oracle so the rows just polled are
    // included in the same pass rather than waiting for the next one.
    //
    // Deliberately NON-FATAL. oracle_prices is the source of truth and has
    // already been written by this point; the snapshot is a derived copy that
    // is incremental by watermark, so a failed pass costs nothing but is
    // retried in full on the next run. Failing the whole worker here would stop
    // oracle polling itself — trading a durable, self-healing gap for a live
    // outage. The 0139 guard inside the copy is the most likely reason to land
    // here, and it is a data condition an operator must resolve, not something
    // a retry fixes.
    let rates_snapshotted = match writer
        .populate_usd_rate_from_oracle(&peg_identities())
        .await
    {
        Ok(stats) => stats.identities,
        Err(err) => {
            tracing::error!(error = %err, "usd_rate snapshot failed; oracle_prices is unaffected");
            0
        }
    };

    Ok(OracleStats {
        queried: TRACKED_SYMBOLS.len(),
        written,
        skipped,
        rates_snapshotted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tracked_symbol_resolves_to_an_identity() {
        // The symbol→identity mapping itself is owned + tested in
        // prices-ingest-core (reflector_key_to_identity); here we only assert
        // the worker's poll list agrees with it, so no row is silently skipped.
        for &symbol in TRACKED_SYMBOLS {
            assert!(
                reflector_key_to_identity(symbol).is_some(),
                "tracked symbol {symbol} has no Stellar identity"
            );
        }
    }

    #[test]
    fn asset_other_scval_is_two_element_vec() {
        let sc = asset_other_scval("BTC").unwrap();
        let ScVal::Vec(Some(ScVec(v))) = sc else {
            panic!("expected vec");
        };
        assert_eq!(v.len(), 2);
        assert!(matches!(&v[0], ScVal::Symbol(s) if s.0.to_string() == "Other"));
        assert!(matches!(&v[1], ScVal::Symbol(s) if s.0.to_string() == "BTC"));
    }

    #[test]
    fn parses_price_data_map() {
        let entries: VecM<_> = vec![
            stellar_xdr::ScMapEntry {
                key: ScVal::Symbol(sc_symbol("price").unwrap()),
                val: ScVal::I128(Int128Parts { hi: 0, lo: 250 }),
            },
            stellar_xdr::ScMapEntry {
                key: ScVal::Symbol(sc_symbol("timestamp").unwrap()),
                val: ScVal::U64(1700),
            },
        ]
        .try_into()
        .unwrap();
        let scval = ScVal::Map(Some(ScMap(entries)));
        let pd = parse_price_data(&scval).expect("some");
        assert_eq!(
            pd,
            PriceData {
                price: 250,
                timestamp: 1700
            }
        );
    }

    #[test]
    fn parses_void_as_none() {
        assert!(parse_price_data(&ScVal::Void).is_none());
    }

    #[test]
    fn builds_a_base64_envelope() {
        let env = build_simulate_envelope(
            REFLECTOR_CEX_DEX,
            "lastprice",
            vec![asset_other_scval("XLM").unwrap()],
        )
        .expect("envelope");
        assert!(!env.is_empty());
        // Round-trips as valid base64 XDR.
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&env)
            .unwrap();
        assert!(TransactionEnvelope::from_xdr(&raw, Limits::none()).is_ok());
    }
}
