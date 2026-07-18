//! S3-backed [`ObjectFetcher`] — the production fetch path.
//!
//! Reads Galexie `*.xdr.zst` objects from BE's `stellar-ledger-data` bucket by
//! their derived key. A `NoSuchKey` is mapped to `Ok(None)` (a gap → the
//! reconcile loop stops cleanly), every other S3 error to `Err`. Bucket name
//! arrives via env var (CDK injects it from `/platform/{env}/…` SSM at deploy).

use aws_sdk_s3::Client;

use super::{FetchError, ObjectFetcher};

pub struct S3Fetcher {
    client: Client,
    bucket: String,
}

impl S3Fetcher {
    pub fn new(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }

    /// Build from the ambient AWS config (Lambda execution role).
    pub async fn from_env(bucket: impl Into<String>) -> Self {
        let cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        Self::new(Client::new(&cfg), bucket)
    }
}

impl ObjectFetcher for S3Fetcher {
    async fn fetch(&self, key: &str) -> Result<Option<Vec<u8>>, FetchError> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => {
                let data = out.body.collect().await.map_err(|e| FetchError::Backend {
                    key: key.to_string(),
                    detail: e.to_string(),
                })?;
                Ok(Some(data.into_bytes().to_vec()))
            }
            Err(err) => {
                let svc = err.into_service_error();
                if svc.is_no_such_key() {
                    Ok(None)
                } else {
                    Err(FetchError::Backend {
                        key: key.to_string(),
                        detail: svc.to_string(),
                    })
                }
            }
        }
    }
}
