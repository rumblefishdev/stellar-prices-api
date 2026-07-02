//! mTLS certificate NotAfter probe (task 0056).
//!
//! §7 names the mTLS-NotAfter alarm a security primitive; §11.4's "mTLS cert
//! expiry not detected" risk row pegs its mitigation here. This scheduled
//! Lambda reads each per-env mTLS client-cert bundle from Secrets Manager,
//! parses the X.509 cert, and publishes the days remaining until its `NotAfter`
//! boundary as a CloudWatch metric. The alarm fires when the minimum across the
//! probed certs drops below the operator-set threshold (30 days by default).
//!
//! Split for testability: the X.509 parsing + day-math ([`not_after_unix`],
//! [`days_to_not_after`]) and the target-spec parsing ([`parse_probe_targets`])
//! are feature-free and unit-tested against an embedded cert; the Secrets
//! Manager fetch and CloudWatch publish are gated behind the `lambda` /
//! `aws-mtls` features.

/// CloudWatch namespace for the cert-expiry metrics. Must match the
/// `cloudwatch:namespace` condition on the Lambda role's `PutMetricData` grant
/// and the alarm wiring in `infra/`.
pub const METRIC_NAMESPACE: &str = "Prices/Mtls";

/// Per-role metric (dimensioned by `Role`): days remaining on that role's
/// client cert. Forensic — lets an operator see which identity is closest to
/// expiry.
pub const PER_ROLE_METRIC: &str = "DaysToNotAfter";

/// Aggregate metric (Environment-only dimension): the minimum days-to-NotAfter
/// across all probed certs. The cert-expiry alarm watches this single value so
/// one alarm covers "either cert is close to expiry".
pub const MIN_METRIC: &str = "MinDaysToNotAfter";

#[derive(Debug, thiserror::Error)]
pub enum NotAfterError {
    #[error("PEM parse failed: {0}")]
    Pem(String),
    #[error("X.509 parse failed: {0}")]
    X509(String),
}

/// One mTLS identity to probe: its role label (used as the `Role` dimension)
/// and the Secrets Manager secret name holding its `{cert,key,ca}` bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertProbe {
    pub role: String,
    pub secret_name: String,
}

/// Days remaining until a probed cert's NotAfter boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDays {
    pub role: String,
    pub days: f64,
}

/// Parse the `MTLS_PROBE_SECRETS` spec — a comma-separated list of
/// `role=secret-name` pairs, e.g.
/// `ingestion=prices/production/...-ingestion,api=prices/production/...-api`.
/// Blank / malformed entries are skipped so a trailing comma or stray whitespace
/// is tolerated.
pub fn parse_probe_targets(spec: &str) -> Vec<CertProbe> {
    spec.split(',')
        .filter_map(|pair| {
            let (role, name) = pair.trim().split_once('=')?;
            let (role, name) = (role.trim(), name.trim());
            if role.is_empty() || name.is_empty() {
                return None;
            }
            Some(CertProbe {
                role: role.to_string(),
                secret_name: name.to_string(),
            })
        })
        .collect()
}

/// Extract the `NotAfter` boundary (unix seconds) of the first certificate in a
/// PEM chain.
pub fn not_after_unix(cert_pem: &str) -> Result<i64, NotAfterError> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| NotAfterError::Pem(e.to_string()))?;
    let cert = pem
        .parse_x509()
        .map_err(|e| NotAfterError::X509(e.to_string()))?;
    Ok(cert.validity().not_after.timestamp())
}

/// Days remaining until the cert's NotAfter, relative to `now_unix`. Negative
/// once the cert has expired — the alarm's `< threshold` comparison catches
/// both "expiring soon" and "already expired".
pub fn days_to_not_after(cert_pem: &str, now_unix: i64) -> Result<f64, NotAfterError> {
    Ok((not_after_unix(cert_pem)? - now_unix) as f64 / 86_400.0)
}

/// Minimum days-to-NotAfter across the probed certs (`None` if none succeeded).
pub fn min_days(samples: &[RoleDays]) -> Option<f64> {
    samples
        .iter()
        .map(|s| s.days)
        .fold(None, |acc, d| Some(acc.map_or(d, |a: f64| a.min(d))))
}

/// Publish per-role `DaysToNotAfter` (dimensioned `Environment` + `Role`) and
/// the aggregate `MinDaysToNotAfter` (`Environment` only) to CloudWatch under
/// [`METRIC_NAMESPACE`], in one `PutMetricData` call.
#[cfg(feature = "lambda")]
pub async fn publish(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    samples: &[RoleDays],
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

    if samples.is_empty() {
        return Ok(());
    }

    let env_dim = Dimension::builder()
        .name("Environment")
        .value(environment)
        .build();

    let mut data: Vec<MetricDatum> = samples
        .iter()
        .map(|s| {
            MetricDatum::builder()
                .metric_name(PER_ROLE_METRIC)
                .value(s.days)
                .unit(StandardUnit::None)
                .dimensions(env_dim.clone())
                .dimensions(Dimension::builder().name("Role").value(&s.role).build())
                .build()
        })
        .collect();

    if let Some(min) = min_days(samples) {
        data.push(
            MetricDatum::builder()
                .metric_name(MIN_METRIC)
                .value(min)
                .unit(StandardUnit::None)
                .dimensions(env_dim.clone())
                .build(),
        );
    }

    client
        .put_metric_data()
        .namespace(METRIC_NAMESPACE)
        .set_metric_data(Some(data))
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Self-signed P-256 cert, CN=prices-ingestion-test, notAfter 2036-06-29.
    // Only the NotAfter field matters here; the day-math is asserted relative to
    // the parsed boundary so the test never hardcodes an absolute clock.
    const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBlTCCATugAwIBAgIUWg03A0EhmTKn77u+XiT4MC78qjIwCgYIKoZIzj0EAwIw\n\
IDEeMBwGA1UEAwwVcHJpY2VzLWluZ2VzdGlvbi10ZXN0MB4XDTI2MDcwMjEzNDMw\n\
NVoXDTM2MDYyOTEzNDMwNVowIDEeMBwGA1UEAwwVcHJpY2VzLWluZ2VzdGlvbi10\n\
ZXN0MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEvSQ/cafHQVfzFuoB5gojHyMH\n\
Yb9CbqH7OcQlWeHSjiga2LMFvUcfstPqyfsIdIcv63X7GqwZ56FbfH3sQz7mqaNT\n\
MFEwHQYDVR0OBBYEFINF25NZebHWGrpDtXYwozeCzB5PMB8GA1UdIwQYMBaAFINF\n\
25NZebHWGrpDtXYwozeCzB5PMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwID\n\
SAAwRQIhAIHxsUUMkQGQd+19fM0wyGHUpz9+0gsWKOvn0TgA0p9VAiAeLda8x1Dr\n\
kwg73Ku/X8+SLCFGmPy4BZ32I+3lgsifzQ==\n\
-----END CERTIFICATE-----\n";

    #[test]
    fn parses_not_after_from_pem() {
        let na = not_after_unix(TEST_CERT).expect("parse");
        // 2036-06-29T13:43:05Z (== 2_098_359_785) — sanity bound: past 2033,
        // before 2040.
        assert!(na > 2_000_000_000 && na < 2_200_000_000, "got {na}");
    }

    #[test]
    fn days_math_is_relative_to_not_after() {
        let na = not_after_unix(TEST_CERT).unwrap();
        let days = days_to_not_after(TEST_CERT, na - 10 * 86_400).unwrap();
        assert!((days - 10.0).abs() < 0.001, "expected ~10 days, got {days}");
    }

    #[test]
    fn expired_cert_reports_negative_days() {
        let na = not_after_unix(TEST_CERT).unwrap();
        let days = days_to_not_after(TEST_CERT, na + 5 * 86_400).unwrap();
        assert!(
            days < 0.0,
            "an already-expired cert must read negative: {days}"
        );
    }

    #[test]
    fn garbage_pem_errors() {
        assert!(not_after_unix("not a pem").is_err());
        assert!(
            days_to_not_after(
                "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
                0
            )
            .is_err()
        );
    }

    #[test]
    fn parses_multi_role_spec() {
        let t = parse_probe_targets("ingestion=prices/prod/a , api=prices/prod/b,");
        assert_eq!(
            t,
            vec![
                CertProbe {
                    role: "ingestion".into(),
                    secret_name: "prices/prod/a".into()
                },
                CertProbe {
                    role: "api".into(),
                    secret_name: "prices/prod/b".into()
                },
            ]
        );
    }

    #[test]
    fn skips_malformed_entries() {
        // No '=', empty role, empty name — all dropped; the valid one survives.
        let t = parse_probe_targets("garbage,=noname,role=,ingestion=s");
        assert_eq!(
            t,
            vec![CertProbe {
                role: "ingestion".into(),
                secret_name: "s".into()
            }]
        );
    }

    #[test]
    fn min_days_picks_smallest() {
        let s = vec![
            RoleDays {
                role: "ingestion".into(),
                days: 45.0,
            },
            RoleDays {
                role: "api".into(),
                days: 12.0,
            },
        ];
        assert_eq!(min_days(&s), Some(12.0));
        assert_eq!(min_days(&[]), None);
    }
}
