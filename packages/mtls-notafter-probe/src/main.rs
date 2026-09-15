//! mTLS NotAfter probe Lambda entrypoint (task 0056).
//!
//! EventBridge `rate(1 day)` → this binary. Each run reads the mTLS client-cert
//! bundles named in `MTLS_PROBE_SECRETS` from Secrets Manager (via the
//! Parameters and Secrets Extension, reusing the 0052 fetch), parses each cert's
//! X.509 NotAfter, and publishes days-to-expiry to CloudWatch for the
//! cert-expiry alarm.
//!
//! The same run also carries the daily stuck-alarm digest (task 0214) — a
//! second, independent job that happens to want exactly this schedule. See
//! `alarm_digest`.
//!
//!     cargo lambda build -p mtls-notafter-probe --release --arm64 --features lambda
//!
//! Requires the `lambda` feature (the default build/test exercises the X.509
//! parsing + day-math in `lib.rs` without the AWS runtime / mTLS-fetch stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use mtls_notafter_probe::{
        CertProbe, RoleDays, alarm_digest, days_to_not_after, parse_probe_targets, publish,
    };
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    prices_clickhouse::observability::init_tracing();

    // The set of certs to probe is CDK-provided: `role=secret-name` pairs. A
    // missing/empty spec is a deploy misconfiguration — fail Init rather than
    // silently probing nothing (which would leave expiry undetected).
    let spec = prices_clickhouse::env::env_or("MTLS_PROBE_SECRETS", "");
    let targets: Vec<CertProbe> = parse_probe_targets(&spec);
    if targets.is_empty() {
        return Err(lambda_runtime::Error::from(
            "MTLS_PROBE_SECRETS is empty or malformed — expected `role=secret-name[,role=secret-name]`",
        ));
    }
    let targets = Arc::new(targets);

    // Where the daily stuck-alarm digest goes (task 0214). Empty means the CDK
    // wiring is missing: fail Init, exactly as for MTLS_PROBE_SECRETS, rather
    // than run for months publishing a digest into nothing — which is the same
    // class of silence this digest exists to end.
    let topic_arn = prices_clickhouse::env::env_or("OPS_ALARMS_TOPIC_ARN", "");
    if topic_arn.is_empty() {
        return Err(lambda_runtime::Error::from(
            "OPS_ALARMS_TOPIC_ARN is empty — the stuck-alarm digest has nowhere to publish",
        ));
    }
    let topic_arn = Arc::new(topic_arn);

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let sns = Arc::new(aws_sdk_sns::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(
        environment = %environment,
        roles = targets.len(),
        "mtls-notafter-probe cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let targets = targets.clone();
        let cw = cw.clone();
        let sns = sns.clone();
        let topic_arn = topic_arn.clone();
        let environment = environment.clone();
        async move {
            let now_unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let mut samples: Vec<RoleDays> = Vec::with_capacity(targets.len());
            let mut failures: Vec<String> = Vec::new();
            for target in targets.iter() {
                // A failed cert contributes no metric, so it is EXCLUDED from
                // MinDaysToNotAfter — a healthy sibling would otherwise mask it
                // and its expiry would go unmonitored (the whole §11.4 risk).
                // Collect each failure and surface it below: any per-cert
                // failure (not only a total wipeout) fails the invocation, which
                // trips the probe's ops-wired error alarm. Healthy certs are
                // still published first so their days-to-expiry stays fresh.
                let bundle = match prices_clickhouse::mtls::fetch_bundle_from_extension(
                    &target.secret_name,
                )
                .await
                {
                    Ok(b) => b,
                    Err(err) => {
                        tracing::error!(role = %target.role, error = %err, "bundle fetch failed");
                        failures.push(format!("{} (fetch: {err})", target.role));
                        continue;
                    }
                };
                match days_to_not_after(&bundle.cert_pem, now_unix) {
                    Ok(days) => {
                        tracing::info!(role = %target.role, days, "cert days-to-NotAfter");
                        samples.push(RoleDays {
                            role: target.role.clone(),
                            days,
                        });
                    }
                    Err(err) => {
                        tracing::error!(role = %target.role, error = %err, "cert parse failed");
                        failures.push(format!("{} (parse: {err})", target.role));
                    }
                }
            }

            // Refresh the healthy certs' metrics (and MinDaysToNotAfter over
            // them) before surfacing any failure — a partial outage must not
            // stale-out the certs that DID read cleanly. `publish` no-ops on an
            // empty slice, so a total failure just skips straight to the error.
            if !samples.is_empty() {
                publish(&cw, &environment, &samples).await?;
            }

            // Second job on the same schedule (task 0214): re-read every
            // prices-{env}- alarm and name the ones stuck off OK. Deliberately
            // BEFORE the cert failure check — a cert this probe cannot read must
            // not also silence the digest, which is the one thing that would
            // surface such a latch. It collects its own failure the same way.
            let (stuck, digest_failure) =
                match alarm_digest::run(&cw, &sns, &topic_arn, &environment, now_unix).await {
                    Ok(n) => {
                        tracing::info!(stuck_alarms = n, "stuck-alarm digest complete");
                        (Some(n), None)
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "stuck-alarm digest failed");
                        (None, Some(err))
                    }
                };

            // Any cert we could not read/parse is unmonitored until fixed, and
            // its silence is invisible on the days-to-expiry alarm. Fail the run
            // so the error alarm pages instead of a healthy sibling hiding it.
            // The two jobs are reported separately: a run that failed only the
            // digest must not read as a cert outage.
            let mut problems: Vec<String> = Vec::new();
            if !failures.is_empty() {
                problems.push(format!(
                    "{}/{} cert(s) unreadable — days-to-expiry unmonitored for: [{}]",
                    failures.len(),
                    targets.len(),
                    failures.join(", "),
                ));
            }
            if let Some(err) = digest_failure {
                problems.push(format!("stuck-alarm digest did not run: {err}"));
            }
            if !problems.is_empty() {
                return Err(lambda_runtime::Error::from(format!(
                    "mtls-notafter-probe: {}",
                    problems.join("; "),
                )));
            }

            let published: Vec<_> = samples
                .iter()
                .map(|s| serde_json::json!({ "role": s.role, "days": s.days }))
                .collect();
            tracing::info!(
                roles = samples.len(),
                stuck_alarms = stuck.unwrap_or(0),
                "mtls-notafter-probe run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "published": published,
                "stuck_alarms": stuck,
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "mtls-notafter-probe: build with `--features lambda` (or `cargo lambda build -p \
         mtls-notafter-probe --release --arm64 --features lambda`) for the AWS Lambda entrypoint."
    );
}
