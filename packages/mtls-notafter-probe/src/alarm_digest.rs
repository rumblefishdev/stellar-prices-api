//! Daily digest of alarms stuck off OK (task 0214).
//!
//! `prices-production-enrichment-errors` sat in ALARM for **28 days** while the
//! enrichment pass failed on every single invocation, and nobody acted. The
//! alarm was not broken: it fired, it routed to Slack, and it was then scrolled
//! past for four weeks. CloudWatch notifies on **state transitions only**, so an
//! alarm that fires once and is missed never speaks again — and [[0204]] gap 3
//! deliberately left two MV-drift alarms designed to latch.
//!
//! This is the re-read. Once a day the probe asks CloudWatch which of our alarms
//! are not OK, and if any have been that way for more than an hour it publishes
//! one message to the ops topic. It is the cheapest thing that would have caught
//! the 28-day latch on day two.
//!
//! ## What it deliberately does NOT do
//!
//! - **It stays silent when everything is OK.** A daily "all good" would be new
//!   noise on the same channel this task exists to keep readable.
//! - **It ignores anything younger than [`MIN_STUCK_SECONDS`].** A once-a-day
//!   snapshot would otherwise report a three-minute flap as news;
//!   `prices-production-oracle-errors` changed state 62 times in the week of
//!   2026-09-15, and reprinting those would defeat the point. Flap volume is
//!   [[0223]]'s and [[0226]]'s problem, not this one's.
//! - **It does not re-state the alarms' own thresholds or try to diagnose.** It
//!   reports which alarm, which state, and for how long. The alarm description
//!   carries the diagnosis.
//!
//! ## Why the message is a JSON envelope and not a string
//!
//! The ops topic's subscriber is **AWS Chatbot** (`observability-stack.ts:489`),
//! which forwards only payloads it recognises — a CloudWatch alarm event, or the
//! documented *custom notification* schema. An arbitrary string published to the
//! topic is silently dropped, never reaching the channel. So the digest speaks
//! Chatbot's schema ([`chatbot_envelope`]); the readable part is
//! [`digest_description`], which is what the unit tests assert on.

/// One alarm reduced to what the digest needs: what CloudWatch calls it, the
/// state it is in, and when it entered that state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmState {
    pub name: String,
    /// `OK`, `ALARM` or `INSUFFICIENT_DATA`, verbatim from CloudWatch.
    pub state: String,
    /// `StateTransitionedTimestamp` as unix seconds — when the state last
    /// actually CHANGED. Not `StateUpdatedTimestamp`: see [`describe`].
    pub since_unix: i64,
}

/// How long an alarm must have been off OK before the digest mentions it.
///
/// One hour, against a once-a-day run. Lower and a snapshot starts reporting
/// flaps as stuck alarms; higher and a stall that began this morning waits a
/// second day to be named. The 2026-07-27 latch would have been reported on its
/// first digest either way.
pub const MIN_STUCK_SECONDS: i64 = 3_600;

/// The alarms this digest speaks about: not OK, and not OK for a while.
/// Oldest first, because the top line is the one that has been ignored longest.
pub fn stuck(alarms: &[AlarmState], now_unix: i64) -> Vec<&AlarmState> {
    let mut out: Vec<&AlarmState> = alarms
        .iter()
        .filter(|a| a.state != "OK")
        .filter(|a| now_unix.saturating_sub(a.since_unix) >= MIN_STUCK_SECONDS)
        .collect();
    out.sort_by_key(|a| a.since_unix);
    out
}

/// Most alarms listed in one message. A digest naming this many is not a list
/// to work through, it is an outage — and Chatbot caps `description` length.
const MAX_LISTED: usize = 25;

/// The readable digest, or `None` when there is nothing to say.
///
/// `None` is the normal case and means **publish nothing** — see the module
/// docs on why silence is the design rather than an omission.
pub fn digest_description(
    environment: &str,
    alarms: &[AlarmState],
    now_unix: i64,
) -> Option<String> {
    let stuck = stuck(alarms, now_unix);
    if stuck.is_empty() {
        return None;
    }

    let mut out = format!(
        "{} alarm(s) in {} have been off OK for over {}.\n```\n",
        stuck.len(),
        environment,
        humanise_age(MIN_STUCK_SECONDS)
    );
    for a in stuck.iter().take(MAX_LISTED) {
        out.push_str(&format!(
            "{:<17} {}  (for {})\n",
            a.state,
            a.name,
            humanise_age(now_unix.saturating_sub(a.since_unix))
        ));
    }
    if stuck.len() > MAX_LISTED {
        out.push_str(&format!("... and {} more\n", stuck.len() - MAX_LISTED));
    }
    out.push_str(
        "```\nDaily re-read (task 0214). CloudWatch notifies on state changes only, so an \
         alarm that fired once and was scrolled past never speaks again. This is that second \
         chance, not a new incident.",
    );
    Some(out)
}

/// Wrap a description in AWS Chatbot's custom-notification schema. Anything
/// else published to the ops topic is dropped before it reaches Slack.
pub fn chatbot_envelope(title: &str, description: &str) -> String {
    serde_json::json!({
        "version": "1.0",
        "source": "custom",
        "content": {
            "textType": "client-markdown",
            "title": title,
            "description": description,
        }
    })
    .to_string()
}

/// The full payload to `sns:Publish`, or `None` when everything is OK.
pub fn digest_payload(environment: &str, alarms: &[AlarmState], now_unix: i64) -> Option<String> {
    let description = digest_description(environment, alarms, now_unix)?;
    Some(chatbot_envelope(
        &format!("{environment}: alarms stuck off OK"),
        &description,
    ))
}

/// Which of CloudWatch's two timestamps dates a stuck alarm. Pulled out of
/// [`describe`] so the choice is testable without an AWS client — the reason it
/// matters is in that function's docs.
pub fn since_from(transitioned: Option<i64>, updated: Option<i64>) -> Option<i64> {
    transitioned.or(updated)
}

/// Every `prices-{env}-` alarm's current state, for [`digest_payload`].
///
/// Reads metric alarms only — this stack defines no composite alarms — and
/// pages, because `DescribeAlarms` caps a response at 100 and the account is
/// already near that.
///
/// ## `StateTransitionedTimestamp`, not `StateUpdatedTimestamp`
///
/// The two read alike and are equal on all 50 production alarms right now, but
/// AWS defines them differently: `StateUpdatedTimestamp` is the last update to
/// `StateValue` **or `EvaluationState`**, while `StateTransitionedTimestamp` is
/// when `StateValue` itself last changed — which is the quantity "how long has
/// this been off OK" actually means. A transient `PARTIAL_DATA` flip on a
/// 28-day latch refreshes the former and not the latter, so reading the former
/// would silently drop the alarm from the digest, or print `2h 13m` for
/// something ignored for a month. That is this task's own bug, re-introduced
/// one field over.
#[cfg(feature = "lambda")]
pub async fn describe(
    client: &aws_sdk_cloudwatch::Client,
    name_prefix: &str,
) -> Result<Vec<AlarmState>, String> {
    let mut pages = client
        .describe_alarms()
        .alarm_name_prefix(name_prefix)
        .alarm_types(aws_sdk_cloudwatch::types::AlarmType::MetricAlarm)
        .into_paginator()
        .send();

    let mut out = Vec::new();
    while let Some(page) = pages.next().await {
        let page = page.map_err(|e| format!("DescribeAlarms failed: {e}"))?;
        for a in page.metric_alarms() {
            // An alarm missing any of the three is not one we can date, and
            // guessing a state would either invent an incident or hide one.
            let (Some(name), Some(state), Some(since)) = (
                a.alarm_name(),
                a.state_value(),
                since_from(
                    a.state_transitioned_timestamp().map(|t| t.secs()),
                    a.state_updated_timestamp().map(|t| t.secs()),
                ),
            ) else {
                continue;
            };
            out.push(AlarmState {
                name: name.to_string(),
                state: state.as_str().to_string(),
                since_unix: since,
            });
        }
    }
    Ok(out)
}

/// Read the alarms, and publish one message if any are stuck. Returns how many
/// were reported — `0` means the account is healthy and nothing was published.
#[cfg(feature = "lambda")]
pub async fn run(
    cw: &aws_sdk_cloudwatch::Client,
    sns: &aws_sdk_sns::Client,
    topic_arn: &str,
    environment: &str,
    now_unix: i64,
) -> Result<usize, String> {
    let prefix = format!("prices-{environment}-");
    let alarms = describe(cw, &prefix).await?;
    tracing::info!(prefix, matched = alarms.len(), "alarm digest read");

    // Zero matches is never legitimate here — production alone has 50 — so it
    // means the prefix is wrong (an unset or renamed `ENV_NAME` yields
    // `prices-unknown-`). Left as `Ok(0)` that reads as a clean bill of health
    // and the digest stays green and mute forever, which is precisely the
    // silent no-op this whole task exists to end. Fail loudly instead.
    if alarms.is_empty() {
        return Err(format!(
            "no alarms matched `{prefix}` — check ENV_NAME; the digest would report \
             a healthy account by reading nothing"
        ));
    }

    let reported = stuck(&alarms, now_unix).len();
    if let Some(payload) = digest_payload(environment, &alarms, now_unix) {
        sns.publish()
            .topic_arn(topic_arn)
            .subject(format!("{environment}: {reported} alarm(s) stuck off OK"))
            .message(payload)
            .send()
            .await
            .map_err(|e| format!("SNS publish failed: {e}"))?;
    }
    Ok(reported)
}

/// `93784` → `1d 2h`. Coarse on purpose: the digest answers "how long has this
/// been ignored", where minutes stop mattering after the first hour.
pub fn humanise_age(seconds: i64) -> String {
    let s = seconds.max(0);
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3_600, (s % 3_600) / 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_789_000_000;

    fn alarm(name: &str, state: &str, age_seconds: i64) -> AlarmState {
        AlarmState {
            name: name.to_string(),
            state: state.to_string(),
            since_unix: NOW - age_seconds,
        }
    }

    #[test]
    fn a_healthy_account_says_nothing_at_all() {
        let alarms = vec![alarm("prices-production-a", "OK", 10 * 86_400)];
        assert_eq!(digest_description("production", &alarms, NOW), None);
        assert_eq!(digest_payload("production", &alarms, NOW), None);
        assert!(stuck(&alarms, NOW).is_empty());
    }

    #[test]
    fn a_flap_younger_than_the_floor_is_not_stuck() {
        // oracle-errors flips ALARM → OK within minutes, 62 times a week. A daily
        // snapshot that caught one mid-flip must not report it as ignored.
        let alarms = vec![alarm("prices-production-oracle-errors", "ALARM", 180)];
        assert!(stuck(&alarms, NOW).is_empty());
        assert_eq!(digest_payload("production", &alarms, NOW), None);
    }

    #[test]
    fn the_28_day_latch_is_reported_with_its_age() {
        let alarms = vec![alarm(
            "prices-production-enrichment-errors",
            "ALARM",
            28 * 86_400 + 4 * 3_600,
        )];
        let msg = digest_description("production", &alarms, NOW).expect("one stuck alarm");
        assert!(msg.contains("1 alarm(s) in production"), "{msg}");
        assert!(msg.contains("prices-production-enrichment-errors"), "{msg}");
        assert!(msg.contains("28d 4h"), "{msg}");
        assert!(msg.contains("task 0214"), "{msg}");
    }

    #[test]
    fn insufficient_data_counts_as_off_ok() {
        // A publisher that died leaves its alarm here, not in ALARM — and that is
        // exactly the state nothing else re-surfaces.
        let alarms = vec![alarm(
            "prices-production-current-prices-freshness",
            "INSUFFICIENT_DATA",
            3 * 3_600,
        )];
        let msg = digest_description("production", &alarms, NOW).expect("one stuck alarm");
        assert!(msg.contains("INSUFFICIENT_DATA"), "{msg}");
        assert!(msg.contains("3h 0m"), "{msg}");
    }

    #[test]
    fn the_payload_is_the_schema_chatbot_forwards() {
        // A payload Chatbot does not recognise is dropped before Slack, so the
        // envelope is load-bearing: these four fields are what make it render.
        let alarms = vec![alarm(
            "prices-production-enrichment-errors",
            "ALARM",
            86_400,
        )];
        let payload = digest_payload("production", &alarms, NOW).expect("one stuck alarm");
        let v: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        assert_eq!(v["version"], "1.0");
        assert_eq!(v["source"], "custom");
        assert_eq!(v["content"]["textType"], "client-markdown");
        assert_eq!(v["content"]["title"], "production: alarms stuck off OK");
        assert!(
            v["content"]["description"]
                .as_str()
                .expect("description is a string")
                .contains("prices-production-enrichment-errors")
        );
    }

    #[test]
    fn a_burning_account_is_truncated_rather_than_rejected() {
        let alarms: Vec<AlarmState> = (0..MAX_LISTED + 5)
            .map(|i| alarm(&format!("prices-production-a{i}"), "ALARM", 7 * 86_400))
            .collect();
        let msg = digest_description("production", &alarms, NOW).expect("all stuck");
        assert!(
            msg.contains(&format!("{} alarm(s) in production", MAX_LISTED + 5)),
            "{msg}"
        );
        assert!(msg.contains("... and 5 more"), "{msg}");
    }

    #[test]
    fn the_longest_ignored_alarm_comes_first() {
        let alarms = vec![
            alarm("prices-production-young", "ALARM", 2 * 3_600),
            alarm("prices-production-old", "ALARM", 9 * 86_400),
            alarm("prices-production-fine", "OK", 9 * 86_400),
        ];
        let names: Vec<&str> = stuck(&alarms, NOW)
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["prices-production-old", "prices-production-young"]
        );
    }

    #[test]
    fn a_state_transition_dates_the_latch_not_a_state_update() {
        // The 28-day latch, with an EvaluationState flip 20 minutes ago that
        // refreshed StateUpdatedTimestamp without changing StateValue. Reading
        // the refreshed one drops the alarm from the digest entirely — this
        // task's own bug, one field over.
        let transitioned = NOW - (28 * 86_400 + 4 * 3_600);
        let updated = NOW - 1_200;
        assert_eq!(
            since_from(Some(transitioned), Some(updated)),
            Some(transitioned)
        );
        // Fall back only when CloudWatch omits the transition timestamp.
        assert_eq!(since_from(None, Some(updated)), Some(updated));
        assert_eq!(since_from(None, None), None);

        let alarms = vec![AlarmState {
            name: "prices-production-enrichment-errors".to_string(),
            state: "ALARM".to_string(),
            since_unix: since_from(Some(transitioned), Some(updated)).expect("dated"),
        }];
        let msg = digest_description("production", &alarms, NOW).expect("still stuck");
        assert!(msg.contains("28d 4h"), "{msg}");
    }

    #[test]
    fn ages_read_the_way_an_operator_asks_the_question() {
        assert_eq!(humanise_age(0), "0m");
        assert_eq!(humanise_age(59), "0m");
        assert_eq!(humanise_age(3_600), "1h 0m");
        assert_eq!(humanise_age(5_400), "1h 30m");
        assert_eq!(humanise_age(86_400), "1d 0h");
        assert_eq!(humanise_age(28 * 86_400 + 4 * 3_600), "28d 4h");
        // A clock skew that puts the state in the future must not underflow.
        assert_eq!(humanise_age(-10), "0m");
    }
}
