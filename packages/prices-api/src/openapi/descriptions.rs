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
//! `every_published_text_is_present_and_reader_facing` in `tests/openapi.rs`
//! fails if a schema or a field reaches the document without a description, or
//! carries one written for a maintainer rather than an integrator. That is
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
        "One OHLCV candle, expressed in `base_currency`.\n\nThe prices come only from the \
         bucket's own price-forming trades: `open` and `close` are the first and last of \
         them, `high` and `low` their extremes. Nothing is carried over from a neighbouring \
         bucket, so a period that traded only in amounts too small to carry a price has no \
         price at all — see `pf_trade_count`.\n\nThe price fields — `open`, `high`, `low`, \
         `close`, `vwap` and `pf_vwap` — are `null`, not omitted, on such a bucket and on one \
         that has no USD value: either the conversion has not caught up with the newest \
         buckets yet, or the bucket traded only against a quote asset with no USD reference. \
         `volume_base`, `volume_quote_usd` and `trade_count` are always present, so such a \
         bucket still shows its activity.",
    ),
    (
        "ErrorEnvelope",
        "The body of every error the API itself returns. `code` is stable and meant for \
         programs; `message` is for people and its wording may change. Responses the API \
         Gateway writes on the API's behalf — `403` for a missing or unknown key, `429` when \
         throttled — carry a `GatewayMessage` instead.",
    ),
    (
        "Granularity",
        "Candle bucket size. Case-sensitive: `1m` is one minute, `1M` one month.",
    ),
    (
        "GatewayMessage",
        "The body API Gateway answers with when it handles a request itself: `403` for a \
         missing, unknown or disabled `x-api-key`, `429` when throttled, and some `5xx`. It \
         has no `code`.",
    ),
    (
        "HealthStatus",
        "Body of `GET /health`, answered at the edge.",
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
    ("SourceQuote", "One venue's entry in `sources`."),
    (
        "SdexStream",
        "Progress of the SDEX archive stream, which walks the ledger history BACKWARD, \
         from the chain tip toward genesis.",
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
        "`running`, `paused`, `completed`, `error`, or `stalled`. `stalled` is \
         derived at read time, not stored: a stream still recorded as \
         `running` whose last push is more than 7 days old is reported as \
         `stalled`, because nothing writes a terminal state when a run is \
         killed. `paused` is the normal resting state of a finished run \
         that stopped at its planned end rather than at the chain tip.",
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
        "`native` (XLM), `credit` (a classic issued asset) or `contract` (a Soroban token).",
    ),
    (
        "AssetDetail",
        "code",
        "Display code: `XLM` for `native`, the asset code for `credit`, the token's own \
         `symbol()` for `contract` — `\"\"` until that symbol is resolved. Not an identifier: \
         codes are not unique, and a token's symbol is whatever its contract declares.",
    ),
    (
        "AssetDetail",
        "contract",
        "Contract address (`C…`) of a Soroban asset; `\"\"` otherwise.",
    ),
    (
        "AssetDetail",
        "home_domain",
        "The issuer's home domain (SEP-1). Not populated yet: `\"\"` for every asset \
         today.",
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
        "The issuer's home domain (SEP-1). Not populated yet: `\"\"` for every asset \
         today.",
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
         `PriceResponse.price_usd`, including the `method` it is attributed to and the \
         decimal-string form and its precision rationale.",
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
        "Trailing 24-hour USD volume of every trade the asset took part in, on either side of \
         the pair, across all venues — a total, never filtered. Same meaning as \
         `PriceResponse.volume_24h_usd`, and likewise larger than the sum of `sources`.",
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
        "Current ledger sequence of the network, from the live ingest cursor, \
         which advances every batch. Falls back to the SDEX stream's \
         `target_ledger` only while that cursor is unset, and is `0` when \
         neither is available. It was previously read from `target_ledger` \
         alone, which is not a chain tip: the backfill rewrites that column \
         when it pushes, so it freezes when the backfill stops.",
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
        "Closing price of the bucket, in `base_currency`: the LAST price-forming trade of \
         this bucket, never one carried over from an earlier one. `null` when the bucket has \
         no price — see `pf_trade_count`. Exact in USD mode — the other price fields are \
         scaled, see `derived`.",
    ),
    (
        "Candle",
        "close_divergent",
        "Whether `close` sits more than 1% away from `pf_vwap`. The close is a SINGLE trade \
         — the last one that formed a price — while `pf_vwap` is the whole bucket's \
         price-forming mean, so a wide gap marks a thin or one-sided bucket rather than an \
         error: the close is still what it last traded at, and a poor summary of what it is \
         worth. Prefer `pf_vwap` when ranking or valuing, and `close` when charting the last \
         price.\n\n`null` when either value is `null` — nothing was compared, which is not \
         the same as the two agreeing.",
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
        "Highest price among this bucket's price-forming trades, in `base_currency`; `null` \
         when the bucket has no price — see `pf_trade_count`.",
    ),
    (
        "Candle",
        "low",
        "Lowest price among this bucket's price-forming trades, in `base_currency`; `null` \
         when the bucket has no price — see `pf_trade_count`.",
    ),
    (
        "Candle",
        "method",
        "Where the USD rate behind this bucket came from:\n\n* `assumed-par` — nothing was \
         measured; the literal 1.0 supplied the value, i.e. USDC was taken at 1 USD.\n* \
         `external` — an imported, measured USDC/USD series supplied the rate.\n* `oracle` — \
         a measured Reflector reading supplied the rate.\n* `traded` — priced through a \
         reference asset's own trades.\n* `peg` — only on the synthesized USDC self-series \
         (`GET /assets/USDC:<issuer>/ohlcv`): no measured USDC/USD observation covered the \
         bucket, so the $1 fallback was rendered. Never appears on a quote leg — there the \
         same situation is `assumed-par`.\n\nEach value names the INPUT the rate came from, so \
         `assumed-par` and `external` are never interchangeable: one is an assumption, the \
         other a measurement that may sit percent off par.\n\nOn the USDC self-series a \
         bucket that holds both a Reflector reading and an imported one reports `oracle`: a \
         measured poll outranks an imported rate outright, whichever was observed first. \
         Observation time only breaks ties between rows of the same kind. The imported \
         series is loaded at HOURLY grain, so an \
         hourly request over an imported day reports `external` — and that hour's own \
         measured rate — for each of its twenty-four hours; on 2023-03-11 the 07:00 bucket \
         reports the trough rather than the day's close. Where only a daily row exists for \
         a day, that one row prices every bucket of its UTC day at every `granularity`, so \
         an imported day mixes the two only where the oracle epoch falls inside it \
         (2026-03-11): a bucket that extends past 14:00 UTC that day is not priced from \
         the imported series, which ends at 13:00. The self-series carries the rate row's \
         own `method`, so a measured rate that reads exactly 1.0 is `external` there.\n\nOn a \
         quote leg the label is reconstructed at read time — the candle rows carry no \
         provenance column — so one case is deliberately not separated: a bucket reading \
         exactly 1.0 reports `assumed-par` whether the dollar was assumed or a measured \
         rate happened to land on it. The two leave an identical row and carry the same \
         number, so the label is the conservative one; `external` is reported only for a \
         bucket whose value was actually scaled by an imported rate.\n\n`null` in three \
         cases: when the price fields are `null`; always for \
         `base_currency=XLM`, where nothing is converted; and for a USDC-quoted bucket \
         below the oracle epoch that carries a converted rate no imported series covers \
         — nothing can attribute it, and a label would be a claim.",
    ),
    (
        "Candle",
        "open",
        "Opening price of the bucket, in `base_currency`: the FIRST price-forming trade of \
         this bucket. `null` when the bucket has no price — see `pf_trade_count`.",
    ),
    (
        "Candle",
        "pf_trade_count",
        "How many of the bucket's trades formed its price.\n\nA trade's price is the ratio \
         of the two integer amounts exchanged, so a trade of a few of the smallest \
         representable units prints an exact small fraction — 1/17, 5/34 — that is \
         arithmetically correct and can sit hundreds of percent off the market. Those trades \
         are not price-forming, and `open`, `high`, `low` and `close` are taken only from \
         the ones that are. Neither is a trade whose price falls below 0.000000000001 — one \
         hundred times the smallest value the price fields can hold, below which a stored \
         price is rounding noise rather than a measurement. The same floor applies at every \
         `interval`.\n\n`0` is a real answer and the one worth acting on: the bucket \
         traded, its `volume_base` and `trade_count` are reported, and it has NO price — \
         every price field is `null` rather than carrying a dust print. Always at most \
         `trade_count`; `null` only on the synthesized USDC self-series \
         (`GET /assets/USDC:<issuer>/ohlcv`), which is built from rate observations and has \
         no trades to count.",
    ),
    (
        "Candle",
        "pf_vwap",
        "Volume-weighted average price of the bucket's price-forming trades, in \
         `base_currency` — `vwap` with the dust left out, and the value `close_divergent` \
         compares `close` against.\n\nDenominated exactly like `close` and bounded by the \
         published `low` and `high`, so the response stays self-consistent. `null` when the \
         bucket has no price-forming volume, and on the synthesized USDC self-series. Never \
         `0`.",
    ),
    (
        "Candle",
        "quality",
        "How confident the imported series is in that day's observation. Present only \
         alongside `source`, i.e. only on the synthesized USDC self-series \
         (`GET /assets/USDC:<issuer>/ohlcv`) for buckets whose `method` is `external`; \
         `null` on every other asset, on the `peg` fallback, and whenever the price fields \
         are `null`.\n\n* `measured` — a real observation from the primary feed.\n* \
         `measured-disputed` — observed, but a cross-check against a second independent \
         source disagreed by more than the composer's spread tolerance. The number is real; \
         treat it as less certain than a plain `measured` day and prefer not to alert on it \
         alone.\n* `fallback` — the primary feed had nothing for that day, so the secondary \
         source supplied it. Still an observation, and far better than assuming $1, but a \
         different instrument on a different venue.\n\nA day with no quality is not a day of \
         unknown quality — the field simply does not apply outside the imported series.",
    ),
    (
        "Candle",
        "source",
        "Which outside USD series the imported rate came from: `chainlink` or `bitstamp`.\n\n\
         Present only on the synthesized USDC self-series \
         (`GET /assets/USDC:<issuer>/ohlcv`), and there only for buckets whose `method` is \
         `external`. It is `null` on every other asset — not because those candles lack \
         provenance, but because the stored candles carry no column to report it from — \
         `null` on an `oracle` bucket (a poll has no outside series, even where an outranked \
         import shares the bucket), `null` on the `peg` fallback, where no series was \
         consulted at all, and whenever the price fields are `null`. Never the empty \
         string.",
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
        "Volume in units of the asset — the base side of each trade, over EVERY trade in \
         the bucket including the ones too small to form a price. It is therefore a \
         complete activity figure and not a price-quality signal: `pf_trade_count` is the \
         one to read for that, and a bucket that traded only in such amounts reports its \
         volume here with no price at all.\n\nVolume is undistorted by those trades — \
         they carry almost none — which is why it stays unfiltered where the price fields \
         do not.",
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
        "Volume-weighted average price of the bucket, in `base_currency`, over the trades it \
         holds — including the ones too small to form a price. `pf_vwap` is the same mean \
         with those left out. `null` when the bucket has no price.\n\nWHICH trades it \
         weighs depends on the denomination, so the two can differ on a bucket that holds a \
         source with no usable price. With `base_currency=XLM` it is every row of the \
         bucket, as stored. With `USD` it is every row that has a usable rate to convert it \
         with — a row whose own price is below the precision floor, or that has no USD value \
         yet, cannot be restated in USD and is left out of the mean rather than counted at \
         zero.\n\nBounded by the published `low` and `high` so the response stays \
         self-consistent. Those come from the price-forming trades only, so on a bucket \
         where the trades that formed no price carry most of the volume the raw mean falls \
         outside that range and the value reported is the nearer bound. Compare it with \
         `pf_vwap` when the difference matters.",
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
        "GatewayMessage",
        "message",
        "The gateway's own text, for example `Forbidden` or `Too Many Requests`. A `403` \
         reading `Missing Authentication Token` means the path does not exist, not that the \
         key is wrong.",
    ),
    ("HealthStatus", "stack", "The deployment that answered."),
    ("HealthStatus", "status", "`ok` whenever the API is up."),
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
         the age of the price.\n\n**A decimal string, not a JSON number.** Prices are \
         `Decimal(38, 14)` and a JSON number is an IEEE-754 double in every mainstream \
         parser — ~15-16 significant digits against the 19 a five-figure price at this \
         scale carries, so a float round-trip silently drops low-order digits. Assets on \
         this store trade as low as 7e-8, where those digits are the price. Parse with a \
         decimal type; `parseFloat` still works if a float is genuinely wanted, but the \
         reverse is not recoverable.",
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
        "Per-venue breakdown, keyed by venue name (`aquarius`, `phoenix`, `sdex`, \
         `soroswap`, `sushiswap`), each a `SourceQuote`. A venue's `volume_24h` counts only \
         the trades where this asset is the one being priced — the base of the pair — so the \
         entries add up to less than `volume_24h_usd`, which counts both sides. A venue is \
         absent when the volume threshold or the outlier filter excluded it, or when it has no \
         USD-priced close in the window. `{}` means no venue qualified — always so for an \
         asset priced by `oracle` — and is not an error.",
    ),
    (
        "PriceResponse",
        "updated_at",
        "Time of the snapshot, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`).",
    ),
    (
        "PriceResponse",
        "volume_24h_usd",
        "Trailing 24-hour USD volume of every trade the asset took part in, on either side of \
         the pair, across all venues — a total, never reduced by the volume threshold or the \
         outlier filter. It is therefore larger than the sum of `sources`, which counts one \
         side: XLM, quoted against most assets, shows the gap most; USDC, which only ever \
         quotes, has all of its volume here and `{}` in `sources`.",
    ),
    (
        "PriceResponse",
        "vwap_24h",
        "Trailing 24-hour volume-weighted average USD price across venues. Venues at or below \
         the volume threshold (100 USD by default — conditional: a low-volume venue is kept \
         when no venue on the asset clears it) and venues whose price is an outlier against \
         the cross-venue median are excluded from the weighting. `min_volume_usd` re-weights \
         it with a caller-supplied threshold. `\"0\"` when no venue qualifies, as for an asset \
         priced by `oracle`.",
    ),
    (
        "SourceQuote",
        "price",
        "The venue's latest USD price for the asset, as a decimal string.",
    ),
    (
        "SourceQuote",
        "volume_24h",
        "The venue's trailing 24-hour USD volume for the asset, counted where the asset is \
         the base of the trade.",
    ),
    (
        "SdexStream",
        "current_ledger",
        "The OLDEST ledger sequence reflected so far. This stream walks backward from the \
         chain tip toward genesis, so it descends over the life of the run and reaches \
         `start_ledger` when the archive is complete — it is not the newest ledger ingested. \
         Caveat: it is the lowest completed run *start*, so it asserts a floor, not proven \
         contiguous coverage between that floor and `target_ledger`.",
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
        "`current_ledger − start_ledger` — how far the archive's floor still sits above \
         genesis. This stream walks backward, so what remains is below `current_ledger`, \
         not above it.",
    ),
    (
        "SdexStream",
        "progress_pct",
        "Share of the ledger span covered, in percent: `(target_ledger − current_ledger) / \
         (target_ledger − start_ledger) × 100`. The SDEX archive walks backward from the tip \
         toward genesis, so `current_ledger` is the OLDEST ledger reflected and the covered \
         span is `[current_ledger, target_ledger]`; `0` while the span is unknown or nothing \
         has been reflected yet.",
    ),
    (
        "SdexStream",
        "start_ledger",
        "Low end of the span this stream covers — genesis (`1`) for the SDEX archive. Because \
         the stream walks backward it is where the walk *ends*, not where it begins.",
    ),
    (
        "SdexStream",
        "status",
        "`running`, `paused`, `completed`, `error`, or `stalled`. `stalled` is \
         derived at read time, not stored: a stream still recorded as \
         `running` whose last push is more than 7 days old is reported as \
         `stalled`, because nothing writes a terminal state when a run is \
         killed. `paused` is the normal resting state of a finished run \
         that stopped at its planned end rather than at the chain tip.",
    ),
    (
        "SdexStream",
        "target_ledger",
        "Ledger sequence the run aims for.",
    ),
];

/// Per-field examples: `(schema, field, JSON)`, taken from production
/// responses on 2026-09-23 (task 0306).
///
/// On the fields rather than as one object per schema, for two reasons. The
/// portal assembles a schema's example from its properties in document order,
/// which `preserve_order` makes struct order — the order serde writes — while
/// a whole-object example is a `serde_json::Map`, sorted alphabetically here.
/// And a field-level example is what an array or a reference to the schema is
/// built from, so one entry serves every place the schema appears.
///
/// A field left out is left out of the rendered example too, as the API
/// leaves it out: `ErrorEnvelope.details` and `OhlcvResponse.backfill_note`
/// are omitted when absent. Fields that are references (`BackfillStatus.sdex`,
/// `OhlcvResponse.granularity`, the arrays of objects) take their example from
/// the referenced schema. `every_property_has_an_example_or_a_reason` in
/// `tests/openapi.rs` holds that to the document.
pub(super) const EXAMPLES: &[(&str, &str, &str)] = &[
    ("AmmStream", "status", r#""paused""#),
    ("AmmStream", "last_push_at", r#""2026-07-14T17:54:24Z""#),
    ("AmmStream", "completed_at", r#"null"#),
    (
        "AmmStream",
        "earliest_data_available",
        r#""2024-03-08T19:00:00Z""#,
    ),
    (
        "AssetDetail",
        "asset",
        r#""USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN""#,
    ),
    ("AssetDetail", "asset_kind", r#""credit""#),
    ("AssetDetail", "code", r#""USDC""#),
    (
        "AssetDetail",
        "issuer",
        r#""GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN""#,
    ),
    ("AssetDetail", "contract", r#""""#),
    ("AssetDetail", "home_domain", r#""""#),
    ("AssetDetail", "is_active", r#"true"#),
    ("AssetListItem", "asset_code", r#""AQUA""#),
    ("AssetListItem", "asset_type", r#""classic""#),
    (
        "AssetListItem",
        "issuer_address",
        r#""GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA""#,
    ),
    ("AssetListItem", "contract_address", r#""""#),
    ("AssetListItem", "home_domain", r#""""#),
    ("AssetListItem", "price_usd", r#""0.00037300596178""#),
    ("AssetListItem", "change_24h_pct", r#""2.124""#),
    ("AssetListItem", "change_7d_pct", r#""11.997""#),
    (
        "AssetListItem",
        "volume_24h_usd",
        r#""479235.22108659319489""#,
    ),
    ("AssetListItem", "vwap_24h", r#""0.00037306538294""#),
    (
        "AssetListItem",
        "sources",
        r#"{"aquarius": {"price": "0.00037307545161", "volume_24h": "409795.80206351425926"}, "sdex": {"price": "0.00037300596178", "volume_24h": "69438.16887823331656"}}"#,
    ),
    ("AssetListItem", "updated_at", r#""2026-09-23T08:08:00Z""#),
    ("AssetListItem", "method", r#""traded""#),
    (
        "AssetListResponse",
        "cursor",
        r#""eyJ2IjoiMTU3MC45MDI4NTIwMDAxNzU5MyIsImlkIjo4N30""#,
    ),
    ("AssetListResponse", "has_more", r#"true"#),
    ("BackfillStatus", "realtime_tip_ledger", r#"64573020"#),
    (
        "BatchRequest",
        "assets",
        r#"["native", "FOO:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"]"#,
    ),
    (
        "BatchResponse",
        "not_found",
        r#"["FOO:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"]"#,
    ),
    ("Candle", "timestamp", r#""2026-09-22T08:15:00Z""#),
    ("Candle", "open", r#""0.21155612364244""#),
    ("Candle", "high", r#""0.21336358305927""#),
    ("Candle", "low", r#""0.21019557835832""#),
    ("Candle", "close", r#""0.2104557806765""#),
    ("Candle", "volume_base", r#""256744.7651102""#),
    ("Candle", "volume_quote_usd", r#""54161.26730887106528""#),
    ("Candle", "vwap", r#""0.2109568287069""#),
    ("Candle", "trade_count", r#"666"#),
    ("Candle", "method", r#""oracle""#),
    ("Candle", "derived", r#"true"#),
    ("Candle", "pf_trade_count", r#"666"#),
    ("Candle", "pf_vwap", r#""0.21095682870692""#),
    ("Candle", "close_divergent", r#"false"#),
    ("Candle", "source", r#"null"#),
    ("Candle", "quality", r#"null"#),
    ("ErrorEnvelope", "code", r#""not_found""#),
    ("ErrorEnvelope", "message", r#""unknown asset""#),
    ("GatewayMessage", "message", r#""Forbidden""#),
    ("HealthStatus", "status", r#""ok""#),
    ("HealthStatus", "stack", r#""prices-production""#),
    ("OhlcvResponse", "asset", r#""native""#),
    ("OracleEntry", "name", r#""reflector""#),
    ("OracleEntry", "price_usd", r#""0.2180727167089""#),
    ("OracleEntry", "updated_at", r#""2026-09-23T08:05:00Z""#),
    ("OraclesResponse", "asset", r#""native""#),
    ("PriceResponse", "asset", r#""native""#),
    ("PriceResponse", "price_usd", r#""0.22086251378147""#),
    ("PriceResponse", "price_xlm", r#""1""#),
    ("PriceResponse", "vwap_24h", r#""0.220818422853""#),
    (
        "PriceResponse",
        "volume_24h_usd",
        r#""9232178.49610106508283""#,
    ),
    ("PriceResponse", "change_24h_pct", r#""4.2307""#),
    (
        "PriceResponse",
        "sources",
        r#"{"aquarius": {"price": "0.22086251378147", "volume_24h": "3496887.57491671686026"}, "phoenix": {"price": "0.21951246345991", "volume_24h": "143.33960639826163"}, "sdex": {"price": "0.22080609088657", "volume_24h": "3706994.74575814385738"}, "soroswap": {"price": "0.22083791103349", "volume_24h": "12137.72993461573015"}, "sushiswap": {"price": "0.21972140395799", "volume_24h": "98918.83560484752044"}}"#,
    ),
    ("PriceResponse", "updated_at", r#""2026-09-23T08:08:00Z""#),
    ("PriceResponse", "method", r#""traded""#),
    ("SdexStream", "status", r#""completed""#),
    ("SdexStream", "current_ledger", r#"1"#),
    ("SdexStream", "start_ledger", r#"1"#),
    ("SdexStream", "target_ledger", r#"63795749"#),
    ("SdexStream", "progress_pct", r#"100.0"#),
    ("SdexStream", "ledgers_remaining", r#"0"#),
    ("SdexStream", "last_push_at", r#""2026-08-11T02:43:32Z""#),
    (
        "SdexStream",
        "earliest_data_available",
        r#""2015-11-18T03:47:00Z""#,
    ),
    ("SourceQuote", "price", r#""0.22080609088657""#),
    ("SourceQuote", "volume_24h", r#""3706994.74575814385738""#),
];

/// Whole-schema examples, for the few schemas a field cannot speak for: a
/// field that references an enum cannot carry an example of its own, so the
/// portal falls back to the enum's first value — `1m` beside a window whose
/// default bucket is `15m`.
pub(super) const SCHEMA_EXAMPLES: &[(&str, &str)] = &[("Granularity", r#""15m""#)];

/// Sets [`SCHEMAS`], [`FIELDS`], [`EXAMPLES`] and [`SCHEMA_EXAMPLES`] on the
/// built document.
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

/// Write `example` into whichever schema variant this is. A `$ref` cannot
/// carry one (OpenAPI 3.1 allows only `summary` and `description` beside a
/// reference), which is why [`EXAMPLES`] names no reference fields.
fn exemplify(schema: &mut RefOr<Schema>, example: serde_json::Value) -> bool {
    match schema {
        RefOr::T(Schema::Object(o)) => o.example = Some(example),
        RefOr::T(Schema::Array(a)) => a.example = Some(example),
        RefOr::T(Schema::OneOf(o)) => o.example = Some(example),
        _ => return false,
    }
    true
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
        // Unlike a missing description, which the published-text test reports
        // by name, a misspelt example would just vanish from the page — so a
        // table entry that lands nowhere stops the build of the document.
        for (name, field, json) in EXAMPLES {
            let example = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("example for {name}.{field} is not JSON: {e}"));
            let property = match components.schemas.get_mut(*name) {
                Some(RefOr::T(Schema::Object(object))) => object.properties.get_mut(*field),
                _ => None,
            }
            .unwrap_or_else(|| panic!("example for {name}.{field}: no such property"));
            assert!(
                exemplify(property, example),
                "example for {name}.{field}: the property is a reference and cannot carry one"
            );
        }
        for (name, json) in SCHEMA_EXAMPLES {
            let example = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("example for {name} is not JSON: {e}"));
            let schema = components
                .schemas
                .get_mut(*name)
                .unwrap_or_else(|| panic!("example for {name}: no such schema"));
            assert!(
                exemplify(schema, example),
                "example for {name}: cannot carry one"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `method` value the API can put on the wire is named in the
    /// published description (task 0268 review, WR-03). Two emitters feed the
    /// one `Candle` schema: `queries_ch::usd_method_expr` (the candle path:
    /// `assumed-par` / `external` / `oracle` / `traded`) and
    /// `queries_ch::ohlcv_peg_series` (USDC's own series: `peg` / `oracle`). A
    /// client generated from the schema must never meet a value the contract
    /// does not name — that was true before 0268 split the vocabulary, and the
    /// split dropped `peg` from the list while the self-series kept emitting it.
    ///
    /// The candle-path vocabulary is READ OFF THE EMITTER (review IN-13): the
    /// single-quoted literals of `usd_method_expr`'s rendered `multiIf` are the
    /// labels it can return, so a sixth arm added there fails here until the
    /// description names it. Only `peg` — emitted by the self-series builder,
    /// whose SQL carries many unrelated literals — is still listed by hand.
    #[test]
    fn candle_method_description_names_every_value_either_emitter_produces() {
        let (_, _, text) = FIELDS
            .iter()
            .find(|(schema, field, _)| *schema == "Candle" && *field == "method")
            .expect("Candle.method is described");
        // ⚠️ Read the published vocabulary, NOT every quoted literal in the
        // statement: the `external` arm consults `usd_rate`, so the rendered SQL
        // also carries `'UTC'`, `'credit'`, `'USDC'` and the issuer address,
        // none of which are `method` values.
        let rendered = crate::assets::queries_ch::usd_method_expr(
            2,
            &[7],
            crate::assets::queries_ch::Granularity::H1,
        );
        let emitted = crate::assets::queries_ch::CANDLE_METHOD_LABELS;
        for value in emitted {
            assert!(
                rendered.contains(&format!("'{value}'")),
                "the emitter no longer renders `{value}`: {rendered}"
            );
        }
        for value in emitted.iter().copied().chain(["peg"]) {
            assert!(
                text.contains(&format!("`{value}`")),
                "Candle.method description does not name `{value}`:\n{text}"
            );
        }
        // And `peg` is scoped to where it can appear, so nobody reads it as a
        // quote-leg value again.
        assert!(text.contains("USDC:<issuer>"), "{text}");
        assert!(text.contains("Never appears on a quote leg"), "{text}");
    }
}
