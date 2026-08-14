//! Live-ClickHouse integration test for rollup-MV drift detection (task 0142).
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-clickhouse --test rollup_drift_it -- --ignored
//!
//! Runs against ClickHouse pinned to the production version (26.3.10.60),
//! because every part of this is a claim about the server's own DDL handling:
//! that `IF NOT EXISTS` swallows an edit, and that `formatQuerySingleLine`
//! renders the file and `system.tables.create_table_query` into the same form.
//! Neither can be verified against a mock, and both could differ across CH
//! releases.
//!
//! The centrepiece is `an_edited_body_is_reported_as_drift_because_the_reapply_
//! silently_no_ops`, which reproduces the defect and the detector in one test:
//! it applies the chain, edits a body, re-applies (proving the apply reports
//! success and changes nothing), and asserts the check reports drift anyway.
//!
//! Owns an isolated scratch database per test and drops it at the end.

use clickhouse::Client;
use prices_clickhouse::drift::{DriftField, MvStatus, check_mv_drift};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Retarget embedded schema SQL onto a scratch database. Same trick as
/// `views_it.rs` / `rollup_chain_it.rs`; the second replace catches `init.sql`'s
/// unqualified `CREATE DATABASE IF NOT EXISTS prices`.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// Scratch database holding the init tables and the real shipped rollup chain.
async fn setup_scratch(db: &str) -> Client {
    let client = Client::default().with_url(ch_url());
    client
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::ROLLUPS_SQL, db))
        .await
        .unwrap();
    client
}

async fn drop_scratch(client: &Client, db: &str) {
    client
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
}

async fn live_ddl(client: &Client, db: &str, name: &str) -> Option<String> {
    client
        .query("SELECT create_table_query FROM system.tables WHERE database = ? AND name = ?")
        .bind(db)
        .bind(name)
        .fetch_all::<String>()
        .await
        .unwrap()
        .into_iter()
        .next()
}

/// The baseline the whole check depends on: a chain applied straight from
/// `rollups.sql` must compare clean.
///
/// This is not a trivial assertion. ClickHouse re-serialises the DDL it stores —
/// dropping `IF NOT EXISTS`, injecting the target's column list and a `DEFINER`
/// clause, and rewriting `INTERVAL 15 MINUTE` as `toIntervalMinute(15)`. A
/// comparison that did not account for all four would report drift on a freshly
/// applied, byte-identical chain, and a permanently-red check is worse than
/// none: the real drift arrives unnoticed inside the noise.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_freshly_applied_chain_reports_no_drift() {
    let db = "it_drift_clean";
    let client = setup_scratch(db).await;

    let reports = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .unwrap();

    assert_eq!(reports.len(), 6, "all six MVs must be reported");
    for report in &reports {
        assert_eq!(
            report.status,
            MvStatus::InSync,
            "{} should be in sync, got {:?}",
            report.name,
            report.status
        );
        assert!(
            !report.needs_attention(),
            "{} should not need attention",
            report.name
        );
        assert!(
            report.live.as_ref().is_some_and(|f| f.is_append()),
            "{} must be live in APPEND mode",
            report.name
        );
    }

    drop_scratch(&client, db).await;
}

/// The task in one test: an edit to an MV body does NOT land on a target that
/// already holds the MV, the apply says success anyway, and the drift check is
/// what makes that visible.
///
/// The edit used is the real one task 0146 needs — replacing the unguarded
/// `argMax(close_usd, …)` with the `argMaxIf(…, close_usd > 0)` guard from task
/// 0145 — so this doubles as evidence for why 0142 blocks it.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn an_edited_body_is_reported_as_drift_because_the_reapply_silently_no_ops() {
    let db = "it_drift_edited";
    let client = setup_scratch(db).await;

    let before = live_ddl(&client, db, "mv_ohlcv_1m_to_15m")
        .await
        .expect("the MV exists after setup");

    let edited = prices_clickhouse::ROLLUPS_SQL.replace(
        "argMax(close_usd, t.timestamp)            AS close_usd",
        "argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd",
    );
    assert_ne!(
        edited,
        prices_clickhouse::ROLLUPS_SQL,
        "the test's own edit must apply — if the projection is reworded in \
         rollups.sql this test goes blind and must be updated, not deleted"
    );
    assert_eq!(
        edited.matches("argMaxIf(close_usd").count(),
        6,
        "the edit must reach every MV in the chain"
    );

    // Re-apply the EDITED file, exactly as an unwitting operator would.
    prices_clickhouse::apply_sql(&client, &rewrite(&edited, db))
        .await
        .expect("re-applying an edited rollups.sql reports SUCCESS — that is the defect");

    let after = live_ddl(&client, db, "mv_ohlcv_1m_to_15m")
        .await
        .expect("the MV still exists");
    assert_eq!(
        before, after,
        "IF NOT EXISTS must have swallowed the edit — if this ever fails, the \
         rollup MVs became re-appliable and task 0142's premise has changed"
    );
    assert!(
        !after.contains("argMaxIf"),
        "the live definition must still hold the OLD projection"
    );

    // The check compares the file to the target, so the edited file is the
    // source of truth here — the same input the operator just applied.
    let reports = check_mv_drift(&client, db, &edited).await.unwrap();
    assert_eq!(reports.len(), 6);

    for report in &reports {
        let MvStatus::Drifted(differences) = &report.status else {
            panic!(
                "{} must report drift after an edit that did not land, got {:?}",
                report.name, report.status
            );
        };
        assert!(report.needs_attention());
        assert_eq!(
            differences.len(),
            1,
            "{}: only the body changed, got {:?}",
            report.name,
            differences
        );
        let d = &differences[0];
        assert_eq!(d.field, DriftField::Body);
        assert!(
            d.declared.contains("argMaxIf(close_usd"),
            "{}: the declared side must carry the edit",
            report.name
        );
        assert!(
            !d.live.contains("argMaxIf(close_usd"),
            "{}: the live side must still carry the old projection",
            report.name
        );
    }

    // And the control: checking the target against the UNEDITED file — what is
    // actually deployed — must still be clean. Without this the test would also
    // pass for a check that reports drift unconditionally.
    let control = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .unwrap();
    for report in &control {
        assert_eq!(
            report.status,
            MvStatus::InSync,
            "{} must be in sync against the unedited file",
            report.name
        );
    }

    drop_scratch(&client, db).await;
}

/// A tier whose MV was dropped — the task 0136 shape, where a recovery drops an
/// MV and nothing re-creates it — must report as missing rather than as in sync.
/// A missing MV is silent by nature: the target table simply stops receiving
/// rows, which looks identical to a quiet market until someone reads a chart.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_dropped_mv_is_reported_as_missing() {
    let db = "it_drift_missing";
    let client = setup_scratch(db).await;

    client
        .query(&format!("DROP VIEW {db}.mv_ohlcv_4h_to_1d"))
        .execute()
        .await
        .unwrap();

    let reports = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .unwrap();

    let dropped = reports
        .iter()
        .find(|r| r.name == "mv_ohlcv_4h_to_1d")
        .expect("the dropped MV must still be reported, not omitted");
    assert_eq!(dropped.status, MvStatus::Missing);
    assert!(dropped.live.is_none());
    assert!(dropped.needs_attention());

    assert_eq!(
        reports
            .iter()
            .filter(|r| r.status == MvStatus::InSync)
            .count(),
        5,
        "the other five must be unaffected"
    );

    drop_scratch(&client, db).await;
}

/// A live MV re-created without `APPEND` is the task 0090 production data loss:
/// each refresh atomically replaces the whole target table with just the bounded
/// window, deleting pre-rolled history. It must surface as drift AND as a
/// not-append condition — the second is what tells an operator this is
/// destroying data now, not merely stale.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_replace_mode_mv_is_reported_as_drift_and_as_not_append() {
    let db = "it_drift_replace_mode";
    let client = setup_scratch(db).await;

    // Re-create the weekly MV in replace mode — the exact regression 0095 fixed.
    // Done by DROP + CREATE because that is the only way the shape can arise.
    client
        .query(&format!("DROP VIEW {db}.mv_ohlcv_1d_to_1w"))
        .execute()
        .await
        .unwrap();
    let replace_mode = rewrite(prices_clickhouse::ROLLUPS_SQL, db)
        .split(';')
        .map(str::trim)
        .find(|s| s.contains("mv_ohlcv_1d_to_1w"))
        .expect("the weekly statement")
        .replace("REFRESH EVERY 1 DAY APPEND", "REFRESH EVERY 1 DAY");
    client.query(&replace_mode).execute().await.unwrap();

    let reports = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .unwrap();

    let weekly = reports
        .iter()
        .find(|r| r.name == "mv_ohlcv_1d_to_1w")
        .expect("reported");
    let MvStatus::Drifted(differences) = &weekly.status else {
        panic!("expected drift, got {:?}", weekly.status);
    };
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].field, DriftField::Refresh);
    assert_eq!(differences[0].declared, "EVERY 1 DAY APPEND");
    assert_eq!(differences[0].live, "EVERY 1 DAY");

    let live = weekly.live.as_ref().expect("live fingerprint");
    assert!(
        !live.is_append(),
        "a refresh clause with no APPEND must not report as append mode"
    );
    assert!(weekly.needs_attention());

    drop_scratch(&client, db).await;
}

/// An MV re-created by hand WITHOUT `REFRESH` is an insert-trigger MV, not a
/// refreshable one, and the check cannot fingerprint it. It must degrade that
/// one row — not the report.
///
/// This is the review finding on PR #216: the live-side parse failure used to
/// propagate out of `check_mv_drift`, so an unreadable definition on the FIRST
/// statement in the file hid the other five entirely, including any that had
/// lost `APPEND`. The tool's whole purpose is defeated by a report that goes
/// quiet at the first surprise.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn an_unreadable_definition_degrades_one_row_not_the_whole_report() {
    let db = "it_drift_unreadable";
    let client = setup_scratch(db).await;

    // Re-create the FIRST declared MV as an insert-trigger MV. First on purpose:
    // under the old behaviour this is the one that hid all the others.
    client
        .query(&format!("DROP VIEW {db}.mv_ohlcv_1m_to_15m"))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "CREATE MATERIALIZED VIEW {db}.mv_ohlcv_1m_to_15m \
             TO {db}.price_ohlcv_15m AS SELECT * FROM {db}.price_ohlcv_1m"
        ))
        .execute()
        .await
        .unwrap();

    let reports = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .expect("an unreadable live definition must not abort the check");

    assert_eq!(reports.len(), 6, "every declared MV must still be reported");

    let broken = reports
        .iter()
        .find(|r| r.name == "mv_ohlcv_1m_to_15m")
        .expect("reported");
    let MvStatus::Unparseable(rendering) = &broken.status else {
        panic!("expected Unparseable, got {:?}", broken.status);
    };
    assert!(
        rendering.contains("mv_ohlcv_1m_to_15m"),
        "the report must carry the live rendering so the operator can see what it is"
    );
    assert!(broken.needs_attention());

    // The point of the finding: the other five are still compared.
    assert_eq!(
        reports
            .iter()
            .filter(|r| r.status == MvStatus::InSync)
            .count(),
        5,
        "the remaining MVs must still be checked, got {:?}",
        reports
            .iter()
            .map(|r| (&r.name, &r.status))
            .collect::<Vec<_>>()
    );

    drop_scratch(&client, db).await;
}

/// An MV that is not in the file but writes into a table the declared MVs own.
/// Walking `rollups.sql` alone cannot find it, so without the sweep the tool
/// would print an all-clear while two MVs insert into one ReplacingMergeTree.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn an_undeclared_writer_into_a_rollup_target_is_reported() {
    let db = "it_drift_undeclared";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "CREATE MATERIALIZED VIEW {db}.mv_ohlcv_leftover \
             REFRESH EVERY 1 DAY APPEND TO {db}.price_ohlcv_1d AS \
             SELECT * FROM {db}.price_ohlcv_4h"
        ))
        .execute()
        .await
        .unwrap();

    // A second MV writing into a _bak table must NOT be flagged: the cluster
    // carries those (task 0177) and they are not the live targets.
    client
        .query(&format!(
            "CREATE TABLE {db}.price_ohlcv_1d_bak AS {db}.price_ohlcv_1d"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "CREATE MATERIALIZED VIEW {db}.mv_ohlcv_to_bak \
             REFRESH EVERY 1 DAY APPEND TO {db}.price_ohlcv_1d_bak AS \
             SELECT * FROM {db}.price_ohlcv_4h"
        ))
        .execute()
        .await
        .unwrap();

    let reports = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .unwrap();

    let extra = reports
        .iter()
        .find(|r| r.name == "mv_ohlcv_leftover")
        .expect("the undeclared writer must be reported");
    assert_eq!(extra.status, MvStatus::Undeclared);
    assert!(extra.needs_attention());

    assert!(
        !reports.iter().any(|r| r.name == "mv_ohlcv_to_bak"),
        "an MV writing into price_ohlcv_1d_bak must not be read as a writer into \
         price_ohlcv_1d — the trailing-character guard is what stops that"
    );

    // The six declared MVs are untouched and still compare clean.
    assert_eq!(
        reports
            .iter()
            .filter(|r| r.status == MvStatus::InSync)
            .count(),
        6
    );

    drop_scratch(&client, db).await;
}

/// The check must not depend on the chain having been applied by this crate: a
/// hand-edited MV on a provisioned cluster is the realistic drift, and it is
/// what an `IF NOT EXISTS` apply can never correct.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_hand_edited_window_is_reported_as_drift() {
    let db = "it_drift_hand_edited";
    let client = setup_scratch(db).await;

    client
        .query(&format!("DROP VIEW {db}.mv_ohlcv_1h_to_4h"))
        .execute()
        .await
        .unwrap();
    // Widen the window by hand — plausible as an operator catch-up tweak, and
    // exactly the kind of change that must not silently persist unrecorded.
    let widened = rewrite(prices_clickhouse::ROLLUPS_SQL, db)
        .split(';')
        .map(str::trim)
        .find(|s| s.contains("mv_ohlcv_1h_to_4h"))
        .expect("the 4h statement")
        .replace("now() - INTERVAL 1 DAY", "now() - INTERVAL 7 DAY");
    client.query(&widened).execute().await.unwrap();

    let reports = check_mv_drift(&client, db, prices_clickhouse::ROLLUPS_SQL)
        .await
        .unwrap();

    let four_hour = reports
        .iter()
        .find(|r| r.name == "mv_ohlcv_1h_to_4h")
        .expect("reported");
    let MvStatus::Drifted(differences) = &four_hour.status else {
        panic!("expected drift, got {:?}", four_hour.status);
    };
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].field, DriftField::Body);
    assert!(
        differences[0].declared.contains("toIntervalDay(1)")
            && differences[0].live.contains("toIntervalDay(7)"),
        "the report must show both windows so the operator can see which is which"
    );

    // Everything else stays clean — drift is per-MV, not a whole-chain verdict.
    assert_eq!(
        reports
            .iter()
            .filter(|r| r.status == MvStatus::InSync)
            .count(),
        5
    );

    drop_scratch(&client, db).await;
}
