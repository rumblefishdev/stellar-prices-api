//! The committed allow-list (task 0100, decision D4).
//!
//! `allowlist.toml` next to `Cargo.toml` is compiled into the binary and parsed
//! once at cold start. An entry takes a contract out of the sweep's residual on
//! purpose; see the file header for the two entry kinds.

use serde::Deserialize;
use std::collections::HashSet;

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
    #[error("allow-list entry refused: {0}")]
    Invalid(String),
}

fn require(who: &str, field: &str, value: &str) -> Result<(), AllowListError> {
    if value.trim().is_empty() {
        Err(AllowListError::Invalid(format!(
            "{who}: `{field}` is empty"
        )))
    } else {
        Ok(())
    }
}

/// Exactly 64 characters of `[0-9a-f]`.
fn is_wasm_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The allow-list compiled into the binary.
pub const EMBEDDED: &str = include_str!("../allowlist.toml");

impl AllowList {
    /// Parse an allow-list from TOML text and [`validate`](Self::validate) it.
    pub fn parse(src: &str) -> Result<Self, AllowListError> {
        let list: AllowList = toml::from_str(src)?;
        list.validate()?;
        Ok(list)
    }

    /// The allow-list compiled into the binary.
    pub fn embedded() -> Result<Self, AllowListError> {
        Self::parse(EMBEDDED)
    }

    /// Enforce the rules a list must meet before it may hide anything:
    /// - every entry has a non-empty `reason` and `task`;
    /// - every `[[wasm]]` entry has a non-empty `until` (D4: a permanent
    ///   family-wide ignore would hide the next pool of a real venue);
    /// - every contract `id` is a valid `C…` strkey (an account key or a typo
    ///   would silently match nothing);
    /// - every hash, including a contract entry's informational `wasm`, is
    ///   exactly 64 lowercase hex characters (the SQL renders `lower(hex(…))`);
    /// - no contract id and no hash appears twice.
    ///
    /// Called by [`AllowList::parse`], so no unvalidated list reaches a sweep.
    pub fn validate(&self) -> Result<(), AllowListError> {
        let mut ids = HashSet::new();
        for e in &self.contract {
            let who = format!("[[contract]] {:?}", e.id);
            if stellar_strkey::Contract::from_string(&e.id).is_err() {
                return Err(AllowListError::Invalid(format!(
                    "{who}: id is not a valid contract (C…) strkey"
                )));
            }
            if let Some(w) = &e.wasm
                && !is_wasm_hash(w)
            {
                return Err(AllowListError::Invalid(format!(
                    "{who}: wasm {w:?} is not 64 lowercase hex characters"
                )));
            }
            require(&who, "reason", &e.reason)?;
            require(&who, "task", &e.task)?;
            if !ids.insert(e.id.as_str()) {
                return Err(AllowListError::Invalid(format!("{who}: duplicate id")));
            }
        }
        let mut hashes = HashSet::new();
        for e in &self.wasm {
            let who = format!("[[wasm]] {:?}", e.hash);
            if !is_wasm_hash(&e.hash) {
                return Err(AllowListError::Invalid(format!(
                    "{who}: hash is not 64 lowercase hex characters"
                )));
            }
            require(&who, "reason", &e.reason)?;
            require(&who, "task", &e.task)?;
            require(&who, "until", &e.until)?;
            if !hashes.insert(e.hash.as_str()) {
                return Err(AllowListError::Invalid(format!("{who}: duplicate hash")));
            }
        }
        Ok(())
    }

    /// Entries no row matched, as `contract:<id>` / `wasm:<hash>`, in file
    /// order. Informational: a stale entry (especially an `until` entry after
    /// its task shipped) is a candidate for removal.
    pub fn unmatched_entries(&self, rows: &[crate::sweep::SweepRow]) -> Vec<String> {
        let hit: HashSet<String> = rows
            .iter()
            .filter_map(|r| self.match_row(&r.strkey, r.wasm.as_deref()))
            .collect();
        self.contract
            .iter()
            .map(|e| format!("contract:{}", e.id))
            .chain(self.wasm.iter().map(|e| format!("wasm:{}", e.hash)))
            .filter(|key| !hit.contains(key))
            .collect()
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
        assert_eq!(list.contract.len(), 18, "{list:?}");
        assert_eq!(list.wasm.len(), 3, "{list:?}");
        // Every family-wide entry is temporary and names the task that ends it:
        // SushiSwap V3 (0290) and the Comet BLND/USDC pool (0300).
        assert!(
            list.wasm
                .iter()
                .all(|w| w.until == "0290" || w.until == "0300"),
            "{list:?}"
        );
    }

    #[test]
    fn comet_pool_family_is_allow_listed_until_0300() {
        // Phase-1 classification (task 0100): the Comet BLND/USDC pool is a real
        // venue we do not index yet; its family waits for task 0300.
        let list = AllowList::embedded().unwrap();
        let comet = "8abc28913035c07411ed5d134e6bfeab4723d97ddd4d1a22a0605d35c94d1a36";
        assert_eq!(
            list.match_row(
                "CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM",
                Some(comet)
            ),
            Some(format!("wasm:{comet}"))
        );
        assert!(
            list.wasm
                .iter()
                .any(|w| w.hash == comet && w.until == "0300")
        );
    }

    #[test]
    fn soroswap_factory_is_allow_listed() {
        // Its `[String("SoroswapFactory"), Symbol("new_pair")]` event matches
        // the sweep's `swap` filter; without this entry every new Soroswap pair
        // would page (found by the 2026-04 back-test, task 0100).
        let list = AllowList::embedded().unwrap();
        let factory = "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2";
        assert_eq!(
            list.match_row(factory, None),
            Some(format!("contract:{factory}"))
        );
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

    const GOOD_ID: &str = "CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH";

    fn contract_toml(id: &str, reason: &str, task: &str) -> String {
        format!("[[contract]]\nid = \"{id}\"\nreason = \"{reason}\"\ntask = \"{task}\"\n")
    }

    fn wasm_toml(hash: &str, until: Option<&str>) -> String {
        let until = until.map_or(String::new(), |u| format!("until = \"{u}\"\n"));
        format!("[[wasm]]\nhash = \"{hash}\"\nreason = \"r\"\ntask = \"0290\"\n{until}")
    }

    fn refused(src: &str) -> String {
        match AllowList::parse(src) {
            Ok(list) => panic!("accepted: {list:?}\n{src}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn embedded_list_validates() {
        AllowList::embedded().unwrap().validate().unwrap();
    }

    #[test]
    fn a_minimal_valid_list_parses() {
        let src = contract_toml(GOOD_ID, "router", "0285") + &wasm_toml(SUSHI_V3, Some("0290"));
        AllowList::parse(&src).unwrap();
    }

    #[test]
    fn wasm_entry_without_until_is_refused() {
        let e = refused(&wasm_toml(SUSHI_V3, None));
        assert!(e.contains(SUSHI_V3), "{e}");
        refused(&wasm_toml(SUSHI_V3, Some("")));
        refused(&wasm_toml(SUSHI_V3, Some("  ")));
    }

    #[test]
    fn duplicates_are_refused() {
        let e = refused(&(contract_toml(GOOD_ID, "a", "1") + &contract_toml(GOOD_ID, "b", "2")));
        assert!(e.contains(GOOD_ID), "{e}");
        refused(&(wasm_toml(SUSHI_V3, Some("0290")) + &wasm_toml(SUSHI_V3, Some("0290"))));
    }

    #[test]
    fn contract_id_must_be_a_contract_strkey() {
        // An account G-key that spells SWAP — the sweep's own false positive.
        let e = refused(&contract_toml(
            "GAA574QTUD4JAFCXNI2HKFTSWAPN5KQTR2UPB5OFRCYHEPCP7IGZ73DD",
            "r",
            "t",
        ));
        assert!(e.contains("GAA574"), "{e}");
        // A one-character typo breaks the checksum.
        refused(&contract_toml(
            "CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDA",
            "r",
            "t",
        ));
    }

    #[test]
    fn hashes_must_be_64_lowercase_hex() {
        refused(&wasm_toml(&SUSHI_V3.to_uppercase(), Some("0290")));
        refused(&wasm_toml(&SUSHI_V3[..63], Some("0290")));
        refused(&wasm_toml(&format!("{}g", &SUSHI_V3[..63]), Some("0290")));
        // The informational wasm on a contract entry is held to the same rule.
        let src = format!(
            "[[contract]]\nid = \"{GOOD_ID}\"\nwasm = \"{}\"\nreason = \"r\"\ntask = \"t\"\n",
            SUSHI_V3.to_uppercase()
        );
        refused(&src);
    }

    #[test]
    fn empty_reason_or_task_is_refused() {
        refused(&contract_toml(GOOD_ID, "", "0285"));
        refused(&contract_toml(GOOD_ID, "router", ""));
        refused(&contract_toml(GOOD_ID, " ", "0285"));
        refused(
            "[[wasm]]\nhash = \"003710b383f9da7d650a7f719a7be479110266427817ebbed61d924505fcd7c7\"\nreason = \"\"\ntask = \"0290\"\nuntil = \"0290\"\n",
        );
    }

    #[test]
    fn unknown_keys_are_refused() {
        refused(&(contract_toml(GOOD_ID, "r", "t") + "note = \"x\"\n"));
        refused("[[pool]]\nid = \"x\"\n");
    }

    #[test]
    fn unmatched_entries_lists_what_no_row_hit() {
        use crate::sweep::SweepRow;
        let list = AllowList::embedded().unwrap();
        let row = |strkey: &str, wasm: Option<&str>| SweepRow {
            strkey: strkey.into(),
            wasm: wasm.map(str::to_string),
            contract_surrogate: 1,
            events: 1,
            txs: 1,
            first_ledger: 1,
            last_ledger: 1,
            top_action: String::new(),
            top_shape: String::new(),
        };
        let rows = vec![
            row(AQUARIUS_ROUTER, None),
            row(
                "CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH",
                None,
            ),
            row(
                "CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL",
                None,
            ),
            row(
                "CAUF4DFYSX52L2KJ4J7OFW3WDQMEUDVXNB7PG5VIC4VVOA3BCLWXDO2E",
                None,
            ),
            row("CSOMEPOOL", Some(SUSHI_V3)),
        ];
        let unmatched = list.unmatched_entries(&rows);
        // The five entries the rows hit are never reported...
        for hit in [
            format!("contract:{AQUARIUS_ROUTER}"),
            "contract:CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH".to_string(),
            "contract:CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL".to_string(),
            "contract:CAUF4DFYSX52L2KJ4J7OFW3WDQMEUDVXNB7PG5VIC4VVOA3BCLWXDO2E".to_string(),
            format!("wasm:{SUSHI_V3}"),
        ] {
            assert!(!unmatched.contains(&hit), "{hit} reported: {unmatched:?}");
        }
        // ...and every other entry is, contracts and wasm families alike.
        assert_eq!(
            unmatched.len(),
            list.contract.len() + list.wasm.len() - 5,
            "{unmatched:?}"
        );
        for missed in [
            "contract:CA7RQDMMV6E53P5EDZA5GPWBZ33AMW2ZNO42XLI2RGRIAP4QXIARUOJQ",
            "contract:CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2",
            "wasm:95a8e0018530226701ef8d31c7d4c2fe20ed9d7c303a14a70c3d96848ca4fa54",
            "wasm:8abc28913035c07411ed5d134e6bfeab4723d97ddd4d1a22a0605d35c94d1a36",
        ] {
            assert!(unmatched.iter().any(|u| u == missed), "{missed} missing");
        }
    }
}
