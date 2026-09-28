//! **A failed portal load writes one alarm line, and its cooldown writes none**
//! — one test, one test binary (task 0311, review WR-02 / WR-04).
//!
//! # Why a binary of its own
//!
//! The same reason as `portal_keys_logs.rs`: `tracing` caches each callsite's
//! `Interest` globally, set by whichever thread reaches it first, while
//! `tracing::subscriber::set_default` is thread-local. Beside parallel tests
//! that also fail a load, the `portal sources failed to load` callsite can be
//! cached as `never` before this test's subscriber exists, and the capture
//! comes back empty — measured: it did, in `portal_lazy_load.rs`, about one
//! run in seven. Alone in a process there is no other thread to lose to.
//!
//! # What it proves
//!
//! The alarm `prices-${env}-api-handler-portal-load-failed` counts log lines
//! by their message prefix, so the prefix has to mean "a load failed":
//!
//! - a failed load logs it exactly once, and login adds no ERROR of its own
//!   (the old "deployment that cannot complete one" line, review WR-04);
//! - an ask inside `LOAD_FAILURE_COOLDOWN` starts no load and logs a WARN
//!   WITHOUT the prefix, so throttling cannot multiply alarm datapoints by
//!   page views (review WR-02).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use prices_api::config::PortalLoadError;
use prices_api::portal::auth::secret::SecretError;
use prices_api::portal::sources::{LOAD_FAILURE_COOLDOWN, LoadFuture, PortalSources};
use prices_api::{AppConfig, AppState, app_with_portal};
use tower::ServiceExt;

/// The alarm's prefix, as the metric filter and the guard test hold it.
const ALARM_PREFIX: &str = "portal sources failed to load";

/// A `MakeWriter` that keeps every byte the subscriber emits.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl CapturedLogs {
    /// Takes the lines written so far, leaving the buffer empty.
    fn take(&self) -> Vec<String> {
        let bytes = std::mem::take(&mut *self.0.lock().unwrap());
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// Sources whose every load fails, counting the loads.
fn failing() -> (PortalSources, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let sources = PortalSources::lazy(Arc::new(move || -> LoadFuture {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(PortalLoadError::Oauth(SecretError::NoSource)) })
    }));
    (sources, calls)
}

fn config() -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: true,
        portal_oauth: None,
        portal_endpoints: Default::default(),
        portal_keys: None,
        portal_eligibility: None,
        portal_rate_limit: None,
        portal_web_origin: None,
    }
}

async fn get_status(router: &axum::Router, uri: &str) -> StatusCode {
    router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test(start_paused = true)]
async fn a_failed_load_alarms_once_and_its_cooldown_never() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .finish();
    // A guard rather than `with_default`'s closure: `#[tokio::test]` drives
    // this future on this thread, so the subscriber covers every `await`.
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::error!("probe: the captured writer sees ERROR");
    assert_eq!(logs.take().len(), 1, "the capture is not live");

    // Login, both arms: one ERROR each, and it is the alarm's.
    for uri in ["/api/auth/login", "/api/auth/login?action=issue"] {
        let (sources, _) = failing();
        let router = app_with_portal(&config(), AppState::without_ch(), sources);
        assert_eq!(get_status(&router, uri).await, StatusCode::SEE_OTHER);
        let errors: Vec<_> = logs
            .take()
            .into_iter()
            .filter(|line| line.contains("ERROR"))
            .collect();
        assert_eq!(errors.len(), 1, "{uri}: {errors:#?}");
        assert!(errors[0].contains(ALARM_PREFIX), "{uri}: {errors:#?}");
    }

    // A burst of page views on one environment while loads fail.
    let (sources, calls) = failing();
    let router = app_with_portal(&config(), AppState::without_ch(), sources);
    assert_eq!(get_status(&router, "/api/config").await, StatusCode::OK);
    let first = logs.take();
    assert_eq!(
        first.iter().filter(|l| l.contains(ALARM_PREFIX)).count(),
        1,
        "{first:#?}"
    );

    for _ in 0..5 {
        tokio::time::advance(LOAD_FAILURE_COOLDOWN / 10).await;
        assert_eq!(get_status(&router, "/api/config").await, StatusCode::OK);
        assert_eq!(
            get_status(&router, "/api/usage").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    let cooling = logs.take();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a load ran inside the cooldown"
    );
    assert!(
        cooling
            .iter()
            .all(|l| !l.contains(ALARM_PREFIX) && !l.contains("ERROR")),
        "the cooldown alarmed: {cooling:#?}"
    );
    assert_eq!(
        cooling
            .iter()
            .filter(|l| l.contains("WARN") && l.contains("without a load"))
            .count(),
        10,
        "{cooling:#?}"
    );

    // After it, a load again — and its failure alarms again, once.
    tokio::time::advance(LOAD_FAILURE_COOLDOWN).await;
    assert_eq!(get_status(&router, "/api/config").await, StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let after = logs.take();
    assert_eq!(
        after.iter().filter(|l| l.contains(ALARM_PREFIX)).count(),
        1,
        "{after:#?}"
    );
}
