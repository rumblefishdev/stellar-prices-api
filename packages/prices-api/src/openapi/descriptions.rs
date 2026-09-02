//! Reader-facing descriptions for the published OpenAPI document.
//!
//! **Why the text is here and not on the types.** `utoipa` copies a doc
//! comment into the document verbatim, and the two audiences want opposite
//! things: a doc comment is written for the next maintainer and carries the
//! implementation history — which task moved a column, which measurement
//! settled a classification, which defect a guard exists for — while the
//! published document is read by an integrator who has none of that context
//! and to whom our task numbers, ADRs and internal section marks mean
//! nothing (and are not ours to publish). Rewriting the comments to suit the
//! second reader would have destroyed what the first one needs, so the
//! document's text lives here instead and the comments stay untouched.
//!
//! This is a [`Modify`] pass, applied when the document is built, so it also
//! covers `bin/extract_openapi`. Operation summaries, parameter and response
//! descriptions do not need it — those are already `#[utoipa::path(...)]`
//! attribute strings, published as written.
//!
//! `every_schema_and_property_is_described` in `tests/openapi.rs` fails if a
//! schema or a field reaches the document without a description, which is
//! what catches a new field, a rename, or an entry left behind here.

use utoipa::Modify;
use utoipa::openapi::{OpenApi, RefOr, Schema};

/// Per-schema descriptions: `(schema, description)`.
pub(super) const SCHEMAS: &[(&str, &str)] = &[
    ("AmmStream", "Progress of the one-shot Soroban AMM import."),
    (
        "AssetDetail",
        "Metadata for one asset. Fields that do not apply to the asset's kind are empty \
         strings.",
    ),
    (
        "AssetListItem",
        "One row of the asset listing: the asset's identity and its current-price snapshot.",
    ),
    ("AssetListResponse", "One page of the asset listing."),
    (
        "BackfillStatus",
        "Progress of the historical backfill streams.",
    ),
    (
        "BaseCurrency",
        "The currency candles are expressed in (`base_currency`). The tokens are uppercase; \
         all-lowercase `usd`/`xlm` are accepted as aliases and any other casing is rejected.",
    ),
    ("BatchRequest", "Request body of `POST /prices/batch`."),
    ("BatchResponse", "Response of `POST /prices/batch`."),
    (
        "Candle",
        "One OHLCV candle, expressed in `base_currency`.\n\nThe price fields — `open`, \
         `high`, `low`, `close` and `vwap` — are `null`, not omitted, on a bucket that traded \
         but has no USD value: either the conversion has not caught up with the newest \
         buckets yet, or the bucket traded only against a quote asset with no USD reference. \
         `volume_base`, `volume_quote_usd` and `trade_count` are always present, so such a \
         bucket still shows its activity.",
    ),
    (
        "ErrorEnvelope",
        "The body of every error response. `code` is stable and meant for programs; `message` \
         is for people and its wording may change.",
    ),
    (
        "Granularity",
        "Candle bucket size. Case-sensitive: `1m` is one minute, `1M` one month.",
    ),
    ("OhlcvResponse", "Candlestick series for one asset."),
    ("OracleEntry", "The most recent reading from one oracle."),
    ("OraclesResponse", "Latest oracle readings for one asset."),
    ("Order", "Sort direction (`order`)."),
    (
        "PriceResponse",
        "Current price snapshot for one asset, computed over the trailing 24-hour window. \
         Every numeric field is a decimal string, so no precision is lost in transport.",
    ),
    (
        "SdexStream",
        "Progress of the SDEX archive stream, which walks the ledger history in order.",
    ),
    (
        "SortCol",
        "Sort column of the asset listing (`sort`). Case-sensitive.",
    ),
    (
        "Timeframe",
        "Window ending now, for the candlestick history. `all` starts at Stellar genesis \
         (2015-09-30).",
    ),
    (
        "TypeFilter",
        "Asset kind filter of the listing (`type`): `classic` (classic assets, including the \
         native asset), `soroban` (Soroban contracts) or `all`.",
    ),
];

/// Per-field descriptions: `(schema, field, description)`.
pub(super) const FIELDS: &[(&str, &str, &str)] = &[
    (
        "AmmStream",
        "completed_at",
        "Time the import finished, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`); `null` while it is \
         still running.",
    ),
    (
        "AmmStream",
        "earliest_data_available",
        "Timestamp of the oldest candle this stream has written so far, ISO 8601 UTC \
         (`YYYY-MM-DDTHH:MM:SSZ`); `null` until the first.",
    ),
    (
        "AmmStream",
        "last_push_at",
        "Time of the most recent successful write, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`); \
         `null` before the first.",
    ),
    (
        "AmmStream",
        "status",
        "`running`, `paused`, `completed` or `error`.",
    ),
    (
        "AssetDetail",
        "asset",
        "The asset as requested, in canonical form: `native`, `CODE:ISSUER` or a contract \
         address.",
    ),
    (
        "AssetDetail",
        "asset_kind",
        "`native` (XLM), `credit` (a classic issued asset) or `contract` (a Soroban token or \
         SAC).",
    ),
    (
        "AssetDetail",
        "code",
        "Asset code of a classic asset; `\"\"` for `native` and `contract`.",
    ),
    (
        "AssetDetail",
        "contract",
        "Contract address (`C…`) of a Soroban asset; `\"\"` otherwise.",
    ),
    (
        "AssetDetail",
        "home_domain",
        "The issuer's home domain (SEP-1); `\"\"` when unknown.",
    ),
    (
        "AssetDetail",
        "is_active",
        "Whether the asset is currently marked active in the registry.",
    ),
    (
        "AssetDetail",
        "issuer",
        "Issuer public key (`G…`) of a classic asset; `\"\"` otherwise.",
    ),
    (
        "AssetListItem",
        "asset_code",
        "Asset code; `XLM` for the native asset. Empty for a Soroban asset whose token symbol \
         has not been resolved yet.",
    ),
    (
        "AssetListItem",
        "asset_type",
        "`classic` (including the native asset) or `soroban` — the vocabulary of the `type` \
         filter.",
    ),
    (
        "AssetListItem",
        "change_24h_pct",
        "Percentage change of `price_usd` against the oldest priced close in the trailing \
         24-hour window; `\"0\"` without a baseline.",
    ),
    (
        "AssetListItem",
        "change_7d_pct",
        "Percentage change of `price_usd` against the oldest priced close between 7 and 5 \
         days ago; `\"0\"` when there is no baseline in that band.",
    ),
    (
        "AssetListItem",
        "contract_address",
        "Contract address (`C…`) of a Soroban asset; `\"\"` otherwise.",
    ),
    (
        "AssetListItem",
        "home_domain",
        "The issuer's home domain (SEP-1); `\"\"` when unknown.",
    ),
    (
        "AssetListItem",
        "issuer_address",
        "Issuer public key (`G…`) of a classic asset; `\"\"` for the native asset and for \
         Soroban assets.",
    ),
    (
        "AssetListItem",
        "method",
        "How `price_usd` was obtained: `traded`, `oracle` or `\"\"` (unavailable). Same \
         meaning as `PriceResponse.method`.",
    ),
    (
        "AssetListItem",
        "price_usd",
        "Latest USD price for the asset; `\"0\"` when none is available. Same meaning as \
         `PriceResponse.price_usd`, including the `method` it is attributed to.",
    ),
    (
        "AssetListItem",
        "sources",
        "Per-venue breakdown, same shape and rules as `PriceResponse.sources`, including the \
         `min_volume_usd` override.",
    ),
    (
        "AssetListItem",
        "updated_at",
        "Time of the snapshot, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`).",
    ),
    (
        "AssetListItem",
        "volume_24h_usd",
        "Trailing 24-hour USD volume across all venues — a total, never filtered.",
    ),
    (
        "AssetListItem",
        "vwap_24h",
        "Trailing 24-hour volume-weighted average USD price across venues, with the volume \
         threshold and the outlier filter applied. Same meaning as `PriceResponse.vwap_24h`.",
    ),
    (
        "AssetListResponse",
        "cursor",
        "Opaque cursor for the next page; `null` on the last page. Pass it as `cursor` with \
         the same `sort` and `order`.",
    ),
    (
        "AssetListResponse",
        "data",
        "The page's rows, in the requested order.",
    ),
    (
        "AssetListResponse",
        "has_more",
        "Whether a further page exists.",
    ),
    (
        "BackfillStatus",
        "realtime_tip_ledger",
        "Approximate current ledger sequence of the network — the SDEX stream's \
         `target_ledger`; `0` when that stream has not reported.",
    ),
    (
        "BackfillStatus",
        "sdex",
        "The SDEX archive stream; absent if it has never reported.",
    ),
    (
        "BackfillStatus",
        "soroban_amm",
        "The Soroban AMM import; absent if it has never reported.",
    ),
    (
        "BatchRequest",
        "assets",
        "Asset identifiers — `native`, `CODE:ISSUER` or a contract address — 1 to 100 of \
         them. A duplicated identifier is answered each time.",
    ),
    (
        "BatchResponse",
        "not_found",
        "Identifiers with no current price, in canonical form and in the order of the \
         request.",
    ),
    (
        "BatchResponse",
        "prices",
        "Current prices for the assets that have one, in the order of the request.",
    ),
    (
        "Candle",
        "close",
        "Closing price of the bucket, in `base_currency`; `null` when the bucket has no \
         price. Exact in USD mode — the other price fields are scaled, see `derived`.",
    ),
    (
        "Candle",
        "derived",
        "Whether `open`, `high`, `low` and `vwap` were derived by scaling rather than \
         measured. Normally `close` is exact and the other price fields are scaled from the \
         quote-asset values with one rate per bucket, so the true USD high may have occurred \
         at a different moment than the quote-asset high. For USDC, which has no trades of \
         its own, every price field comes from the rate and `derived` is `true`.\n\n`null` \
         when the price fields are `null`, and always `null` for `base_currency=XLM`.",
    ),
    (
        "Candle",
        "high",
        "Highest price in the bucket, in `base_currency`; `null` when the bucket has no \
         price.",
    ),
    (
        "Candle",
        "low",
        "Lowest price in the bucket, in `base_currency`; `null` when the bucket has no price.",
    ),
    (
        "Candle",
        "method",
        "Where the USD rate behind this bucket came from:\n\n* `peg` — no measured rate was \
         available; USDC was taken at 1 USD.\n* `oracle` — a measured oracle reading.\n* \
         `traded` — priced through a reference asset's own trades.\n\n`null` when the price \
         fields are `null`, and always `null` for `base_currency=XLM`, where nothing is \
         converted.",
    ),
    (
        "Candle",
        "open",
        "Opening price of the bucket, in `base_currency`; `null` when the bucket has no \
         price.",
    ),
    (
        "Candle",
        "timestamp",
        "Bucket start, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`).",
    ),
    (
        "Candle",
        "trade_count",
        "Number of trades in the bucket. The documented maximum, 2^53 − 1, is the largest \
         integer JSON carries exactly — a transport limit, not a domain one.",
    ),
    (
        "Candle",
        "volume_base",
        "Volume in units of the asset — the base side of each trade.",
    ),
    (
        "Candle",
        "volume_quote_usd",
        "USD volume of the bucket, summed over the trades that have a USD value. A trade not \
         yet priced counts in `volume_base` and `trade_count` but not here, so this is the \
         USD volume that can be accounted for, not the bucket's total restated in USD.",
    ),
    (
        "Candle",
        "vwap",
        "Volume-weighted average price of the bucket, in `base_currency`; `null` when the \
         bucket has no price.",
    ),
    (
        "ErrorEnvelope",
        "code",
        "Machine-readable error code: `invalid_id`, `invalid_query`, `invalid_body`, \
         `not_found`, `unauthorized`, `db_error` or `quote_unavailable`.",
    ),
    (
        "ErrorEnvelope",
        "details",
        "Optional structured context; omitted when absent.",
    ),
    (
        "ErrorEnvelope",
        "message",
        "Human-readable explanation; the wording may change.",
    ),
    (
        "OhlcvResponse",
        "asset",
        "The asset as requested, in canonical form: `native`, `CODE:ISSUER` or a contract \
         address.",
    ),
    (
        "OhlcvResponse",
        "backfill_note",
        "Present only for `timeframe=all` while the historical backfill is still running: \
         names the earliest date available so far and points to `GET /backfill/status`.",
    ),
    (
        "OhlcvResponse",
        "base_currency",
        "`USD` or `XLM` — the currency the candles are expressed in.",
    ),
    (
        "OhlcvResponse",
        "data",
        "The candles, in ascending time order.",
    ),
    (
        "OhlcvResponse",
        "granularity",
        "The bucket size actually used — the request's `granularity`, or the one selected \
         from the window.",
    ),
    (
        "OracleEntry",
        "name",
        "Oracle name, for example `reflector`.",
    ),
    (
        "OracleEntry",
        "price_usd",
        "Latest USD price reported by the oracle, as a decimal string.",
    ),
    (
        "OracleEntry",
        "updated_at",
        "Time of that reading, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`).",
    ),
    (
        "OraclesResponse",
        "asset",
        "The asset as requested, in canonical form: `native`, `CODE:ISSUER` or a contract \
         address.",
    ),
    (
        "OraclesResponse",
        "oracles",
        "One entry per oracle, ordered by name; empty when no oracle has a reading for the \
         asset.",
    ),
    (
        "PriceResponse",
        "asset",
        "The asset as requested, in canonical form: `native`, `CODE:ISSUER` or a contract \
         address.",
    ),
    (
        "PriceResponse",
        "change_24h_pct",
        "Percentage change of `price_usd` against the oldest priced close in the trailing \
         24-hour window. `\"0\"` when there is no baseline to compare against.",
    ),
    (
        "PriceResponse",
        "method",
        "How `price_usd` was obtained:\n\n* `traded` — aggregated from the asset's own \
         trades.\n* `oracle` — taken from an oracle rate, for an asset that never trades as \
         the base of a market and so has no candles of its own (currently only USDC).\n* \
         `\"\"` — unavailable: no priced trade in the window, so `price_usd` is \
         `\"0\"`.\n\n`oracle` does not mean \"more accurate than traded\"; it means the price \
         came from a rate rather than from this asset's trades.",
    ),
    (
        "PriceResponse",
        "price_usd",
        "Latest USD price for the asset: its own last priced close in the trailing \
         24-hour window, or — for an asset that never trades as the base of a market — a \
         rate. `method` says which. `\"0\"` means neither was available.\n\nThe value is not \
         age-bounded: for an asset that has stopped trading it is the last close inside \
         the window, up to 24 hours old. `updated_at` is the time of the snapshot, not \
         the age of the price.",
    ),
    (
        "PriceResponse",
        "price_xlm",
        "`price_usd` expressed in XLM: `price_usd` divided by the latest XLM/USD close. \
         Shares the `\"0\"` sentinel. The two closes are dated independently, so this is not \
         a price at any single instant.",
    ),
    (
        "PriceResponse",
        "sources",
        "Per-venue breakdown, keyed by venue name (for example `sdex`, `soroswap`, \
         `aquarius`), each `{\"price\": \"…\", \"volume_24h\": \"…\"}` as decimal strings. A \
         venue is absent when the volume threshold or the outlier filter excluded it, or when \
         it has no USD-priced close in the window. `{}` means no venue qualified; it is not \
         an error.",
    ),
    (
        "PriceResponse",
        "updated_at",
        "Time of the snapshot, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`).",
    ),
    (
        "PriceResponse",
        "volume_24h_usd",
        "Trailing 24-hour USD volume across all venues — a total, never reduced by the volume \
         threshold or the outlier filter.",
    ),
    (
        "PriceResponse",
        "vwap_24h",
        "Trailing 24-hour volume-weighted average USD price across venues. Venues at or below \
         the volume threshold (100 USD by default — conditional: a low-volume venue is kept \
         when no venue on the asset clears it) and venues whose price is an outlier against \
         the cross-venue median are excluded from the weighting. `min_volume_usd` re-weights \
         it with a caller-supplied threshold.",
    ),
    (
        "SdexStream",
        "current_ledger",
        "Last ledger sequence ingested.",
    ),
    (
        "SdexStream",
        "earliest_data_available",
        "Timestamp of the oldest candle this stream has written so far, ISO 8601 UTC \
         (`YYYY-MM-DDTHH:MM:SSZ`); `null` until the first. Moves backwards as older history \
         is ingested.",
    ),
    (
        "SdexStream",
        "last_push_at",
        "Time of the most recent successful write, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`); \
         `null` before the first.",
    ),
    (
        "SdexStream",
        "ledgers_remaining",
        "`target_ledger − current_ledger`.",
    ),
    (
        "SdexStream",
        "progress_pct",
        "Share of the ledger span consumed, in percent: `(current_ledger − start_ledger) / \
         (target_ledger − start_ledger) × 100`; `0` while the span is unknown.",
    ),
    (
        "SdexStream",
        "start_ledger",
        "First ledger sequence of this run.",
    ),
    (
        "SdexStream",
        "status",
        "`running`, `paused`, `completed` or `error`.",
    ),
    (
        "SdexStream",
        "target_ledger",
        "Ledger sequence the run aims for.",
    ),
];

/// Sets [`SCHEMAS`] and [`FIELDS`] on the built document.
pub(crate) struct Descriptions;

/// Write `description` into whichever schema variant this is. A `$ref` takes
/// one too: OpenAPI 3.1 lets a reference carry its own description, and that
/// is how an optional reference (`Option<T>`, emitted as a one-of) says what
/// the FIELD means rather than what the referenced component is.
fn describe(schema: &mut RefOr<Schema>, text: &str) {
    match schema {
        RefOr::Ref(r) => r.description = text.to_string(),
        RefOr::T(Schema::Object(o)) => o.description = Some(text.to_string()),
        RefOr::T(Schema::Array(a)) => a.description = Some(text.to_string()),
        RefOr::T(Schema::OneOf(o)) => o.description = Some(text.to_string()),
        RefOr::T(Schema::AllOf(a)) => a.description = Some(text.to_string()),
        RefOr::T(Schema::AnyOf(a)) => a.description = Some(text.to_string()),
        _ => {}
    }
}

impl Modify for Descriptions {
    fn modify(&self, openapi: &mut OpenApi) {
        let Some(components) = openapi.components.as_mut() else {
            return;
        };
        for (name, text) in SCHEMAS {
            if let Some(schema) = components.schemas.get_mut(*name) {
                describe(schema, text);
            }
        }
        for (name, field, text) in FIELDS {
            let Some(RefOr::T(Schema::Object(object))) = components.schemas.get_mut(*name) else {
                continue;
            };
            if let Some(property) = object.properties.get_mut(*field) {
                describe(property, text);
            }
        }
    }
}
