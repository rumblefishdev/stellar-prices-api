use std::path::{Path, PathBuf};

use super::{Cursor, CursorError};

pub struct StubFileCursor {
    path: PathBuf,
}

impl StubFileCursor {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Cursor for StubFileCursor {
    async fn read(&self) -> Result<u64, CursorError> {
        let raw = match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => raw,
            // No checkpoint file yet is the first-run signal, distinct from a
            // real IO failure — mirrors ClickHouseCursor's Empty/Read split.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CursorError::Empty),
            Err(e) => return Err(CursorError::Read(e.to_string())),
        };
        raw.trim()
            .parse::<u64>()
            .map_err(|e| CursorError::Parse(e.to_string()))
    }

    async fn write(&self, value: u64) -> Result<(), CursorError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| CursorError::Write(e.to_string()))?;
        }
        tokio::fs::write(&self.path, format!("{value}\n"))
            .await
            .map_err(|e| CursorError::Write(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn write_then_read_roundtrips() {
        let dir = tempdir().unwrap();
        let c = StubFileCursor::new(dir.path().join("cursor.txt"));
        c.write(62_528_059).await.unwrap();
        assert_eq!(c.read().await.unwrap(), 62_528_059);
    }

    #[tokio::test]
    async fn missing_file_is_empty_not_read_error() {
        let dir = tempdir().unwrap();
        let c = StubFileCursor::new(dir.path().join("nope.txt"));
        // A missing checkpoint is the first-run signal (Empty), NOT a read error.
        assert!(matches!(c.read().await, Err(CursorError::Empty)));
    }
}
