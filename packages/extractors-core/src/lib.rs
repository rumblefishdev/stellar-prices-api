use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Venue {
    Soroswap,
    Aquarius,
    Phoenix,
    /// SushiSwap V3 — a Uniswap-v3-style concentrated-liquidity venue (task
    /// 0290). Its pool `swap` carries signed `amount0`/`amount1`, the CLMM
    /// shape the `soroswap-extractor` crate already decodes, and it resolves
    /// its tokens through that crate's pair registry exactly as
    /// [`Venue::Soroswap`] does — a separate instance, so the two venues never
    /// share a `contract_id`.
    Sushiswap,
    /// The Comet weighted pool — a Balancer-style pool; the one indexed is
    /// Blend's backstop BLND/USDC (task 0300). It emits one
    /// `[Symbol("POOL"), Symbol("swap")]` per swap with the tokens AND the raw
    /// amounts inline in a data map, so, like [`Venue::Aquarius`], it needs no
    /// pair registry. The pool has no factory, so it is registered through
    /// `prices-ingest-core`'s committed `STATIC_POOLS` list.
    Comet,
}

impl Venue {
    /// Canonical lowercase source name — the same string used as a candle's
    /// `source` and persisted in the discovered `pool_registry` artifact.
    pub fn as_source(&self) -> &'static str {
        match self {
            Venue::Soroswap => "soroswap",
            Venue::Aquarius => "aquarius",
            Venue::Phoenix => "phoenix",
            Venue::Sushiswap => "sushiswap",
            Venue::Comet => "comet",
        }
    }

    /// Inverse of [`Venue::as_source`] — rehydrate a venue from its persisted
    /// source name. `None` for an unknown string.
    pub fn from_source(s: &str) -> Option<Venue> {
        match s {
            "soroswap" => Some(Venue::Soroswap),
            "aquarius" => Some(Venue::Aquarius),
            "phoenix" => Some(Venue::Phoenix),
            "sushiswap" => Some(Venue::Sushiswap),
            "comet" => Some(Venue::Comet),
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
    /// A swap amount that is zero or negative. Zero includes an amount whose
    /// string did not parse: the typed-JSON conversion reads that as 0.
    #[error("non-positive amount in {field}: {value} (0 may be an unparseable value)")]
    NonPositiveAmount { field: String, value: i128 },
}

pub type VenueRegistry = HashMap<String, Venue>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_venue_round_trips_through_its_source_name() {
        for venue in [
            Venue::Soroswap,
            Venue::Aquarius,
            Venue::Phoenix,
            Venue::Sushiswap,
            Venue::Comet,
        ] {
            assert_eq!(Venue::from_source(venue.as_source()), Some(venue));
        }
    }

    #[test]
    fn comet_is_named_comet() {
        // Task 0300 D5: the one literal behind pool_registry.venue, the
        // pre-roll filter and the amm_ticks source tag.
        assert_eq!(Venue::Comet.as_source(), "comet");
        assert_eq!(Venue::from_source("comet"), Some(Venue::Comet));
        assert_eq!(Venue::from_source("Comet"), None);
    }
}
