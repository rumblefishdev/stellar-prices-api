use extractors_core::{SorobanEventRow, TaggedValue};

pub const XLM_USDC_POOL: &str = "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX";
pub const PHO_USDC_POOL: &str = "CD5XNKK3B6BEF2N7ULNHHGAMOKZ7P6456BFNIHRF4WNTEDKBRWAE7IAA";
pub const TRADER: &str = "GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD";
pub const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
pub const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
pub const TX_HASH: &str = "559498bdf567340c0780b80f2bfa07bcc58713fc328e659ef72461849a326aa8";

pub fn make_phoenix_xyk_events(pool: &str, base_index: u32) -> Vec<SorobanEventRow> {
    let fields: &[(&str, TaggedValue)] = &[
        ("sender", TaggedValue::Address(TRADER.into())),
        ("sell_token", TaggedValue::Address(XLM_SAC.into())),
        ("offer_amount", TaggedValue::I128(11659417676)),
        ("actual received amount", TaggedValue::I128(11659417676)),
        ("buy_token", TaggedValue::Address(USDC_SAC.into())),
        ("return_amount", TaggedValue::I128(1857322909)),
        ("spread_amount", TaggedValue::I128(503808)),
        ("referral_fee_amount", TaggedValue::I128(0)),
    ];

    fields
        .iter()
        .enumerate()
        .map(|(i, (name, data))| SorobanEventRow {
            contract_id: pool.to_string(),
            transaction_id: TX_HASH.to_string(),
            ledger_sequence: 62460522,
            event_index: base_index + i as u32,
            topics: vec![
                TaggedValue::String("swap".into()),
                TaggedValue::String((*name).into()),
            ],
            data: data.clone(),
        })
        .collect()
}

pub fn common_xyk_wasm_hash() -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = 0x16;
    h[1] = 0x7a;
    h[2] = 0xb4;
    h[3] = 0x14;
    h
}

pub fn alt_xyk_wasm_hash() -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = 0x13;
    h[1] = 0xb1;
    h[2] = 0x58;
    h[3] = 0x65;
    h
}
