#!/usr/bin/env bash
# 0136 recovery test — DOES `DETACH`+`ATTACH` RESTORE A WEDGED TABLE?
#
# LOCAL DOCKER CH ONLY (26.3.10.60, prod pin). Never run against ch-prod-01.
#
# Companion to scripts/repro-0136-merge-freeze.sh, which reproduced the FREEZE.
# This one tests the proposed RECOVERY, and the operational risks around it:
#
#   T1  DETACH+ATTACH on a wedged table -> do merges resume, does the backlog
#       drain, does the pending ALTER DELETE finally run, is data preserved?
#   T2  OPTIMIZE on a wedged table (the "be the scheduler by hand" stopgap).
#   T3  Can a table be DETACHed while a materialized view writes into it,
#       and does the MV resume after ATTACH?  <- the flagged unknown
#
# LIMIT OF THE SIMULATION: the local wedge is built with SYSTEM STOP MERGES,
# which prod FALSIFIED as its cause. So T1/T2 answer "what does this command do
# to a table whose merges are stopped", not "what does it do to prod". T3 and
# the data-integrity assertions transfer regardless of the wedge mechanism.
set -uo pipefail

CH="docker exec -i stellar-prices-api-clickhouse-1 clickhouse-client"
DB=recov_0136

q()  { $CH --query "$1"; }
qf() { $CH --format=PrettyCompact --query "$1"; }

hr() { echo; echo "=============================================================="; echo "$1"; echo "=============================================================="; }

echo "### CH version: $(q 'SELECT version()')  (prod pin = 26.3.10.60)"

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
  local tbl=$1 n=$2 base=${3:-0}
  for i in $(seq 1 "$n"); do
    q "INSERT INTO $DB.$tbl SELECT
         toDateTime('2026-07-01 00:00:00') + number * 900,
         number % 50,
         if(number % 3 = 0, 'phoenix', 'sdex'),
         1.5,
         $((base + i))
       FROM numbers(200)"
  done
}

state() {
  local tbl=$1 label=$2
  echo
  echo "--- $label / $tbl ---"
  qf "SELECT count() AS active_parts, min(level) AS min_lvl, max(level) AS max_lvl,
             sum(rows) AS rows_in_parts
      FROM system.parts WHERE database='$DB' AND table='$tbl' AND active"
  qf "SELECT mutation_id, is_done, parts_to_do, latest_fail_time,
             substring(latest_fail_reason,1,50) AS fail_reason
      FROM system.mutations WHERE database='$DB' AND table='$tbl'"
  qf "SELECT event_type, count() AS n FROM system.part_log
      WHERE database='$DB' AND table='$tbl' GROUP BY event_type ORDER BY n DESC"
}

# Logical data, independent of parts/merges. FINAL == what a reader sees.
data() {
  local tbl=$1 label=$2
  echo "  [data/$label] $(q "SELECT concat('rows_final=', toString(count())) FROM $DB.$tbl FINAL") \
$(q "SELECT concat('phoenix=', toString(countIf(source='phoenix')), ' sdex=', toString(countIf(source='sdex'))) FROM $DB.$tbl FINAL")"
}

settle() { sleep "$1"; q "SYSTEM FLUSH LOGS" >/dev/null 2>&1; }

wedge() {                       # build a table matching prod's observables
  local tbl=$1
  mktable "$tbl"
  seed "$tbl" 10 0
  settle 6
  q "SYSTEM STOP MERGES $DB.$tbl"
  q "ALTER TABLE $DB.$tbl DELETE WHERE source = 'phoenix'"
  seed "$tbl" 10 100           # inserts keep arriving, as the rollup MV did
  settle 8
}

################################################################################
hr "T1 — DETACH + ATTACH on a wedged table"
################################################################################
wedge t1
state t1 "T1: WEDGED (pre-recovery)"
data  t1 "pre"
PARTS_BEFORE=$(q "SELECT count() FROM system.parts WHERE database='$DB' AND table='t1' AND active")
ROWS_BEFORE=$(q "SELECT count() FROM $DB.t1 FINAL")

echo
echo ">>> DETACH TABLE $DB.t1   (timed; 60s timeout to catch a hang)"
T0=$(date +%s)
timeout 60 $CH --query "DETACH TABLE $DB.t1" ; DET_RC=$?
T1S=$(( $(date +%s) - T0 ))
echo "    exit=$DET_RC  elapsed=${T1S}s   $( [ $DET_RC -eq 124 ] && echo '<-- HUNG' )"
echo "    visible in system.tables now: $(q "SELECT count() FROM system.tables WHERE database='$DB' AND name='t1'")"

echo
echo ">>> ATTACH TABLE $DB.t1"
T0=$(date +%s)
timeout 60 $CH --query "ATTACH TABLE $DB.t1" ; ATT_RC=$?
T2S=$(( $(date +%s) - T0 ))
echo "    exit=$ATT_RC  elapsed=${T2S}s"

echo
echo ">>> waiting 25s for the (hopefully new) scheduler to act..."
settle 25
state t1 "T1: AFTER detach/attach"
data  t1 "post"
PARTS_AFTER=$(q "SELECT count() FROM system.parts WHERE database='$DB' AND table='t1' AND active")
ROWS_AFTER=$(q "SELECT count() FROM $DB.t1 FINAL")
MUT_DONE=$(q "SELECT max(is_done) FROM system.mutations WHERE database='$DB' AND table='t1'")
MERGED=$(q "SELECT count() FROM system.part_log WHERE database='$DB' AND table='t1' AND event_type='MergeParts'")

echo
echo "### T1 VERDICT"
echo "    active parts   : $PARTS_BEFORE -> $PARTS_AFTER   (drop == merges resumed)"
echo "    MergeParts logged after attach: $MERGED"
echo "    pending mutation is_done      : $MUT_DONE"
echo "    rows (FINAL)   : $ROWS_BEFORE -> $ROWS_AFTER"

################################################################################
hr "T2 — OPTIMIZE on a wedged table (the by-hand stopgap)"
################################################################################
wedge t2
state t2 "T2: WEDGED"
echo
echo ">>> OPTIMIZE TABLE $DB.t2 PARTITION 202607   (120s timeout)"
T0=$(date +%s)
timeout 120 $CH --query "OPTIMIZE TABLE $DB.t2 PARTITION 202607" 2>&1 | head -5 ; OPT_RC=${PIPESTATUS[0]}
echo "    exit=$OPT_RC  elapsed=$(( $(date +%s) - T0 ))s  $( [ $OPT_RC -eq 124 ] && echo '<-- HUNG' )"
settle 5
state t2 "T2: AFTER optimize"

################################################################################
hr "T3 — can a table be DETACHed while a materialized view writes into it?"
################################################################################
mktable t3_src
mktable t3_target
seed t3_src 3 0
settle 3

echo ">>> creating refreshable MV t3_mv -> t3_target (REFRESH EVERY 5 SECOND APPEND)"
q "CREATE MATERIALIZED VIEW $DB.t3_mv
   REFRESH EVERY 5 SECOND APPEND
   TO $DB.t3_target AS
   SELECT timestamp, asset_id, source, close, sum(version) AS version
   FROM $DB.t3_src
   GROUP BY timestamp, asset_id, source, close" 2>&1 | head -3

settle 12
echo "    t3_target rows after MV ticks: $(q "SELECT count() FROM $DB.t3_target")"
echo "    MV status: $(q "SELECT concat(status, ' exception=', substring(last_refresh_result,1,40)) FROM system.view_refreshes WHERE database='$DB' AND view='t3_mv'" 2>/dev/null || echo 'n/a')"

echo
echo ">>> DETACH TABLE $DB.t3_target  (the MV's TO target, while the MV is live)"
timeout 60 $CH --query "DETACH TABLE $DB.t3_target" 2>&1 | head -3 ; T3_DET_RC=${PIPESTATUS[0]}
echo "    exit=$T3_DET_RC  $( [ $T3_DET_RC -eq 0 ] && echo '<-- DETACH ALLOWED with a dependent MV' )"

echo ">>> letting the MV tick 12s with its target GONE (expect refresh errors, no crash)"
settle 12
echo "    MV status while target detached: $(q "SELECT concat(status,' | ',substring(exception,1,70)) FROM system.view_refreshes WHERE database='$DB' AND view='t3_mv'" 2>/dev/null || echo 'n/a')"

echo
echo ">>> ATTACH TABLE $DB.t3_target"
timeout 60 $CH --query "ATTACH TABLE $DB.t3_target" 2>&1 | head -3 ; T3_ATT_RC=${PIPESTATUS[0]}
echo "    exit=$T3_ATT_RC"
ROWS_AT_ATTACH=$(q "SELECT count() FROM $DB.t3_target")
echo "    t3_target rows immediately after attach: $ROWS_AT_ATTACH"
echo ">>> waiting 20s to see whether the MV resumes appending on its own..."
settle 20
ROWS_LATER=$(q "SELECT count() FROM $DB.t3_target")
echo "    t3_target rows 20s later: $ROWS_LATER   (increase == MV self-recovered)"
echo "    MV status: $(q "SELECT concat(status,' | ',substring(exception,1,70)) FROM system.view_refreshes WHERE database='$DB' AND view='t3_mv'" 2>/dev/null || echo 'n/a')"

hr "SUMMARY"
echo "T1 detach rc=$DET_RC attach rc=$ATT_RC | parts $PARTS_BEFORE->$PARTS_AFTER | mutation done=$MUT_DONE | rows $ROWS_BEFORE->$ROWS_AFTER"
echo "T2 optimize-while-wedged rc=$OPT_RC"
echo "T3 detach-with-dependent-MV rc=$T3_DET_RC attach rc=$T3_ATT_RC | rows $ROWS_AT_ATTACH -> $ROWS_LATER"
