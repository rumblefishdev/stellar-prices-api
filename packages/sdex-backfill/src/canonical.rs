use std::collections::HashMap;

use stellar_xdr::curr::{Asset, PublicKey};

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
}

impl AssetRegistry {
    pub fn from_existing(existing: Vec<(u32, AssetIdentity)>) -> Self {
        let mut next_id = 1u32;
        let mut by_identity = HashMap::with_capacity(existing.len());
        for (id, identity) in existing {
            next_id = next_id.max(id + 1);
            by_identity.insert(identity, id);
        }
        Self {
            by_identity,
            next_id,
        }
    }

    pub fn get_or_assign(&mut self, identity: &AssetIdentity) -> u32 {
        if let Some(&id) = self.by_identity.get(identity) {
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.by_identity.insert(identity.clone(), id);
        id
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
