use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Venue {
    Soroswap,
    Aquarius,
    Phoenix,
}

impl Venue {
    /// Canonical lowercase source name — the same string used as a candle's
    /// `source` and persisted in the discovered `pool_registry` artifact.
    pub fn as_source(&self) -> &'static str {
        match self {
            Venue::Soroswap => "soroswap",
            Venue::Aquarius => "aquarius",
            Venue::Phoenix => "phoenix",
        }
    }

    /// Inverse of [`Venue::as_source`] — rehydrate a venue from its persisted
    /// source name. `None` for an unknown string.
    pub fn from_source(s: &str) -> Option<Venue> {
        match s {
            "soroswap" => Some(Venue::Soroswap),
            "aquarius" => Some(Venue::Aquarius),
            "phoenix" => Some(Venue::Phoenix),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SorobanEventRow {
    pub contract_id: String,
    pub transaction_id: String,
    pub ledger_sequence: u64,
    pub event_index: u32,
    pub topics: Vec<TaggedValue>,
    pub data: TaggedValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaggedValue {
    Symbol(String),
    String(String),
    Address(String),
    I128(i128),
    Map(Vec<(TaggedValue, TaggedValue)>),
    Vec(Vec<TaggedValue>),
    Null,
}

impl TaggedValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            TaggedValue::Symbol(s) | TaggedValue::String(s) | TaggedValue::Address(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i128(&self) -> Option<i128> {
        match self {
            TaggedValue::I128(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_address(&self) -> Option<&str> {
        match self {
            TaggedValue::Address(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeRow {
    pub venue: Venue,
    pub contract_id: String,
    pub transaction_id: String,
    pub ledger_sequence: u64,
    pub first_event_index: u32,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: i128,
    pub amount_out: i128,
    pub fee: Option<i128>,
    pub trader: Option<String>,
}

#[derive(Debug)]
pub struct ExtractResult {
    pub trades: Vec<TradeRow>,
    pub rows_consumed: usize,
}

pub trait SwapExtractor {
    fn extract(&self, rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("not enough rows: need {expected}, got {actual}")]
    InsufficientRows { expected: usize, actual: usize },
    #[error("missing field in event group: {0}")]
    MissingField(String),
    #[error("unexpected topic shape in row at event_index {0}")]
    UnexpectedTopicShape(u32),
}

pub type VenueRegistry = HashMap<String, Venue>;
