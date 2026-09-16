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
