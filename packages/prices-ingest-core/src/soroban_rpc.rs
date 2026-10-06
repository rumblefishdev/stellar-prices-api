//! Read-only calls to a Soroban token contract over RPC `simulateTransaction`.
//!
//! Two readers share this: asset-discovery's `symbol()` stage (task 0210) and
//! the ingest's `decimals()` lookup (task 0329). Both need the same envelope and
//! the same answer to one question — did the simulation actually evaluate the
//! contract? — so that boundary lives here once rather than in each caller.

use base64::Engine;
use stellar_xdr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Limits, Memo,
    MuxedAccount, Operation, OperationBody, Preconditions, ReadXdr, ScAddress, ScSymbol, ScVal,
    SequenceNumber, Transaction, TransactionEnvelope, TransactionExt, TransactionV1Envelope,
    Uint256, VecM, WriteXdr,
};

/// Default public Soroban RPC endpoint (overridable via `SOROBAN_RPC_URL`).
/// Needs no auth — same endpoint and same reasoning as `oracle-worker`.
pub const DEFAULT_SOROBAN_RPC: &str = "https://mainnet.sorobanrpc.com";

/// Per-request RPC timeout. Each caller's time bound is this times the number
/// of calls it allows itself per run.
pub const RPC_TIMEOUT_SECS: u64 = 5;

/// `SOROBAN_RPC_URL`, or [`DEFAULT_SOROBAN_RPC`].
pub fn rpc_url_from_env() -> String {
    std::env::var("SOROBAN_RPC_URL").unwrap_or_else(|_| DEFAULT_SOROBAN_RPC.to_string())
}

/// An HTTP client bounded by [`RPC_TIMEOUT_SECS`].
///
/// # Panics
///
/// If the client cannot be built — in practice a TLS backend that failed to
/// initialise, which is an environment fault at cold start, identical on every
/// invocation. Deliberately not a silent fallback to `Client::default()`: that
/// client carries **no timeout**, so every caller's time bound would quietly
/// become unbounded. A loud failure at startup beats an outage with no obvious
/// cause.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(RPC_TIMEOUT_SECS))
        .build()
        .expect("build an HTTP client with a timeout")
}

/// Base64 `TransactionEnvelope` invoking the zero-argument `func()` on
/// `contract`, for a read-only `simulateTransaction` (source account / fee / seq
/// are not checked by simulation).
///
/// `None` when `contract` is not a well-formed C-strkey.
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

/// What one simulated call produced.
///
/// The Absent/Transient split is load-bearing for every caller: Absent is a
/// fact about the contract that will repeat, Transient means we learned
/// nothing and must ask again.
#[derive(Debug, PartialEq)]
pub enum Simulated {
    /// The contract evaluated the call and returned this value.
    Value(ScVal),
    /// The contract evaluated the call and errored, or returned undecodable
    /// bytes, or the address is not a contract at all. Repeats on every call.
    Absent,
    /// No answer about the contract: transport failure, JSON-RPC error,
    /// archived state, an empty body. Ask again later.
    Transient,
}

#[derive(serde::Deserialize)]
struct RpcResponse {
    result: Option<RpcResult>,
    /// JSON-RPC 2.0 reports failure *here*, with HTTP 200 — which is how public
    /// endpoints signal quota exhaustion, a disabled method, and a node that is
    /// not synced. Without this field such a body deserialises to
    /// `result: None` and would read as a fact about the contract.
    #[serde(default)]
    error: Option<serde_json::Value>,
}
#[derive(serde::Deserialize)]
struct RpcResult {
    #[serde(default)]
    results: Vec<RpcInvokeResult>,
    #[serde(default)]
    error: Option<String>,
    /// Set when the contract's instance or Wasm is archived but restorable —
    /// a live contract, not one that lacks the function.
    #[serde(default, rename = "restorePreamble")]
    restore_preamble: Option<serde_json::Value>,
}
#[derive(serde::Deserialize)]
struct RpcInvokeResult {
    xdr: String,
}

/// Call the zero-argument `func()` on `contract` by simulation and classify
/// the result. Never returns an error: every failure is Absent or Transient,
/// which is exactly the decision a caller has to make.
pub async fn simulate(
    http: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
    func: &str,
) -> Simulated {
    let Some(envelope) = build_simulate_envelope(contract, func) else {
        tracing::warn!(contract, func, "not a well-formed contract address");
        return Simulated::Absent;
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
    // rate-limited run from recording every contract it touches as Absent.
    let resp = match http
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(contract, func, error = %err, "simulate rpc failed");
            return Simulated::Transient;
        }
    };
    match resp.json::<RpcResponse>().await {
        Ok(r) => classify(r, contract, func),
        Err(err) => {
            tracing::warn!(contract, func, error = %err, "simulate rpc body unparseable");
            Simulated::Transient
        }
    }
}

/// Decide what a parsed simulate response means for this contract.
///
/// The line between the two failure arms is **whether the simulation actually
/// evaluated the contract**. If it did, its outcome is a fact that will repeat,
/// so it is Absent. If we never got that far — a JSON-RPC error, archived
/// state, an empty body — we learned nothing, so it is Transient. A wrong
/// Transient costs a repeated call; a wrong Absent can make a caller give up
/// on a contract for good.
fn classify(resp: RpcResponse, contract: &str, func: &str) -> Simulated {
    // Arrives with HTTP 200, so `error_for_status` cannot see it.
    if let Some(err) = resp.error {
        tracing::warn!(contract, func, error = %err, "simulate returned a json-rpc error");
        return Simulated::Transient;
    }
    let Some(result) = resp.result else {
        tracing::warn!(contract, func, "simulate returned neither result nor error");
        return Simulated::Transient;
    };
    if result.restore_preamble.is_some() {
        tracing::warn!(contract, func, "simulate needs a state restore");
        return Simulated::Transient;
    }
    // The simulation ran and the contract itself errored — deterministic, so a
    // fact. This is the one arm that stays permanent.
    if let Some(err) = result.error {
        tracing::debug!(contract, func, error = %err, "contract errored in simulation");
        return Simulated::Absent;
    }
    let Some(first) = result.results.into_iter().next() else {
        tracing::warn!(contract, func, "simulated without a result or an error");
        return Simulated::Transient;
    };
    match base64::engine::general_purpose::STANDARD
        .decode(first.xdr)
        .ok()
        .and_then(|bytes| ScVal::from_xdr(&bytes, Limits::none()).ok())
    {
        Some(v) => Simulated::Value(v),
        // It answered, and the answer does not decode. A fact, and otherwise an
        // invisible one, so name the contract.
        None => {
            tracing::warn!(contract, func, "simulate result is not a decodable ScVal");
            Simulated::Absent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_json(body: &str) -> Simulated {
        classify(
            serde_json::from_str(body).expect("test body parses"),
            "C_TEST",
            "symbol",
        )
    }

    #[test]
    fn a_json_rpc_error_retries_instead_of_being_absent() {
        // JSON-RPC reports this with HTTP 200, so `error_for_status` cannot see
        // it. Reading it as Absent would turn an RPC outage into a fact about
        // every contract the run touched.
        let out = classify_json(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not enabled"}}"#,
        );
        assert_eq!(out, Simulated::Transient);
    }

    #[test]
    fn an_empty_or_archived_response_retries() {
        // Neither result nor error: we learned nothing about the contract.
        assert_eq!(
            classify_json(r#"{"jsonrpc":"2.0","id":1}"#),
            Simulated::Transient
        );
        // Archived-but-restorable state is a live contract.
        assert_eq!(
            classify_json(r#"{"result":{"results":[],"restorePreamble":{"minResourceFee":"1"}}}"#),
            Simulated::Transient,
        );
        // Simulated clean, returned nothing at all — not a fact either.
        assert_eq!(
            classify_json(r#"{"result":{"results":[]}}"#),
            Simulated::Transient
        );
    }

    #[test]
    fn a_contract_error_is_the_one_permanent_arm() {
        assert_eq!(
            classify_json(r#"{"result":{"results":[],"error":"HostError: missing symbol"}}"#),
            Simulated::Absent,
        );
    }

    #[test]
    fn a_returned_value_is_decoded() {
        let xdr = base64::engine::general_purpose::STANDARD
            .encode(ScVal::U32(8).to_xdr(Limits::none()).unwrap());
        let body = format!(r#"{{"result":{{"results":[{{"xdr":"{xdr}"}}]}}}}"#);
        assert_eq!(classify_json(&body), Simulated::Value(ScVal::U32(8)));
        // Bytes that are not an ScVal: it answered, with garbage.
        assert_eq!(
            classify_json(r#"{"result":{"results":[{"xdr":"AAAA//8="}]}}"#),
            Simulated::Absent
        );
    }

    #[test]
    fn builds_an_envelope_that_decodes_back_to_the_call() {
        // A locally-derived valid C-strkey: self-contained, no network, and it
        // cannot rot the way a pasted mainnet address can.
        let contract = stellar_strkey::Contract([7u8; 32]).to_string();
        let b64 = build_simulate_envelope(&contract, "decimals").expect("valid contract address");
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
        assert_eq!(args.function_name.0.to_utf8_string().unwrap(), "decimals");
        assert_eq!(args.args.len(), 0, "the call takes no arguments");
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
