//! The re-issue cap: a revoked key is replaced only in the next quota period
//! (task 0191).
//!
//! **The rule.** "Replace my key" deactivates the current key **immediately**
//! and issues nothing. A new key can be issued only once the quota period in
//! which the revocation happened has rolled — the 1st of the next calendar
//! month, 00:00 UTC, under [`Period`]'s rule. A leak on the 3rd is dead on the
//! 3rd; the replacement comes on the 1st.
//!
//! **Why the wait is the point, not a cost to engineer away.** Quota is scoped
//! to `(usagePlanId, apiKeyId)`, so a new key is a clean counter. If a revoke
//! handed out a replacement, "replace my key" would be the button people press
//! on the 20th of a heavy month and the monthly quota would be decorative. The
//! wait is the same cost the owner would pay by simply not using the leaked
//! key, minus the risk of somebody else using it.
//!
//! **What is compared.** The revoked key stays in the account, disabled; its
//! `lastUpdatedDate` is the revocation instant, and a re-issue is allowed only
//! when that instant falls strictly **before** the current period began. There
//! is no stored timestamp because API Gateway already keeps this one — which
//! is why task 0190's registry stayed cancelled.
//!
//! **Worked example:** revoked on 3 August → "Get my API key" is refused until
//! 1 September 00:00 UTC, naming the date, and allowed from that instant.
//!
//! Pure, so the whole boundary table is decidable by a test with dates typed
//! by hand. No clock is read here: the caller supplies the period.

use super::super::period::Period;

/// Whether a key may be re-issued now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cap {
    /// The revocation predates the current period: a new key may be issued.
    Allowed,
    /// Revoked inside the current period (or undatable — see [`decide`]).
    /// Refused until the period rolls.
    Capped {
        /// The instant a new key becomes available, RFC 3339 — the
        /// `next_eligible_at` in the envelopes and the revoke answer.
        next_eligible_at: String,
        /// The same instant as a bare `YYYY-MM-DD`, for the callback's landing
        /// query: digits and dashes only, so no request-derived byte can reach
        /// a `Location` header through it.
        next_eligible_date: String,
    },
}

/// Decide the cap for a key revoked at `revoked_at` (Unix seconds — the
/// disabled key's `lastUpdatedDate`) against `period`.
///
/// Strictly **before** the period start: a revocation at exactly 00:00:00 on
/// the 1st happened inside the period that begins then, and the replacement
/// waits for the next one — the boundary instant belongs to the period it
/// begins.
///
/// `None` — a key AWS did not date — is **capped**, not allowed. The service
/// always sends `lastUpdatedDate`, so this is a shape that should never occur;
/// when it does, the cap cannot prove the revocation predates the period, and
/// the failure that matters is a quota laundered through a revoke-and-reissue,
/// not a replacement delayed to the 1st. Logged, because a deployment where it
/// fires has a control plane answering in a shape nobody has seen.
pub fn decide(revoked_at: Option<u64>, period: &Period) -> Cap {
    let capped = || Cap::Capped {
        next_eligible_at: period.resets_at(),
        next_eligible_date: period.next_start_ymd(),
    };
    match revoked_at {
        Some(revoked_at) if revoked_at < period.start_secs() => Cap::Allowed,
        Some(_) => capped(),
        None => {
            tracing::warn!(
                "the revoked key carries no lastUpdatedDate; refusing the re-issue rather than \
                 assuming the revocation predates the period"
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

    /// The worked example, both halves: revoked on 3 August, refused for the
    /// rest of August naming 1 September — and allowed from the first second
    /// of 1 September.
    #[test]
    fn revoked_on_3_august_refuses_until_1_september_and_succeeds_on_it() {
        let revoked = at(2026, 8, 3, 14, 30, 0);

        for day in [3, 4, 20, 31] {
            assert_eq!(
                decide(Some(revoked), &period(2026, 8, day)),
                Cap::Capped {
                    next_eligible_at: "2026-09-01T00:00:00Z".into(),
                    next_eligible_date: "2026-09-01".into(),
                },
                "August {day}"
            );
        }

        assert_eq!(decide(Some(revoked), &period(2026, 9, 1)), Cap::Allowed);
        assert_eq!(decide(Some(revoked), &period(2026, 9, 30)), Cap::Allowed);
    }

    /// A key issued AND revoked on the same day is capped like any other: the
    /// cap is about the revocation instant, so a fresh key revoked at once
    /// cannot be turned into a fresh counter.
    #[test]
    fn a_key_revoked_minutes_after_issue_is_capped_too() {
        assert!(matches!(
            decide(Some(at(2026, 8, 21, 12, 0, 0)), &period(2026, 8, 21)),
            Cap::Capped { .. }
        ));
    }

    /// The boundary instant belongs to the period it begins.
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

    /// A revocation AWS did not date cannot be shown to predate the period.
    #[test]
    fn an_undated_revocation_is_capped_not_waved_through() {
        assert!(matches!(
            decide(None, &period(2026, 8, 15)),
            Cap::Capped { .. }
        ));
    }

    /// The landing-query form is digits and dashes only, by construction.
    #[test]
    fn the_date_form_is_url_safe_by_construction() {
        let Cap::Capped {
            next_eligible_date, ..
        } = decide(Some(u64::MAX), &period(2026, 8, 15))
        else {
            panic!("a future revocation is capped");
        };
        assert!(
            next_eligible_date
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-')
        );
        assert_eq!(next_eligible_date.len(), 10);
    }
}
