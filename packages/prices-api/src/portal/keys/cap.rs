//! The rework cap: one new key per quota period (task 0191).
//!
//! **The rule.** A key may be replaced only when it was created **before** the
//! current quota period began — the 1st of the calendar month, 00:00 UTC,
//! under [`Period`]'s rule. A rework deletes the old key and creates a new
//! one, so the surviving key's `createdDate` *is* the instant of the last
//! rework; no stored timestamp is needed, which is the fact that let task 0190
//! cancel the registry table.
//!
//! **Why creation time, not a separate "last rotated" stamp.** Quota is scoped
//! to `(usagePlanId, apiKeyId)`, so a new key is a clean counter. If only
//! reworks were capped, a user could take a key on the 1st, spend the whole
//! quota, and rework on the 2nd into a fresh 100 000 — the exact loophole the
//! cap exists to close. Any key acquired inside the current period was created
//! inside it, so it can never be reworked inside it: one key per period, one
//! quota.
//!
//! **Worked example** (the 2026-08-07 meeting's): a key reworked on 3 August
//! is refused until 1 September 00:00 UTC, and allowed from that instant. The
//! refusal carries the date, because the wait is weeks and "try again later"
//! would be a lie by omission.
//!
//! Pure, so the whole boundary table is decidable by a test with dates typed
//! by hand. No clock is read here: the caller supplies the period.

use super::super::period::Period;

/// Whether a key may be replaced now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cap {
    /// The key predates the current period: a rework is allowed.
    Allowed,
    /// The key was created inside the current period (or cannot be dated —
    /// see [`decide`]). Refused until the period rolls.
    Capped {
        /// The instant the next rework becomes available, RFC 3339 — the
        /// `next_eligible_at` in the `409` envelope.
        next_eligible_at: String,
        /// The same instant as a bare `YYYY-MM-DD`, for the callback's landing
        /// query: digits and dashes only, so no request-derived byte can reach
        /// a `Location` header through it.
        next_eligible_date: String,
    },
}

/// Decide the cap for a key created at `created_at` (Unix seconds, as API
/// Gateway reports `createdDate`) against `period`.
///
/// Strictly **before** the period start: a key created at exactly
/// 00:00:00 on the 1st was created inside the period and is capped until the
/// next one — the boundary instant belongs to the period it begins.
///
/// `None` — a key AWS did not date — is **capped**, not allowed. The service
/// always sends `createdDate`, so this is a shape that should never occur;
/// when it does, the cap cannot prove the key predates the period, and the
/// failure that matters is a quota laundered through a rework, not a rework
/// delayed to the 1st. Logged, because a deployment where it fires has a
/// control plane answering in a shape nobody has seen.
pub fn decide(created_at: Option<u64>, period: &Period) -> Cap {
    let capped = || Cap::Capped {
        next_eligible_at: period.resets_at(),
        next_eligible_date: period.next_start_ymd(),
    };
    match created_at {
        Some(created_at) if created_at < period.start_secs() => Cap::Allowed,
        Some(_) => capped(),
        None => {
            tracing::warn!(
                "the current key carries no createdDate; refusing the rework rather than \
                 assuming it predates the period"
            );
            capped()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn period(y: i32, m: u32, d: u32) -> Period {
        Period::containing(NaiveDate::from_ymd_opt(y, m, d).unwrap())
    }

    /// Unix seconds for a UTC date-time, so the table below reads as dates.
    fn at(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> u64 {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, s)
            .unwrap()
            .and_utc()
            .timestamp() as u64
    }

    /// The meeting's worked example, both halves: reworked on 3 August,
    /// refused for the rest of August naming 1 September — and allowed from
    /// the first second of 1 September.
    #[test]
    fn reworked_on_3_august_refuses_until_1_september_and_succeeds_on_it() {
        let reworked = at(2026, 8, 3, 14, 30, 0);

        for day in [3, 4, 20, 31] {
            assert_eq!(
                decide(Some(reworked), &period(2026, 8, day)),
                Cap::Capped {
                    next_eligible_at: "2026-09-01T00:00:00Z".into(),
                    next_eligible_date: "2026-09-01".into(),
                },
                "August {day}"
            );
        }

        assert_eq!(decide(Some(reworked), &period(2026, 9, 1)), Cap::Allowed);
        assert_eq!(decide(Some(reworked), &period(2026, 9, 30)), Cap::Allowed);
    }

    /// The cap is about creation, not about reworks: a key *issued* inside
    /// the period is just as capped as one reworked inside it. This is the
    /// loophole the fallback closes — take a key on the 1st, drain it, rework
    /// on the 2nd into a clean counter.
    #[test]
    fn a_key_issued_inside_the_period_cannot_be_reworked_inside_it() {
        let issued_on_the_1st = at(2026, 8, 1, 9, 0, 0);
        assert!(matches!(
            decide(Some(issued_on_the_1st), &period(2026, 8, 2)),
            Cap::Capped { .. }
        ));
    }

    /// The boundary instant belongs to the period it begins: a key created at
    /// exactly 00:00:00 on the 1st is inside the period; one second earlier
    /// is in the previous one and may be reworked.
    #[test]
    fn the_boundary_instant_is_inside_the_new_period() {
        let midnight = at(2026, 8, 1, 0, 0, 0);
        assert!(matches!(
            decide(Some(midnight), &period(2026, 8, 15)),
            Cap::Capped { .. }
        ));
        assert_eq!(
            decide(Some(midnight - 1), &period(2026, 8, 15)),
            Cap::Allowed
        );
    }

    /// December names January of the following year.
    #[test]
    fn a_december_refusal_names_the_new_year() {
        assert_eq!(
            decide(Some(at(2026, 12, 10, 0, 0, 0)), &period(2026, 12, 24)),
            Cap::Capped {
                next_eligible_at: "2027-01-01T00:00:00Z".into(),
                next_eligible_date: "2027-01-01".into(),
            }
        );
    }

    /// A key AWS did not date cannot be shown to predate the period, so it
    /// is refused — the failure that matters is the laundered quota, not the
    /// delayed rework.
    #[test]
    fn an_undated_key_is_capped_not_waved_through() {
        assert!(matches!(
            decide(None, &period(2026, 8, 15)),
            Cap::Capped { .. }
        ));
    }

    /// The landing-query form is digits and dashes only, by construction —
    /// it reaches a `Location` header.
    #[test]
    fn the_date_form_is_url_safe_by_construction() {
        let Cap::Capped {
            next_eligible_date, ..
        } = decide(Some(u64::MAX), &period(2026, 8, 15))
        else {
            panic!("a future key is capped");
        };
        assert!(
            next_eligible_date
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-')
        );
        assert_eq!(next_eligible_date.len(), 10);
    }
}
