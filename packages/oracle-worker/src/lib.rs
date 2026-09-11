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

pub mod metrics;

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
/// identity by the shared [`reflector_key_to_identity`], which is the single
/// authority on what may reach `prices.oracle_prices`; a symbol it drops is
/// skipped *before* the RPC fetch. `every_tracked_symbol_resolves_to_an_identity`
/// pins the two together, so this list holds no dead entries.
///
/// ⚠️ **`USDT` was removed here by task 0172, together with its arm in the
/// mapping.** The feed named "USDT" prices Tether's own token, which is at par;
/// we were filing that reading under the Stellar IOU issued by `USDT_ISSUER`,
/// which depegged in June 2022 and trades at ~$0.13. Because the oracle tier runs
/// *before* the peg-pivot tier and wins where it applies, an oracle row on that
/// identity re-pegs every USDT-quoted candle to $1.00 — the exact error 0172
/// removed from the peg tier. USDT is now priced by measurement through the pivot
/// (its own USDC market) and needs no oracle arm. Restoring it requires fixing
/// the symbol→issuer mapping first (task 0173).
pub const TRACKED_SYMBOLS: &[&str] = &["XLM", "USDC"];

/// The `oracle_prices.oracle_name` this worker writes, and the one the task-0167
/// snapshot copies. Must match the enrichment tier's `ORACLE_NAME` (default
/// `reflector`) — the rate we snapshot has to be the rate that priced the
/// candles, or 0154's constraint-5 reconciliation compares two different things.
pub const ORACLE_NAME: &str = "reflector";

/// The identities whose oracle readings are snapshotted into `prices.usd_rate`
/// (task 0167). Deliberately a **subset** of [`TRACKED_SYMBOLS`].
///
/// ⚠️ **XLM is not here because it is not a peg**, and that is the whole of the
/// distinction: this set is named for what its members are. XLM is snapshotted
/// by [`measured_identities`] beside this fn, added by task 0228 — as
/// `method = 'oracle'`, `hops = 0`, which is what a polled reading factually is,
/// no pivot involved.
///
/// ⏳ The paragraph that stood here deferred that decision to whenever `202509`
/// aged out of a 13-month window, which was wrong on the facts and is now
/// resolved. Measured 2026-09-11: `oracle_prices` carries **no TTL** (prod
/// `engine_full`, `init.sql`); its retention is the cleanup worker's 13-month
/// policy (`cleanup-worker/src/lib.rs:33`), which is **deployed but dark**; so
/// the earliest possible loss of an XLM reading is **2027-04-11**. The deferral
/// was never as urgent as it read, and it has been decided rather than expired.
/// ⚠️ **USDT was removed from this list by task 0172, and must not be restored
/// without fixing the symbol→issuer mapping first (task 0173).** Reflector
/// publishes a feed named for the TICKER "USDT" — Tether's own token, which is
/// genuinely at par. We were storing that reading against
/// `USDT_ISSUER`'s address, i.e. asserting ~$1.00 for a Stellar IOU that has
/// traded at ~$0.13 since it depegged in June 2022 (confirmed by two independent
/// markets; see `ReferenceIds::pivot_ids`). The oracle was not wrong about
/// Tether — the identity we filed it under was wrong. An asset code is not an
/// identity on Stellar: `prices.assets` holds ~220 distinct issuers using the
/// code "USDT" and ~220 using "USDC".
///
/// Rows already written under that identity are still in `prices.usd_rate` and
/// are still wrong; cleaning them is tracked separately.
pub fn peg_identities() -> Vec<AssetIdentity> {
    // Built rather than declared const: AssetIdentity::Credit holds Strings.
    // Sourced from the same const the enrichment peg tier and views.sql use,
    // so the three cannot drift apart.
    vec![AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: prices_clickhouse::USDC_ISSUER.to_string(),
    }]
}

/// The identities whose oracle readings are snapshotted into `prices.usd_rate`
/// because they are **measured, non-peg** references (task 0228). Today: the
/// native asset, XLM.
///
/// Kept separate from [`peg_identities`] rather than folded into it, because the
/// two sets are named for what their members ARE. A peg is a claim that the
/// asset should sit at a dollar and that the reading is there to detect when it
/// does not; XLM makes no such claim. Same destination table, same
/// `method = 'oracle'`, `hops = 0`, `reference_asset = ''` — that is what a
/// polled reading factually is, whatever the asset — but a different reason for
/// membership, and a reader of either list should not have to infer which one
/// applies.
///
/// ## The identity evidence, which is the only thing that matters here
///
/// Task 0173's defect was a set whose member earned its place on an asset CODE.
/// Reflector publishes a feed named for the ticker "USDT" — Tether's own token,
/// genuinely at par — and we filed that reading against `USDT_ISSUER`'s address,
/// a Stellar IOU trading at ~$0.13. The oracle was not wrong; the identity we
/// filed it under was. `prices.assets` holds ~220 distinct issuers using the code
/// "USDT" and ~220 using "USDC". **An asset code is not an identity on Stellar.**
///
/// XLM is the one case where that failure cannot occur, for three independent
/// reasons:
///
/// 1. **The mapping is the authority, and it is unambiguous.**
///    `reflector_key_to_identity` is the single gate on what may reach
///    `prices.oracle_prices`, and it maps `"XLM" | "native"` to
///    [`AssetIdentity::Native`] — not to a code-and-issuer pair that has to be
///    matched. `measured_identities_resolve_from_the_reflector_symbol_mapping`
///    calls it, so this claim is code rather than prose (task 0267's
///    `check_identity` standard).
/// 2. **There is no issuer to mis-attribute.** `identity_columns(Native)` is
///    `("native", "XLM", "", "")` — the issuer and contract columns are empty by
///    construction, so there is no address to get wrong.
/// 3. **The native asset has exactly one identity on Stellar.** It is the
///    protocol's own asset; nobody can issue a second one under the same name,
///    which is precisely the property the "USDT" ticker lacked.
///
/// ⚠️ Adding a member here is a claim that its feed names THAT identity — not
/// merely that a code matches. Write the evidence above before changing the set;
/// [`tests::measured_identities_is_exactly_the_native_asset`] pins it so the
/// change cannot happen silently.
pub fn measured_identities() -> Vec<AssetIdentity> {
    vec![AssetIdentity::Native]
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

/// Values at or above this are **milliseconds**; below it, **seconds**.
///
/// `1e11` seconds is the year 5138 and `1e11` milliseconds is 1973 — no real
/// reading can be ambiguous between the two, so the gap either side of this
/// line is roughly three thousand years wide. It is a magnitude check, not a
/// guess: [`reflector_timestamp_to_epoch_seconds`] rejects anything that lands
/// outside the plausible window after conversion, so a value in the dead zone
/// fails loudly instead of being filed under the wrong unit.
pub const REFLECTOR_MILLIS_THRESHOLD: u64 = 100_000_000_000;

/// Earliest reading that can be real.
///
/// ⚠️ **This is `prices_ingest_core::ORACLE_EPOCH_FLOOR`, deliberately, and not
/// a floor of this module's own.** That constant (2020-01-01) already gates the
/// `usd_rate` snapshot at `writer.rs:506`. A looser floor here — Stellar genesis
/// in 2015 was the first thing tried — would admit a 2018 reading into
/// `oracle_prices`, which the snapshot then drops silently: no rejection log, no
/// row downstream, no signal anywhere. Two floors that disagree produce exactly
/// the quiet gap this task exists to remove, so there is one floor.
pub const EARLIEST_PLAUSIBLE_SECS: u64 = prices_ingest_core::writer::ORACLE_EPOCH_FLOOR as u64;

/// How far ahead of our own clock a reading may sit before it is malformed.
/// Reflector stamps the observation, not the reply, so a fresh reading is
/// normally in the recent past; an hour absorbs clock skew without admitting a
/// value that has plainly been mis-scaled.
pub const FUTURE_SKEW_SECS: u64 = 3_600;

/// Why a Reflector timestamp was refused. Carries the raw value so the log line
/// is enough to diagnose from — see [`reflector_timestamp_to_epoch_seconds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BadTimestamp {
    #[error("timestamp {raw} converts to {secs}s, before Stellar genesis")]
    BeforeGenesis { raw: u64, secs: u64 },
    #[error("timestamp {raw} converts to {secs}s, more than an hour ahead of now ({now}s)")]
    InTheFuture { raw: u64, secs: u64, now: u64 },
}

/// Convert a Reflector `PriceData.timestamp` to `oracle_prices.timestamp`
/// (epoch **seconds**), refusing anything implausible.
///
/// # 🔴 Why this is not a `/ 1000`
///
/// It was, and that is task 0227. The poll path took `lastprice`'s timestamp as
/// milliseconds and divided unconditionally, on the strength of a comment
/// claiming it matched the event-decoded path. It does not: the two arms read
/// **different fields carrying different units**. The event path takes
/// `topic[2]` of an `update` event, which really is milliseconds
/// (`prices-ingest-core` `soroban.rs:652-667`); `lastprice` returns **seconds**.
///
/// Every row the poll path ever wrote landed at 1970-01-21 — 3,264 per asset on
/// prod, 100% of that writer's output, against 48,311 correctly-stamped rows
/// from the event path with zero mixing. Nothing was lost (each corrupt row is
/// an exact `Decimal(38,14)` price twin of an event row) and nothing was priced
/// wrongly (a 1970 row wins the enrichment ASOF join and is then rejected by the
/// 300 s staleness guard, 471,087 times out of 471,087) — but the readings were
/// unusable, and silently so.
///
/// # The lesson this signature encodes
///
/// A cross-reference to another code site is not evidence about an upstream
/// payload. So this function no longer trusts a declared unit at all: it decides
/// from the **magnitude**, and refuses to write anything that is implausible
/// under either reading. A future Reflector change of unit therefore produces a
/// loud rejection, not five months of 1970.
///
/// `now_secs` is passed in rather than read here so the boundary is testable.
pub fn reflector_timestamp_to_epoch_seconds(raw: u64, now_secs: u64) -> Result<u32, BadTimestamp> {
    let secs = if raw >= REFLECTOR_MILLIS_THRESHOLD {
        raw / 1000
    } else {
        raw
    };
    if secs < EARLIEST_PLAUSIBLE_SECS {
        return Err(BadTimestamp::BeforeGenesis { raw, secs });
    }
    if secs > now_secs.saturating_add(FUTURE_SKEW_SECS) {
        return Err(BadTimestamp::InTheFuture {
            raw,
            secs,
            now: now_secs,
        });
    }
    // Unreachable while the future check holds — u32::MAX is 2106 — but the cast
    // is lossy and `oracle_prices.timestamp` is a DateTime, so clamp rather than
    // wrap if those bounds ever move.
    Ok(secs.min(u32::MAX as u64) as u32)
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
    /// Every reading that produced no row, for **any** reason: no Reflector
    /// price, a failed fetch, or a timestamp refused by the 0227 guard. A
    /// superset of [`Self::timestamp_rejected`], not a disjoint bucket.
    pub skipped: usize,
    /// Readings refused by [`reflector_timestamp_to_epoch_seconds`] — a subset
    /// of [`Self::skipped`], counted apart because it is the one skip reason
    /// that means *the feed's contract changed under us* rather than *a fetch
    /// went wrong*. Buried in the general skip total, a Reflector unit change
    /// arrives as a handful of extra skips and looks like nothing; on its own it
    /// is a step off a flat zero (task 0231).
    pub timestamp_rejected: usize,
    /// Rows written into `prices.usd_rate` by this pass for the PEG set,
    /// [`peg_identities`] — canonical USDC (task 0167). Zero is normal on a
    /// steady-state run — the snapshot only copies observations it does not
    /// already hold. It is NOT normal for it to be zero forever while `written`
    /// keeps climbing; see [`run_oracle`] on why that needs a signal.
    ///
    /// ⚠️ Peg set ONLY, deliberately not a total. Since task 0228 the enrichment
    /// pivot's only post-epoch rate is USDC's `oracle` rows, and the snapshot is
    /// non-fatal — its failure is visible nowhere but here. The measured set
    /// lands rows on every pass, so a sum would never read zero and would hide
    /// exactly the stall this series exists to show (0228 review finding 1).
    pub rates_snapshotted: u64,
    /// Rows written into `prices.usd_rate` by this pass for the MEASURED set,
    /// [`measured_identities`] — the native asset (task 0228). Its own series,
    /// for the reason on [`Self::rates_snapshotted`].
    pub measured_rates_snapshotted: u64,
}

/// A pass that failed, carrying the counts it had already measured.
///
/// Exists because the rejections happen in the per-symbol loop, **ahead** of
/// the ClickHouse writes that are the likely reason a pass dies. So a pass can
/// genuinely refuse a Reflector reading and then fail on `write_oracle`, and a
/// bare error would drop the one signal task 0231 exists to produce — on
/// exactly the invocation that had something to say.
#[derive(Debug)]
pub struct OracleFailure {
    pub error: OracleError,
    /// Readings refused by [`reflector_timestamp_to_epoch_seconds`] before the
    /// failure. Zero when the pass died before the loop ran at all.
    pub timestamp_rejected: usize,
}

impl std::fmt::Display for OracleFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for OracleFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Poll Reflector for each tracked symbol and write the prices to
/// `oracle_prices`. Best-effort: per-symbol failures are skipped; only a CH
/// write failure fails the run.
pub async fn run_oracle(
    writer: &OhlcvWriter,
    http: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
) -> Result<OracleStats, OracleFailure> {
    // The count lives out here so a failure inside can still report it. The
    // inner function keeps `OracleError` as its error type, so every `?` on a
    // ClickHouse call converts as before.
    let mut timestamp_rejected = 0usize;
    run_oracle_inner(writer, http, rpc_url, contract, &mut timestamp_rejected)
        .await
        .map_err(|error| OracleFailure {
            error,
            timestamp_rejected,
        })
}

async fn run_oracle_inner(
    writer: &OhlcvWriter,
    http: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
    timestamp_rejected: &mut usize,
) -> Result<OracleStats, OracleError> {
    let existing = writer.load_assets().await?;
    let mut registry = AssetRegistry::from_existing(existing);
    let known_before = registry.assets().count();
    let mut samples = Vec::new();
    let mut skipped = 0usize;
    // Read once per pass, not per symbol: the plausibility window must not move
    // underneath a batch, or the same reading could be accepted for one symbol
    // and refused for the next.
    //
    // On the impossible branch (a system clock before 1970) the future bound is
    // disabled rather than made infinitely strict: the guard exists to catch a
    // unit mistake, which the genesis bound and the magnitude threshold already
    // catch, and a broken clock must not silently stop the feed.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);

    for &symbol in TRACKED_SYMBOLS {
        // Filter out any polled symbol with no Stellar identity (shared with the
        // event-decode path, so the two oracle arms map symbols identically).
        let Some(identity) = reflector_key_to_identity(symbol) else {
            continue;
        };
        match fetch_lastprice(http, rpc_url, contract, symbol).await {
            Ok(Some(pd)) => {
                // ⚠️ `lastprice` reports SECONDS — see
                // `reflector_timestamp_to_epoch_seconds`, which decides from the
                // magnitude rather than from any declared unit. The previous
                // version of this line divided by 1000 unconditionally, on the
                // strength of a comment about the event-decoded path, and so
                // wrote every one of its readings to 1970-01-21 (task 0227).
                let timestamp = match reflector_timestamp_to_epoch_seconds(pd.timestamp, now_secs) {
                    Ok(ts) => ts,
                    Err(err) => {
                        // Loud, and NOT written. A rejected reading costs one
                        // 5-minute sample; a written one is indistinguishable
                        // from a real observation for as long as it is stored.
                        //
                        // Counted twice on purpose: in the general skip total,
                        // and again on its own so the dedicated
                        // `OracleTimestampRejected` alarm (task 0231) sees a
                        // unit change instead of a rounding error in the skips.
                        skipped += 1;
                        *timestamp_rejected += 1;
                        tracing::error!(
                            symbol,
                            raw = pd.timestamp,
                            error = %err,
                            "reflector timestamp rejected; row not written"
                        );
                        continue;
                    }
                };
                let asset_id = registry.get_or_assign(&identity);
                samples.push(OracleSample {
                    timestamp,
                    asset_id,
                    oracle_name: ORACLE_NAME.to_string(),
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
    // ⚠️ TWO CALLS, not one list (task 0228). `populate_usd_rate_from_oracle`
    // runs its task-0139 identity guard as a PRE-PASS over the whole slice and
    // returns before writing anything for ANY identity if one of them fails
    // (`writer.rs`, pinned by `a_collision_on_one_peg_writes_nothing_for_any_peg`).
    // Appending XLM to the peg call would therefore let a collision on XLM's
    // asset_id silently stop USDC's snapshot as well — the two sets have nothing
    // to do with each other, and one's data condition must not cost the other its
    // rows. Each call gets its own non-fatal arm and its own per-identity log.
    let snapshot = async |set: Vec<AssetIdentity>, label: &'static str| match writer
        .populate_usd_rate_from_oracle(&set, ORACLE_NAME)
        .await
    {
        Ok(stats) => {
            // Logged per-identity: a `max()` across identities would report a
            // healthy frontier while one of them sat stalled at zero.
            tracing::info!(
                set = label,
                identities = stats.identities,
                rows = stats.rows_inserted,
                newest = ?stats.newest,
                "usd_rate snapshot"
            );
            stats.rows_inserted
        }
        Err(err) => {
            tracing::error!(
                set = label,
                error = %err,
                "usd_rate snapshot failed; oracle_prices is unaffected"
            );
            0
        }
    };
    // Counted PER SET, never summed: `OracleUsdRatesSnapshotted` keeps its
    // pre-0228 meaning (the peg set, i.e. canonical USDC) so a stalled USDC
    // snapshot still reads zero, and XLM gets `OracleMeasuredRatesSnapshotted`
    // (0228 review finding 1 — a sum hid the stall the series is for).
    let rates_snapshotted = snapshot(peg_identities(), "peg").await;
    let measured_rates_snapshotted = snapshot(measured_identities(), "measured").await;

    Ok(OracleStats {
        queried: TRACKED_SYMBOLS.len(),
        written,
        skipped,
        timestamp_rejected: *timestamp_rejected,
        rates_snapshotted,
        measured_rates_snapshotted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plausible "now" for the timestamp tests: 2026-08-19T12:00:00Z.
    ///
    /// ⚠️ This comment said 2026-08-27 until it was checked against
    /// `date -u -d @1787140800`. In a change whose entire root cause is a
    /// comment about a timestamp that was trusted and wrong, an unverified
    /// timestamp comment is not a small thing.
    const NOW: u64 = 1_787_140_800;

    /// 🔑 Task 0227, the defect itself. `lastprice` reports **seconds**, and a
    /// seconds reading must survive untouched.
    ///
    /// The old code divided by 1000 unconditionally, so this exact input became
    /// 1,787,140 — 1970-01-21. That was 100% of the poll path's output for at
    /// least five months.
    #[test]
    fn a_seconds_reading_is_taken_as_seconds() {
        let ts = reflector_timestamp_to_epoch_seconds(NOW, NOW).unwrap();
        assert_eq!(ts, NOW as u32, "a seconds reading must not be divided");
    }

    /// The other unit still converts, so the function is safe to point at the
    /// event path's `topic[2]` too, and safe if Reflector ever changes.
    #[test]
    fn a_millis_reading_is_converted() {
        let ts = reflector_timestamp_to_epoch_seconds(NOW * 1000, NOW).unwrap();
        assert_eq!(ts, NOW as u32);
    }

    /// 🔑 The boundary is **uninhabitable**, and that is the property worth
    /// pinning — not where exactly it sits.
    ///
    /// A value just below the threshold is the year 5138 read as seconds; at the
    /// threshold it is 1973 read as millis. So neither side of the line can be
    /// accepted, and the exact placement of `REFLECTOR_MILLIS_THRESHOLD` is not
    /// load-bearing: any threshold inside the ~3,000-year gap behaves
    /// identically. The unit decision itself is pinned by the two tests above,
    /// on values that are real under exactly one reading.
    ///
    /// ⚠️ A first version of this test asserted the two sides were *accepted*
    /// with different units. It failed — correctly — which is how the dead zone
    /// got documented instead of assumed.
    #[test]
    fn the_threshold_sits_in_a_dead_zone_where_neither_unit_is_plausible() {
        assert!(
            matches!(
                reflector_timestamp_to_epoch_seconds(REFLECTOR_MILLIS_THRESHOLD - 1, NOW),
                Err(BadTimestamp::InTheFuture { .. })
            ),
            "just below the threshold, read as seconds, is the year 5138"
        );
        assert!(
            matches!(
                reflector_timestamp_to_epoch_seconds(REFLECTOR_MILLIS_THRESHOLD, NOW),
                Err(BadTimestamp::BeforeGenesis { .. })
            ),
            "at the threshold, read as millis, is 1973"
        );
    }

    /// A malformed reading is REFUSED, not written — the criterion 0227 adds
    /// after five months of rows that looked like data.
    ///
    /// Both arms matter and they fail differently: a seconds value read as
    /// millis lands in 1970 (too small), and a millis value read as seconds
    /// lands in the far future. Neither can reach `oracle_prices` now.
    #[test]
    fn an_implausible_reading_is_rejected_loudly() {
        // What the 0227 bug produced: a real reading already divided once.
        let already_divided = NOW / 1000;
        assert!(matches!(
            reflector_timestamp_to_epoch_seconds(already_divided, NOW),
            Err(BadTimestamp::BeforeGenesis { .. })
        ));

        // A value in the dead zone below the threshold — the shape a unit
        // change would produce. Year 5138 if believed as seconds.
        let too_far_ahead = REFLECTOR_MILLIS_THRESHOLD - 1;
        assert!(matches!(
            reflector_timestamp_to_epoch_seconds(too_far_ahead, NOW),
            Err(BadTimestamp::InTheFuture { .. })
        ));

        // Zero — the value a missing field decodes to in more languages than
        // one, and the one most likely to arrive by accident.
        assert!(reflector_timestamp_to_epoch_seconds(0, NOW).is_err());
    }

    /// Clock skew is tolerated; a mis-scaled value is not. The boundary between
    /// those two is what `FUTURE_SKEW_SECS` buys, so pin it.
    #[test]
    fn a_reading_slightly_ahead_of_our_clock_is_accepted() {
        assert!(reflector_timestamp_to_epoch_seconds(NOW + FUTURE_SKEW_SECS, NOW).is_ok());
        assert!(reflector_timestamp_to_epoch_seconds(NOW + FUTURE_SKEW_SECS + 1, NOW).is_err());
    }

    /// The floor here and the floor the `usd_rate` snapshot enforces must be
    /// the SAME value, or the gap between them is a silent hole: a reading in
    /// it is written to `oracle_prices`, then dropped by the copy at
    /// `writer.rs:506` with no log and no row — undiscoverable except by
    /// noticing an absence.
    ///
    /// A first version of this guard used Stellar genesis (2015-09-30) and
    /// opened exactly that 4.3-year window.
    #[test]
    fn the_plausibility_floor_is_the_one_the_usd_rate_snapshot_uses() {
        assert_eq!(
            EARLIEST_PLAUSIBLE_SECS,
            prices_ingest_core::writer::ORACLE_EPOCH_FLOOR as u64
        );
        // 2017-07-14 — inside the window the looser floor would have admitted.
        assert!(
            reflector_timestamp_to_epoch_seconds(1_500_000_000, NOW).is_err(),
            "a reading the snapshot would silently drop must be refused here"
        );
    }

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

    /// Task 0196. The peg set is an **allowlist**, and every member needs a
    /// documented reason the feed's ticker actually names *that issuer* — not
    /// merely an asset code that matches. This test pins the set to an exact
    /// literal so adding a member cannot happen silently: it forces a test edit,
    /// which forces someone to write the justification down.
    ///
    /// The failure this guards against is not hypothetical. `USDT` sat here on
    /// the strength of its code alone; Reflector's "USDT" feed prices Tether's
    /// own token, while `USDT_ISSUER` is a Stellar IOU that depegged in June
    /// 2022 and trades at ~$0.13. That mis-attribution reached `usd_rate` and
    /// `oracle_prices` and took 90,741 deleted rows to undo (task 0196).
    ///
    /// **Basis for the one current member:** Reflector's `USDC` feed and
    /// canonical Circle USDC agree — the snapshotted rate measured 1.000086 to
    /// 1.000639 over 2026-03 → 2026-08, i.e. par, which is what real Circle USDC
    /// does. Before adding anything here, establish the same for it (task 0173).
    #[test]
    fn peg_identities_is_exactly_canonical_usdc() {
        let ids = peg_identities();
        assert_eq!(
            ids,
            vec![AssetIdentity::Credit {
                code: "USDC".to_string(),
                issuer: prices_clickhouse::USDC_ISSUER.to_string(),
            }],
            "the peg set must be exactly canonical USDC. Adding a member is a \
             claim that its oracle feed names that ISSUER, not just that code — \
             write the evidence in the doc comment above before changing this."
        );
    }

    /// Task 0228, the sibling of the test above and for the same reason: the
    /// measured set is an ALLOWLIST, pinned to an exact literal so a member
    /// cannot be added silently. Adding one forces a test edit, which forces
    /// someone to write the justification down first.
    ///
    /// **Basis for the one current member:** Reflector's `XLM` feed resolves
    /// through `reflector_key_to_identity` to the NATIVE asset (pinned by the
    /// test below), which has no issuer to mis-attribute and exactly one identity
    /// on Stellar. That is precisely the property task 0173's `USDT` entry
    /// lacked, and it is the whole argument — see the doc comment on
    /// [`measured_identities`].
    #[test]
    fn measured_identities_is_exactly_the_native_asset() {
        assert_eq!(
            measured_identities(),
            vec![AssetIdentity::Native],
            "the measured set must be exactly the native asset. Adding a member \
             is a claim that its oracle feed names THAT identity, not just that \
             an asset code matches — write the evidence in the doc comment above \
             before changing this."
        );
    }

    /// 🔑 THE IDENTITY CLAIM, AS CODE. `measured_identities`'s doc says Reflector's
    /// `XLM` symbol is the native asset; this calls the mapping that decides it,
    /// so the claim is checked rather than asserted in prose (task 0267's
    /// `check_identity` standard).
    ///
    /// `reflector_key_to_identity` is the single authority on what may reach
    /// `prices.oracle_prices`, so if it ever stopped mapping `XLM` to `Native` —
    /// or started mapping it to a code-and-issuer pair — the rows this set
    /// snapshots would be filed under a different identity than the one named
    /// here, and nothing else in the pipeline would notice.
    #[test]
    fn measured_identities_resolve_from_the_reflector_symbol_mapping() {
        assert_eq!(
            reflector_key_to_identity("XLM"),
            Some(AssetIdentity::Native),
            "Reflector's XLM feed must resolve to the native asset"
        );
        assert_eq!(
            reflector_key_to_identity("native"),
            Some(AssetIdentity::Native),
            "the mapping's other spelling of the same identity"
        );
        // Every member of the set is reachable from a symbol this worker polls —
        // a member no feed can produce would snapshot nothing, forever, quietly.
        for identity in measured_identities() {
            assert!(
                TRACKED_SYMBOLS
                    .iter()
                    .any(|s| reflector_key_to_identity(s) == Some(identity.clone())),
                "{identity:?} is in the measured set but no tracked symbol resolves to it"
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
