use std::path::{Path, PathBuf};

use super::{FetchError, ObjectFetcher};

pub struct LocalDiskFetcher {
    root: PathBuf,
}

impl LocalDiskFetcher {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }
}

impl ObjectFetcher for LocalDiskFetcher {
    async fn fetch(&self, key: &str) -> Result<Option<Vec<u8>>, FetchError> {
        let path = self.root.join(key);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(FetchError::Io {
                key: key.to_string(),
                source,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn hit_returns_bytes() {
        let dir = tempdir().unwrap();
        let key = "subdir/file.bin";
        tokio::fs::create_dir_all(dir.path().join("subdir"))
            .await
            .unwrap();
        tokio::fs::write(dir.path().join(key), b"hello")
            .await
            .unwrap();
        let f = LocalDiskFetcher::new(dir.path());
        assert_eq!(f.fetch(key).await.unwrap(), Some(b"hello".to_vec()));
    }

    #[tokio::test]
    async fn miss_returns_none() {
        let dir = tempdir().unwrap();
        let f = LocalDiskFetcher::new(dir.path());
        assert_eq!(f.fetch("nope").await.unwrap(), None);
    }
}
