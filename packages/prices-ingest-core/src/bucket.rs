use std::collections::HashMap;

use rust_decimal::Decimal;
use tracing::warn;

use crate::tick::TradeTick;

/// The largest value a `Decimal(38, 14)` column can hold: 38 significant digits
/// with 14 after the point, so the integer part must stay below 10^24.
///
/// Saturation targets THIS, not `Decimal::MAX` (~7.9e28) and not `i128::MAX`
/// (~1.7e38, where `decimal_to_i128` saturates). A value between the column
/// domain and either of those converts to a mantissa of more than 38 digits,
/// which ClickHouse accepts and reads back out of precision — a silently wrong
/// number instead of a clamped one.
const COLUMN_DOMAIN_MAX_INTEGER: i128 = 10i128.pow(24) - 1;

#[derive(Debug, Clone)]
pub struct OhlcvCandle {
    pub minute_start: u32,
    pub asset_id: u32,
    pub quote_asset_id: u32,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume_base: Decimal,
    pub volume_quote: Decimal,
    pub vwap: Decimal,
    pub trade_count: u32,
    pub version: u64,
    /// Fills of this bucket that were allowed to set a price (ADR 0287 §1).
    pub pf_trade_count: u32,
    /// Σ `volume_base` over those fills — BASE volume, not quote.
    pub pf_volume: Decimal,
    /// Σ price × `volume_base` over those fills.
    pub pf_price_volume: Decimal,
}

/// One fill of an open bucket, kept until the bucket is flushed. EVERY fill is
/// kept, not just the price-forming ones: the volume sums are computed from
/// this list too, in its sorted order, so no column of the candle depends on
/// the order ingest happened to see the fills in.
#[derive(Debug, Clone)]
struct Fill {
    /// `TradeTick::lex_key()` verbatim — the ONE definition of fill order in
    /// this crate. Copied rather than re-listed so the candle's order and the
    /// tick's documented order cannot drift apart (task 0286 WR-03).
    ///
    /// ⚠️ Deliberately NOT the source of `version`: that is computed in
    /// [`CandleAccumulator::merge`] from the tick's NAMED `ledger_sequence` and
    /// `operation_index` fields, because this tuple's second element is the
    /// TRANSACTION index (task 0286 F4).
    key: (u32, u16, u16, u16),
    price: Decimal,
    volume_base: Decimal,
    volume_quote: Decimal,
    price_forming: bool,
}

impl Fill {
    /// A TOTAL order over the minute's fills: the fill key first, then price and
    /// both volumes. The tie-break is not decoration — an AMM tick's
    /// `operation_index` is a masked event index and its `claim_index` is always
    /// 0, so two swaps of one transaction can collide on the key outright. What
    /// remains tied is two fills identical in every summed field, and swapping
    /// those cannot change any sum.
    fn sort_key(&self) -> ((u32, u16, u16, u16), Decimal, Decimal, Decimal) {
        (self.key, self.price, self.volume_base, self.volume_quote)
    }
}

/// An in-progress minute: the row so far, plus every fill that lands in it.
struct OpenBucket {
    candle: OhlcvCandle,
    fills: Vec<Fill>,
}

type BucketKey = (u32, u32, u32); // (minute_start, asset_id, quote_asset_id)

pub struct CandleAccumulator {
    buckets: HashMap<BucketKey, OpenBucket>,
}

impl Default for CandleAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl CandleAccumulator {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Fold one fill into its minute. Nothing but `trade_count` and `version` is
    /// computed here — every price and every sum is derived at flush from the
    /// bucket's fills in sorted order (see [`finalise`]), because neither "the
    /// first and last price-forming fill in fill order" nor a reproducible sum
    /// is a property a single fill can update incrementally when fills arrive
    /// out of order.
    ///
    /// No state crosses minutes or chunks: a bucket knows only its own fills,
    /// so nothing is carried forward from a previous candle (ADR 0287).
    pub fn merge(&mut self, tick: &TradeTick) {
        let minute_start = (tick.closed_at as u32 / 60) * 60;
        let key = (minute_start, tick.base_id, tick.quote_id);
        // ⚠️ Task 0286 F4: read from the NAMED fields, never positionally from
        // `TradeTick::lex_key`. That tuple is four elements wide, so its second
        // element is the TRANSACTION index — a positional read here would
        // silently redefine `version` and make every new row incomparable,
        // under ReplacingMergeTree, with every row already in price_ohlcv_1m.
        let version = tick.ledger_sequence as u64 * 1000 + tick.operation_index as u64;

        let bucket = self.buckets.entry(key).or_insert_with(|| OpenBucket {
            candle: OhlcvCandle {
                minute_start,
                asset_id: tick.base_id,
                quote_asset_id: tick.quote_id,
                // A minute with no price-forming fill keeps these zeros and is
                // still written: it traded, it just has no price (ADR 0287 §1).
                open: Decimal::ZERO,
                high: Decimal::ZERO,
                low: Decimal::ZERO,
                close: Decimal::ZERO,
                volume_base: Decimal::ZERO,
                volume_quote: Decimal::ZERO,
                vwap: Decimal::ZERO,
                trade_count: 0,
                version,
                pf_trade_count: 0,
                pf_volume: Decimal::ZERO,
                pf_price_volume: Decimal::ZERO,
            },
            fills: Vec::new(),
        });

        // EVERY fill is recorded, dust included: it is a real trade and its
        // volume is real. Only the PRICE is withheld from a non-price-forming
        // one, by the `price_forming` flag this carries into `finalise`.
        bucket.candle.trade_count += 1;
        if bucket.candle.version < version {
            bucket.candle.version = version;
        }
        bucket.fills.push(Fill {
            key: tick.lex_key(),
            price: tick.price,
            volume_base: tick.volume_base,
            volume_quote: tick.volume_quote,
            price_forming: tick.price_forming,
        });
    }

    pub fn flush_older_than(&mut self, current_minute: u32) -> Vec<OhlcvCandle> {
        let mut flushed = Vec::new();
        self.buckets.retain(|key, bucket| {
            if key.0 < current_minute {
                flushed.push(finalise(bucket));
                false
            } else {
                true
            }
        });
        flushed
    }

    pub fn flush_all(&mut self) -> Vec<OhlcvCandle> {
        let mut flushed: Vec<OhlcvCandle> = self
            .buckets
            .drain()
            .map(|(_, mut bucket)| finalise(&mut bucket))
            .collect();
        flushed.sort_by_key(|c| (c.minute_start, c.asset_id, c.quote_asset_id));
        flushed
    }
}

/// Close a bucket: the candle ADR 0287 defines, from this minute's fills alone.
///
/// DEPLOY ORDER (task 0286 §4.9): schema first, then the enrichment worker plus
/// the coarse sweep plus prices-api, then the rollup MV re-CREATE, and the
/// ingest — this code — LAST. A pre-0286 MV takes `min(low)` over a dust-only
/// minute and turns the new zero low into a zero low for the whole coarse
/// bucket; pre-0286 enrichment re-inserts a row without the pf columns, so they
/// fall back to their DEFAULTs and a dust-only minute reports itself as fully
/// price-forming. Both failures are silent.
///
/// ⚠️ A SECOND write of the same minute now carries more weight than it used
/// to. `price_ohlcv_1m` is `ReplacingMergeTree(version)` and a re-written
/// minute wins on the later ledger's version, so if the TAIL of that minute
/// holds only dust the replacement row is `open = high = low = close = 0,
/// pf_trade_count = 0` and it ERASES a correctly priced row. Three paths reach
/// that shape: a minute straddling `sdex-backfill`'s 64k-ledger partition
/// boundary (F14), a reconcile run re-emitting a partial minute, and an
/// `events-backfill` re-run. Task 0282's one-write-per-minute-bucket fix is
/// therefore a DEPLOY PRECONDITION for this code, not merely a related bug.
fn finalise(bucket: &mut OpenBucket) -> OhlcvCandle {
    // Sorted before anything is computed, so no column — price, pf sum or
    // volume — can depend on the order ingest happened to see the fills in.
    bucket.fills.sort_by_key(|f| f.sort_key());

    let mut clamp = Clamp::default();

    // Volumes, trade_count and vwap count EVERY fill: they are volume
    // statistics, not price prints, and excluding dust from them would make
    // them disagree with the trade count beside them.
    let mut volume_base = Decimal::ZERO;
    let mut volume_quote = Decimal::ZERO;
    for f in &bucket.fills {
        volume_base = clamp.add(volume_base, f.volume_base);
        volume_quote = clamp.add(volume_quote, f.volume_quote);
    }
    bucket.candle.volume_base = volume_base;
    bucket.candle.volume_quote = volume_quote;
    bucket.candle.vwap = if volume_base.is_zero() {
        Decimal::ZERO
    } else {
        clamp.div(volume_quote, volume_base)
    };

    let mut priced = bucket.fills.iter().filter(|f| f.price_forming).peekable();
    if let Some(first) = priced.peek().copied() {
        let mut high = first.price;
        let mut low = first.price;
        let mut last = first;
        let mut pf_trade_count: u32 = 0;
        let mut pf_volume = Decimal::ZERO;
        let mut pf_price_volume = Decimal::ZERO;
        for f in priced {
            high = high.max(f.price);
            low = low.min(f.price);
            pf_trade_count += 1;
            pf_volume = clamp.add(pf_volume, f.volume_base);
            let notional = clamp.mul(f.price, f.volume_base);
            pf_price_volume = clamp.add(pf_price_volume, notional);
            last = f;
        }
        bucket.candle.open = clamp.value(first.price);
        bucket.candle.close = clamp.value(last.price);
        bucket.candle.high = clamp.value(high);
        bucket.candle.low = clamp.value(low);
        bucket.candle.pf_trade_count = pf_trade_count;
        bucket.candle.pf_volume = pf_volume;
        bucket.candle.pf_price_volume = pf_price_volume;
    }
    // No price-forming fill: the prices stay zero. The row is still emitted.

    if clamp.fired {
        // Never silent. A clamped value is indistinguishable from a real one
        // downstream — S3's pf_vwap and its divergence flag would be computed
        // from a fabricated number — so say which candle it happened to
        // (task 0286 WR-05).
        warn!(
            minute_start = bucket.candle.minute_start,
            asset_id = bucket.candle.asset_id,
            quote_asset_id = bucket.candle.quote_asset_id,
            trade_count = bucket.candle.trade_count,
            "candle value saturated at the Decimal(38, 14) column domain — a \
             price or volume of this minute exceeds what the column can hold, \
             and the written value is clamped, not measured"
        );
    }

    bucket.candle.clone()
}

/// The saturation target: the largest value the `Decimal(38, 14)` columns hold.
fn column_domain_max() -> Decimal {
    Decimal::from_i128_with_scale(COLUMN_DOMAIN_MAX_INTEGER, 0)
}

/// Decimal arithmetic that saturates into the column domain instead of
/// panicking, and REMEMBERS whether it had to (task 0286 T-jiv-04, WR-05).
///
/// A `rust_decimal` overflow panics, and a panic here would abort a whole
/// ingest run over one hostile amount. Clamping instead is the right trade —
/// but a clamp that leaves no trace turns a fabricated number into an
/// indistinguishable one, so `fired` is what the caller logs.
#[derive(Default)]
struct Clamp {
    fired: bool,
}

impl Clamp {
    /// Clamp into the column's domain. Symmetric, though a negative value
    /// should be unreachable — a price or volume that went negative is a bug
    /// elsewhere, and clamping it still beats handing ClickHouse a value it
    /// will reject or silently wrap.
    fn value(&mut self, value: Decimal) -> Decimal {
        let max = column_domain_max();
        if value > max {
            self.fired = true;
            return max;
        }
        if value < -max {
            self.fired = true;
            return -max;
        }
        value
    }

    fn saturated(&mut self, negative: bool) -> Decimal {
        self.fired = true;
        if negative {
            -column_domain_max()
        } else {
            column_domain_max()
        }
    }

    fn add(&mut self, a: Decimal, b: Decimal) -> Decimal {
        match a.checked_add(b) {
            Some(sum) => self.value(sum),
            None => self.saturated(b.is_sign_negative()),
        }
    }

    fn mul(&mut self, a: Decimal, b: Decimal) -> Decimal {
        match a.checked_mul(b) {
            Some(product) => self.value(product),
            None => self.saturated(a.is_sign_negative() != b.is_sign_negative()),
        }
    }

    /// Caller guarantees a non-zero divisor; `checked_div` still covers the
    /// overflow case, which a huge quote volume over a dust base volume reaches.
    fn div(&mut self, a: Decimal, b: Decimal) -> Decimal {
        match a.checked_div(b) {
            Some(quotient) => self.value(quotient),
            None => self.saturated(a.is_sign_negative() != b.is_sign_negative()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{AssetIdentity, AssetRegistry};
    use crate::filter::RawTrade;
    use crate::tick::raw_trade_to_tick;

    // A minute is `floor(closed_at / 60) * 60`. These three timestamps: the
    // first two share minute M0, the third is in the next minute M1.
    const T_M0_A: i64 = 1_700_000_000; // minute 1_699_999_980
    const T_M0_B: i64 = 1_700_000_030; // same minute
    const T_M1: i64 = 1_700_000_100; // minute 1_700_000_100
    const M0: u32 = 1_699_999_980;
    const M1: u32 = 1_700_000_100;

    #[allow(clippy::too_many_arguments)]
    fn tick(
        ledger: u32,
        op: u16,
        claim: u16,
        base: u32,
        quote: u32,
        price: i64,
        vol_base: i64,
        vol_quote: i64,
        closed_at: i64,
    ) -> TradeTick {
        tx_tick(
            ledger, 0, op, claim, base, quote, price, vol_base, vol_quote, closed_at,
        )
    }

    /// [`tick`] with an explicit transaction index — the term that distinguishes
    /// two fills of one ledger that share an operation index (task 0286 D1).
    /// Every fill built here is price-forming; the dust cases live in the
    /// price-forming tests below.
    #[allow(clippy::too_many_arguments)]
    fn tx_tick(
        ledger: u32,
        tx: u16,
        op: u16,
        claim: u16,
        base: u32,
        quote: u32,
        price: i64,
        vol_base: i64,
        vol_quote: i64,
        closed_at: i64,
    ) -> TradeTick {
        TradeTick {
            ledger_sequence: ledger,
            closed_at,
            transaction_index: tx,
            operation_index: op,
            claim_index: claim,
            base_id: base,
            quote_id: quote,
            price: Decimal::from(price),
            volume_base: Decimal::from(vol_base),
            volume_quote: Decimal::from(vol_quote),
            price_forming: true,
        }
    }

    /// Task 0286 F4 — the trap that widening the fill key sets. `version` is
    /// `ledger_sequence * 1000 + operation_index` and must STAY that: it is the
    /// ReplacingMergeTree discriminator every row already written to
    /// `price_ohlcv_1m` was scored with, so redefining it silently makes new
    /// rows incomparable with old ones. A `merge` that read the key tuple
    /// POSITIONALLY would now be taking the transaction index as its second
    /// term and would compute 100_007 here instead of 100_005.
    #[test]
    fn version_stays_ledger_times_1000_plus_operation_index() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tx_tick(100, 7, 3, 0, 1, 2, 10, 1, 10, T_M0_A));
        acc.merge(&tx_tick(100, 0, 5, 0, 1, 2, 20, 1, 20, T_M0_B));
        let c = &acc.flush_all()[0];
        assert_eq!(
            c.version, 100_005,
            "version = max(ledger * 1000 + operation_index), never the tx index"
        );
    }

    /// Task 0286 D1, the 2026-04-02 shape from the analysis: a path payment in
    /// the FIRST transaction consumes several offers (claim 3 is its last fill),
    /// and a manage-offer in the SECOND transaction fills once (claim 0). Both
    /// operations are at index 0 because `operation_index` restarts per
    /// transaction, so a key without the transaction index ranks claim 3 above
    /// claim 0 and closes the minute on the wrong fill.
    #[test]
    fn close_is_the_last_fill_in_transaction_apply_order() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tx_tick(100, 0, 0, 3, 1, 2, 10, 1, 10, T_M0_A));
        acc.merge(&tx_tick(100, 1, 0, 0, 1, 2, 20, 1, 20, T_M0_A));
        let c = &acc.flush_all()[0];
        assert_eq!(c.open, 10.into(), "open = the path payment's first fill");
        assert_eq!(
            c.close,
            20.into(),
            "close = the manage-offer fill: its transaction applied second"
        );
    }

    #[test]
    fn single_trade_seeds_flat_ohlc() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(100, 0, 0, 1, 2, 7, 3, 21, T_M0_A));
        let out = acc.flush_all();
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.minute_start, M0);
        assert_eq!((c.asset_id, c.quote_asset_id), (1, 2));
        assert_eq!(
            (c.open, c.high, c.low, c.close),
            (7.into(), 7.into(), 7.into(), 7.into())
        );
        assert_eq!(c.trade_count, 1);
        assert_eq!(c.volume_base, 3.into());
        assert_eq!(c.volume_quote, 21.into());
        assert_eq!(c.vwap, 7.into()); // 21 / 3
        assert_eq!(c.version, 100_000); // ledger*1000 + op
    }

    /// Task 0286 replaces the pre-0286 wording of this test (open/close were
    /// simply the lowest/highest fill key) with the ADR 0287 definition: the
    /// first and last PRICE-FORMING fill in fill order. With every fill of this
    /// minute price-forming the two coincide, which is the point — the rule did
    /// not change for ordinary minutes, only for ones holding dust.
    #[test]
    fn open_and_close_follow_lex_order_not_arrival_order() {
        let mut acc = CandleAccumulator::new();
        // Insert out of ledger/op order; open must be the lowest lex, close the
        // highest — regardless of insertion order.
        acc.merge(&tick(100, 2, 0, 1, 2, 5, 1, 5, T_M0_A)); // mid lex, price 5
        acc.merge(&tick(100, 0, 0, 1, 2, 10, 1, 10, T_M0_A)); // first lex, price 10
        acc.merge(&tick(100, 5, 0, 1, 2, 20, 1, 20, T_M0_B)); // last lex, price 20
        let c = &acc.flush_all()[0];
        assert_eq!(c.open, 10.into(), "open = earliest lex trade");
        assert_eq!(c.close, 20.into(), "close = latest lex trade");
        assert_eq!(c.high, 20.into());
        assert_eq!(c.low, 5.into());
        assert_eq!(c.trade_count, 3);
        assert_eq!(c.volume_base, 3.into());
        assert_eq!(c.volume_quote, 35.into());
        assert_eq!(c.version, 100_005, "version = max(ledger*1000 + op)");
    }

    #[test]
    fn two_pairs_in_one_minute_are_separate_candles() {
        // Scenario 1: XLM/USDC (1,2); Scenario 2: PHO/USDC (3,2) — same minute,
        // must not collide.
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(100, 0, 0, 1, 2, 10, 2, 20, T_M0_A));
        acc.merge(&tick(100, 1, 0, 3, 2, 4, 5, 20, T_M0_B));
        let out = acc.flush_all();
        assert_eq!(out.len(), 2);
        let xlm = out.iter().find(|c| c.asset_id == 1).unwrap();
        let pho = out.iter().find(|c| c.asset_id == 3).unwrap();
        assert_eq!(xlm.close, 10.into());
        assert_eq!(xlm.vwap, 10.into()); // 20/2
        assert_eq!(pho.close, 4.into());
        assert_eq!(pho.vwap, 4.into()); // 20/5
    }

    #[test]
    fn flush_older_than_keeps_the_current_minute() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(100, 0, 0, 1, 2, 10, 1, 10, T_M0_A)); // minute M0
        acc.merge(&tick(101, 0, 0, 1, 2, 12, 1, 12, T_M1)); // minute M1
        // Flushing "older than M1" emits only the completed M0 candle.
        let flushed = acc.flush_older_than(M1);
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].minute_start, M0);
        assert_eq!(flushed[0].vwap, 10.into(), "vwap finalised on flush");
        // M1 is still open and flushes on flush_all.
        let rest = acc.flush_all();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].minute_start, M1);
    }

    #[test]
    fn flush_all_sorts_by_minute_then_pair() {
        let mut acc = CandleAccumulator::new();
        acc.merge(&tick(101, 0, 0, 3, 2, 1, 1, 1, T_M1)); // M1, pair 3
        acc.merge(&tick(100, 0, 0, 3, 2, 1, 1, 1, T_M0_A)); // M0, pair 3
        acc.merge(&tick(100, 1, 0, 1, 2, 1, 1, 1, T_M0_B)); // M0, pair 1
        let out = acc.flush_all();
        let keys: Vec<_> = out.iter().map(|c| (c.minute_start, c.asset_id)).collect();
        assert_eq!(keys, vec![(M0, 1), (M0, 3), (M1, 3)]);
    }

    // ---- ADR 0287: the candle is built from its price-forming fills ---------

    const USDC_ISSUER_ADDR: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

    /// A classic XLM→USDC fill, built through the REAL classifier: the test
    /// never sets `price_forming` by hand, so what it pins is the behaviour an
    /// actual ledger produces. `sold` is XLM stroops, `bought` USDC stroops.
    fn fill(
        registry: &mut AssetRegistry,
        tx: u16,
        sold: i64,
        bought: i64,
        closed_at: i64,
    ) -> TradeTick {
        raw_trade_to_tick(
            &RawTrade {
                ledger_sequence: 100,
                closed_at,
                transaction_index: tx,
                operation_index: 0,
                claim_index: 0,
                asset_sold: AssetIdentity::Native,
                amount_sold: sold,
                asset_bought: AssetIdentity::Credit {
                    code: "USDC".to_string(),
                    issuer: USDC_ISSUER_ADDR.to_string(),
                },
                amount_bought: bought,
                price_source: crate::filter::PriceSource::AmountRatio,
            },
            registry,
        )
    }

    /// Five XLM for one USDC — an ordinary fill, price 0.2.
    fn ordinary(registry: &mut AssetRegistry, tx: u16, xlm: i64, usdc: i64) -> TradeTick {
        fill(registry, tx, xlm, usdc, T_M0_A)
    }

    /// The dust of task 0286: 17 stroops for 1, printing the exact fraction
    /// 1/17 — about 0.059, an order of magnitude below any real XLM/USDC price.
    fn dust(registry: &mut AssetRegistry, tx: u16) -> TradeTick {
        fill(registry, tx, 17, 1, T_M0_A)
    }

    /// ADR 0287 §1: a minute whose every fill is dust has NO price, but it is
    /// still a minute in which trading happened — the volumes, the trade count
    /// and the vwap are all real and the row is still written. Reporting the
    /// dust price instead is the bug this task exists to fix.
    #[test]
    fn dust_only_minute_has_volume_but_no_price() {
        let mut registry = AssetRegistry::from_existing(vec![]);
        let mut acc = CandleAccumulator::new();
        acc.merge(&dust(&mut registry, 0));
        acc.merge(&dust(&mut registry, 1));

        let out = acc.flush_all();
        assert_eq!(out.len(), 1, "the row is still emitted");
        let c = &out[0];
        assert_eq!(c.trade_count, 2);
        assert_eq!(
            c.volume_base,
            Decimal::new(34, 7),
            "two fills of 17 stroops"
        );
        assert_eq!(c.volume_quote, Decimal::new(2, 7));
        assert_eq!(
            c.vwap,
            Decimal::ONE / Decimal::from(17),
            "vwap is Sigma quote / Sigma base over ALL fills, dust included"
        );
        assert_eq!(c.pf_trade_count, 0);
        assert_eq!(c.pf_volume, Decimal::ZERO);
        assert_eq!(c.pf_price_volume, Decimal::ZERO);
        assert_eq!(
            (c.open, c.high, c.low, c.close),
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
            "no price-forming fill means no price at all"
        );
    }

    /// The headline case: a dust fill last in the minute used to become its
    /// close, and its 1/17 used to become the minute's low. Neither may move
    /// now — but the dust still counts in the volumes and the trade count.
    #[test]
    fn dust_beside_ordinary_fills_moves_neither_close_nor_low() {
        let mut registry = AssetRegistry::from_existing(vec![]);
        let mut acc = CandleAccumulator::new();
        acc.merge(&ordinary(&mut registry, 0, 50_000_000, 10_000_000)); // 5 XLM -> 1 USDC, 0.2
        acc.merge(&ordinary(&mut registry, 2, 100_000_000, 30_000_000)); // 10 XLM -> 3 USDC, 0.3
        acc.merge(&dust(&mut registry, 3)); // last in fill order, price ~0.059

        let c = &acc.flush_all()[0];
        assert_eq!(c.open, Decimal::new(2, 1), "0.2 — the first ordinary fill");
        assert_eq!(c.close, Decimal::new(3, 1), "0.3 — NOT the trailing dust");
        assert_eq!(c.high, Decimal::new(3, 1));
        assert_eq!(c.low, Decimal::new(2, 1), "the dust does not drag low down");

        assert_eq!(c.pf_trade_count, 2);
        assert_eq!(
            c.pf_volume,
            Decimal::from(15),
            "5 XLM + 10 XLM, base volume"
        );
        assert_eq!(
            c.pf_price_volume,
            Decimal::from(4),
            "0.2 * 5 + 0.3 * 10 — Sigma price x base volume"
        );

        assert_eq!(c.trade_count, 3, "the dust is still a trade");
        assert_eq!(
            c.volume_base,
            Decimal::from(15) + Decimal::new(17, 7),
            "and still counts in volume"
        );
    }

    /// Open and close are the first and last PRICE-FORMING fill, not the first
    /// and last fill: dust at both ends of the minute changes nothing.
    #[test]
    fn open_and_close_are_the_first_and_last_price_forming_fill() {
        let mut registry = AssetRegistry::from_existing(vec![]);
        let mut acc = CandleAccumulator::new();
        acc.merge(&dust(&mut registry, 0));
        acc.merge(&ordinary(&mut registry, 1, 50_000_000, 10_000_000)); // 0.2
        acc.merge(&ordinary(&mut registry, 3, 100_000_000, 30_000_000)); // 0.3
        acc.merge(&dust(&mut registry, 5));

        let c = &acc.flush_all()[0];
        assert_eq!(c.open, Decimal::new(2, 1));
        assert_eq!(c.close, Decimal::new(3, 1));
        assert_eq!(c.pf_trade_count, 2);
    }

    /// Task 0286, VERIFY-0286-local discrepancy 4 — the one row shape ADR 0287
    /// rules out, found on real data: asset 2643 vs native, 2026-04-02 06:39,
    /// a single fill of 33 387 840 110.63 base units for 0.0001622 quote. Both
    /// legs clear the rounding bound by ten orders of magnitude, so the minute
    /// used to be written `pf_trade_count = 1` — while the price itself,
    /// ~4.86e-15, is under the `Decimal(38, 14)` resolution and stores as 0.
    /// The pre-roll then carried that zero into every coarse tier and `/ohlcv`
    /// published `0` where the ADR asks for `null`. A bucket has a
    /// price-forming fill AND a price, or neither.
    #[test]
    fn a_fill_whose_price_underflows_the_price_column_prices_nothing() {
        const BASE_STROOPS: i64 = 333_878_401_106_300_000; // 33 387 840 110.63 XLM
        const QUOTE_STROOPS: i64 = 1_622; // 0.0001622 USDC
        assert!(
            crate::price::price_forming_i64(BASE_STROOPS, QUOTE_STROOPS),
            "the rounding bound passes this fill — the fixture proves nothing otherwise"
        );

        let mut registry = AssetRegistry::from_existing(vec![]);
        let mut acc = CandleAccumulator::new();
        acc.merge(&fill(&mut registry, 0, BASE_STROOPS, QUOTE_STROOPS, T_M0_A));

        let out = acc.flush_all();
        assert_eq!(out.len(), 1, "the row is still emitted");
        let c = &out[0];
        assert_eq!(
            c.pf_trade_count, 0,
            "a fill that cannot print a price does not form one"
        );
        assert_eq!(
            (c.open, c.high, c.low, c.close),
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
            "no price-forming fill means no price at all"
        );
        assert_eq!(c.pf_volume, Decimal::ZERO);
        assert_eq!(c.pf_price_volume, Decimal::ZERO);

        assert_eq!(c.trade_count, 1, "it is still a trade");
        assert_eq!(
            c.volume_base,
            Decimal::new(BASE_STROOPS, 7),
            "and still counts in volume"
        );
        assert_eq!(c.volume_quote, Decimal::new(QUOTE_STROOPS, 7));
    }

    /// EVERY field of the candle is computed from the minute's fills in their
    /// own sorted order, never in arrival order — the prices, the pf sums AND
    /// the two volume sums with the vwap derived from them. It matters because
    /// a `Decimal` sum rounds once the terms span more significant digits than
    /// it carries, and a sum of rounded terms depends on the order they were
    /// added in. Ingest sees a ledger's fills in whatever order the chunking
    /// and grouping produced, and phase 3 reconciles `sum(volume_base)` to the
    /// stroop against a FREEZE snapshot — neither survives a candle that
    /// depends on arrival order (task 0286 WR-04).
    #[test]
    fn arrival_order_does_not_change_the_candle() {
        let amounts = [
            (1u16, 30_000_000_000_001i64, 10_000_000_000_003i64),
            (2, 50_000_000_000_011, 70_000_000_000_013),
            (3, 110_000_000_000_017, 130_000_000_000_019),
        ];

        type Snapshot = (
            Decimal, // open
            Decimal, // high
            Decimal, // low
            Decimal, // close
            u32,     // pf_trade_count
            Decimal, // pf_volume
            Decimal, // pf_price_volume
            Decimal, // volume_base
            Decimal, // volume_quote
            Decimal, // vwap
        );
        let mut reference: Option<Snapshot> = None;
        for rotation in 0..amounts.len() {
            let mut registry = AssetRegistry::from_existing(vec![]);
            let mut acc = CandleAccumulator::new();
            for i in 0..amounts.len() {
                let (tx, sold, bought) = amounts[(i + rotation) % amounts.len()];
                acc.merge(&ordinary(&mut registry, tx, sold, bought));
            }
            let c = &acc.flush_all()[0];
            let got: Snapshot = (
                c.open,
                c.high,
                c.low,
                c.close,
                c.pf_trade_count,
                c.pf_volume,
                c.pf_price_volume,
                c.volume_base,
                c.volume_quote,
                c.vwap,
            );
            match &reference {
                None => reference = Some(got),
                Some(want) => assert_eq!(&got, want, "rotation {rotation} changed the candle"),
            }
        }
    }

    /// T-jiv-04 / WR-01: a hostile or merely enormous fill must not abort an
    /// ingest run, and must not leave ANY value the `Decimal(38, 14)` columns
    /// cannot store.
    ///
    /// The fixture is the shape that actually reaches this: a Soroban AMM fill
    /// whose raw i128 amounts PASS the rounding bound (both above 1000, product
    /// of the shifted factors far above 10^6) yet whose quotient is enormous —
    /// 1001 units in against 7.9e28 out. After `AMM_AMOUNT_SCALE` the price is
    /// ~7.9e25, well past the 10^24 an integer part may reach, and so is the
    /// vwap derived from the same two volumes. Clamping the pf sums alone would
    /// leave `open`/`high`/`low`/`close`/`vwap` to be saturated by
    /// `decimal_to_i128` at `i128::MAX` instead — a number ClickHouse accepts
    /// and reads back out of precision, which is the silent wrap the clamp
    /// exists to prevent.
    #[test]
    fn an_oversized_fill_saturates_every_column_into_its_domain() {
        // The amounts really are price-forming — this is not an unreachable
        // fixture smuggled past the classifier.
        const RAW_IN: i128 = 1001;
        const RAW_OUT: i128 = 79_000_000_000_000_000_000_000_000_000;
        assert!(
            crate::price::price_forming_i128(RAW_IN, RAW_OUT),
            "the fixture must pass the bound, or it proves nothing"
        );

        let amount_in = Decimal::try_from_i128_with_scale(RAW_IN, 7).unwrap();
        let amount_out = Decimal::try_from_i128_with_scale(RAW_OUT, 7).unwrap();
        let price = amount_out / amount_in;

        let mut acc = CandleAccumulator::new();
        acc.merge(&TradeTick {
            ledger_sequence: 100,
            closed_at: T_M0_A,
            transaction_index: 0,
            operation_index: 0,
            claim_index: 0,
            base_id: 1,
            quote_id: 2,
            price,
            volume_base: amount_in,
            volume_quote: amount_out,
            price_forming: true,
        });

        let c = &acc.flush_all()[0];
        let domain_max = Decimal::from_i128_with_scale(10i128.pow(24) - 1, 0);
        for (what, value) in [
            ("open", c.open),
            ("high", c.high),
            ("low", c.low),
            ("close", c.close),
            ("volume_base", c.volume_base),
            ("volume_quote", c.volume_quote),
            ("vwap", c.vwap),
            ("pf_volume", c.pf_volume),
            ("pf_price_volume", c.pf_price_volume),
        ] {
            assert!(
                value <= domain_max,
                "{what} left the Decimal(38, 14) domain: {value}"
            );
            // And the clamped value survives the conversion the writer performs
            // without wrapping the i128 mantissa.
            assert!(
                crate::writer::decimal_to_i128(value) >= 0,
                "{what} wrapped in decimal_to_i128"
            );
        }
        // The corners genuinely needed clamping — otherwise the loop above is
        // asserting nothing.
        assert_eq!(c.close, domain_max, "close saturated at the column max");
        assert_eq!(c.vwap, domain_max, "vwap saturated at the column max");
        assert!(c.pf_volume > Decimal::ZERO);
    }
}
