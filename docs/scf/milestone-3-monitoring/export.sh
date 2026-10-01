#!/bin/sh
# Export the CloudWatch series behind the Tranche 3 AC 9 report (task 0296).
#
# Window: launch 2026-09-23 09:40 CEST (07:40 UTC) + 7 days. Run it after the
# window closes and before the 1-minute datapoints of 09-23 expire (15 days,
# ~2026-10-08). The output keeps the 1-minute series, so report.py can be
# re-run after CloudWatch has rolled them up.
#
#   export AWS_PROFILE=soroban-admin
#   export API_KEY=<reviewer key from milestone-2-evidence.md>   # optional
#   sh docs/scf/milestone-3-monitoring/export.sh [out-dir]
#   python3 docs/scf/milestone-3-monitoring/report.py [out-dir]
set -eu
: "${AWS_PROFILE:?export AWS_PROFILE=soroban-admin first}"
export AWS_REGION=eu-central-1

START=2026-09-23T07:40:00Z
END=2026-09-30T07:40:00Z
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${1:-$HERE/data}
mkdir -p "$OUT"

account=$(aws sts get-caller-identity --query Account --output text)
[ "$account" = 750702271865 ] || { echo "account is $account, expected 750702271865" >&2; exit 1; }

metric_data() { # $1 = query file, $2 = output name
  aws cloudwatch get-metric-data --start-time "$START" --end-time "$END" \
    --scan-by TimestampAscending --metric-data-queries "file://$1" \
    --output json > "$OUT/$2.json"
}

metric_data "$HERE/queries-minute.json" minute
metric_data "$HERE/queries-rollup.json" rollup
# The same aggregate queries over the whole window and per 24 h from the start.
# Periods are aligned to START, so each "day" runs 09:40 → 09:40 CEST.
for spec in 604800:window 86400:daily; do
  sed "s/\"PERIOD\"/${spec%%:*}/g" "$HERE/queries-aggregate.json" > "$OUT/queries-${spec#*:}.json"
  metric_data "$OUT/queries-${spec#*:}.json" "${spec#*:}"
done

aws cloudwatch describe-alarm-history --history-item-type StateUpdate \
  --start-date "$START" --end-date "$END" --output json > "$OUT/alarm-history.json"

# Point-in-time samples, taken at export time: the data depth
# (earliest_data_available) and the live tip against the network's.
if [ -n "${API_KEY:-}" ]; then
  curl -sS -H "x-api-key: $API_KEY" \
    https://prices-api.sorobanscan.rumblefish.dev/v1/backfill/status > "$OUT/backfill-status.json"
  curl -sS 'https://horizon.stellar.org/ledgers?order=desc&limit=1' > "$OUT/network-tip.json"
else
  echo "API_KEY not set: backfill-status.json and network-tip.json skipped" >&2
fi

date -u +%Y-%m-%dT%H:%M:%SZ > "$OUT/exported-at.txt"
echo "exported to $OUT"
