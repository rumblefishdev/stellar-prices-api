//! Soroban token symbol resolution (task 0210).
//!
//! Soroban-native assets carry no `asset_code` — a SEP-41 token's display name
//! lives on the token contract, behind a `symbol()` call. This stage reads it
//! over Soroban RPC `simulateTransaction` and persists it to
//! `prices.asset_symbol`, whence the API composes it into the `asset_code`
//! field at read time.
//!
//! **Triggering is on absence, not staleness.** A contract that has any row in
//! `prices.asset_symbol` is never re-fetched: `symbol()` is fixed at deploy for
//! every real token implementation, so a freshness threshold would buy
//! re-verification nobody asked for and pay for it with permanent RPC load.
//! Steady state is therefore *zero work*; a newly discovered Soroban asset is
//! picked up on the next scheduled run. This is why there is no stalest-first
//! ordering and no time budget here, unlike `supply-worker` — its queue is the
//! whole 207k-row registry and never empties, this one is 52 rows and then
//! nothing.
//!
//! The three-way outcome policy *is* borrowed from `supply-worker::run_supply`,
//! because it is what keeps the queue from starving: see [`Outcome`].

use base64::Engine;
use clickhouse::Client;
use serde::Serialize;
use stellar_xdr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Limits, Memo,
    MuxedAccount, Operation, OperationBody, Preconditions, ReadXdr, ScAddress, ScString, ScSymbol,
    ScVal, SequenceNumber, Transaction, TransactionEnvelope, TransactionExt, TransactionV1Envelope,
    Uint256, VecM, WriteXdr,
};

/// Default public Soroban RPC endpoint (overridable via `SOROBAN_RPC_URL`).
/// Needs no auth — same endpoint and same reasoning as `oracle-worker`.
pub const DEFAULT_SOROBAN_RPC: &str = "https://mainnet.sorobanrpc.com";

/// Per-request RPC timeout. Together with [`MAX_CONTRACTS_PER_RUN`] this is the
/// stage's entire time bound: worst case is the product of the two, which must
/// leave the ledger scan its headroom inside the Lambda's 5-minute limit.
pub const RPC_TIMEOUT_SECS: u64 = 5;

/// How many unresolved contracts one run will attempt.
///
/// A blast-radius guard, not a tuning knob: the real backlog is the 52 Soroban
/// rows measured on prod, and it clears in three runs. The bound that matters is
/// `MAX_CONTRACTS_PER_RUN × RPC_TIMEOUT_SECS` = 125 s worst case, so this stage
/// cannot eat the ledger scan's budget even if every RPC call hangs.
pub const MAX_CONTRACTS_PER_RUN: usize = 25;

/// Longest symbol accepted. XDR already bounds `ScSymbol` at 32; `ScVal::String`
/// is unbounded, so this is the arm that needs the cap.
pub const MAX_SYMBOL_LEN: usize = 32;

/// What one contract's `symbol()` resolution produced.
///
/// The Absent/Transient split is load-bearing. Absent writes a sentinel row so
/// the contract leaves the queue permanently; Transient writes nothing so it is
/// retried next run. Collapsing them either way is a real failure: sentinelling
/// a transient error names 52 tokens `""` forever, and retrying an absent one
/// re-polls RPC every hour for a contract that will never answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A usable symbol.
    Symbol(String),
    /// Asked, and this contract exposes no usable symbol. Sentinel it.
    Absent,
    /// Transport-level failure. Leave it for the next run.
    Transient,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct SymbolRow {
    contract_address: String,
    symbol: String,
}

/// Outcome of a [`run_symbols`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SymbolStats {
    /// Unresolved contracts this run picked up.
    pub considered: usize,
    /// Contracts that yielded a usable symbol.
    pub resolved: usize,
    /// Contracts sentinelled as having no usable symbol.
    pub absent: usize,
    /// Contracts left for the next run after a transient failure.
    pub skipped: usize,
}

/// An HTTP client bounded by [`RPC_TIMEOUT_SECS`].
///
/// Built here rather than at the call site so the stage's time bound stays next
/// to the constant that states it.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(RPC_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

/// Base64 `TransactionEnvelope` invoking the zero-argument `func()` on
/// `contract`, for a read-only `simulateTransaction` (source account / fee / seq
/// are not checked by simulation).
///
/// `None` when `contract` is not a well-formed C-strkey — a data-integrity
/// problem in `prices.assets`, permanent for that row, so the caller sentinels
/// it rather than failing the run.
pub fn build_simulate_envelope(contract: &str, func: &str) -> Option<String> {
    let contract_hash = stellar_strkey::Contract::from_string(contract).ok()?.0;
    let invoke = InvokeContractArgs {
        contract_address: ScAddress::Contract(ContractId(Hash(contract_hash))),
        function_name: ScSymbol(func.try_into().ok()?),
        args: VecM::default(),
    };
    let op = Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(invoke),
            auth: VecM::default(),
        }),
    };
    let tx = Transaction {
        source_account: MuxedAccount::Ed25519(Uint256([0u8; 32])),
        fee: 0,
        seq_num: SequenceNumber(0),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: vec![op].try_into().ok()?,
        ext: TransactionExt::V0,
    };
    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    });
    let xdr = envelope.to_xdr(Limits::none()).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(xdr))
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

/// Extract a UTF-8 string from a `symbol()` return value.
///
/// Both arms are needed: the SEP-41 interface declares `Symbol`, but deployed
/// tokens return `ScVal::String` in practice.
pub fn scval_to_string(v: &ScVal) -> Option<String> {
    match v {
        ScVal::Symbol(ScSymbol(s)) => s.to_utf8_string().ok(),
        ScVal::String(ScString(s)) => s.to_utf8_string().ok(),
        _ => None,
    }
}

/// Bound and clean a symbol read off-chain, or reject it.
///
/// ponytail: a length and control-character floor only. `symbol()` is a string
/// the contract itself controls, so this bounds what we store and print — it
/// does **not** establish that the token is what it claims to be. A hostile
/// contract returning `"USDC"` passes this check. Asset identity verification
/// (SEP-1 `home_domain`, a verification flag on the response) is task 0252; this
/// task deliberately keeps the blast radius to display only, leaving `?search=`
/// and `sort=code` on the raw `assets.asset_code` column.
pub fn sanitize_symbol(raw: &str) -> Option<String> {
    let s = raw.trim();
    (!s.is_empty() && s.chars().count() <= MAX_SYMBOL_LEN && !s.chars().any(char::is_control))
        .then(|| s.to_string())
}

/// Call `symbol()` on a Soroban token contract and classify the result.
///
/// Never returns an error: every failure mode is either Absent (permanent for
/// this contract) or Transient (retry next run), which is exactly the decision
/// the caller has to make.
pub async fn resolve_symbol(http: &reqwest::Client, rpc_url: &str, contract: &str) -> Outcome {
    let Some(envelope) = build_simulate_envelope(contract, "symbol") else {
        tracing::warn!(contract, "not a well-formed contract address; sentinelling");
        return Outcome::Absent;
    };
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateTransaction",
        "params": { "transaction": envelope },
    });

    // A transport-level failure — connection, timeout, non-2xx (a 429 or 5xx
    // from the public endpoint lands here), unparseable body — is systemic, not
    // a fact about this contract. `error_for_status` is what keeps a
    // rate-limited run from sentinelling every contract it touches.
    let resp = match http
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(contract, error = %err, "symbol() rpc failed; retrying next run");
            return Outcome::Transient;
        }
    };
    let resp: RpcResponse = match resp.json().await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(contract, error = %err, "symbol() rpc body unparseable; retrying next run");
            return Outcome::Transient;
        }
    };

    // Everything below is a fact about the contract: it simulated, and did not
    // give back a usable symbol. Sentinel so it stops leading the queue.
    let Some(result) = resp.result else {
        return Outcome::Absent;
    };
    if let Some(err) = result.error {
        tracing::debug!(contract, error = %err, "symbol() simulate returned error");
        return Outcome::Absent;
    }
    let decoded = result
        .results
        .into_iter()
        .next()
        .and_then(|r| base64::engine::general_purpose::STANDARD.decode(r.xdr).ok())
        .and_then(|raw| ScVal::from_xdr(&raw, Limits::none()).ok())
        .as_ref()
        .and_then(scval_to_string)
        .as_deref()
        .and_then(sanitize_symbol);

    match decoded {
        Some(s) => Outcome::Symbol(s),
        None => Outcome::Absent,
    }
}

/// Soroban contracts in `prices.assets` with no `prices.asset_symbol` row yet.
///
/// `NOT IN` over a subquery rather than a `LEFT JOIN` so the result cannot
/// depend on the session's `join_use_nulls`; no `FINAL` on either side, since
/// both are read only for set membership on a sort-key column, where a
/// ReplacingMergeTree's un-merged duplicates are indistinguishable. `DISTINCT`
/// matters: 10 of the 52 Soroban rows share an `asset_id` with another row
/// (task 0139), and without it those would produce duplicate RPC calls.
pub async fn load_unresolved_contracts(
    client: &Client,
    limit: usize,
) -> Result<Vec<String>, clickhouse::error::Error> {
    client
        .query(
            "SELECT DISTINCT contract_address \
             FROM prices.assets \
             WHERE contract_address != '' \
               AND contract_address NOT IN ( \
                   SELECT contract_address FROM prices.asset_symbol \
               ) \
             ORDER BY contract_address \
             LIMIT ?",
        )
        .bind(limit as u64)
        .fetch_all::<String>()
        .await
}

/// Write resolved symbols and sentinels into `prices.asset_symbol`.
///
/// ⚠️ Typed insert, deliberately — **not** `format!`-ed SQL the way
/// `supply-worker::write_supplies` builds its VALUES list. That worker formats
/// already-parsed `Decimal`s, which carry no injection surface; a symbol is an
/// attacker-influenced string read off-chain.
async fn write_symbols(
    client: &Client,
    rows: &[SymbolRow],
) -> Result<(), clickhouse::error::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut insert = client.insert("prices.asset_symbol")?;
    for row in rows {
        insert.write(row).await?;
    }
    insert.end().await
}

/// Resolve `symbol()` for every Soroban contract that has no `asset_symbol` row,
/// up to [`MAX_CONTRACTS_PER_RUN`], and persist the results.
///
/// Only a ClickHouse failure fails the run: an RPC problem is per-contract and
/// costs at most a run's worth of progress. Writes happen once at the end rather
/// than in flushed batches (`supply-worker`'s shape) because the batch is bounded
/// at 25 rows — a mid-run Lambda kill loses one run, and absence-triggering means
/// the next run picks up exactly where this one stopped.
pub async fn run_symbols(
    ch: &Client,
    http: &reqwest::Client,
    rpc_url: &str,
    limit: usize,
) -> Result<SymbolStats, clickhouse::error::Error> {
    let contracts = load_unresolved_contracts(ch, limit).await?;
    let mut stats = SymbolStats {
        considered: contracts.len(),
        ..Default::default()
    };
    let mut rows: Vec<SymbolRow> = Vec::new();

    for contract in &contracts {
        match resolve_symbol(http, rpc_url, contract).await {
            Outcome::Symbol(symbol) => {
                tracing::debug!(contract, symbol, "resolved soroban symbol");
                rows.push(SymbolRow {
                    contract_address: contract.clone(),
                    symbol,
                });
                stats.resolved += 1;
            }
            Outcome::Absent => {
                // Sentinel: an empty symbol records "asked, no usable answer",
                // which is what stops this contract being re-polled every hour.
                rows.push(SymbolRow {
                    contract_address: contract.clone(),
                    symbol: String::new(),
                });
                stats.absent += 1;
            }
            Outcome::Transient => stats.skipped += 1,
        }
    }

    write_symbols(ch, &rows).await?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_plain_symbol() {
        assert_eq!(sanitize_symbol("SolvBTC").as_deref(), Some("SolvBTC"));
        assert_eq!(sanitize_symbol("  XAUM \n").as_deref(), Some("XAUM"));
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert_eq!(sanitize_symbol(""), None);
        assert_eq!(sanitize_symbol("   \t\n "), None);
    }

    #[test]
    fn rejects_over_the_length_cap() {
        // `ScVal::String` is unbounded in XDR, so this arm is the one that can
        // hand us a megabyte of text.
        assert!(sanitize_symbol(&"A".repeat(MAX_SYMBOL_LEN)).is_some());
        assert_eq!(sanitize_symbol(&"A".repeat(MAX_SYMBOL_LEN + 1)), None);
    }

    #[test]
    fn rejects_embedded_control_characters() {
        // Trimming alone would let an interior newline through, and this string
        // reaches a JSON response and operators' terminals.
        assert_eq!(sanitize_symbol("US\nDC"), None);
        assert_eq!(sanitize_symbol("US\u{0}DC"), None);
        assert_eq!(sanitize_symbol("\u{1b}[31mUSDC"), None);
    }

    #[test]
    fn counts_characters_not_bytes_for_the_cap() {
        // 32 multi-byte characters is a legitimate symbol; a byte-length cap
        // would reject it.
        let s = "ż".repeat(MAX_SYMBOL_LEN);
        assert_eq!(s.len(), MAX_SYMBOL_LEN * 2);
        assert!(sanitize_symbol(&s).is_some());
    }

    #[test]
    fn reads_both_symbol_and_string_scval_arms() {
        let sym = ScVal::Symbol(ScSymbol("SolvBTC".try_into().unwrap()));
        assert_eq!(scval_to_string(&sym).as_deref(), Some("SolvBTC"));

        let string = ScVal::String(ScString("SolvBTC".try_into().unwrap()));
        assert_eq!(scval_to_string(&string).as_deref(), Some("SolvBTC"));

        assert_eq!(scval_to_string(&ScVal::U32(7)), None);
    }

    #[test]
    fn builds_an_envelope_that_decodes_back_to_symbol_on_the_contract() {
        // A locally-derived valid C-strkey: self-contained, no network, and it
        // cannot rot the way a pasted mainnet address can.
        let contract = stellar_strkey::Contract([7u8; 32]).to_string();
        let b64 = build_simulate_envelope(&contract, "symbol").expect("valid contract address");
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .expect("valid base64");
        let env = TransactionEnvelope::from_xdr(&raw, Limits::none()).expect("valid xdr");
        let TransactionEnvelope::Tx(v1) = env else {
            panic!("expected a v1 envelope")
        };
        let OperationBody::InvokeHostFunction(op) = &v1.tx.operations[0].body else {
            panic!("expected InvokeHostFunction")
        };
        let HostFunction::InvokeContract(args) = &op.host_function else {
            panic!("expected InvokeContract")
        };
        assert_eq!(args.function_name.0.to_utf8_string().unwrap(), "symbol");
        assert_eq!(args.args.len(), 0, "symbol() takes no arguments");
        assert_eq!(
            args.contract_address,
            ScAddress::Contract(ContractId(Hash([7u8; 32])))
        );
    }

    #[test]
    fn rejects_a_malformed_contract_address() {
        assert_eq!(build_simulate_envelope("not-a-contract", "symbol"), None);
        assert_eq!(build_simulate_envelope("", "symbol"), None);
    }
}
