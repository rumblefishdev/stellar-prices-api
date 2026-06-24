//! Object-source trait — the production-swap seam where prototype-mode
//! local-disk reads become `aws_sdk_s3::Client::get_object` calls.

use std::future::Future;

pub mod local_disk;
#[cfg(feature = "lambda")]
pub mod s3;

pub use local_disk::LocalDiskFetcher;
#[cfg(feature = "lambda")]
pub use s3::S3Fetcher;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("i/o error fetching {key}: {source}")]
    Io {
        key: String,
        #[source]
        source: std::io::Error,
    },
    #[error("object-store error fetching {key}: {detail}")]
    Backend { key: String, detail: String },
}

pub trait ObjectFetcher {
    /// `Ok(Some(bytes))` on hit, `Ok(None)` on miss (treat as a gap and
    /// stop the reconcile run), `Err(...)` on a hard error.
    fn fetch(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>, FetchError>> + Send;
}
