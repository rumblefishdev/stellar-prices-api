#!/usr/bin/env bash
#
# item7-quota-rollover.sh — task 0180 item 7
#
# Measures WHEN an API Gateway usage-plan quota resets, and in which timezone,
# by watching a DAY-period scratch plan across a rollover.
#
# Why it polls the endpoint instead of reading GetUsage:
#   Item 8 measured that GetUsage is not read-after-write and can report a pair
#   inconsistent with `limit` for over a minute. Reading the reset instant off
#   the counter would therefore measure the REPORTING lag, not the moment
#   enforcement changes. So the verdict comes from the data plane: drain the
#   quota until requests are rejected, then poll until one is served again.
#   The first success IS the reset instant, +/- the poll interval, and it is
#   immune to any lag in the counter. GetUsage is sampled alongside anyway --
#   if the two disagree, that disagreement is itself a finding.
#
# What it discriminates, beyond the timezone:
#   `manual-api-key-tier.md` asks whether a period is calendar-aligned or runs
#   from plan creation. Those two hypotheses predict different instants only if
#   the plan is created far from midnight UTC -- so the setup timestamp is
#   recorded in the log header, and `analyse` reports the observed reset against
#   both.
#
# Safety:
#   - Refuses to touch anything whose name does not start with lore0180-scratch.
#   - Creates its own REST API with a MOCK integration; no Lambda, no ClickHouse.
#   - Does NOT loop on UpdateUsagePlan: that call is throttled to 1 per 20s per
#     account, non-adjustable, and the control plane shares 10rps/burst-40 with
#     our deploys. Setup makes a handful of writes, spaced. The poll loop makes
#     data-plane requests and read-only GetUsage calls.
#
# Usage:
#   ./item7-quota-rollover.sh setup       # create scratch API + plan + key
#   ./item7-quota-rollover.sh drain       # exhaust the quota
#   ./item7-quota-rollover.sh poll [h]    # poll for h hours (default 26)
#   ./item7-quota-rollover.sh run         # setup + drain + poll, one shot
#   ./item7-quota-rollover.sh analyse     # read the verdict off the log
#   ./item7-quota-rollover.sh status      # show current state
#   ./item7-quota-rollover.sh teardown    # delete everything it created
#
set -euo pipefail

AWS_PROFILE_NAME="${AWS_PROFILE_NAME:-stellar}"
REGION="${AWS_REGION:-eu-central-1}"
NAME_PREFIX="lore0180-scratch"
STAGE="test"
QUOTA_LIMIT="${QUOTA_LIMIT:-3}"
POLL_INTERVAL_SEC="${POLL_INTERVAL_SEC:-60}"
USAGE_EVERY_N_POLLS="${USAGE_EVERY_N_POLLS:-5}"
DEFAULT_HOURS=26

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$HERE/data"
STATE_FILE="$DATA_DIR/state.env"
LOG_FILE="${LOG_FILE:-$DATA_DIR/item7-poll.tsv}"

mkdir -p "$DATA_DIR"

aws_() { aws --profile "$AWS_PROFILE_NAME" --region "$REGION" --output json "$@"; }
now_utc() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { printf '[%s] %s\n' "$(now_utc)" "$*" >&2; }
die() { printf '[%s] FATAL: %s\n' "$(now_utc)" "$*" >&2; exit 1; }

require_auth() {
  aws_ sts get-caller-identity >/dev/null 2>&1 \
    || die "no valid AWS session for profile '$AWS_PROFILE_NAME'. Run: aws sso login --profile $AWS_PROFILE_NAME"
}

# Refuse to operate on anything not created by this script. The production plan
# is pricing-api-free-<env> and its id is published to SSM; this is the guard
# that makes "never the production plan" true by construction rather than care.
assert_scratch() {
  local name="$1" kind="$2"
  case "$name" in
    "$NAME_PREFIX"*) : ;;
    *) die "refusing to touch $kind '$name' -- name does not start with '$NAME_PREFIX'" ;;
  esac
}

load_state() {
  [[ -f "$STATE_FILE" ]] || die "no state file at $STATE_FILE -- run 'setup' first"
  # shellcheck disable=SC1090
  source "$STATE_FILE"
  : "${API_ID:?}" "${PLAN_ID:?}" "${KEY_ID:?}" "${SETUP_UTC:?}"
}

endpoint_url() { printf 'https://%s.execute-api.%s.amazonaws.com/%s/' "$API_ID" "$REGION" "$STAGE"; }

# The key value is never written to disk -- read it at call time.
key_value() { aws_ apigateway get-api-key --api-key "$KEY_ID" --include-value --query 'value' --output text; }

hit() { # -> HTTP status code
  curl -s -o /dev/null -w '%{http_code}' --max-time 20 -H "x-api-key: $1" "$(endpoint_url)" || echo "000"
}

read_usage() { # -> "used,remaining" for today, or "?,?"
  local start end
  start="$(date -u -d 'yesterday' +%Y-%m-%d)"
  end="$(date -u -d 'tomorrow' +%Y-%m-%d)"
  aws_ apigateway get-usage --usage-plan-id "$PLAN_ID" --key-id "$KEY_ID" \
      --start-date "$start" --end-date "$end" \
      --query "items.\"$KEY_ID\"[-1]" --output text 2>/dev/null \
    | awk '{ if (NF>=2) printf "%s,%s", $1, $2; else printf "?,?" }'
}

cmd_setup() {
  require_auth
  [[ -f "$STATE_FILE" ]] && die "state file already exists at $STATE_FILE -- run 'teardown' first, or delete it to adopt existing resources"

  local api_name="$NAME_PREFIX-item7"
  local plan_name="$NAME_PREFIX-item7-plan"
  local key_name="$NAME_PREFIX-item7-key"
  assert_scratch "$api_name" "rest-api"
  assert_scratch "$plan_name" "usage-plan"
  assert_scratch "$key_name" "api-key"

  log "creating REST API $api_name"
  API_ID="$(aws_ apigateway create-rest-api --name "$api_name" \
      --description "task 0180 item 7 -- quota rollover measurement, safe to delete" \
      --query 'id' --output text)"

  local root_id
  root_id="$(aws_ apigateway get-resources --rest-api-id "$API_ID" --query 'items[0].id' --output text)"

  log "wiring MOCK integration on GET / (api $API_ID)"
  aws_ apigateway put-method --rest-api-id "$API_ID" --resource-id "$root_id" \
      --http-method GET --authorization-type NONE --api-key-required >/dev/null
  aws_ apigateway put-integration --rest-api-id "$API_ID" --resource-id "$root_id" \
      --http-method GET --type MOCK \
      --request-templates '{"application/json":"{\"statusCode\":200}"}' >/dev/null
  aws_ apigateway put-method-response --rest-api-id "$API_ID" --resource-id "$root_id" \
      --http-method GET --status-code 200 >/dev/null
  aws_ apigateway put-integration-response --rest-api-id "$API_ID" --resource-id "$root_id" \
      --http-method GET --status-code 200 \
      --response-templates '{"application/json":"{\"ok\":true}"}' >/dev/null

  log "deploying to stage $STAGE"
  aws_ apigateway create-deployment --rest-api-id "$API_ID" --stage-name "$STAGE" >/dev/null

  log "creating usage plan with a DAY quota of $QUOTA_LIMIT"
  PLAN_ID="$(aws_ apigateway create-usage-plan --name "$plan_name" \
      --description "task 0180 item 7 -- DAY period proxy for the MONTH rollover" \
      --api-stages "apiId=$API_ID,stage=$STAGE" \
      --quota "limit=$QUOTA_LIMIT,period=DAY" \
      --throttle "rateLimit=10,burstLimit=20" \
      --query 'id' --output text)"

  log "creating api key"
  KEY_ID="$(aws_ apigateway create-api-key --name "$key_name" --enabled \
      --description "task 0180 item 7" --query 'id' --output text)"
  aws_ apigateway create-usage-plan-key --usage-plan-id "$PLAN_ID" \
      --key-id "$KEY_ID" --key-type API_KEY >/dev/null

  SETUP_UTC="$(now_utc)"
  local key_created
  key_created="$(aws_ apigateway get-api-key --api-key "$KEY_ID" --query 'createdDate' --output text)"

  cat >"$STATE_FILE" <<EOF
API_ID=$API_ID
PLAN_ID=$PLAN_ID
KEY_ID=$KEY_ID
SETUP_UTC=$SETUP_UTC
KEY_CREATED=$key_created
QUOTA_LIMIT=$QUOTA_LIMIT
EOF

  log "state written to $STATE_FILE"
  log "setup completed at $SETUP_UTC -- this is the creation anchor 'analyse' tests against"
  # Item 8 measured ~25s for key enable/disable to reach the data plane; a new
  # key and deployment need at least as long before the first request is honest.
  log "waiting 45s for the key to reach the data plane"
  sleep 45
}

cmd_drain() {
  require_auth; load_state
  local kv status i
  kv="$(key_value)"
  log "draining quota of $QUOTA_LIMIT against $(endpoint_url)"
  for ((i = 1; i <= QUOTA_LIMIT + 2; i++)); do
    status="$(hit "$kv")"
    log "  drain request $i -> $status"
    [[ "$status" == "429" ]] && { log "quota exhausted after $i requests"; return 0; }
    sleep 2
  done
  die "quota never rejected after $((QUOTA_LIMIT + 2)) requests -- check the plan is attached to the stage"
}

cmd_poll() {
  require_auth; load_state
  local hours="${1:-$DEFAULT_HOURS}"
  local deadline
  deadline=$(( $(date -u +%s) + $(awk -v h="$hours" 'BEGIN{printf "%d", h*3600}') ))
  local kv n=0 status usage

  if [[ ! -s "$LOG_FILE" ]]; then
    {
      printf '# task 0180 item 7 -- quota rollover\n'
      printf '# api=%s plan=%s key=%s quota=%s/DAY region=%s\n' "$API_ID" "$PLAN_ID" "$KEY_ID" "$QUOTA_LIMIT" "$REGION"
      printf '# setup_utc=%s key_created=%s\n' "$SETUP_UTC" "${KEY_CREATED:-unknown}"
      printf '# poll_interval=%ss -- reset instant is resolved to +/- this\n' "$POLL_INTERVAL_SEC"
      printf 'utc\thttp\tused\tremaining\n'
    } >"$LOG_FILE"
  fi

  log "polling every ${POLL_INTERVAL_SEC}s for ${hours}h -> $LOG_FILE"
  kv="$(key_value)"
  while [[ $(date -u +%s) -lt $deadline ]]; do
    status="$(hit "$kv")"
    usage="-,-"
    if (( n % USAGE_EVERY_N_POLLS == 0 )); then usage="$(read_usage)"; fi
    printf '%s\t%s\t%s\t%s\n' "$(now_utc)" "$status" "${usage%%,*}" "${usage##*,}" >>"$LOG_FILE"
    n=$((n + 1))
    sleep "$POLL_INTERVAL_SEC"
  done
  log "poll window finished after ${hours}h"
  cmd_analyse
}

cmd_analyse() {
  [[ -s "$LOG_FILE" ]] || die "no log at $LOG_FILE"
  local setup_utc
  setup_utc="$(awk -F'setup_utc=' '/^# setup_utc=/ {print $2}' "$LOG_FILE" | awk '{print $1}')"

  echo
  echo "=== item 7 verdict ==="
  echo "log:        $LOG_FILE"
  echo "setup:      ${setup_utc:-unknown}  (creation anchor)"

  # The reset is the first success that follows at least one rejection.
  awk -F'\t' '
    /^[0-9]/ {
      if ($2 == "429") { seen429 = 1; last429 = $1 }
      else if ($2 == "200" && seen429 && !found) { found = 1; first200 = $1; prev429 = last429 }
    }
    END {
      if (!seen429) { print "no 429 seen -- the quota was never exhausted; nothing to measure"; exit }
      if (!found)   { print "still rejected at end of window -- no rollover observed yet"; exit }
      printf "last reject: %s\n", prev429
      printf "first serve: %s   <-- RESET INSTANT (bounded by the two lines above)\n", first200
    }' "$LOG_FILE"

  echo
  echo "Read it against two hypotheses:"
  echo "  calendar-aligned  -> reset lands on 00:00Z"
  echo "  creation-anchored -> reset lands on the setup time-of-day, 24h on"
  echo
  echo "Counter vs enforcement: if 'used/remaining' in the log still shows a drained"
  echo "pair after the first 200, GetUsage lagged the reset -- item 8 saw the same"
  echo "and it is worth recording separately."
  echo
  echo "NOTE: a DAY observation is evidence for MONTH, not proof. The real MONTH"
  echo "      rollover is 1 September 2026."
}

cmd_status() {
  require_auth; load_state
  echo "api:   $API_ID   ($(endpoint_url))"
  echo "plan:  $PLAN_ID"
  echo "key:   $KEY_ID"
  echo "setup: $SETUP_UTC"
  echo "quota: $(aws_ apigateway get-usage-plan --usage-plan-id "$PLAN_ID" --query 'quota' --output json)"
  echo "usage: $(read_usage)"
  [[ -s "$LOG_FILE" ]] && echo "log:   $(grep -c '^[0-9]' "$LOG_FILE") samples in $LOG_FILE"
}

cmd_teardown() {
  require_auth; load_state
  local plan_name
  plan_name="$(aws_ apigateway get-usage-plan --usage-plan-id "$PLAN_ID" --query 'name' --output text)"
  assert_scratch "$plan_name" "usage-plan"

  log "detaching and deleting key $KEY_ID"
  aws_ apigateway delete-usage-plan-key --usage-plan-id "$PLAN_ID" --key-id "$KEY_ID" >/dev/null 2>&1 || true
  aws_ apigateway delete-api-key --api-key "$KEY_ID" >/dev/null 2>&1 || true
  sleep 5
  log "deleting usage plan $PLAN_ID ($plan_name)"
  aws_ apigateway delete-usage-plan --usage-plan-id "$PLAN_ID" >/dev/null 2>&1 || true
  sleep 5
  log "deleting rest api $API_ID"
  aws_ apigateway delete-rest-api --rest-api-id "$API_ID" >/dev/null 2>&1 || true
  mv "$STATE_FILE" "$STATE_FILE.torn-down-$(date -u +%Y%m%dT%H%M%SZ)"
  log "teardown complete -- the log is kept, it is the result"
}

case "${1:-}" in
  setup)    cmd_setup ;;
  drain)    cmd_drain ;;
  poll)     shift; cmd_poll "${1:-$DEFAULT_HOURS}" ;;
  run)      cmd_setup; cmd_drain; cmd_poll "${2:-$DEFAULT_HOURS}" ;;
  analyse)  cmd_analyse ;;
  status)   cmd_status ;;
  teardown) cmd_teardown ;;
  *) sed -n '2,40p' "${BASH_SOURCE[0]}"; exit 1 ;;
esac
