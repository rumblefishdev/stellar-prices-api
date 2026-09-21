//! The committed allow-list (task 0100, decision D4).
//!
//! `allowlist.toml` next to `Cargo.toml` is compiled into the binary and parsed
//! once at cold start. An entry takes a contract out of the sweep's residual on
//! purpose; see the file header for the two entry kinds.

use serde::Deserialize;

/// The allow-list as checked in: `[[contract]]` and `[[wasm]]` entries.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AllowList {
    #[serde(default)]
    pub contract: Vec<ContractEntry>,
    #[serde(default)]
    pub wasm: Vec<WasmEntry>,
}

/// One contract, permanently allow-listed (routers, aggregators, non-AMM).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContractEntry {
    /// The contract's `C…` strkey.
    pub id: String,
    /// Informational only: the wasm hash seen when the entry was added. It
    /// does not widen the match to the whole family.
    #[serde(default)]
    pub wasm: Option<String>,
    pub reason: String,
    pub task: String,
}

/// A whole wasm family, allow-listed until `until` ships.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WasmEntry {
    /// 64 lowercase hex characters.
    pub hash: String,
    pub reason: String,
    pub task: String,
    /// The task whose delivery removes this entry. Required (D4).
    #[serde(default)]
    pub until: String,
}

/// Why an allow-list was refused.
#[derive(Debug, thiserror::Error)]
pub enum AllowListError {
    #[error("allow-list is not valid TOML for the expected schema: {0}")]
    Parse(#[from] toml::de::Error),
}

/// The allow-list compiled into the binary.
pub const EMBEDDED: &str = include_str!("../allowlist.toml");

impl AllowList {
    /// Parse an allow-list from TOML text.
    pub fn parse(src: &str) -> Result<Self, AllowListError> {
        let list: AllowList = toml::from_str(src)?;
        Ok(list)
    }

    /// The allow-list compiled into the binary.
    pub fn embedded() -> Result<Self, AllowListError> {
        Self::parse(EMBEDDED)
    }

    /// The entry that allow-lists a row, as `contract:<id>` or `wasm:<hash>`,
    /// or `None` when the row is unclassified. A contract entry wins over a
    /// wasm entry. An empty strkey (a contract BE has not resolved) never
    /// matches a contract entry.
    pub fn match_row(&self, strkey: &str, wasm: Option<&str>) -> Option<String> {
        if !strkey.is_empty()
            && let Some(e) = self.contract.iter().find(|e| e.id == strkey)
        {
            return Some(format!("contract:{}", e.id));
        }
        let wasm = wasm.filter(|w| !w.is_empty())?;
        self.wasm
            .iter()
            .find(|e| e.hash == wasm)
            .map(|e| format!("wasm:{}", e.hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUSHI_V3: &str = "003710b383f9da7d650a7f719a7be479110266427817ebbed61d924505fcd7c7";
    const AQUARIUS_ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";

    #[test]
    fn embedded_list_parses_with_the_seeded_entries() {
        let list = AllowList::embedded().expect("embedded allow-list parses");
        assert_eq!(list.contract.len(), 5, "{list:?}");
        assert_eq!(list.wasm.len(), 2, "{list:?}");
        assert!(list.wasm.iter().all(|w| w.until == "0290"));
    }

    #[test]
    fn match_row_prefers_contract_then_wasm() {
        let list = AllowList::embedded().unwrap();
        assert_eq!(
            list.match_row(AQUARIUS_ROUTER, None),
            Some(format!("contract:{AQUARIUS_ROUTER}"))
        );
        assert_eq!(
            list.match_row("CCR2CH4G", Some(SUSHI_V3)),
            Some(format!("wasm:{SUSHI_V3}"))
        );
        assert_eq!(list.match_row("CXYZ", Some("8abc")), None);
        assert_eq!(list.match_row("", None), None);
    }
}
