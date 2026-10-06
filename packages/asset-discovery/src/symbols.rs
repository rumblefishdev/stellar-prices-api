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

use clickhouse::Client;
use prices_ingest_core::soroban_rpc::{Simulated, simulate};
// Re-exported so `main.rs` and the RPC integration test keep their paths; the
// envelope and the Absent/Transient boundary moved to `prices-ingest-core`
// (task 0329), which the ingest's `decimals()` lookup shares.
pub use prices_ingest_core::soroban_rpc::{
    DEFAULT_SOROBAN_RPC, RPC_TIMEOUT_SECS, build_simulate_envelope, http_client,
};
use serde::Serialize;
use stellar_xdr::{ScString, ScSymbol, ScVal};

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
/// The Absent/Transient split is load-bearing, and neither arm is a sentinel on
/// its own. Absent records one negative answer and increments `attempts`;
/// Transient writes nothing at all. A contract is only sentinelled once it has
/// answered negatively [`MAX_SYMBOL_ATTEMPTS`] times in a row, because the one
/// thing we cannot do is publish an empty symbol that nothing re-polls.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A usable symbol.
    Symbol(String),
    /// Asked, and got a negative answer. Counts against [`MAX_SYMBOL_ATTEMPTS`].
    Absent,
    /// Never got an answer at all. Leave it for the next run, uncounted.
    Transient,
}

/// Consecutive negative answers before a contract is sentinelled for good.
///
/// Three rather than one because the simulation's `error` field cannot
/// distinguish "this contract has no `symbol()`" from "this node failed to read
/// a ledger entry". A deterministic failure exhausts the counter in three runs;
/// a transient one resolves and resets it. See the `asset_symbol` DDL comment.
pub const MAX_SYMBOL_ATTEMPTS: u8 = 3;

#[derive(Debug, Serialize, clickhouse::Row)]
struct SymbolRow {
    contract_address: String,
    symbol: String,
    /// 0 on success. On a negative answer, the prior count plus one.
    attempts: u8,
}

/// Outcome of a [`run_symbols`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SymbolStats {
    /// Unresolved contracts this run picked up.
    pub considered: usize,
    /// Contracts that yielded a usable symbol.
    pub resolved: usize,
    /// Contracts sentinelled for good this run — a negative answer that took
    /// `attempts` to [`MAX_SYMBOL_ATTEMPTS`]. Not every negative answer: the
    /// earlier ones are still retryable and count as `skipped`.
    pub absent: usize,
    /// Contracts coming back next run, either because nothing answered
    /// (transient) or because a negative answer has not yet exhausted the count.
    pub skipped: usize,
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
/// A length and character-class floor only. `symbol()` is a string the contract
/// itself controls, so this bounds what we store and print — it does **not**
/// establish that the token is what it claims to be. A hostile contract
/// returning `"USDC"` passes this check, and so does one returning a Cyrillic
/// homoglyph of it. Asset identity verification (SEP-1 `home_domain`, a
/// verification flag on the response) is task 0252; this task deliberately keeps
/// the blast radius to display only, leaving `?search=` and `sort=code` on the
/// raw `assets.asset_code` column.
///
/// The class is a positive one because the complement cannot be written with
/// `char::is_control`, which matches Unicode category Cc alone: `U+202E`
/// (right-to-left override), `U+200B` and `U+FEFF` are Cf and would pass,
/// letting a contract reorder or hide text in every consumer UI and in operator
/// terminals.
// ponytail: an allow-list of alphanumerics plus the punctuation real symbols
// use. Widen it if a legitimate token is ever rejected — the log line names the
// contract.
pub fn sanitize_symbol(raw: &str) -> Option<String> {
    let s = raw.trim();
    let printable = |c: char| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | '+' | ':' | ' ');
    (!s.is_empty() && s.chars().count() <= MAX_SYMBOL_LEN && s.chars().all(printable))
        .then(|| s.to_string())
}

/// Call `symbol()` on a Soroban token contract and classify the result.
///
/// Never returns an error: every failure mode is either Absent (permanent for
/// this contract) or Transient (retry next run), which is exactly the decision
/// the caller has to make. The line between the two is
/// [`prices_ingest_core::soroban_rpc::simulate`]'s.
pub async fn resolve_symbol(http: &reqwest::Client, rpc_url: &str, contract: &str) -> Outcome {
    match simulate(http, rpc_url, contract, "symbol").await {
        Simulated::Value(v) => symbol_outcome(&v, contract),
        Simulated::Absent => Outcome::Absent,
        Simulated::Transient => Outcome::Transient,
    }
}

/// A `symbol()` return value as an [`Outcome`].
fn symbol_outcome(v: &ScVal, contract: &str) -> Outcome {
    let raw = scval_to_string(v);
    match raw.as_deref().and_then(sanitize_symbol) {
        Some(s) => Outcome::Symbol(s),
        // It answered, and the answer is unusable. A fact — but a permanent and
        // otherwise invisible one, so say which contract and what it returned.
        // This is the line to grep if a legitimate symbol is ever rejected.
        None => {
            tracing::warn!(
                contract,
                raw = raw.as_deref().unwrap_or("<not a string>"),
                "symbol() returned nothing usable; sentinelling"
            );
            Outcome::Absent
        }
    }
}

/// Soroban contracts still worth asking about, each with the count of negative
/// answers it has already given.
///
/// A contract leaves this queue two ways: it resolved (`symbol != ''`), or it
/// exhausted [`MAX_SYMBOL_ATTEMPTS`]. Everything else — no row at all, or a row
/// still under the cap — comes back.
///
/// `NOT IN` over a subquery rather than a `LEFT JOIN` so the result cannot
/// depend on the session's `join_use_nulls`. No `FINAL`: both exit conditions
/// are monotonic — `attempts` only rises and a resolved symbol is never
/// un-resolved — so *any* row meeting them is decisive, and un-merged duplicates
/// cannot change the answer. `DISTINCT` matters: 10 of the 52 Soroban rows share
/// an `asset_id` with another row (task 0139), and without it those would
/// produce duplicate RPC calls.
///
/// The second query reads the counts. It is separate rather than a join for the
/// same `join_use_nulls` reason, and is bounded by the first query's `LIMIT`.
pub async fn load_unresolved_contracts(
    client: &Client,
    limit: usize,
) -> Result<Vec<(String, u8)>, clickhouse::error::Error> {
    let contracts = client
        .query(
            "SELECT DISTINCT contract_address \
             FROM prices.assets \
             WHERE contract_address != '' \
               AND contract_address NOT IN ( \
                   SELECT contract_address FROM prices.asset_symbol \
                   WHERE symbol != '' OR attempts >= ? \
               ) \
             ORDER BY contract_address \
             LIMIT ?",
        )
        .bind(MAX_SYMBOL_ATTEMPTS)
        .bind(limit as u64)
        .fetch_all::<String>()
        .await?;

    if contracts.is_empty() {
        return Ok(Vec::new());
    }

    let prior = client
        .query(
            "SELECT contract_address, argMax(attempts, fetched_at) \
             FROM prices.asset_symbol \
             WHERE contract_address IN ? \
             GROUP BY contract_address",
        )
        .bind(&contracts)
        .fetch_all::<(String, u8)>()
        .await?;
    let prior: std::collections::HashMap<String, u8> = prior.into_iter().collect();

    Ok(contracts
        .into_iter()
        .map(|c| {
            let attempts = prior.get(&c).copied().unwrap_or(0);
            (c, attempts)
        })
        .collect())
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
/// at 25 rows — a mid-run Lambda kill loses one run, and the queue being derived
/// from stored state means the next run picks up exactly where this one stopped.
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

    for (contract, prior_attempts) in &contracts {
        match resolve_symbol(http, rpc_url, contract).await {
            Outcome::Symbol(symbol) => {
                tracing::debug!(contract, symbol, "resolved soroban symbol");
                // attempts back to 0: a contract that answered after failing is
                // evidence those failures were transient, not a fact about it.
                rows.push(SymbolRow {
                    contract_address: contract.clone(),
                    symbol,
                    attempts: 0,
                });
                stats.resolved += 1;
            }
            Outcome::Absent => {
                // One negative answer, recorded. It only becomes a permanent
                // sentinel once the count reaches MAX_SYMBOL_ATTEMPTS — which is
                // what the queue reads, so nothing else has to enforce it.
                let attempts = prior_attempts.saturating_add(1);
                if attempts >= MAX_SYMBOL_ATTEMPTS {
                    tracing::info!(
                        contract,
                        attempts,
                        "symbol() gave a negative answer {attempts}x; sentinelling for good"
                    );
                    stats.absent += 1;
                } else {
                    tracing::debug!(contract, attempts, "symbol() negative; will retry");
                    stats.skipped += 1;
                }
                rows.push(SymbolRow {
                    contract_address: contract.clone(),
                    symbol: String::new(),
                    attempts,
                });
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
    fn rejects_invisible_and_bidi_characters() {
        // `char::is_control` is Cc only, so these all passed the earlier check.
        // U+202E reverses everything after it wherever the symbol is rendered.
        assert_eq!(sanitize_symbol("USD\u{202e}CBA"), None, "rtl override");
        assert_eq!(sanitize_symbol("US\u{200b}DC"), None, "zero-width space");
        assert_eq!(sanitize_symbol("\u{feff}USDC"), None, "byte-order mark");
        assert_eq!(sanitize_symbol("US\u{ad}DC"), None, "soft hyphen");
        // Punctuation real symbols do use still passes.
        assert!(sanitize_symbol("sky-USD").is_some());
        assert!(sanitize_symbol("v1.2_beta").is_some());
    }

    #[test]
    fn an_unusable_answer_is_absent() {
        // Answered, but not a string: a fact about the contract, not a retry.
        assert_eq!(symbol_outcome(&ScVal::U32(7), "C_TEST"), Outcome::Absent);
        let sym = ScVal::Symbol(ScSymbol("XAUM".try_into().unwrap()));
        assert_eq!(
            symbol_outcome(&sym, "C_TEST"),
            Outcome::Symbol("XAUM".to_string())
        );
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
}
