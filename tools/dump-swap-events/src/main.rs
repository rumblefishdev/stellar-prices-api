//! Lore task 0001: dump Soroban contract event topics + data from a directory
//! of zstd-compressed `LedgerCloseMetaBatch` files (Galexie history-archive
//! layout). Designed to answer the open question §11.1 in
//! `docs/database-schema/amm-trades-schema.md`: what is `topics[0]` for
//! Soroswap / Aquarius / Phoenix swap events, and what does the `data`
//! payload look like?
//!
//! Self-contained: no dependency on the soroban-block-explorer workspace.
//!
//! Usage:
//!     cargo run --release -- \
//!         --dir ../../.temp/FC4DB5FF--62016000-62079999
//!
//!     cargo run --release -- --dir <DIR> --no-filter --limit 5
//!     cargo run --release -- --dir <DIR> --symbol transfer

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;
use stellar_xdr::curr::{
    ContractEvent, ContractEventBody, ContractEventType, ContractId, LedgerCloseMeta,
    LedgerCloseMetaBatch, Limits, ReadXdr, ScVal, ScVec, TransactionMeta, WriteXdr,
};

#[derive(Clone, Copy, Debug, Serialize)]
enum EventSource {
    /// Tx-level (V3 `soroban_meta.events` or V4 `events`). Hashed.
    TxLevel,
    /// V4 `operations[i].events` — CAP-67 per-op consensus events. Hashed.
    PerOp,
    /// V3/V4 `diagnostic_events`. NOT hashed; in diagnostic mode this
    /// container holds byte-identical Contract-typed mirrors of per-op
    /// consensus events alongside host-VM trace entries. Filter by source,
    /// not inner `event_type`.
    Diagnostic,
}

struct Args {
    dir: PathBuf,
    /// Substring match on topic[0] string form. `None` = no filter.
    symbol: Option<String>,
    /// Stop after N hits.
    limit: Option<usize>,
    /// Include events from the `diagnostic_events` container.
    include_diagnostic: bool,
    /// Pretty-print JSON; default is compact (one line per hit).
    pretty: bool,
    /// Suppress per-event output; only print the topic_0 histogram. Implies
    /// `--no-filter` (we want to count everything). Useful for surveying
    /// what event kinds exist in a ledger range.
    histogram: bool,
    /// Filter by tx hash (hex, lowercase). For pinpointed lookups when we
    /// know the exact transaction we want to decode.
    tx_filter: Option<String>,
    /// Filter by emitter contract id (C... strkey). Useful for locating
    /// activity from a specific pool / router before targeting one tx.
    contract_filter: Option<String>,
    /// Emit raw XDR (base64) of the topics ScVec and data ScVal alongside the
    /// decoded JSON. Used by lore task 0018 to confirm both decode paths
    /// (`ScVal::from_xdr_base64` and `serde_json::from_str::<ScVal>`).
    show_xdr: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut dir: Option<PathBuf> = None;
    let mut symbol: Option<String> = Some("swap".to_string());
    let mut no_filter = false;
    let mut limit: Option<usize> = None;
    let mut include_diagnostic = false;
    let mut pretty = false;
    let mut histogram = false;
    let mut tx_filter: Option<String> = None;
    let mut contract_filter: Option<String> = None;
    let mut show_xdr = false;

    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--dir" => dir = iter.next().map(PathBuf::from),
            "--symbol" => symbol = iter.next(),
            "--no-filter" => no_filter = true,
            "--limit" => {
                limit = iter
                    .next()
                    .ok_or("--limit needs a value")?
                    .parse::<usize>()
                    .ok();
            }
            "--include-diagnostic" => include_diagnostic = true,
            "--pretty" => pretty = true,
            "--histogram" => {
                histogram = true;
                no_filter = true;
            }
            "--tx" => tx_filter = iter.next().map(|s| s.to_ascii_lowercase()),
            "--contract" => contract_filter = iter.next(),
            "--show-xdr" => show_xdr = true,
            "-h" | "--help" => {
                eprintln!(
                    "usage: dump-swap-events --dir <DIR> [--symbol <SUBSTR>|--no-filter] \
                     [--tx <HEX_HASH>] [--contract <C_STRKEY>] [--show-xdr] \
                     [--limit <N>] [--include-diagnostic] [--pretty]\n\
                     \n\
                     defaults: --symbol swap, drops Diagnostic-source events.\n\
                     --show-xdr adds topics_xdr_b64 and data_xdr_b64 fields.\n\
                     --contract narrows to events emitted by one contract id.\n"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }

    Ok(Args {
        dir: dir.ok_or("--dir is required")?,
        symbol: if no_filter { None } else { symbol },
        limit,
        include_diagnostic,
        pretty,
        histogram,
        tx_filter,
        contract_filter,
        show_xdr,
    })
}

/// Strkey-encode a 32-byte Soroban contract id to its `C…` address.
fn contract_strkey(cid: &ContractId) -> String {
    stellar_strkey::Contract((cid.0).0).to_string()
}

/// Best-effort string form of `topic[0]` for filtering and bucketing.
/// Returns `None` if the topic is not a Symbol/String/Bytes value, since
/// only those are typically used as event tags.
fn topic0_string(topics: &[ScVal]) -> Option<String> {
    let first = topics.first()?;
    match first {
        ScVal::Symbol(s) => std::str::from_utf8(s.0.as_slice()).ok().map(String::from),
        ScVal::String(s) => std::str::from_utf8(s.0.as_slice()).ok().map(String::from),
        ScVal::Bytes(b) => std::str::from_utf8(b.0.as_slice()).ok().map(String::from),
        _ => None,
    }
}

fn event_type_label(t: ContractEventType) -> &'static str {
    match t {
        ContractEventType::System => "System",
        ContractEventType::Contract => "Contract",
        ContractEventType::Diagnostic => "Diagnostic",
    }
}

#[derive(Serialize)]
struct EmittedEvent<'a> {
    ledger_seq: u32,
    tx_hash: &'a str,
    event_index: usize,
    source: EventSource,
    inner_type: &'static str,
    contract_id: Option<String>,
    topic_0: Option<String>,
    topics: &'a [ScVal],
    data: &'a ScVal,
    #[serde(skip_serializing_if = "Option::is_none")]
    topics_xdr_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_xdr_b64: Option<String>,
}

struct Walker<'a> {
    args: &'a Args,
    hits: usize,
    topic_counts: std::collections::BTreeMap<String, usize>,
    events_seen: usize,
    files_seen: usize,
    files_failed: usize,
}

impl<'a> Walker<'a> {
    fn new(args: &'a Args) -> Self {
        Self {
            args,
            hits: 0,
            topic_counts: Default::default(),
            events_seen: 0,
            files_seen: 0,
            files_failed: 0,
        }
    }

    /// Decide whether to emit and bump the appropriate counters.
    fn handle_event(
        &mut self,
        ev: &ContractEvent,
        ledger_seq: u32,
        tx_hash: &str,
        event_index: usize,
        source: EventSource,
    ) -> bool {
        self.events_seen += 1;

        if matches!(source, EventSource::Diagnostic) && !self.args.include_diagnostic {
            return false;
        }

        if let Some(want) = &self.args.contract_filter {
            let matches_contract = ev
                .contract_id
                .as_ref()
                .map(contract_strkey)
                .as_deref()
                == Some(want.as_str());
            if !matches_contract {
                return false;
            }
        }

        let ContractEventBody::V0(body) = &ev.body;
        let topics: &[ScVal] = body.topics.as_slice();

        let t0 = topic0_string(topics);
        if let Some(t) = &t0 {
            *self.topic_counts.entry(t.clone()).or_insert(0) += 1;
        }

        if let Some(filter) = &self.args.symbol {
            let Some(t) = &t0 else { return false };
            if !t.contains(filter) {
                return false;
            }
        }

        if self.args.histogram {
            // Counted, but don't emit per-event output.
            self.hits += 1;
            return true;
        }

        let (topics_xdr_b64, data_xdr_b64) = if self.args.show_xdr {
            // Re-encode the topics list as ScVec (the natural XDR container
            // for a list of ScVal) so the base64 form is what a hypothetical
            // raw-XDR storage layer would persist. BE's db-clickhouse writer
            // actually stores serde_json instead — see lore task 0018 G-note
            // for the storage-format finding.
            let topics_xdr = topics
                .to_vec()
                .try_into()
                .ok()
                .and_then(|vm| ScVec(vm).to_xdr_base64(Limits::none()).ok());
            let data_xdr = body.data.to_xdr_base64(Limits::none()).ok();
            (topics_xdr, data_xdr)
        } else {
            (None, None)
        };

        let emitted = EmittedEvent {
            ledger_seq,
            tx_hash,
            event_index,
            source,
            inner_type: event_type_label(ev.type_),
            contract_id: ev.contract_id.as_ref().map(contract_strkey),
            topic_0: t0,
            topics,
            data: &body.data,
            topics_xdr_b64,
            data_xdr_b64,
        };

        let json = if self.args.pretty {
            serde_json::to_string_pretty(&emitted)
        } else {
            serde_json::to_string(&emitted)
        };
        match json {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("[serialize-err] {e}"),
        }
        self.hits += 1;
        true
    }

    fn limit_reached(&self) -> bool {
        matches!(self.args.limit, Some(l) if self.hits >= l)
    }

    fn process_tx_meta(&mut self, meta: &TransactionMeta, tx_hash: &str, ledger_seq: u32) {
        if let Some(filter) = &self.args.tx_filter {
            if !tx_hash.eq_ignore_ascii_case(filter) {
                return;
            }
        }
        match meta {
            TransactionMeta::V3(v3) => {
                if let Some(sm) = &v3.soroban_meta {
                    let mut idx = 0usize;
                    for ev in sm.events.iter() {
                        self.handle_event(ev, ledger_seq, tx_hash, idx, EventSource::TxLevel);
                        idx += 1;
                        if self.limit_reached() {
                            return;
                        }
                    }
                    for diag in sm.diagnostic_events.iter() {
                        self.handle_event(
                            &diag.event,
                            ledger_seq,
                            tx_hash,
                            idx,
                            EventSource::Diagnostic,
                        );
                        idx += 1;
                        if self.limit_reached() {
                            return;
                        }
                    }
                }
            }
            TransactionMeta::V4(v4) => {
                let mut idx = 0usize;
                for tev in v4.events.iter() {
                    self.handle_event(
                        &tev.event,
                        ledger_seq,
                        tx_hash,
                        idx,
                        EventSource::TxLevel,
                    );
                    idx += 1;
                    if self.limit_reached() {
                        return;
                    }
                }
                for op in v4.operations.iter() {
                    for ev in op.events.iter() {
                        self.handle_event(ev, ledger_seq, tx_hash, idx, EventSource::PerOp);
                        idx += 1;
                        if self.limit_reached() {
                            return;
                        }
                    }
                }
                for diag in v4.diagnostic_events.iter() {
                    self.handle_event(
                        &diag.event,
                        ledger_seq,
                        tx_hash,
                        idx,
                        EventSource::Diagnostic,
                    );
                    idx += 1;
                    if self.limit_reached() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }

    fn process_meta(&mut self, meta: &LedgerCloseMeta) {
        let (ledger_seq, txs): (u32, Vec<(String, &TransactionMeta)>) = match meta {
            LedgerCloseMeta::V0(v) => (
                v.ledger_header.header.ledger_seq,
                v.tx_processing
                    .iter()
                    .map(|p| (hex::encode(p.result.transaction_hash.0), &p.tx_apply_processing))
                    .collect(),
            ),
            LedgerCloseMeta::V1(v) => (
                v.ledger_header.header.ledger_seq,
                v.tx_processing
                    .iter()
                    .map(|p| (hex::encode(p.result.transaction_hash.0), &p.tx_apply_processing))
                    .collect(),
            ),
            LedgerCloseMeta::V2(v) => (
                v.ledger_header.header.ledger_seq,
                v.tx_processing
                    .iter()
                    .map(|p| (hex::encode(p.result.transaction_hash.0), &p.tx_apply_processing))
                    .collect(),
            ),
        };

        for (tx_hash, meta) in &txs {
            self.process_tx_meta(meta, tx_hash, ledger_seq);
            if self.limit_reached() {
                return;
            }
        }
    }

    fn process_file(&mut self, path: &Path) {
        self.files_seen += 1;
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[read-err] {}: {e}", path.display());
                self.files_failed += 1;
                return;
            }
        };
        let xdr = match zstd::decode_all(bytes.as_slice()) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[zstd-err] {}: {e}", path.display());
                self.files_failed += 1;
                return;
            }
        };
        let batch = match LedgerCloseMetaBatch::from_xdr(&xdr, Limits::none()) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[xdr-err] {}: {e}", path.display());
                self.files_failed += 1;
                return;
            }
        };
        for meta in batch.ledger_close_metas.iter() {
            self.process_meta(meta);
            if self.limit_reached() {
                return;
            }
        }
    }
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let entries = match fs::read_dir(&args.dir) {
        Ok(it) => it,
        Err(e) => {
            eprintln!("error: cannot read --dir {}: {e}", args.dir.display());
            return ExitCode::from(2);
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|r| r.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().and_then(|x| x.to_str()) == Some("zst")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(".xdr.zst"))
                    .unwrap_or(false)
        })
        .collect();
    files.sort();

    eprintln!(
        "[dump-swap-events] dir={} files={} filter={} include_diagnostic={}",
        args.dir.display(),
        files.len(),
        args.symbol.as_deref().unwrap_or("<none>"),
        args.include_diagnostic
    );

    let mut walker = Walker::new(&args);
    for path in &files {
        walker.process_file(path);
        if walker.limit_reached() {
            eprintln!("[dump-swap-events] limit reached, stopping");
            break;
        }
    }

    eprintln!();
    eprintln!("=== summary ===");
    eprintln!("  files scanned:  {}", walker.files_seen);
    eprintln!("  files failed:   {}", walker.files_failed);
    eprintln!("  events seen:    {}", walker.events_seen);
    eprintln!("  events emitted: {}", walker.hits);

    if walker.args.symbol.is_none() {
        eprintln!();
        eprintln!("=== topic[0] histogram ===");
        let mut sorted: Vec<(&String, &usize)> = walker.topic_counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (topic, count) in sorted.iter().take(100) {
            eprintln!("  {:>10}  {}", count, topic);
        }
    }

    ExitCode::SUCCESS
}
