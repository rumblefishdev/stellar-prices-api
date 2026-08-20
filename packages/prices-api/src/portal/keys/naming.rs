//! Key names, the exact-match filter, and the winner rule (task 0187).
//!
//! Everything in this file is pure: no AWS, no I/O, no clock. That is
//! deliberate, because this is where the two ways this slice could quietly harm
//! somebody live — picking the wrong key to hand out, and picking the wrong key
//! to **delete** — and both must be decidable by a test that runs in
//! microseconds against a list somebody typed by hand.
//!
//! # The name, and why the suffix is not decoration
//!
//! A user's key is named `discord-<userId>-key`. `GetApiKeys(nameQuery = …)` is
//! a **case-sensitive prefix match** — measured on 2026-08-12, recorded in the
//! archived `0180/notes/R-apigw-namequery-quota-and-disable.md` — so a query is
//! answered with everything that *starts with* what was asked for, and AWS
//! never promised otherwise.
//!
//! Discord snowflakes are 17-19 digits, so ids prefix one another constantly:
//! `308994132968210433` starts with `30899413`. Had the name been
//! `discord-<userId>`, a query for the shorter id would return the longer id's
//! key, and [`choose_winner`] below would rank a stranger's key against the
//! caller's — with [`losers`] then **deleting** whichever lost. The trailing
//! `-key` is what makes that impossible: for `discord-30899413-key` to prefix
//! another user's name, that user's id would have to begin `30899413-key`, and
//! ids are digits.
//!
//! # The client-side exact filter is load-bearing too
//!
//! The suffix closes the snowflake hazard; it does not make the query exact.
//! `discord-123-key` still prefixes `discord-123-key-old`, `discord-123-keys`
//! and anything else a human types in the console — and console-created keys
//! are exactly what a reconciler with `DeleteApiKey` must not touch. So the
//! result of every list is put through [`exact_matches`] before anything is
//! ranked, returned or deleted.
//!
//! **Do not simplify this away** on the grounds that the query already narrows
//! it. AWS returns prefixes and never promised not to.

/// Fixed head of every self-service key name.
pub const NAME_PREFIX: &str = "discord-";

/// Fixed tail. See the module docs — this is a safety property, not a label.
pub const NAME_SUFFIX: &str = "-key";

/// Longest Discord user id accepted into a key name.
///
/// Snowflakes are 17-19 digits today and grow by one digit roughly every few
/// decades, so 32 is far past any real value while still being a bound. It
/// exists so that a `sub` from a forged-or-corrupt session cannot produce an
/// unbounded `nameQuery`.
const MAX_USER_ID_LEN: usize = 32;

/// The API Gateway key name for a Discord user, or `None` if the id is not one.
///
/// The id arrives from a session cookie this service signed, so it is not
/// attacker-chosen in any deployment where the signing key is intact. Validated
/// anyway, and digits-only rather than merely non-empty, because the digits are
/// what the module's prefix argument rests on: an id containing `-key` would
/// undo it. This is the one check standing between "our signing key leaked" and
/// "the reconciler can be aimed at an arbitrary name".
pub fn key_name(user_id: &str) -> Option<String> {
    let plausible = !user_id.is_empty()
        && user_id.len() <= MAX_USER_ID_LEN
        && user_id.bytes().all(|b| b.is_ascii_digit());
    plausible.then(|| format!("{NAME_PREFIX}{user_id}{NAME_SUFFIX}"))
}

/// One API Gateway key, reduced to the three fields this slice reasons about.
///
/// Carries no value — see [`super::gateway::KeyValue`], which is a separate type
/// precisely so that a record can be logged and a value cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRecord {
    pub id: String,
    pub name: String,
    /// Creation time in Unix seconds, as reported by API Gateway.
    ///
    /// `None` when the field is absent from the response. AWS always sends it;
    /// the option exists because the SDK types it that way, and [`choose_winner`]
    /// ranks a missing value **last** so that a key we cannot date can never
    /// beat one we can.
    pub created_at: Option<u64>,
}

/// Keep only the records whose name is **exactly** `name`.
///
/// The filter the module docs argue for. Byte equality, not
/// case-insensitive: the control plane's own match is case-sensitive, so
/// relaxing it here would accept a `DISCORD-123-KEY` that the query would never
/// have returned and the console treats as a different key.
pub fn exact_matches(records: Vec<KeyRecord>, name: &str) -> Vec<KeyRecord> {
    records.into_iter().filter(|r| r.name == name).collect()
}

/// Rank order: earliest creation first, then id.
///
/// The id tie-break is not cosmetic. It is what makes the choice **the same on
/// both sides of a double-submit**: two Lambda invocations that list the same
/// two keys must agree about which one survives, or each deletes the other's and
/// the user ends up with none. `created_at` alone does not give that — API
/// Gateway reports it in whole seconds, so two keys created in the same second
/// tie, and a tie broken by list order is broken by whatever order the service
/// happened to return.
fn rank(record: &KeyRecord) -> (u64, &str) {
    (record.created_at.unwrap_or(u64::MAX), record.id.as_str())
}

/// The key that survives reconciliation: earliest `created_at`, id as tie-break.
pub fn choose_winner(records: &[KeyRecord]) -> Option<&KeyRecord> {
    records.iter().min_by_key(|r| rank(r))
}

/// Everything that is not the winner, in the order they should be deleted.
///
/// Returned as a separate step from [`choose_winner`] so that a caller cannot
/// accidentally delete without having named a winner first — and so that the
/// list of things about to be destroyed is a value a test can assert on rather
/// than a control-flow path it has to reproduce.
pub fn losers<'a>(records: &'a [KeyRecord], winner: &KeyRecord) -> Vec<&'a KeyRecord> {
    records.iter().filter(|r| r.id != winner.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, name: &str, created_at: Option<u64>) -> KeyRecord {
        KeyRecord {
            id: id.into(),
            name: name.into(),
            created_at,
        }
    }

    #[test]
    fn a_name_is_the_id_wrapped_in_both_fixed_parts() {
        assert_eq!(
            key_name("308994132968210433").as_deref(),
            Some("discord-308994132968210433-key")
        );
    }

    /// The suffix is what stops one snowflake's query matching a longer one.
    /// Stated as an assertion rather than a comment so that deleting it from
    /// [`key_name`] fails here.
    #[test]
    fn a_shorter_id_cannot_prefix_a_longer_ids_name() {
        let short = key_name("30899413").unwrap();
        let long = key_name("308994132968210433").unwrap();
        assert!(
            !long.starts_with(&short),
            "`{long}` starts with `{short}` — the -key suffix has been lost, and \
             GetApiKeys(nameQuery) is a PREFIX match, so the reconciler would rank \
             and delete another user's key"
        );
    }

    #[test]
    fn a_non_snowflake_gets_no_name_at_all() {
        for hostile in [
            "",
            "abc",
            "123-key",
            "123 456",
            "*",
            "../../etc",
            "12345678901234567890123456789012345",
        ] {
            assert_eq!(key_name(hostile), None, "accepted `{hostile}`");
        }
    }

    /// The query narrows; only this makes it exact.
    #[test]
    fn the_filter_drops_names_the_prefix_query_would_still_return() {
        let name = key_name("123").unwrap();
        let listed = vec![
            record("a", "discord-123-key", Some(10)),
            record("b", "discord-123-key-old", Some(11)),
            record("c", "discord-123-keys", Some(12)),
            record("d", "discord-1234-key", Some(13)),
            record("e", "DISCORD-123-KEY", Some(14)),
        ];
        let kept = exact_matches(listed, &name);
        assert_eq!(
            kept.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
    }

    #[test]
    fn the_earliest_key_wins_and_the_rest_are_losers() {
        let records = vec![
            record("late", "n", Some(200)),
            record("early", "n", Some(100)),
            record("middle", "n", Some(150)),
        ];
        let winner = choose_winner(&records).unwrap();
        assert_eq!(winner.id, "early");
        assert_eq!(
            losers(&records, winner)
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["late", "middle"]
        );
    }

    /// Both sides of a double-submit must compute the same winner from the same
    /// list, whatever order the service returned it in. Without the id
    /// tie-break, two keys created in the same second are ranked by list order
    /// and each invocation deletes the other's.
    #[test]
    fn a_same_second_tie_is_broken_identically_whatever_the_list_order() {
        let a = record("aaa", "n", Some(100));
        let b = record("bbb", "n", Some(100));
        let one = vec![a.clone(), b.clone()];
        let other = vec![b, a];
        assert_eq!(choose_winner(&one).unwrap().id, "aaa");
        assert_eq!(choose_winner(&other).unwrap().id, "aaa");
    }

    /// A key we cannot date must never beat one we can — otherwise a missing
    /// field turns into a deletion of the real key.
    #[test]
    fn an_undated_key_ranks_last() {
        let records = vec![
            record("undated", "n", None),
            record("dated", "n", Some(999)),
        ];
        assert_eq!(choose_winner(&records).unwrap().id, "dated");
    }

    #[test]
    fn nothing_wins_and_nothing_loses_in_an_empty_list() {
        assert!(choose_winner(&[]).is_none());
    }

    #[test]
    fn a_single_key_has_no_losers() {
        let records = vec![record("only", "n", Some(1))];
        let winner = choose_winner(&records).unwrap();
        assert!(losers(&records, winner).is_empty());
    }
}
