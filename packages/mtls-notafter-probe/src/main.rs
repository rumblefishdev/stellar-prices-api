//! mTLS NotAfter probe Lambda entrypoint (task 0056).
//!
//! EventBridge `rate(1 day)` → this binary. Each run reads the mTLS client-cert
//! bundles named in `MTLS_PROBE_SECRETS` from Secrets Manager (via the
//! Parameters and Secrets Extension, reusing the 0052 fetch), parses each cert's
//! X.509 NotAfter, and publishes days-to-expiry to CloudWatch for the
//! cert-expiry alarm.
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
        CertProbe, RoleDays, days_to_not_after, parse_probe_targets, publish,
    };
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

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

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(
        environment = %environment,
        roles = targets.len(),
        "mtls-notafter-probe cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let targets = targets.clone();
        let cw = cw.clone();
        let environment = environment.clone();
        async move {
            let now_unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let mut samples: Vec<RoleDays> = Vec::with_capacity(targets.len());
            for target in targets.iter() {
                // Per-cert failures are logged and skipped so one unreadable
                // secret does not blind the probe to the others — but a run
                // where EVERY cert fails errors below, so total failure trips
                // the worker's error alarm (expiry must never go silently
                // unmonitored).
                let bundle = match prices_clickhouse::mtls::fetch_bundle_from_extension(
                    &target.secret_name,
                )
                .await
                {
                    Ok(b) => b,
                    Err(err) => {
                        tracing::error!(role = %target.role, error = %err, "bundle fetch failed");
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
                    }
                }
            }

            if samples.is_empty() {
                return Err(lambda_runtime::Error::from(
                    "all mTLS cert probes failed — no days-to-NotAfter could be computed",
                ));
            }

            publish(&cw, &environment, &samples).await?;

            let published: Vec<_> = samples
                .iter()
                .map(|s| serde_json::json!({ "role": s.role, "days": s.days }))
                .collect();
            tracing::info!(roles = samples.len(), "mtls-notafter-probe run complete");
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "published": published,
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
