//! S3 key derivation for Galexie ledger objects.
//!
//! Mirrors BE's indexer (`soroban-block-explorer/crates/indexer/src/handler/mod.rs:263`).
//! Coupled to Galexie's datastore schema (`ledgers_per_file = 1`,
//! `files_per_partition = 64000`). A wrong key reads as a gap and stalls the tail.

const FILES_PER_PARTITION: i64 = 64_000;

pub fn ledger_s3_key(ledger: i64) -> String {
    let part_start = (ledger / FILES_PER_PARTITION) * FILES_PER_PARTITION;
    let part_end = part_start + FILES_PER_PARTITION - 1;
    let part_prefix = 0xFFFF_FFFFu32 - part_start as u32;
    let file_prefix = 0xFFFF_FFFFu32 - ledger as u32;
    format!("{part_prefix:08X}--{part_start}-{part_end}/{file_prefix:08X}--{ledger}.xdr.zst")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_verified_live_key() {
        // From BE: L = 62528059 → FC45E5FF--62528000-62591999/FC45E5C4--62528059.xdr.zst
        assert_eq!(
            ledger_s3_key(62_528_059),
            "FC45E5FF--62528000-62591999/FC45E5C4--62528059.xdr.zst"
        );
    }

    #[test]
    fn ledgers_in_same_partition_share_prefix() {
        let key_a = ledger_s3_key(64_000);
        let key_b = ledger_s3_key(127_999);
        let prefix_a = key_a.split('/').next().unwrap();
        let prefix_b = key_b.split('/').next().unwrap();
        assert_eq!(prefix_a, prefix_b);
        assert!(prefix_a.ends_with("--64000-127999"));
    }

    #[test]
    fn partition_boundary_changes_prefix() {
        let last = ledger_s3_key(127_999);
        let first_next = ledger_s3_key(128_000);
        let prefix_last = last.split('/').next().unwrap();
        let prefix_next = first_next.split('/').next().unwrap();
        assert_ne!(prefix_last, prefix_next);
        assert!(prefix_next.ends_with("--128000-191999"));
    }
}
