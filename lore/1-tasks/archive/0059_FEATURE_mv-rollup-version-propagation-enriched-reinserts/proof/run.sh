#!/usr/bin/env bash
# Reproduce the 0059 rollup-propagation proof against a throwaway ClickHouse.
# Usage: ./run.sh           (starts a container, runs all experiments, prints results)
#        KEEP=1 ./run.sh    (leave the container running afterwards)
set -euo pipefail

CONTAINER=ch-proof-0059
IMAGE=clickhouse/clickhouse-server:24.8
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ch() { docker exec -i "$CONTAINER" clickhouse-client "$@"; }

echo "### starting $IMAGE as $CONTAINER"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
docker run -d --name "$CONTAINER" --ulimit nofile=262144:262144 "$IMAGE" >/dev/null
for _ in $(seq 1 30); do ch -q "SELECT 1" >/dev/null 2>&1 && break; sleep 1; done
echo "### clickhouse $(ch -q 'SELECT version()')"

echo "### 01_schema.sql (draft MV is alias-fixed; verbatim draft fails to compile — see file header)"
ch --multiquery < "$HERE/01_schema.sql"

echo "### 02_seed.sql — 15 one-minute rows as 15 SEPARATE inserts"
ch --multiquery < "$HERE/02_seed.sql"

echo "### _1m FINAL (truth):"
ch -q "SELECT sum(volume_base) volume_base, sum(volume_quote_usd) volume_quote_usd, sum(trade_count) trade_count FROM prices.price_ohlcv_1m FINAL FORMAT PrettyCompact"

echo "### Target A — DRAFT insert-trigger MV, _15m_draft FINAL (expect under-count to 1/15):"
ch -q "SELECT volume_base, volume_quote_usd, trade_count, version FROM prices.price_ohlcv_15m_draft FINAL FORMAT PrettyCompact"

echo "### 03_enrich_and_fix.sql — enrich minute 00:06, then re-aggregate from _1m FINAL"
ch --multiquery < "$HERE/03_enrich_and_fix.sql"

echo "### Target A — _15m_draft FINAL AFTER enrichment (expect STILL 0 — no propagation):"
ch -q "SELECT volume_base, volume_quote_usd, version FROM prices.price_ohlcv_15m_draft FINAL FORMAT PrettyCompact"

echo "### see 03_enrich_and_fix.sql SELECT output above for the correct re-aggregated bucket."

if [ "${KEEP:-0}" != "1" ]; then
  echo "### removing $CONTAINER (set KEEP=1 to keep it)"
  docker rm -f "$CONTAINER" >/dev/null
fi
