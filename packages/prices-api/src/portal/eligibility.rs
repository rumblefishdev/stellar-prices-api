//! Who may be issued a key (task 0189).
//!
//! The whole of the epic's abuse story, per ADR 0010 §3: a key is issuable
//! only by a **member of the Stellar Discord** whose account is **older than a
//! configured minimum** — and both facts are proved *per action*, by a fresh
//! OAuth round-trip, never carried in the session (§8). A signed "eligible"
//! claim would date the verdict to sign-in time; this module's verdicts are
//! dated to the moment the callback holds a fresh token.
//!
//! | Path | Re-auth | Checks |
//! | --- | --- | --- |
//! | Sign in | — | identity only |
//! | Issue a key | **yes** | membership (`pending === false`) + account age |
//! | Reveal / usage | no | session only |
//! | Replace / revoke ([0191]) | no | session only — a **deliberate exception**: a user must be able to kill a leaked key while Discord is down, and the action issues nothing. The replacement is an ordinary issue in the next quota period |
//!
//! Account age is checked only at issuance because an account old enough once
//! is old enough forever.
//!
//! # The non-goal, stated so nobody "fixes" it
//!
//! **A user who leaves the guild after issuance keeps their key.** Sign-in
//! proves membership at the moment of issuance and nothing afterwards; reveal
//! and usage never consult Discord at all. What a departed member loses is the
//! right to be issued a *replacement* after a revoke ([0191]) — that is an
//! issue, and issue re-proves membership.
//! The registry stores no membership data — every check reads Discord live at
//! the moment it matters, which is also why there is nothing here to expire.
//!
//! # Four refusals, not two
//!
//! [`decide`] answers eligible / not-a-member / **pending screening** /
//! too-young / **unknown**, and
//! `Unknown` is a first-class verdict rather than an error: a throttled or
//! down Discord must refuse issuance *without accusing the visitor of
//! non-membership*, because "could not verify" is fixable by waiting and "you
//! are not a member" is an accusation they can only disprove by joining again.
//! Only a confirmed `404` carrying Discord's own "no such membership" code
//! ever reads as not-a-member — see `discord::classify_member_response`.
//!
//! # The two knobs are operator-seeded SSM parameters
//!
//! `/prices/{env}/discord-guild-id` and `/prices/{env}/min-account-age-minutes`,
//! read **at runtime, per action** — never `valueForStringParameter`, which
//! freezes the value into the deployed template, and never a CDK-owned
//! `StringParameter`, which the next `cdk deploy` would silently restore
//! (un-flipping production back to the test guild after [0179] step 4). The
//! same ownership split as the OAuth secret: CDK owns the *names*, the
//! operator owns the *values*. Reads go through the Parameters and Secrets
//! extension like the plan id (`config::fetch_plan_id`), whose in-process
//! cache (~5 min) is the only delay between an operator's `put-parameter` and
//! the running Lambda honouring it — no redeploy.

use serde::Serialize;

use super::auth::discord::MemberLookup;

/// Discord's epoch: 2015-01-01T00:00:00Z, in milliseconds since Unix epoch.
/// The high 42 bits of a snowflake are milliseconds since this instant.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// How long one parameter read may take.
///
/// The reads go to the Parameters and Secrets extension on localhost, which
/// answers from its own cache in microseconds when warm and makes a real SSM
/// call when cold — and had no bound at all, so a cold, throttled read could
/// sit inside the sign-in callback until the Lambda was killed. Two seconds is
/// far above a warm read and below what the callback can afford: it is the
/// `parameters` term in the arithmetic written out on `REQUEST_TIMEOUT` in
/// `portal/auth/discord.rs`, which is what keeps the worst case under the 15s
/// invocation timeout.
///
/// A read that exceeds it is [`EligibilityError::Fetch`], which every caller
/// already renders as "could not verify" rather than as an accusation.
/// Only the extension client can be slow — the `Direct` source is a value
/// already in memory, and the build without the client fails immediately — so
/// the constant lives with the code that can actually wait.
///
/// `pub(crate)` and unconditional so `auth::issue`'s budget test can add it
/// up against the invocation timeout in every build, not only the one with
/// the client that waits on it.
#[cfg_attr(not(feature = "aws-mtls"), allow(dead_code))]
pub(crate) const PARAMETER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Where one eligibility parameter's value comes from.
#[derive(Debug, Clone)]
pub enum ParamSource {
    /// A literal value, for local runs and tests. Only constructed from the
    /// environment in non-`lambda` builds — see `config::load_portal_eligibility`.
    Direct(String),
    /// The **name** of an SSM parameter, fetched per action so an operator's
    /// change takes effect without a redeploy.
    Ssm(String),
}

impl ParamSource {
    /// Resolve the current value, trimmed.
    ///
    /// Trimmed for the same reason the plan id is: an operator's
    /// `echo <id> | aws ssm put-parameter` leaves a trailing newline, and the
    /// guild id becomes a URL path segment.
    pub async fn resolve(&self) -> Result<String, EligibilityError> {
        match self {
            Self::Direct(value) => Ok(value.trim().to_string()),
            Self::Ssm(name) => Ok(fetch_parameter(name).await?.trim().to_string()),
        }
    }
}

/// The two operator-tunable knobs, resolved per action.
#[derive(Debug, Clone)]
pub struct EligibilitySettings {
    pub guild_id: ParamSource,
    pub min_account_age: ParamSource,
}

impl EligibilitySettings {
    /// The guild whose membership gates issuance.
    pub async fn guild_id(&self) -> Result<String, EligibilityError> {
        let id = self.guild_id.resolve().await?;
        if id.is_empty() {
            return Err(EligibilityError::Empty {
                what: "discord-guild-id",
            });
        }
        // The SHAPE, not merely the presence — and by the same predicate the
        // caller uses to build the member URL, so the two cannot drift.
        //
        // Checking only for emptiness here is what let a guild *name* through:
        // `stellar_test` passed the cold-start probe, so the deploy that
        // opened the portal came up green, and the refusal happened once per
        // visitor instead — as `Unknown`, which is the arm that deliberately
        // says nothing about anybody's membership. Every member would have
        // been told "we could not verify", indefinitely, with the actual fault
        // one `put-parameter` away. This is the failure the probe exists to
        // turn into a cold-start error — a closed portal and a
        // `portal closed at cold start` log line, per
        // `AppConfig::load_portal_or_close`.
        if !crate::portal::auth::discord::is_snowflake(&id) {
            return Err(EligibilityError::NotSnowflake {
                what: "discord-guild-id",
            });
        }
        Ok(id)
    }

    /// The minimum account age, in minutes.
    pub async fn min_account_age_minutes(&self) -> Result<u64, EligibilityError> {
        let raw = self.min_account_age.resolve().await?;
        raw.parse().map_err(|_| EligibilityError::NotMinutes {
            what: "min-account-age-minutes",
        })
    }
}

/// Why a parameter could not be resolved.
///
/// At cold start (the probe in `config::load_portal_eligibility`) any of these
/// is fatal; at action time they all land in [`Eligibility::Unknown`] — the
/// visitor is refused without accusation, and the log names the real fault.
#[derive(Debug, thiserror::Error)]
pub enum EligibilityError {
    #[error("reading SSM parameter `{name}` failed: {message}")]
    Fetch { name: String, message: String },
    #[error("the `{what}` parameter is empty")]
    Empty { what: &'static str },
    #[error(
        "the `{what}` parameter is not a Discord snowflake (all digits — the official \
         Stellar Developers guild is 897514728459468821) — a guild NAME will not work here"
    )]
    NotSnowflake { what: &'static str },
    #[error("the `{what}` parameter is not a whole number of minutes")]
    NotMinutes { what: &'static str },
}

/// Read one parameter through the Parameters and Secrets extension — the same
/// localhost listener, token and in-process cache the mTLS bundle, the OAuth
/// secret and the plan id already use. The extension's cache is what bounds
/// how quickly an operator's change is honoured (~5 min), and is also why a
/// per-action read does not call Systems Manager on a warm container.
#[cfg(feature = "aws-mtls")]
async fn fetch_parameter(name: &str) -> Result<String, EligibilityError> {
    let fetch = prices_clickhouse::mtls::fetch_parameter_string(name);
    match tokio::time::timeout(PARAMETER_TIMEOUT, fetch).await {
        Ok(result) => result.map_err(|e| EligibilityError::Fetch {
            name: name.to_string(),
            message: e.to_string(),
        }),
        Err(_) => {
            tracing::error!(
                name,
                timeout_secs = PARAMETER_TIMEOUT.as_secs(),
                "eligibility parameter read timed out"
            );
            Err(EligibilityError::Fetch {
                name: name.to_string(),
                message: format!(
                    "the parameter read did not answer within {}s",
                    PARAMETER_TIMEOUT.as_secs()
                ),
            })
        }
    }
}

#[cfg(not(feature = "aws-mtls"))]
async fn fetch_parameter(name: &str) -> Result<String, EligibilityError> {
    Err(EligibilityError::Fetch {
        name: name.to_string(),
        message: "this build has no Parameters and Secrets extension client (build with \
                  `--features lambda`, or set PORTAL_GUILD_ID and \
                  PORTAL_MIN_ACCOUNT_AGE_MINUTES for a local run)"
            .into(),
    })
}

/// The verdict on one issuance attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Eligibility {
    /// A member with `pending == Some(false)` and an old-enough account.
    Eligible,
    /// Confirmed non-membership: Discord's own 10007/10004 on a 404.
    NotMember,
    /// On the server but not through its Membership Screening — the rules
    /// checkbox — so `pending: true`. Its own verdict since task 0254 (it was
    /// folded into [`NotMember`] before): the remedy is "accept the rules",
    /// not "join", and telling a member to join sends them to an invite that
    /// only says they are already in.
    PendingScreening,
    /// The account exists but is younger than the threshold. A *wait*, not a
    /// rejection: `wait_secs` is what the page renders, so the copy follows
    /// the operator's threshold instead of hard-coding one.
    TooYoung { wait_secs: u64 },
    /// Could not verify: Discord answered 401/403/429/5xx or an unrecognised
    /// shape, `pending` was absent, or the snowflake would not parse. Refuse,
    /// but never claim non-membership.
    Unknown,
}

/// When the account behind `snowflake` was created, in ms since Unix epoch.
///
/// `(id >> 22) + DISCORD_EPOCH_MS`, per Discord's snowflake layout. Done in
/// `u64` — the task's `BigInt` note is about *JavaScript*, where `Number`
/// loses integer precision above 2^53; a snowflake fits comfortably in 64
/// bits and this backend never puts one in a float. `None` when the string is
/// not a bare integer, which the caller treats as [`Eligibility::Unknown`].
pub fn account_created_ms(snowflake: &str) -> Option<u64> {
    let id: u64 = snowflake.parse().ok()?;
    Some((id >> 22) + DISCORD_EPOCH_MS)
}

/// Combine one membership answer and one account age into a verdict.
///
/// Pure, so the whole decision table is unit-tested. Precedence: membership
/// first, then age — a non-member's account age is not their problem yet, and
/// "join the server" plus "wait four minutes" as two sequential messages beats
/// both at once.
///
/// The `pending` rules, each deliberate and each reversible once 0180's
/// measurements exist:
///
/// - `Some(false)` **passes**. This is true even when an admin waved the user
///   through via `BYPASSES_VERIFICATION` — ADR 0010 reads "must be a member"
///   as "the guild considers them a full member", and second-guessing an
///   admin's bypass would require the `flags` field this service deliberately
///   does not read.
/// - `Some(true)` is **pending screening** — they joined but have not
///   accepted the rules. Refused before the age check like a non-member, but
///   as its own verdict, because its remedy is a different click.
/// - `None` is **unknown**, never a pass: the docs' presence guarantee for
///   `pending` is written about gateway events, not this REST route, and 0180
///   item 2 (which would settle what absence means here) is unmeasured. If
///   measurement shows the field is simply absent on REST, this one arm is
///   what changes — logged loudly (`pending_absent`) so the gap is visible in
///   CloudWatch the first time it fires rather than silently refusing every
///   member.
pub fn decide(
    member: &MemberLookup,
    snowflake: &str,
    min_age_minutes: u64,
    now_ms: u64,
) -> Eligibility {
    match membership(member) {
        Membership::Member => {}
        Membership::NotMember => return Eligibility::NotMember,
        Membership::PendingScreening => return Eligibility::PendingScreening,
        Membership::Unknown => return Eligibility::Unknown,
    }

    let Some(created_ms) = account_created_ms(snowflake) else {
        tracing::warn!("a Discord user id did not parse as a snowflake; cannot derive account age");
        return Eligibility::Unknown;
    };

    let old_enough_at = created_ms.saturating_add(min_age_minutes.saturating_mul(60_000));
    if now_ms >= old_enough_at {
        return Eligibility::Eligible;
    }
    Eligibility::TooYoung {
        wait_secs: (old_enough_at - now_ms).div_ceil(1000),
    }
}

/// The membership half of the verdict, on its own.
///
/// What a **rework** (task 0191) re-proves: the per-action table at the top
/// of this module says membership only, never age, because an account old
/// enough once is old enough forever. [`decide`] is this plus the age check,
/// so the two paths cannot disagree about what a member is — the `pending`
/// rules below are the single statement of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// `pending == Some(false)` — a full member, admin bypass included.
    Member,
    /// Confirmed non-membership.
    NotMember,
    /// `pending == Some(true)` — joined, rules not yet accepted.
    PendingScreening,
    /// Could not verify, including an absent `pending`. Refuse, never accuse.
    Unknown,
}

/// Classify one membership answer. See [`decide`] for the `pending` rules,
/// which live in this function and are documented there.
pub fn membership(member: &MemberLookup) -> Membership {
    match member {
        MemberLookup::NotMember { .. } => Membership::NotMember,
        MemberLookup::Unknown { status, detail } => {
            tracing::warn!(?status, detail, "membership could not be verified");
            Membership::Unknown
        }
        MemberLookup::Member(m) => match m.pending {
            Some(false) => Membership::Member,
            Some(true) => Membership::PendingScreening,
            None => {
                tracing::warn!(
                    reason = "pending_absent",
                    "the member response carried no `pending` field; refusing without \
                     accusation — see 0180 item 2 before changing this arm"
                );
                Membership::Unknown
            }
        },
    }
}

/// Milliseconds since the Unix epoch, saturating like `state_token::now_secs`:
/// a clock before 1970 makes every account look brand new, which fails closed.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::auth::discord::GuildMember;

    /// A snowflake with a known timestamp: Discord's own documentation example
    /// `175928847299117063` decodes to 2016-04-30 11:18:25.796 UTC.
    const DOCUMENTED_SNOWFLAKE: &str = "175928847299117063";
    const DOCUMENTED_CREATED_MS: u64 = 1_462_015_105_796;

    fn member(pending: Option<bool>) -> MemberLookup {
        MemberLookup::Member(GuildMember { pending })
    }

    fn unknown() -> MemberLookup {
        MemberLookup::Unknown {
            status: None,
            detail: "test".into(),
        }
    }

    #[test]
    fn the_snowflake_epoch_math_matches_discords_documented_example() {
        assert_eq!(
            account_created_ms(DOCUMENTED_SNOWFLAKE),
            Some(DOCUMENTED_CREATED_MS)
        );
    }

    /// The id in the JSON is a string because it exceeds 2^53 — the reason the
    /// task says `BigInt` for JavaScript. In u64 the math is exact; this pins
    /// it with an id above 2^53 so a future refactor through `f64` fails here.
    #[test]
    fn an_id_above_2_pow_53_is_exact_not_rounded() {
        let id: u64 = (1 << 55) + 4_194_305; // low bits would vanish in an f64
        let created = account_created_ms(&id.to_string()).unwrap();
        assert_eq!(created, (id >> 22) + DISCORD_EPOCH_MS);
        // The nearest f64 to `id` is a different integer; if a float snuck in,
        // the shifted result would differ.
        assert_ne!((id as f64) as u64, id);
    }

    #[test]
    fn a_non_numeric_id_is_none_not_a_panic() {
        for bad in ["", "abc", "12.5", "-3", "1e10", " 175928847299117063"] {
            assert_eq!(account_created_ms(bad), None, "parsed {bad:?}");
        }
    }

    #[test]
    fn a_cleared_member_with_an_old_account_is_eligible() {
        let now = DOCUMENTED_CREATED_MS + 6 * 60_000;
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 5, now),
            Eligibility::Eligible
        );
    }

    /// Exactly at the threshold passes — "older than five minutes" is enforced
    /// as `>=` because the boundary instant is not worth a refusal, and the
    /// test pins which side the boundary is on so it cannot drift silently.
    #[test]
    fn the_age_boundary_passes_exactly_at_the_threshold() {
        let threshold = DOCUMENTED_CREATED_MS + 5 * 60_000;
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 5, threshold),
            Eligibility::Eligible
        );
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 5, threshold - 1),
            Eligibility::TooYoung { wait_secs: 1 }
        );
    }

    /// `wait_secs` is a ceiling: 90.001 seconds remaining renders as 91, never
    /// as 90-then-refused-again.
    #[test]
    fn the_wait_is_rounded_up_to_whole_seconds() {
        let now = DOCUMENTED_CREATED_MS + 5 * 60_000 - 90_001;
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 5, now),
            Eligibility::TooYoung { wait_secs: 91 }
        );
    }

    /// A changed SSM value changes the verdict with no other input changing —
    /// the "tunable without a redeploy" property at the decision layer.
    #[test]
    fn the_threshold_is_an_input_not_a_constant() {
        let now = DOCUMENTED_CREATED_MS + 6 * 60_000;
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 5, now),
            Eligibility::Eligible
        );
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 10, now),
            Eligibility::TooYoung { wait_secs: 240 }
        );
        assert_eq!(
            decide(&member(Some(false)), DOCUMENTED_SNOWFLAKE, 0, now),
            Eligibility::Eligible
        );
    }

    #[test]
    fn a_confirmed_non_member_is_refused_before_age_is_even_computed() {
        // The nonsense snowflake would be Unknown if age ran first; the
        // membership verdict takes precedence.
        assert_eq!(
            decide(
                &MemberLookup::NotMember { code: 10_007 },
                "not-a-flake",
                5,
                0
            ),
            Eligibility::NotMember
        );
    }

    /// Joined, rules not accepted: refused, and NOT as a non-member — the
    /// page must be able to say "accept the rules" rather than "join".
    #[test]
    fn a_pending_member_is_refused_as_pending_screening() {
        let now = DOCUMENTED_CREATED_MS + 6 * 60_000;
        assert_eq!(
            decide(&member(Some(true)), DOCUMENTED_SNOWFLAKE, 5, now),
            Eligibility::PendingScreening
        );
    }

    /// …and before the age check, like every membership verdict: a member in
    /// screening on a brand-new account is told about the rules, not to wait.
    #[test]
    fn pending_screening_takes_precedence_over_age() {
        assert_eq!(
            decide(&member(Some(true)), "not-a-flake", 5, 0),
            Eligibility::PendingScreening
        );
    }

    /// The task's own wording: `pending === undefined` "is handled explicitly
    /// and does not silently pass". It is Unknown — refused, not accused.
    #[test]
    fn an_absent_pending_field_is_unknown_never_a_pass() {
        let now = DOCUMENTED_CREATED_MS + 6 * 60_000;
        assert_eq!(
            decide(&member(None), DOCUMENTED_SNOWFLAKE, 5, now),
            Eligibility::Unknown
        );
    }

    #[test]
    fn an_unverifiable_membership_is_unknown_whatever_the_age() {
        let now = DOCUMENTED_CREATED_MS + 6 * 60_000;
        assert_eq!(
            decide(&unknown(), DOCUMENTED_SNOWFLAKE, 5, now),
            Eligibility::Unknown
        );
    }

    /// The cold-start probe must refuse a guild **name**.
    ///
    /// `stellar_test` is the value the task's own parameter table named for
    /// the build period, and before this check it passed: the deploy came up
    /// green and every visitor was told "we could not verify your Discord
    /// membership" instead, because the shape was only checked where the URL
    /// is built. Same predicate both sides now.
    #[tokio::test]
    async fn a_guild_name_is_refused_by_the_probe_rather_than_every_visitor() {
        let settings = EligibilitySettings {
            guild_id: ParamSource::Direct("stellar_test".into()),
            min_account_age: ParamSource::Direct("5".into()),
        };
        assert!(matches!(
            settings.guild_id().await,
            Err(EligibilityError::NotSnowflake { .. })
        ));
    }

    /// …and a real snowflake still resolves, trimmed.
    #[tokio::test]
    async fn a_snowflake_resolves() {
        let settings = EligibilitySettings {
            guild_id: ParamSource::Direct("  897514728459468821\n".into()),
            min_account_age: ParamSource::Direct("5".into()),
        };
        assert_eq!(settings.guild_id().await.unwrap(), "897514728459468821");
    }

    /// The rework path reads [`membership`] alone; it must agree with
    /// [`decide`] on every membership shape, or a rework could be allowed to
    /// someone an issue would refuse (or the reverse).
    #[test]
    fn the_membership_half_agrees_with_the_full_verdict() {
        let now = DOCUMENTED_CREATED_MS + 6 * 60_000;
        let cases = [
            (member(Some(false)), Membership::Member),
            (member(Some(true)), Membership::PendingScreening),
            (member(None), Membership::Unknown),
            (
                MemberLookup::NotMember { code: 10_007 },
                Membership::NotMember,
            ),
            (
                MemberLookup::NotMember { code: 10_004 },
                Membership::NotMember,
            ),
            (unknown(), Membership::Unknown),
        ];
        for (lookup, expected) in cases {
            assert_eq!(membership(&lookup), expected, "{lookup:?}");
            let full = decide(&lookup, DOCUMENTED_SNOWFLAKE, 5, now);
            match expected {
                Membership::Member => assert_eq!(full, Eligibility::Eligible),
                Membership::NotMember => assert_eq!(full, Eligibility::NotMember),
                Membership::PendingScreening => {
                    assert_eq!(full, Eligibility::PendingScreening)
                }
                Membership::Unknown => assert_eq!(full, Eligibility::Unknown),
            }
        }
    }

    #[test]
    fn an_unparseable_snowflake_is_unknown_not_too_young() {
        assert_eq!(
            decide(&member(Some(false)), "not-a-snowflake", 5, u64::MAX),
            Eligibility::Unknown
        );
    }

    #[tokio::test]
    async fn a_direct_source_resolves_trimmed() {
        let settings = EligibilitySettings {
            guild_id: ParamSource::Direct("  897514728459468821\n".into()),
            min_account_age: ParamSource::Direct("5\n".into()),
        };
        assert_eq!(settings.guild_id().await.unwrap(), "897514728459468821");
        assert_eq!(settings.min_account_age_minutes().await.unwrap(), 5);
    }

    #[tokio::test]
    async fn an_empty_or_non_numeric_parameter_is_a_named_error() {
        let empty = EligibilitySettings {
            guild_id: ParamSource::Direct("  ".into()),
            min_account_age: ParamSource::Direct("five".into()),
        };
        assert!(matches!(
            empty.guild_id().await.unwrap_err(),
            EligibilityError::Empty { .. }
        ));
        assert!(matches!(
            empty.min_account_age_minutes().await.unwrap_err(),
            EligibilityError::NotMinutes { .. }
        ));
    }

    /// A build without the extension client refuses an SSM source with a
    /// message naming the local seams — the same behaviour `fetch_plan_id`
    /// has, asserted here so the two cannot drift apart silently.
    #[cfg(not(feature = "aws-mtls"))]
    #[tokio::test]
    async fn an_ssm_source_in_a_non_extension_build_names_the_local_seams() {
        let settings = EligibilitySettings {
            guild_id: ParamSource::Ssm("/prices/test/discord-guild-id".into()),
            min_account_age: ParamSource::Ssm("/prices/test/min-account-age-minutes".into()),
        };
        let error = settings.guild_id().await.unwrap_err();
        assert!(error.to_string().contains("PORTAL_GUILD_ID"));
    }
}
