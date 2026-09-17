use rust_decimal::Decimal;

pub fn stroops_to_decimal(stroops: i64) -> Decimal {
    Decimal::new(stroops, 7)
}

pub fn compute_price(amount_sold: i64, amount_bought: i64, inverted: bool) -> Decimal {
    let sold = stroops_to_decimal(amount_sold);
    let bought = stroops_to_decimal(amount_bought);

    if inverted {
        sold / bought
    } else {
        bought / sold
    }
}

/// The price of a fill that crossed a RESTING OFFER: the offer's own
/// `price.n/d` (ADR 0287 §1, decision D2).
///
/// This price is not computed from the fill's rounded amounts at all — it is
/// the ratio the maker published, at full `i32` precision, and the two stroop
/// amounts are merely what the taker happened to consume of it. That is why an
/// offer-priced fill never faces the rounding bound: a 17-stroop fill against
/// an offer at 397/5000 traded at 0.0794, not at the 1/17 its own amounts
/// print.
///
/// Orientation follows [`compute_price`]. The offer sells A and buys B, and
/// `n/d` is "price of A in terms of B" — the same direction as a claim's
/// `bought / sold`. So a non-inverted pair takes `n/d` and an inverted one
/// `d/n`.
///
/// `None` means the entry cannot price anything and the caller must fall back
/// to the amount ratio plus the bound: `n` and `d` are `i32`s read from a
/// ledger entry, and nothing in the XDR forbids a zero or negative one.
pub fn offer_price(n: i32, d: i32, inverted: bool) -> Option<Decimal> {
    if n <= 0 || d <= 0 {
        return None;
    }
    let (numerator, denominator) = if inverted { (d, n) } else { (n, d) };
    Decimal::from(numerator).checked_div(Decimal::from(denominator))
}

/// The scale of every candle price column: `Decimal(38, 14)`, so 14 digits
/// after the point. `writer::decimal_to_i128` rounds to exactly this before
/// handing ClickHouse the mantissa.
pub const CANDLE_PRICE_SCALE: u32 = 14;

/// Is this price a number a `Decimal(38, 14)` column can carry as a
/// MEASUREMENT — neither noise at the bottom of the column nor past its top
/// (task 0286, VERIFY-0286-local discrepancy 4; review WR-03)?
///
/// The rounding bound below reasons about the AMOUNTS a ratio price divides; it
/// says nothing about where the quotient lands. A fill of 33 387 840 110.63
/// base units for 0.0001622 quote clears the bound on both legs by ten orders
/// of magnitude and still prices at ~4.86e-15 — below the column's 1e-14
/// resolution, so it stores as 0. Calling such a fill price-forming writes the
/// one row shape ADR 0287 forbids: `pf_trade_count = 1` with
/// `open = high = low = close = 0`, which the pre-roll carries into every
/// coarse tier and `/ohlcv` then publishes as `0` instead of `null`. Under the
/// ADR a bucket either has a price-forming fill AND a price, or neither.
///
/// Rounded with the writer's own rule — `Decimal::round_dp`, half to even — so
/// the verdict cannot drift from what `writer::decimal_to_i128` actually
/// stores.
///
/// ## The floor is [`candle_price_floor`], not one tick (review WR-03)
///
/// Storable is not the same as meaningful. A price of a few ticks is
/// quantisation noise, and the read path has always refused it — so the ingest
/// drawing its line at "does not round to 0" while `/ohlcv` and the rollups
/// drew theirs at `1e-12` let a 1m child with `close = 1.86e-12, low = 9e-14`
/// feed its `low` into `minIf` and be published on a coarse row whose own close
/// cleared the floor. ONE line, everywhere: `prices_clickhouse::PRICE_FLOOR_*`.
///
/// ## The other end of the column (review A WR-01 / D F2)
///
/// The same argument runs upward. `Decimal(38, 14)` holds 38 significant
/// digits with 14 after the point, so the integer part must stay below
/// `10^24`; a larger price is CLAMPED into the column by `bucket::finalise`,
/// and a clamped price is a number nobody traded at. Reachable through the
/// AMM path: 1 001 raw units in against 7.9e28 out clears the rounding bound
/// on both legs and prices at ~7.9e25. Such a fill counts in volume and in
/// `trade_count` like any other; it just prices nothing.
///
/// ⚠️ The upper bound is [`candle_price_column_max`] = `10^24 - 1`, while the
/// column actually holds up to `10^24 - 1e-14` (review IN-02). Prices in the
/// open interval `(10^24 - 1, 10^24)` are therefore storable and classified as
/// non-price-forming. Unreachable in practice — there is no market at 1e24 —
/// and the whole-integer bound is the one `bucket.rs`'s volume clamp shares.
pub fn price_survives_column_scale(price: Decimal) -> bool {
    let rounded = price.round_dp(CANDLE_PRICE_SCALE).abs();
    rounded >= candle_price_floor() && rounded <= candle_price_column_max()
}

/// The smallest price that means anything: `prices_clickhouse::PRICE_FLOOR_LITERAL`
/// = `1e-12`, 100 ticks of the `Decimal(38, 14)` price columns.
///
/// Built from parts rather than parsed so it is cheap on the hot path;
/// `the_ingest_floor_is_the_sql_floor` pins it to the shared literal the SQL
/// gates use, so the two cannot drift.
pub fn candle_price_floor() -> Decimal {
    Decimal::from_i128_with_scale(1, 12)
}

/// The largest value a `Decimal(38, 14)` candle column can hold: 38 significant
/// digits, 14 of them after the point, so the integer part stays below `10^24`.
///
/// Saturation targets THIS, not `Decimal::MAX` (~7.9e28) and not `i128::MAX`
/// (~1.7e38, where `writer::decimal_to_i128` saturates). A value between the
/// column domain and either of those converts to a mantissa of more than 38
/// digits, which ClickHouse accepts and reads back out of precision — a
/// silently wrong number instead of a clamped one.
pub fn candle_price_column_max() -> Decimal {
    Decimal::from_i128_with_scale(CANDLE_COLUMN_MAX_INTEGER, 0)
}

/// The integer part of [`candle_price_column_max`], shared with `bucket.rs`'s
/// volume clamp so the two cannot drift.
pub const CANDLE_COLUMN_MAX_INTEGER: i128 = 10i128.pow(24) - 1;

/// The rounding bound of ADR 0287 §1, on the two RAW integer amounts a ratio
/// price is computed from: `1/a + 1/b <= 0.001`.
///
/// A ratio price is only as precise as the amounts it divides. Rounding either
/// by one unit moves the quotient by up to `1/a + 1/b` in relative terms, so a
/// fill of a few stroops prints an exact small fraction (1/17, 5/34) that can
/// be hundreds of percent off the market — which is the whole of task 0286. A
/// tenth of a percent is the budget; a fill that cannot meet it counts in
/// volume but never sets a price.
///
/// Evaluated in the SHIFTED form. `1/a + 1/b <= 1/1000` rearranges to
/// `a*b >= 1000*(a+b)`, and that naive product overflows u128 for the i128-scale
/// amounts Soroban AMMs carry — a `(10^38, 5)` pair would panic in debug and
/// wrap to a WRONG verdict in release, admitting pure dust. The equivalent
/// `(a - 1000)(b - 1000) >= 10^6` only overflows when both factors are already
/// astronomically large, i.e. exactly when the bound holds by a wide margin —
/// so `checked_mul` returning `None` is itself the answer, not an error.
pub fn rounding_bound_holds(a: u128, b: u128) -> bool {
    // At or below 1000, 1/a alone already spends the whole budget and 1/b is
    // strictly positive — no counterpart can rescue it. This is also what keeps
    // the subtractions below from underflowing.
    if a <= 1000 || b <= 1000 {
        return false;
    }
    match (a - 1000).checked_mul(b - 1000) {
        Some(product) => product >= 1_000_000,
        None => true,
    }
}

/// [`rounding_bound_holds`] for a classic SDEX fill's i64 stroop amounts.
///
/// Non-positive amounts never form price: a zero amount has no ratio at all
/// (`filter.rs` drops those before they get here) and a negative one cannot
/// come from a valid claim, so treating it as dust is the safe reading.
pub fn price_forming_i64(amount_sold: i64, amount_bought: i64) -> bool {
    if amount_sold <= 0 || amount_bought <= 0 {
        return false;
    }
    rounding_bound_holds(amount_sold as u128, amount_bought as u128)
}

/// [`rounding_bound_holds`] for a Soroban AMM fill's RAW i128 amounts.
///
/// Must be called on the amounts as the event carries them, in each token's own
/// decimals — BEFORE `soroban::AMM_AMOUNT_SCALE` converts them to `Decimal`.
/// After that conversion the integer unit the bound reasons about is gone, and
/// every fill looks equally precise.
pub fn price_forming_i128(amount_in: i128, amount_out: i128) -> bool {
    if amount_in <= 0 || amount_out <= 0 {
        return false;
    }
    rounding_bound_holds(amount_in as u128, amount_out as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound is `1/a + 1/b <= 0.001` — a fill whose price moves by more than
    /// a tenth of a percent when either amount is rounded by one unit does not
    /// form price (ADR 0287 §1). Equal amounts of 2000 sit exactly on it.
    #[test]
    fn the_bound_is_exact_at_one_part_in_a_thousand() {
        assert!(rounding_bound_holds(2000, 2000), "1/2000 + 1/2000 = 0.001");
        assert!(
            !rounding_bound_holds(2000, 1999),
            "just inside 2000/2000 must fail"
        );
        assert!(
            rounding_bound_holds(1001, 1_001_000),
            "a tiny amount is admissible against a large enough counterpart"
        );
    }

    /// Below 1001 on either side, 1/a alone already exceeds the budget — no
    /// counterpart, however large, can rescue it.
    #[test]
    fn an_amount_of_a_thousand_or_less_never_forms_price() {
        assert!(!rounding_bound_holds(1000, u128::MAX));
        assert!(!rounding_bound_holds(u128::MAX, 1000));
        assert!(!rounding_bound_holds(17, 1), "the stroop-dust pair of 0286");
        assert!(!rounding_bound_holds(0, 0));
    }

    /// The overflow trap. Evaluated as `a*b >= 1000*(a+b)` — the algebraically
    /// equivalent naive form — a pair of i128-scale amounts overflows u128 and
    /// either panics or wraps to a wrong verdict. The shifted form
    /// `(a-1000)(b-1000) >= 10^6` only overflows when BOTH factors are huge,
    /// which is exactly the case where the bound genuinely holds.
    #[test]
    fn huge_amounts_neither_panic_nor_misclassify() {
        assert!(
            rounding_bound_holds(u128::MAX, u128::MAX),
            "an overflowing product means the bound holds by a wide margin"
        );
        let huge = 10u128.pow(38);
        assert!(
            !rounding_bound_holds(huge, 5),
            "a huge amount against 5 units is dust: 1/5 is 200x the budget"
        );
        assert!(!rounding_bound_holds(5, huge));
    }

    /// Review WR-03: ONE floor, `1e-12`, the same line the read path and the
    /// rollups draw. A price below it is quantisation noise wearing a number —
    /// a handful of `Decimal(38, 14)` ticks — so it does not form a price here
    /// either, and no coarse tier can inherit it as a `low`.
    #[test]
    fn a_price_under_the_precision_floor_is_not_price_forming() {
        assert!(
            price_survives_column_scale(candle_price_floor()),
            "1e-12 is the floor itself and forms price"
        );
        assert!(
            !price_survives_column_scale(Decimal::new(99, 14)),
            "9.9e-13 is below the floor: ~99 ticks of quantisation noise"
        );
        assert!(
            !price_survives_column_scale(Decimal::new(1, 14)),
            "1e-14 is one tick — storable, and not a measurement of anything"
        );
        assert!(
            !price_survives_column_scale(Decimal::new(5, 15)),
            "below the column resolution, so it never even stored"
        );
        assert!(
            !price_survives_column_scale(Decimal::ZERO),
            "no price at all is no price"
        );
        assert!(
            price_survives_column_scale(-candle_price_floor()),
            "the rule is about magnitude; a negative price is a bug elsewhere"
        );
        assert!(
            !price_survives_column_scale(Decimal::new(-99, 14)),
            "and magnitude is all it is about: -9.9e-13 is still under the floor"
        );
    }

    /// The floor is one number for the whole system: the ingest's `Decimal` and
    /// the SQL literal the rollups and `/ohlcv` gate on are the same value.
    #[test]
    fn the_ingest_floor_is_the_sql_floor() {
        use std::str::FromStr;
        assert_eq!(
            candle_price_floor(),
            Decimal::from_str(prices_clickhouse::PRICE_FLOOR_LITERAL).unwrap()
        );
        assert!(
            prices_clickhouse::PRICE_FLOOR_SQL.contains(prices_clickhouse::PRICE_FLOOR_LITERAL),
            "the SQL fragment must carry the literal it claims to"
        );
    }

    /// Review A WR-01 / D F2, the mirror of the underflow rule: a price above
    /// the column's `10^24` domain would be CLAMPED on the way in, and a
    /// clamped price is a number nobody traded at. The trigger is the repo's
    /// own AMM fixture: 1 001 raw units in against 7.9e28 out, both legs well
    /// clear of the rounding bound.
    #[test]
    fn a_price_over_the_column_domain_is_not_representable_either() {
        let max = candle_price_column_max();
        assert!(
            price_survives_column_scale(max),
            "the domain max itself fits"
        );
        assert!(
            !price_survives_column_scale(max + Decimal::ONE),
            "one above the domain would be clamped, not stored"
        );
        assert!(
            !price_survives_column_scale(-(max + Decimal::ONE)),
            "the rule is about magnitude"
        );

        const RAW_IN: i128 = 1001;
        const RAW_OUT: i128 = 79_000_000_000_000_000_000_000_000_000;
        assert!(
            price_forming_i128(RAW_IN, RAW_OUT),
            "the fixture must pass the bound, or it proves nothing"
        );
        let price = Decimal::try_from_i128_with_scale(RAW_OUT, 7).unwrap()
            / Decimal::try_from_i128_with_scale(RAW_IN, 7).unwrap();
        assert!(
            !price_survives_column_scale(price),
            "~7.9e25 is past the Decimal(38, 14) integer domain"
        );
    }

    /// The real row behind the rule: asset 2643 vs native, 2026-04-02 06:39 —
    /// 33 387 840 110.63 base units for 0.0001622 quote. The bound holds on
    /// both legs and the quotient still underflows the price column.
    #[test]
    fn the_bound_holding_does_not_mean_the_quotient_is_representable() {
        const BASE_STROOPS: i64 = 333_878_401_106_300_000;
        const QUOTE_STROOPS: i64 = 1_622;
        assert!(
            price_forming_i64(BASE_STROOPS, QUOTE_STROOPS),
            "both legs are far above the rounding bound"
        );
        let price = compute_price(BASE_STROOPS, QUOTE_STROOPS, false);
        assert!(price > Decimal::ZERO, "the ratio itself is positive");
        assert!(
            !price_survives_column_scale(price),
            "~4.86e-15 is below the Decimal(38, 14) resolution"
        );
    }

    /// Classic fills are i64 stroops. Zero amounts are already dropped upstream
    /// (`filter.rs`), but a non-positive amount must never be read as a price.
    #[test]
    fn the_i64_wrapper_rejects_non_positive_amounts() {
        assert!(price_forming_i64(2000, 2000));
        assert!(!price_forming_i64(0, 2000));
        assert!(!price_forming_i64(2000, 0));
        assert!(!price_forming_i64(-2000, 2000));
        assert!(!price_forming_i64(17, 1));
    }

    /// Soroban amounts are raw i128 in each token's own decimals — classified
    /// BEFORE `AMM_AMOUNT_SCALE`, because after scaling the integer information
    /// the bound reads is gone.
    #[test]
    fn the_i128_wrapper_widens_without_losing_the_verdict() {
        assert!(price_forming_i128(500_000_000_000_000_000, 10_000_000));
        assert!(!price_forming_i128(1000, 10_000_000));
        assert!(!price_forming_i128(-1, -1));
        assert!(price_forming_i128(i128::MAX, i128::MAX));
    }
}
