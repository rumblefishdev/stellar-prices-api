use std::collections::HashMap;

use sha2::{Digest, Sha256};
use stellar_xdr::curr::{
    AccountId, AlphaNum4, AlphaNum12, Asset, AssetCode4, AssetCode12, ContractIdPreimage, Hash,
    HashIdPreimage, HashIdPreimageContractId, Limits, PublicKey, Uint256, WriteXdr,
};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum AssetIdentity {
    Native,
    Credit {
        code: String,
        issuer: String,
    },
    /// A Soroban token referenced by its contract address (C-strkey). Used by
    /// the AMM venues (Phoenix/Soroswap/Aquarius) whose swap events carry
    /// contract-address tokens rather than classic (code, issuer) pairs.
    Contract(String),
}

impl AssetIdentity {
    pub fn from_xdr(asset: &Asset) -> Self {
        match asset {
            Asset::Native => Self::Native,
            Asset::CreditAlphanum4(a) => Self::Credit {
                code: String::from_utf8_lossy(a.asset_code.as_slice())
                    .trim_end_matches('\0')
                    .to_string(),
                issuer: stellar_strkey(pubkey_bytes(&a.issuer.0)),
            },
            Asset::CreditAlphanum12(a) => Self::Credit {
                code: String::from_utf8_lossy(a.asset_code.as_slice())
                    .trim_end_matches('\0')
                    .to_string(),
                issuer: stellar_strkey(pubkey_bytes(&a.issuer.0)),
            },
        }
    }

    fn sort_key(&self) -> Vec<u8> {
        match self {
            Self::Native => vec![0],
            Self::Credit { code, issuer } => {
                let mut key = vec![1];
                key.extend_from_slice(code.as_bytes());
                key.push(0);
                key.extend_from_slice(issuer.as_bytes());
                key
            }
            Self::Contract(addr) => {
                let mut key = vec![2];
                key.extend_from_slice(addr.as_bytes());
                key
            }
        }
    }
}

/// Canonical Stellar issuer for circle's USDC. Shared with the Soroban oracle
/// reconciliation (`soroban.rs`) so a Reflector `USDC` symbol resolves to the
/// same `AssetIdentity` (hence the same `asset_id`) used as a trade quote.
pub(crate) const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
/// Canonical Stellar issuer for USDT. Shared with `soroban.rs` (see above).
pub(crate) const USDT_ISSUER: &str = "GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V";

/// Mainnet (Public) network passphrase. The SAC contract id is network-scoped, so
/// this fixes which network's SACs we resolve. The backfill is mainnet-only (the
/// public ledger store), so this is hard-coded; make it a parameter if a testnet
/// backfill is ever needed.
const MAINNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";

fn mainnet_network_id() -> [u8; 32] {
    Sha256::digest(MAINNET_PASSPHRASE.as_bytes()).into()
}

/// Rebuild the classic `Asset` XDR for a Native/Credit identity (so we can derive
/// its SAC contract id). Returns `None` for a `Contract` identity (a pure Soroban
/// token has no classic underlying) or an unparsable issuer / over-long code.
fn identity_to_asset(identity: &AssetIdentity) -> Option<Asset> {
    match identity {
        AssetIdentity::Native => Some(Asset::Native),
        AssetIdentity::Credit { code, issuer } => {
            let key = stellar_strkey::ed25519::PublicKey::from_string(issuer).ok()?;
            let account = AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(key.0)));
            let bytes = code.as_bytes();
            if bytes.len() <= 4 {
                let mut buf = [0u8; 4];
                buf[..bytes.len()].copy_from_slice(bytes);
                Some(Asset::CreditAlphanum4(AlphaNum4 {
                    asset_code: AssetCode4(buf),
                    issuer: account,
                }))
            } else if bytes.len() <= 12 {
                let mut buf = [0u8; 12];
                buf[..bytes.len()].copy_from_slice(bytes);
                Some(Asset::CreditAlphanum12(AlphaNum12 {
                    asset_code: AssetCode12(buf),
                    issuer: account,
                }))
            } else {
                None
            }
        }
        AssetIdentity::Contract(_) => None,
    }
}

/// The deterministic Stellar Asset Contract (SAC) address for a classic asset on
/// the given network: `C…`-strkey of `sha256(HashIdPreimage::ContractId{network,
/// ContractIdPreimage::Asset(asset)})`. This is the standard offline derivation —
/// no RPC, computable from the asset alone.
fn sac_address(asset: &Asset, network_id: &[u8; 32]) -> Option<String> {
    let preimage = HashIdPreimage::ContractId(HashIdPreimageContractId {
        network_id: Hash(*network_id),
        contract_id_preimage: ContractIdPreimage::Asset(asset.clone()),
    });
    let xdr = preimage.to_xdr(Limits::none()).ok()?;
    let hash: [u8; 32] = Sha256::digest(&xdr).into();
    Some(stellar_strkey::Contract(hash).to_string())
}

fn is_preferred_quote(asset: &AssetIdentity) -> Option<u8> {
    match asset {
        AssetIdentity::Credit { code, issuer } if code == "USDC" && issuer == USDC_ISSUER => {
            Some(0)
        }
        AssetIdentity::Credit { code, issuer } if code == "USDT" && issuer == USDT_ISSUER => {
            Some(1)
        }
        AssetIdentity::Native => Some(2),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalPair {
    pub base: AssetIdentity,
    pub quote: AssetIdentity,
    pub base_id: u32,
    pub quote_id: u32,
    pub inverted: bool,
}

pub struct AssetRegistry {
    by_identity: HashMap<AssetIdentity, u32>,
    next_id: u32,
    network_id: [u8; 32],
    /// SAC contract address → the classic identity it wraps (task 0061 §12.4).
    /// Lets the AMM path collapse a SAC-quoted/based token onto the same
    /// `asset_id` as its classic SDEX form, so liquidity is not split across two
    /// ids and the cross-source merge (ADR 0004) holds.
    sac_index: HashMap<String, AssetIdentity>,
}

impl AssetRegistry {
    pub fn from_existing(existing: Vec<(u32, AssetIdentity)>) -> Self {
        let mut next_id = 1u32;
        let mut by_identity = HashMap::with_capacity(existing.len());
        for (id, identity) in existing {
            next_id = next_id.max(id + 1);
            by_identity.insert(identity, id);
        }
        let mut reg = Self {
            by_identity,
            next_id,
            network_id: mainnet_network_id(),
            sac_index: HashMap::new(),
        };
        // Pre-seed the canonical quote SACs so an AMM-via-SAC USDC/USDT/XLM
        // collapses even before that asset's first classic (SDEX) sighting in the
        // run. Then map every already-known classic asset's SAC too.
        reg.register_sac(&AssetIdentity::Native);
        reg.register_sac(&AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: USDC_ISSUER.to_string(),
        });
        reg.register_sac(&AssetIdentity::Credit {
            code: "USDT".to_string(),
            issuer: USDT_ISSUER.to_string(),
        });
        let known: Vec<AssetIdentity> = reg.by_identity.keys().cloned().collect();
        for identity in known {
            reg.register_sac(&identity);
        }
        reg
    }

    /// Record the SAC address of a classic identity (no-op for `Contract` and for
    /// identities whose SAC was already mapped). Cheap: only on first intern.
    fn register_sac(&mut self, identity: &AssetIdentity) {
        if let Some(asset) = identity_to_asset(identity) {
            if let Some(addr) = sac_address(&asset, &self.network_id) {
                self.sac_index
                    .entry(addr)
                    .or_insert_with(|| identity.clone());
            }
        }
    }

    pub fn get_or_assign(&mut self, identity: &AssetIdentity) -> u32 {
        if let Some(&id) = self.by_identity.get(identity) {
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.by_identity.insert(identity.clone(), id);
        self.register_sac(identity);
        id
    }

    /// If `contract_addr` is the SAC of a known classic asset, return that
    /// underlying classic identity (§12.4). The AMM path uses this to collapse a
    /// SAC token onto its classic `asset_id`; a pure Soroban token returns `None`
    /// and keeps its `Contract(address)` identity.
    pub fn resolve_sac(&self, contract_addr: &str) -> Option<AssetIdentity> {
        self.sac_index.get(contract_addr).cloned()
    }

    /// The SAC contract address that wraps a classic identity (Native / Credit),
    /// or `None` for a pure `Contract` identity (no classic underlying). Persisted
    /// onto `prices.assets.sac_address` so a read-time consumer can resolve a
    /// SAC-wrapped leg (their contract address) back to the collapsed classic
    /// price — the §12.4 collapse is write-time, so the price lives under the
    /// classic identity, not under the SAC address.
    pub fn sac_address_of(&self, identity: &AssetIdentity) -> Option<String> {
        identity_to_asset(identity).and_then(|asset| sac_address(&asset, &self.network_id))
    }

    pub fn assets(&self) -> impl Iterator<Item = (&AssetIdentity, &u32)> {
        self.by_identity.iter()
    }
}

pub fn canonicalise(
    asset_sold: &AssetIdentity,
    asset_bought: &AssetIdentity,
    registry: &mut AssetRegistry,
) -> CanonicalPair {
    let pref_sold = is_preferred_quote(asset_sold);
    let pref_bought = is_preferred_quote(asset_bought);

    let (base, quote, inverted) = match (pref_sold, pref_bought) {
        (Some(a), Some(b)) if a < b => (asset_bought.clone(), asset_sold.clone(), true),
        (Some(a), Some(b)) if a > b => (asset_sold.clone(), asset_bought.clone(), false),
        (Some(_), None) => (asset_bought.clone(), asset_sold.clone(), true),
        (None, Some(_)) => (asset_sold.clone(), asset_bought.clone(), false),
        _ => {
            if asset_sold.sort_key() < asset_bought.sort_key() {
                (asset_sold.clone(), asset_bought.clone(), false)
            } else {
                (asset_bought.clone(), asset_sold.clone(), true)
            }
        }
    };

    let base_id = registry.get_or_assign(&base);
    let quote_id = registry.get_or_assign(&quote);

    CanonicalPair {
        base,
        quote,
        base_id,
        quote_id,
        inverted,
    }
}

fn pubkey_bytes(pk: &PublicKey) -> &[u8] {
    match pk {
        PublicKey::PublicKeyTypeEd25519(key) => key.as_slice(),
    }
}

fn stellar_strkey(ed25519: &[u8]) -> String {
    let mut payload = Vec::with_capacity(35);
    payload.push(6 << 3); // G... prefix (account id version byte)
    payload.extend_from_slice(ed25519);

    let checksum = crc16(&payload);
    payload.push((checksum & 0xFF) as u8);
    payload.push((checksum >> 8) as u8);

    base32_encode(&payload)
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        let mut code = crc >> 8 & 0xFF;
        code ^= byte as u16;
        code ^= code >> 4;
        crc = (crc << 8) & 0xFFFF;
        crc ^= code;
        crc ^= (code << 5) & 0xFFFF;
        crc ^= (code << 12) & 0xFFFF;
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    // The canonical native-XLM SAC on mainnet (Public network). A fixed,
    // independently-known value — if our preimage/hash/strkey derivation is
    // correct it reproduces exactly this.
    const NATIVE_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";

    #[test]
    fn native_sac_matches_known_mainnet_address() {
        let addr = sac_address(&Asset::Native, &mainnet_network_id()).unwrap();
        assert_eq!(addr, NATIVE_SAC);
    }

    #[test]
    fn resolve_sac_collapses_native_sac_to_native() {
        // Pre-seeded at construction, even on an empty registry.
        let reg = AssetRegistry::from_existing(vec![]);
        assert_eq!(reg.resolve_sac(NATIVE_SAC), Some(AssetIdentity::Native));
    }

    #[test]
    fn sac_address_of_emits_classic_sac_and_none_for_contract() {
        let reg = AssetRegistry::from_existing(vec![]);
        // Classic identity → its deterministic SAC (the persisted read-seam key).
        assert_eq!(
            reg.sac_address_of(&AssetIdentity::Native).as_deref(),
            Some(NATIVE_SAC)
        );
        // Pure Soroban token → no classic underlying → no SAC.
        assert_eq!(
            reg.sac_address_of(&AssetIdentity::Contract("CXYZ".to_string())),
            None
        );
    }

    #[test]
    fn sac_and_classic_collapse_to_one_asset_id() {
        let usdc = AssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: USDC_ISSUER.to_string(),
        };
        let usdc_sac =
            sac_address(&identity_to_asset(&usdc).unwrap(), &mainnet_network_id()).unwrap();

        let mut reg = AssetRegistry::from_existing(vec![]);
        // SDEX path interns the classic USDC.
        let via_sdex = reg.get_or_assign(&usdc);
        // AMM path sees the USDC SAC contract address → resolves to the classic
        // identity → same asset_id (no split).
        let resolved = reg
            .resolve_sac(&usdc_sac)
            .expect("usdc sac maps to classic");
        let via_amm = reg.get_or_assign(&resolved);
        assert_eq!(resolved, usdc);
        assert_eq!(via_sdex, via_amm);
    }

    #[test]
    fn pure_soroban_token_is_not_resolved() {
        let reg = AssetRegistry::from_existing(vec![]);
        // A contract address that is not a known SAC stays unresolved (the AMM
        // path keeps its Contract(address) identity).
        assert_eq!(
            reg.resolve_sac("CNOTASACADDRESSxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
            None
        );
    }
}

fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = String::with_capacity((data.len() * 8 + 4) / 5);
    let mut buffer: u64 = 0;
    let mut bits = 0;

    for &byte in data {
        buffer = (buffer << 8) | byte as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            result.push(ALPHABET[((buffer >> bits) & 0x1F) as usize] as char);
        }
    }
    if bits > 0 {
        buffer <<= 5 - bits;
        result.push(ALPHABET[(buffer & 0x1F) as usize] as char);
    }
    result
}
