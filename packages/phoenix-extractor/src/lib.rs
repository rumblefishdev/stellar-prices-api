mod registry;
mod stable;
mod xyk;

pub use registry::{PhoenixPool, PhoenixPoolRegistry};
pub use stable::PhoenixStablePoolExtractor;
pub use xyk::PhoenixXykExtractor;

/// Events in a *fully-populated* XYK swap group — the upper bound, not a
/// requirement. Still the slice cap: a group larger than this is more than one
/// swap, and only the first is consumed (see `PHOENIX_XYK_MIN_EVENT_COUNT`).
pub const PHOENIX_XYK_EVENT_COUNT: usize = 8;

/// Events in the SMALLEST priceable XYK swap group: the four fields
/// [`crate::PhoenixXykExtractor`] actually requires (`sell_token`,
/// `offer_amount`, `buy_token`, `return_amount`). `sender` is optional
/// (`TradeRow::trader` is an `Option`), as are `actual received amount`,
/// `spread_amount` and `referral_fee_amount` — all read and discarded.
///
/// Phoenix omits optional fields, so real swap groups are VARIABLE length.
/// Measured on prod (ledgers 50457424..63352611, 2026-07-17): 237,026 groups of
/// 8 and **5,175 groups of 7** — the 7s drop only `actual received amount` and
/// carry every required field. Gating on `== 8` silently discarded all 5,175
/// (~2.1% of Phoenix swaps) in the backfill AND live. Gate on required-field
/// presence instead; the count is only a floor.
pub const PHOENIX_XYK_MIN_EVENT_COUNT: usize = 4;

pub const PHOENIX_STABLE_EVENT_COUNT: usize = 6;

pub const POOL_TYPE_XYK: u32 = 0;

#[cfg(any(test, feature = "test-fixtures"))]
pub mod test_fixtures;
