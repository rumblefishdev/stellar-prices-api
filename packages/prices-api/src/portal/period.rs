//! The quota period under **our** rule: the calendar month, UTC
//! (task 0188's dashboard, task 0191's rework cap).
//!
//! One definition, two readers. The usage route renders a period around
//! AWS's counters, and the rework cap decides whether a key's `createdDate`
//! falls before the current period began. Both used to be able to carry their
//! own copy of "the 1st of the month, 00:00 UTC"; with the cap this becomes a
//! correctness property rather than a label — a dashboard that says the quota
//! resets on the 1st while the cap counts from a different instant would let
//! a rework through that the page said was refused, or the reverse — so the
//! rule lives here and nowhere else.
//!
//! # This is our rule, not AWS's — and what has been measured
//!
//! AWS documents neither the instant its `MONTH` quota rolls nor the timezone
//! it rolls in (ADR 0010, correction #2). The only statement anywhere is an
//! example caption, *"creates a usage plan that resets at the beginning of
//! the month"*, and `offset` is a request count, not a time shift. Task 0191
//! measures the instant on a `DAY`-period scratch plan as a proxy (the
//! result, with its date, is in that task's Step 0 table); the first real
//! `MONTH` rollover observable after this code exists is 1 September 2026.
//! Nothing here is contingent on the answer: if AWS's counter turns out to
//! roll at a different instant, the dashboard label is a UX wrinkle to word
//! around, and the cap is ours to define regardless.

use chrono::{Datelike, NaiveDate, Utc};

/// The current quota period — a calendar month in UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    /// The 1st of the month.
    start: NaiveDate,
    /// The 1st of the following month: the instant the period ends, and the
    /// instant a rework capped in this period becomes available again.
    next_start: NaiveDate,
}

impl Period {
    /// The period containing `today`.
    ///
    /// Pure, so the December rollover and the leap year are decidable by a
    /// unit test rather than by waiting for one. AWS is deliberately not
    /// consulted — see the module docs.
    pub fn containing(today: NaiveDate) -> Self {
        let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .expect("the 1st of a real month exists");
        let next_start = if today.month() == 12 {
            NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
        }
        .expect("the 1st of the following month exists");
        Self { start, next_start }
    }

    /// The period the clock says we are in, UTC.
    pub fn now() -> Self {
        Self::containing(Utc::now().date_naive())
    }

    /// First day of the period, `YYYY-MM-DD`.
    pub fn start_ymd(&self) -> String {
        self.start.format("%Y-%m-%d").to_string()
    }

    /// Last day of the period, inclusive, `YYYY-MM-DD` — what `GetUsage`'s
    /// `endDate` expects.
    pub fn end_ymd(&self) -> String {
        self.next_start
            .pred_opt()
            .expect("the day before the 1st exists")
            .format("%Y-%m-%d")
            .to_string()
    }

    /// First day of the **following** period, `YYYY-MM-DD` — the date a
    /// rework refused in this period names as "next eligible".
    pub fn next_start_ymd(&self) -> String {
        self.next_start.format("%Y-%m-%d").to_string()
    }

    /// The instant this period ends and the next begins, RFC 3339 — the
    /// dashboard's `resets_at` and the rework refusal's `next_eligible_at`
    /// are the same instant by construction.
    pub fn resets_at(&self) -> String {
        format!("{}T00:00:00Z", self.next_start_ymd())
    }

    /// The instant this period began, in Unix seconds — what a key's
    /// `createdDate` (also Unix seconds) is compared against.
    pub fn start_secs(&self) -> u64 {
        let secs = self
            .start
            .and_hms_opt(0, 0, 0)
            .expect("midnight exists")
            .and_utc()
            .timestamp();
        u64::try_from(secs).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(y: i32, m: u32, d: u32) -> Period {
        Period::containing(NaiveDate::from_ymd_opt(y, m, d).unwrap())
    }

    /// Mid-month, mid-year — the ordinary case.
    #[test]
    fn the_period_is_the_calendar_month() {
        let p = period(2026, 8, 19);
        assert_eq!(p.start_ymd(), "2026-08-01");
        assert_eq!(p.end_ymd(), "2026-08-31");
        assert_eq!(p.next_start_ymd(), "2026-09-01");
        assert_eq!(p.resets_at(), "2026-09-01T00:00:00Z");
    }

    /// The 1st and the last day are both inside their own month.
    #[test]
    fn the_boundaries_belong_to_their_month() {
        let first = period(2026, 9, 1);
        assert_eq!(first.start_ymd(), "2026-09-01");
        assert_eq!(first.end_ymd(), "2026-09-30");
        let last = period(2026, 9, 30);
        assert_eq!(last.start_ymd(), "2026-09-01");
        assert_eq!(last.resets_at(), "2026-10-01T00:00:00Z");
    }

    /// December resets into the next year.
    #[test]
    fn december_rolls_into_january() {
        let p = period(2026, 12, 31);
        assert_eq!(p.start_ymd(), "2026-12-01");
        assert_eq!(p.end_ymd(), "2026-12-31");
        assert_eq!(p.resets_at(), "2027-01-01T00:00:00Z");
    }

    /// February in a leap year has its 29th.
    #[test]
    fn a_leap_february_ends_on_the_29th() {
        let p = period(2028, 2, 15);
        assert_eq!(p.end_ymd(), "2028-02-29");
        assert_eq!(p.resets_at(), "2028-03-01T00:00:00Z");
    }

    /// The instant the period began, as the number a `createdDate` is
    /// compared against: 2026-08-01T00:00:00Z is 1 785 542 400.
    #[test]
    fn the_start_instant_is_midnight_utc_on_the_first() {
        assert_eq!(period(2026, 8, 21).start_secs(), 1_785_542_400);
        assert_eq!(period(2026, 8, 1).start_secs(), 1_785_542_400);
        // Epoch-adjacent dates do not underflow.
        assert_eq!(period(1970, 1, 15).start_secs(), 0);
    }
}
