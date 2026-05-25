use std::path::{Path, PathBuf};

pub const BUCKET: &str = "aws-public-blockchain";
pub const ROOT_PREFIX: &str = "v1.1/stellar/ledgers/pubnet";
pub const PARTITION_SIZE: u32 = 64_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub start: u32,
    pub end: u32,
    pub hex: String,
}

impl Partition {
    pub fn from_ledger(seq: u32) -> Self {
        let start = seq - (seq % PARTITION_SIZE);
        let end = start + PARTITION_SIZE - 1;
        let hex = format!("{:08X}", u32::MAX - start);
        Self { start, end, hex }
    }

    pub fn s3_folder(&self) -> String {
        format!(
            "s3://{BUCKET}/{ROOT_PREFIX}/{}--{}-{}/",
            self.hex, self.start, self.end
        )
    }

    pub fn local_folder(&self, temp_dir: &Path) -> PathBuf {
        temp_dir.join(format!("{}--{}-{}", self.hex, self.start, self.end))
    }

    pub fn clamped(&self, run_start: u32, run_end: u32) -> (u32, u32) {
        (run_start.max(self.start), run_end.min(self.end))
    }

    pub fn local_ledger_path(&self, seq: u32, temp_dir: &Path) -> PathBuf {
        let file_hex = format!("{:08X}", u32::MAX - seq);
        self.local_folder(temp_dir)
            .join(format!("{file_hex}--{seq}.xdr.zst"))
    }
}

pub fn partitions_for_range(start: u32, end: u32) -> Vec<Partition> {
    let mut result = Vec::new();
    if start > end {
        return result;
    }
    let mut cursor = start;
    while cursor <= end {
        let p = Partition::from_ledger(cursor);
        let next = p.end + 1;
        result.push(p);
        cursor = next;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_bounds() {
        let p = Partition::from_ledger(62_026_937);
        assert_eq!(p.start, 62_016_000);
        assert_eq!(p.end, 62_079_999);
        assert_eq!(p.hex, "FC4DB5FF");
    }

    #[test]
    fn partition_boundary_start_is_inclusive() {
        let p = Partition::from_ledger(62_016_000);
        assert_eq!(p.start, 62_016_000);
    }

    #[test]
    fn partitions_for_range_single() {
        let ps = partitions_for_range(62_020_000, 62_025_000);
        assert_eq!(ps.len(), 1);
    }

    #[test]
    fn partitions_for_range_empty_when_start_gt_end() {
        let ps = partitions_for_range(100, 50);
        assert!(ps.is_empty());
    }

    #[test]
    fn partitions_for_range_spans_three() {
        let ps = partitions_for_range(62_020_000, 62_150_000);
        assert_eq!(ps.len(), 3);
    }
}
