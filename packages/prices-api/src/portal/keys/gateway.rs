//! The API Gateway **control plane**, wrapped down to six calls (task 0187,
//! plus task 0188's `GetUsage`).
//!
//! Not the data plane. These are `GetApiKeys`, `CreateApiKey`,
//! `CreateUsagePlanKey`, `GetApiKey`, `DeleteApiKey` and `GetUsage` — the API
//! the console drives — and they are the reason this slice needs no database:
//! **API Gateway is the source of truth for whether a key exists** (task 0158's
//! own argument, restated in 0187's context), and for how much it has been
//! used — quota is scoped to `(usagePlanId, apiKeyId)` and counted by AWS, so
//! there is no accounting of our own to store. A registry would buy a hot-path
//! read and a history; neither is required to answer "where is my key" or "how
//! much of my quota is left".
//!
//! # What this module is for
//!
//! Turning the SDK's shapes into the three-field [`KeyRecord`] the pure
//! reconciler in [`super::naming`] reasons about, and keeping two hazards in one
//! place:
//!
//! - **Pagination.** `GetApiKeys` paginates *even when a `nameQuery` is given* —
//!   measured 2026-08-12, archived in `0180/notes/R-apigw-namequery-quota-and-disable.md`.
//!   Ranking off page one can pick a winner from a partial list and then delete
//!   a key it never saw. [`Gateway::list_named`] therefore pages to exhaustion
//!   and **errors rather than truncating** if the pages do not run out.
//! - **The value.** A key value is a bearer credential. It is carried only by
//!   [`KeyValue`], which cannot be printed, and it is never requested where it
//!   is not needed — the listing sets `include_values(false)`, so a reconciler
//!   sweep never pulls one key value into memory, let alone every user's.
//!
//! # Timeouts, and what they are NOT
//!
//! Each call is bounded rather than left to the SDK's defaults, because a
//! control plane slow enough to matter here is one that is down and the caller
//! is better told so.
//!
//! **These constants do not add up to a request-level bound, and an earlier
//! version of this paragraph claimed they did.** The claim was "the Lambda
//! timeout is 15s and one issue makes five calls, so each one is bounded" — but
//! [`OPERATION_TIMEOUT`] is per operation, one attempt makes up to six of them
//! plus a delete per duplicate, [`Gateway::list_named`] alone can walk
//! [`MAX_PAGES`] pages each with its own budget, and the reconciler runs twice.
//! The worst case is minutes, not seconds.
//!
//! The bound lives one level up, as a wall-clock deadline across the whole
//! reconciliation (`super::RECONCILE_DEADLINE`). That is the only place it can
//! live: no per-call ceiling composes into a request-level one when the number
//! of calls is a function of how many pages and duplicates the account holds.
//!
//! The per-call values are deliberately NOT tightened to fit inside the deadline
//! by arithmetic. A shorter attempt timeout would start cutting off cold TLS
//! handshakes and turn a healthy first invocation into a retry; the deadline
//! already guarantees an answer, so these two only decide how many slow calls
//! fit inside it before that answer becomes a `503`.

use std::time::Duration;

use aws_sdk_apigateway::Client;
use aws_sdk_apigateway::config::timeout::TimeoutConfig;

use super::naming::KeyRecord;

/// Page size for [`Gateway::list_named`]. 500 is the service maximum.
const PAGE_LIMIT: i32 = 500;

/// How many pages [`Gateway::list_named`] will walk before giving up.
///
/// At [`PAGE_LIMIT`] that is 25,000 keys sharing one name prefix, which cannot
/// happen for a `discord-<id>-key` query unless something is very wrong. The
/// bound exists so that a `position` token the service never clears becomes an
/// error instead of an infinite loop inside a Lambda invocation — and the error
/// is the honest one, because the alternative (stop early, rank what we have)
/// is exactly the silent truncation this module exists to prevent.
const MAX_PAGES: usize = 50;

/// Ceiling on one control-plane call, retries included.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Ceiling on a single attempt, so a stalled connection is retried rather than
/// consuming the whole operation budget.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);

/// Tag written on every key this service creates.
///
/// Its job is attribution, not authorization: `CreateApiKey` has no ARN to scope
/// an IAM grant to (see `compute-stack.ts`), so the tag is what makes "keys this
/// function created" answerable after the fact — in the console, in Cost
/// Explorer, and to task 0194's audit.
const TAG_MANAGED_BY: (&str, &str) = ("ManagedBy", "prices-portal");

/// The task that introduced the key, so a later slice can tell an 0187 key from
/// one issued after eligibility ([0189]) landed.
const TAG_ISSUED_BY: (&str, &str) = ("IssuedBy", "task-0187");

/// An API key value, which is a bearer credential.
///
/// A distinct type from `String` so that "never log a key value" is enforced by
/// the compiler rather than by review. There is no `Display`, no `Serialize`,
/// and the [`std::fmt::Debug`] below prints nothing — so a `?value` in a
/// `tracing` macro, a `dbg!`, a panic message carrying a struct that contains
/// one, and `#[derive(Debug)]` on any wrapper are all safe by construction.
///
/// [`Self::expose`] is the single way out, named so that every use of it is
/// visible in a grep and has to justify itself. It has exactly one caller: the
/// JSON body of the response that is the whole point of this feature.
pub struct KeyValue(String);

impl KeyValue {
    /// Hand the value to the one thing entitled to it: the response body.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for KeyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyValue(<redacted>)")
    }
}

/// What [`Gateway::attach_to_free_plan`] observed.
///
/// Two outcomes rather than `()` because the caller has to act on the second
/// one: a key that vanished between the listing and the attach is the same race
/// [`Gateway::value_of`] reports with `None`, and the answer to it is to run the
/// reconciliation again, not to tell the caller the control plane is broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attachment {
    /// The key is on the usage plan — this call put it there, or it already was.
    OnPlan,
    /// The key no longer exists, so there was nothing to attach.
    KeyGone,
}

/// One key's consumption over a queried period, as AWS reports it.
///
/// Derived from `GetUsage`'s daily `[used, remaining]` pairs rather than read
/// off a single field: the response carries no `limit` of its own, so the limit
/// is reconstructed as `used + remaining` — the same arithmetic task 0157's
/// close verified against the live plan (`[121, 99879]` against a 100 000
/// quota). Kept here instead of in the handler so the shape of the AWS response
/// stays a concern of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyUsage {
    /// Requests counted against the quota so far this period — the sum of the
    /// daily `used` values.
    pub used: u64,
    /// Requests left, as of the **latest day AWS has data for** — the last
    /// daily pair's `remaining`. Not "limit − used": the two can disagree while
    /// AWS's own reporting lags, and the last-reported value is the honest one.
    pub remaining: u64,
}

impl KeyUsage {
    /// The plan's quota, reconstructed. See the type docs.
    pub fn limit(&self) -> u64 {
        self.used.saturating_add(self.remaining)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("API Gateway `{operation}` failed: {message}")]
    Call {
        operation: &'static str,
        message: String,
    },
    /// The control plane is rate-limiting us — distinct from [`Self::Call`]
    /// because task 0188 answers it differently: these are control-plane calls
    /// sharing an account-wide budget with our own deploys, so a throttle wants
    /// a stale-but-honest answer (the dashboard already renders "last updated")
    /// rather than an error page. The SDK has already retried with backoff by
    /// the time this surfaces; this variant is what is left when backing off
    /// was not enough.
    #[error("API Gateway is rate-limiting `{operation}`; backing off")]
    Throttled { operation: &'static str },
    #[error(
        "API Gateway returned more than {MAX_PAGES} pages for one key name; refusing to rank \
         a partial list"
    )]
    TooManyPages,
    #[error(
        "API Gateway says usage plan `{plan_id}` does not exist; the key was created but could \
         not be attached to a plan, so it will not work against /v1/. Check the SSM parameter \
         named by PORTAL_FREE_PLAN_PARAM (or PORTAL_FREE_PLAN_ID on a local run) against the \
         plan ApiGatewayStack publishes"
    )]
    PlanNotFound { plan_id: String },
    #[error("API Gateway `{operation}` answered without the `{field}` field")]
    Incomplete {
        operation: &'static str,
        field: &'static str,
    },
}

/// The five calls, plus the usage plan they attach to.
#[derive(Clone)]
pub struct Gateway {
    client: Client,
    /// The `pricing-api-free` usage plan id, read from SSM at cold start —
    /// never hard-coded and never a cross-stack reference. See
    /// [`crate::AppConfig::load_portal_keys`].
    free_plan_id: String,
}

/// Prints the plan id and nothing else. The plan id is not a secret (it is in
/// an SSM parameter any operator can read) and it is the one field worth seeing
/// in a diagnostic.
impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("free_plan_id", &self.free_plan_id)
            .finish_non_exhaustive()
    }
}

impl Gateway {
    /// Build a client from the ambient AWS configuration — in the Lambda, the
    /// execution role's credentials from the environment.
    ///
    /// Only ever called when the portal is open, so a production cold start with
    /// `PORTAL_ENABLED=false` resolves no credentials and opens no connections
    /// for this. See [`crate::AppConfig::load_portal_keys`].
    pub async fn from_ambient_config(free_plan_id: String) -> Self {
        let shared =
            aws_config::load_defaults(aws_sdk_apigateway::config::BehaviorVersion::latest()).await;
        let config = aws_sdk_apigateway::config::Builder::from(&shared)
            .timeout_config(timeouts())
            .build();
        Self {
            client: Client::from_conf(config),
            free_plan_id,
        }
    }

    /// Build a client pointed at `endpoint_url`, with static credentials.
    ///
    /// The seam `tests/portal_keys.rs` drives a mock control plane through, and
    /// the one a local `serve` run could aim at a local emulator.
    ///
    /// **Compiled out of the Lambda**, exactly as `discord::Endpoints`' env
    /// overrides and `PORTAL_OAUTH_SECRET_FILE` are, and for the same reason:
    /// the deployed function must contain no code path that can be made to talk
    /// to a control plane of somebody else's choosing. Here the stakes are if
    /// anything higher than for sign-in — a redirected control plane would be
    /// handed the execution role's SigV4 signature on requests that create and
    /// delete production API keys.
    #[cfg(not(feature = "lambda"))]
    pub fn against(endpoint_url: &str, free_plan_id: String) -> Self {
        let config = aws_sdk_apigateway::config::Builder::new()
            .behavior_version(aws_sdk_apigateway::config::BehaviorVersion::latest())
            .region(aws_sdk_apigateway::config::Region::new("eu-central-1"))
            .credentials_provider(aws_sdk_apigateway::config::Credentials::new(
                "portal-keys-test",
                "portal-keys-test",
                None,
                None,
                "portal-keys-test",
            ))
            .endpoint_url(endpoint_url)
            .timeout_config(timeouts())
            .build();
        Self {
            client: Client::from_conf(config),
            free_plan_id,
        }
    }

    /// The usage plan new keys are attached to.
    pub fn free_plan_id(&self) -> &str {
        &self.free_plan_id
    }

    /// Every key whose name starts with `prefix`, across **all** pages.
    ///
    /// `nameQuery` is a prefix match and the caller must still filter to exact
    /// equality — [`super::naming::exact_matches`], and the argument for why in
    /// that module's docs. The parameter is named `prefix` rather than `name` so
    /// that the call site cannot read as if the service had done the narrowing.
    ///
    /// `include_values(false)`: a reconciler never needs a value, and a listing
    /// that carried them would put every matched key's credential in this
    /// process's memory for the sake of a field nothing reads.
    pub async fn list_named(&self, prefix: &str) -> Result<Vec<KeyRecord>, GatewayError> {
        let mut found = Vec::new();
        let mut position: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut request = self
                .client
                .get_api_keys()
                .name_query(prefix)
                .include_values(false)
                .limit(PAGE_LIMIT);
            if let Some(position) = position.as_ref() {
                request = request.position(position);
            }

            let page = match request.send().await {
                Ok(page) => page,
                Err(e) => {
                    // Throttling is separated from every other failure because a
                    // caller can answer it (serve the last good answer, back
                    // off) while a plain failure has nothing to fall back on.
                    let message = sdk_message(&e);
                    return Err(if e.into_service_error().is_too_many_requests_exception() {
                        GatewayError::Throttled {
                            operation: "GetApiKeys",
                        }
                    } else {
                        GatewayError::Call {
                            operation: "GetApiKeys",
                            message,
                        }
                    });
                }
            };

            for item in page.items() {
                // A key with no id cannot be fetched, attached or deleted, so it
                // cannot take part in reconciliation at all. Skipped rather than
                // failed: one malformed record should not deny every user their
                // key, and it cannot be the winner because it cannot be named.
                let (Some(id), Some(name)) = (item.id(), item.name()) else {
                    tracing::warn!("API Gateway listed a key with no id or no name; skipped");
                    continue;
                };
                found.push(KeyRecord {
                    id: id.to_string(),
                    name: name.to_string(),
                    created_at: item
                        .created_date()
                        .map(|d| d.secs())
                        .and_then(|secs| u64::try_from(secs).ok()),
                });
            }

            // The service signals "no more pages" by omitting `position`. An
            // empty string counts as omitted — it is not a cursor, and treating
            // it as one is how a loop like this becomes infinite.
            match page.position() {
                Some(next) if !next.is_empty() => position = Some(next.to_string()),
                _ => return Ok(found),
            }
        }

        Err(GatewayError::TooManyPages)
    }

    /// Create a key and return it together with its value.
    ///
    /// `CreateApiKey` answers with the value, so this is the one path that gets
    /// one without a second call. Tagged on creation rather than afterwards:
    /// a separate `TagResource` could fail and leave an untagged key, which is
    /// the one thing tagging exists to prevent.
    pub async fn create(&self, name: &str) -> Result<(KeyRecord, KeyValue), GatewayError> {
        let created = self
            .client
            .create_api_key()
            .name(name)
            .description("Self-service key issued by the Stellar Prices API portal (task 0187)")
            .enabled(true)
            .tags(TAG_MANAGED_BY.0, TAG_MANAGED_BY.1)
            .tags(TAG_ISSUED_BY.0, TAG_ISSUED_BY.1)
            .send()
            .await
            .map_err(|e| GatewayError::Call {
                operation: "CreateApiKey",
                message: sdk_message(&e),
            })?;

        let id = created.id().ok_or(GatewayError::Incomplete {
            operation: "CreateApiKey",
            field: "id",
        })?;
        let value = created.value().ok_or(GatewayError::Incomplete {
            operation: "CreateApiKey",
            field: "value",
        })?;

        Ok((
            KeyRecord {
                id: id.to_string(),
                // The name we asked for, not the one echoed back — they are the
                // same, and reading it from the response would make the record's
                // name depend on a field the caller then filters on.
                name: name.to_string(),
                created_at: created
                    .created_date()
                    .map(|d| d.secs())
                    .and_then(|secs| u64::try_from(secs).ok()),
            },
            KeyValue(value.to_string()),
        ))
    }

    /// Attach a key to the free usage plan, which is what makes it work against
    /// `/v1/`. A key that exists but is on no plan authenticates and is then
    /// refused by the plan check — the confusing half-state this call closes.
    ///
    /// **Idempotent**, and that is what lets the caller run it on every key it
    /// is about to hand out rather than only on keys it just created. API
    /// Gateway answers `409 ConflictException` when the key is already on the
    /// plan; that is the desired state, so it is a success. Without this the
    /// caller would need to *know* whether a key is attached, and there is no
    /// cheap way to know: `GetApiKey` does not report usage-plan membership, and
    /// asking `GetUsagePlanKeys` would be an extra call and an extra IAM grant
    /// to learn what this call can simply assert.
    ///
    /// A `404` is **ambiguous** and is resolved before it is acted on. API
    /// Gateway answers `NotFoundException` both when the key is gone and when
    /// the usage plan is, and the two want opposite handling: the first is the
    /// deletion race, answered by re-entering the flow (which is why this call
    /// returns [`Attachment::KeyGone`] rather than erroring — it runs before the
    /// read, so it is the first place that race surfaces, and a `502` here would
    /// make a hand-deleted key a dead end again); the second is a deployment
    /// that will never work, and reporting it as a transient race hides it
    /// forever. [`Self::exists`] separates them.
    pub async fn attach_to_free_plan(&self, key_id: &str) -> Result<Attachment, GatewayError> {
        match self
            .client
            .create_usage_plan_key()
            .usage_plan_id(&self.free_plan_id)
            .key_id(key_id)
            .key_type("API_KEY")
            .send()
            .await
        {
            Ok(_) => Ok(Attachment::OnPlan),
            Err(e) => {
                let message = sdk_message(&e);
                let service_error = e.into_service_error();
                if service_error.is_conflict_exception() {
                    Ok(Attachment::OnPlan)
                } else if service_error.is_not_found_exception() {
                    // `NotFoundException` here means EITHER "that key is gone"
                    // OR "that usage plan does not exist", and the error carries
                    // nothing that separates them. Reading it as the first
                    // unconditionally turns a permanent misconfiguration — a
                    // stale or mistyped plan id — into `Ok(None)`, which the
                    // caller retries, exhausts, and reports to the user as "your
                    // key is being changed by something else right now; try
                    // again". Forever, for everybody, while every request leaves
                    // an enabled key attached to nothing.
                    //
                    // So ask. One extra call, only on this path, on a grant we
                    // already hold, and it names the real cause in the error
                    // rather than in a support conversation three days later.
                    if self.exists(key_id).await? {
                        Err(GatewayError::PlanNotFound {
                            plan_id: self.free_plan_id.clone(),
                        })
                    } else {
                        Ok(Attachment::KeyGone)
                    }
                } else {
                    Err(GatewayError::Call {
                        operation: "CreateUsagePlanKey",
                        message,
                    })
                }
            }
        }
    }

    /// Whether a key still exists, without asking for its value.
    ///
    /// Deliberately not [`Self::value_of`]: this is a liveness question asked on
    /// an error path, and pulling a credential into memory to answer it would
    /// put a key value where nothing needs one. `include_value` is left unset,
    /// so the response carries no value to leak.
    pub async fn exists(&self, key_id: &str) -> Result<bool, GatewayError> {
        match self.client.get_api_key().api_key(key_id).send().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let message = sdk_message(&e);
                if e.into_service_error().is_not_found_exception() {
                    Ok(false)
                } else {
                    Err(GatewayError::Call {
                        operation: "GetApiKey",
                        message,
                    })
                }
            }
        }
    }

    /// The value of an existing key, or `None` if it is gone.
    ///
    /// `None` rather than an error for the not-found case, because "somebody
    /// deleted it in the console between the list and this call" is a state this
    /// slice has a defined answer for (re-enter the flow) rather than a failure
    /// to report.
    pub async fn value_of(&self, key_id: &str) -> Result<Option<KeyValue>, GatewayError> {
        match self
            .client
            .get_api_key()
            .api_key(key_id)
            .include_value(true)
            .send()
            .await
        {
            Ok(key) => {
                key.value()
                    .map(|v| Some(KeyValue(v.to_string())))
                    .ok_or(GatewayError::Incomplete {
                        operation: "GetApiKey",
                        field: "value",
                    })
            }
            Err(e) => {
                let message = sdk_message(&e);
                if e.into_service_error().is_not_found_exception() {
                    Ok(None)
                } else {
                    Err(GatewayError::Call {
                        operation: "GetApiKey",
                        message,
                    })
                }
            }
        }
    }

    /// One key's usage against the free plan's quota between `start_date` and
    /// `end_date` (inclusive, `YYYY-MM-DD`), or `None` if AWS has recorded
    /// nothing for it in that window (task 0188).
    ///
    /// `None` is a real and **common** state, not an edge case: `GetUsage` is
    /// not a read-after-write surface (measured 2026-08-12, archived
    /// `0180/notes/R-apigw-namequery-quota-and-disable.md`), so a key issued
    /// minutes ago — the first thing every new user looks at — may have no row
    /// yet. The caller renders that as "nothing recorded yet" rather than
    /// inventing zeros for `remaining` and a limit it does not know.
    ///
    /// **Paginated like the listing is**, and for the same reason: the response
    /// carries a `position` token, and summing off page one would report a
    /// smaller `used` than AWS is counting. With the `keyId` filter the items
    /// map holds one key, so extra pages extend that key's daily series;
    /// `MAX_PAGES` turns a cursor the service never clears into an error
    /// instead of an infinite loop.
    ///
    /// Daily pairs are `[used, remaining]`. `used` sums across days;
    /// `remaining` is the **last** day's value, because it is a running balance
    /// rather than a per-day allowance (task 0157's close observed exactly
    /// this: `[121, 99879]` one day, `[0, 99879]` the next).
    pub async fn usage_of(
        &self,
        key_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<Option<KeyUsage>, GatewayError> {
        let mut days: Vec<(i64, i64)> = Vec::new();
        let mut position: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut request = self
                .client
                .get_usage()
                .usage_plan_id(&self.free_plan_id)
                .key_id(key_id)
                .start_date(start_date)
                .end_date(end_date)
                .limit(PAGE_LIMIT);
            if let Some(position) = position.as_ref() {
                request = request.position(position);
            }

            let page = match request.send().await {
                Ok(page) => page,
                Err(e) => {
                    let message = sdk_message(&e);
                    return Err(if e.into_service_error().is_too_many_requests_exception() {
                        GatewayError::Throttled {
                            operation: "GetUsage",
                        }
                    } else {
                        GatewayError::Call {
                            operation: "GetUsage",
                            message,
                        }
                    });
                }
            };

            // The map is keyed by key id. The `keyId` filter should leave only
            // ours, but that is the service's narrowing, not this module's —
            // read the one entry we asked about and ignore anything else, the
            // same stance `exact_matches` takes on `nameQuery`.
            if let Some(items) = page.items()
                && let Some(values) = items.get(key_id)
            {
                for pair in values {
                    let used = pair.first().copied().unwrap_or(0);
                    let remaining = pair.get(1).copied().unwrap_or(0);
                    days.push((used, remaining));
                }
            }

            match page.position() {
                Some(next) if !next.is_empty() => position = Some(next.to_string()),
                _ => {
                    let Some(&(_, last_remaining)) = days.last() else {
                        return Ok(None);
                    };
                    // Negative values have no defined meaning here and are
                    // clamped to zero — which keeps the arithmetic total
                    // rather than making it right: if AWS ever reported, say,
                    // `[100500, -500]`, the reconstructed limit degrades to
                    // `used + 0` and the dashboard would read "used 100500 of
                    // 100500" against a 100 000 plan. Accepted: nothing in the
                    // response carries the true limit to fall back on, no such
                    // row has been observed, and the page's lag wording
                    // already frames every figure as AWS's report rather than
                    // ground truth.
                    let used = days
                        .iter()
                        .map(|&(used, _)| u64::try_from(used).unwrap_or(0))
                        .fold(0u64, u64::saturating_add);
                    let remaining = u64::try_from(last_remaining).unwrap_or(0);
                    return Ok(Some(KeyUsage { used, remaining }));
                }
            }
        }

        Err(GatewayError::TooManyPages)
    }

    /// Delete a key. A key that is already gone is a success — the caller wanted
    /// it not to exist, and a concurrent invocation having removed it first is
    /// the reconciler working, not failing.
    pub async fn delete(&self, key_id: &str) -> Result<(), GatewayError> {
        match self.client.delete_api_key().api_key(key_id).send().await {
            Ok(_) => Ok(()),
            Err(e) => {
                let message = sdk_message(&e);
                if e.into_service_error().is_not_found_exception() {
                    Ok(())
                } else {
                    Err(GatewayError::Call {
                        operation: "DeleteApiKey",
                        message,
                    })
                }
            }
        }
    }
}

fn timeouts() -> TimeoutConfig {
    TimeoutConfig::builder()
        .operation_timeout(OPERATION_TIMEOUT)
        .operation_attempt_timeout(ATTEMPT_TIMEOUT)
        .build()
}

/// A one-line description of an SDK failure, for a log line and an error.
///
/// `DisplayErrorContext` rather than `to_string()`, which on an `SdkError`
/// prints only the outermost layer ("service error") and drops the cause that
/// says which one. Nothing here can carry a key value: the only operation whose
/// response contains one is `GetApiKey`, and an error response has no body to
/// echo it from.
fn sdk_message<E, R>(error: &aws_sdk_apigateway::error::SdkError<E, R>) -> String
where
    aws_sdk_apigateway::error::SdkError<E, R>: std::error::Error,
{
    format!("{}", aws_sdk_apigateway::error::DisplayErrorContext(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The type-level half of "never log a key value": a `Debug` that prints
    /// nothing, so no format string anywhere can leak one.
    #[test]
    fn a_key_value_prints_as_redacted() {
        let value = KeyValue("A-REAL-LOOKING-KEY-VALUE".into());
        let printed = format!("{value:?}");
        assert_eq!(printed, "KeyValue(<redacted>)");
        assert!(!printed.contains("REAL"));
    }

    /// And the value is still there for the one caller entitled to it.
    #[test]
    fn expose_is_the_only_way_out() {
        assert_eq!(KeyValue("v".into()).expose(), "v");
    }

    /// A `Gateway` can end up in a diagnostic by way of `AppConfig`; nothing in
    /// it may be a credential. The plan id is not one.
    #[test]
    fn a_gateway_prints_only_its_plan_id() {
        let gateway = Gateway::against("http://127.0.0.1:1", "plan-abc".into());
        let printed = format!("{gateway:?}");
        assert!(printed.contains("plan-abc"), "{printed}");
        assert!(!printed.contains("portal-keys-test"), "{printed}");
    }
}
