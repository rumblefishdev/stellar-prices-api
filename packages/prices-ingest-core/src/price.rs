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
