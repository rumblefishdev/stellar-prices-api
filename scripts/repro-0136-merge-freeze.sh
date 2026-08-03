#!/usr/bin/env bash
# 0136 local reproduction — LOCAL DOCKER CH ONLY (26.3.10.60, prod pin).
# Never run against ch-prod-01.
set -uo pipefail

CH="docker exec -i stellar-prices-api-clickhouse-1 clickhouse-client"
DB=repro_0136

q() { $CH --query "$1"; }
qf() { $CH --format=PrettyCompact --query "$1"; }

echo "### CH version: $(q 'SELECT version()')"
echo "### background_pool_size: $(q "SELECT value FROM system.server_settings WHERE name='background_pool_size'")"
echo "### free_entries_to_execute_mutation: $(q "SELECT value FROM system.merge_tree_settings WHERE name='number_of_free_entries_in_pool_to_execute_mutation'")"

q "DROP DATABASE IF EXISTS $DB"
q "CREATE DATABASE $DB"

mktable() {
  q "CREATE TABLE $DB.$1 (
        timestamp DateTime,
        asset_id  UInt64,
        source    String,
        close     Decimal(38,14),
        version   UInt64
     ) ENGINE = ReplacingMergeTree(version)
       PARTITION BY toYYYYMM(timestamp)
       ORDER BY (asset_id, timestamp, source)"
}

# One INSERT == one part. Mirrors the 1-minute rollup MV appending forever.
seed() {
  local tbl=$1 n=$2
  for i in $(seq 1 "$n"); do
    q "INSERT INTO $DB.$tbl SELECT
         toDateTime('2026-07-01 00:00:00') + number * 900,
         number % 50,
         if(number % 3 = 0, 'phoenix', 'sdex'),
         1.5,
         $i
       FROM numbers(200)"
  done
}

report() {
  local tbl=$1 label=$2
  echo
  echo "--- $label / $tbl ---"
  qf "SELECT count() AS active_parts, min(level) AS min_level, max(level) AS max_level,
             sum(rows) AS rows
      FROM system.parts WHERE database='$DB' AND table='$tbl' AND active"
  qf "SELECT mutation_id, is_done, parts_to_do,
             latest_fail_time, substring(latest_fail_reason,1,60) AS fail_reason
      FROM system.mutations WHERE database='$DB' AND table='$tbl'"
  qf "SELECT event_type, count() AS n FROM system.part_log
      WHERE database='$DB' AND table='$tbl' GROUP BY event_type ORDER BY n DESC"
}

wait_settle() {
  for _ in $(seq 1 "$1"); do sleep 2; done
  q "SYSTEM FLUSH LOGS" >/dev/null 2>&1
}

echo
echo "==================== TEST A — merges NORMAL, then mutate ===================="
mktable t_a
seed t_a 12
wait_settle 5
report t_a "A: after seed, before mutation"
q "ALTER TABLE $DB.t_a DELETE WHERE source = 'phoenix'"
wait_settle 6
report t_a "A: after ALTER DELETE"

echo
echo "==================== TEST B — SYSTEM STOP MERGES, then mutate ===================="
mktable t_b
seed t_b 12
wait_settle 5
report t_b "B: after seed, merges still normal"
echo ">>> SYSTEM STOP MERGES $DB.t_b   (LOCAL ONLY)"
q "SYSTEM STOP MERGES $DB.t_b"
q "ALTER TABLE $DB.t_b DELETE WHERE source = 'phoenix'"
seed t_b 12          # inserts keep arriving, as the rollup MV did until 07-21
wait_settle 8
report t_b "B: after STOP MERGES + ALTER DELETE + more inserts"

echo
echo "==================== TEST C — pending mutation ALONE (no stop) ===================="
echo "Covered by A: if A's mutation completed and parts merged, a pending"
echo "mutation by itself does not freeze a table."
