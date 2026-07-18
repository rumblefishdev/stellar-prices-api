-- Seed the two canonical backfill streams (task 0051 / design §3.5).
--
-- Idempotent by construction: inserts only the canonical task_names that are
-- not already present. backfill_progress is ReplacingMergeTree(updated_at)
-- keyed by task_name, so a blind re-insert with a fresh updated_at would
-- REPLACE the live row and reset current_ledger — this NOT IN guard makes a
-- re-run a no-op once the rows exist, preserving real backfill progress.
--
-- Placeholder ledger bounds (0) are filled in by the backfill streams
-- (sdex-backfill / soroban-amm) as they advance; status starts 'running'.
INSERT INTO prices.backfill_progress
    (task_name, start_ledger, target_ledger, current_ledger, status)
SELECT task_name, 0, 0, 0, 'running'
FROM (SELECT arrayJoin(['sdex_archive', 'soroban_amm']) AS task_name) AS canonical
WHERE task_name NOT IN (SELECT task_name FROM prices.backfill_progress);
