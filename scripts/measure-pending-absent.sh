#!/usr/bin/env bash
# Measure whether Discord's REST member object carries `pending` — task 0180
# item 2, 0189's risk R1, and the blocking question on PR #249.
#
# WHY THIS EXISTS. `portal/eligibility.rs` treats an absent `pending` as
# "could not verify" and refuses. That is the safe direction, but it is safe
# in a way that fails for EVERY visitor if Discord never sends the field —
# and to a visitor that looks exactly like a Discord outage. Since 2026-08-26
# the arm also gates SIGN-IN, not just key issuance, so an absent field would
# make the portal unusable rather than merely un-issuing.
#
# WHAT IT DOES. Restarts the local `serve` against the REAL Stellar guild,
# captures its log, waits for you to complete one sign-in in the browser, then
# reads the verdict out of the log. It changes no AWS resource and deploys
# nothing. It does NOT touch the Developer Portal — see PREREQUISITES.
#
# PREREQUISITES, both yours and neither checkable from here:
#
#   1. The Discord application (client_id in .portal-oauth.json) must request
#      the scope pair `identify guilds.members.read`. The code requests it;
#      the REGISTRATION must allow it. If it does not, Discord refuses at the
#      authorize step and this script reports `invalid_scope` rather than a
#      measurement.
#   2. You must sign in with a Discord account that IS a member of the guild
#      being measured. Measuring with an account that is not a member answers
#      "not a member", not the question.
#   3. Know which way Membership Screening is set on that guild before you
#      run — it is what the result means (see GUILD below).
#
# ⚠️ ONE REAL PRODUCTION KEY. Sign-in issues the first key, and
# PORTAL_FREE_PLAN_ID points at the real `pricing-api-free` plan, so a
# successful run creates `discord-<your-id>-key` in account 750702271865.
# The script prints the exact delete command at the end. It does not delete
# anything itself.
set -uo pipefail
cd "$(dirname "$0")/.."

# WHICH GUILD, AND WHY IT DECIDES WHAT YOU LEARN.
#
# Discord populates `pending` only for guilds with Membership Screening
# enabled, so the guild is a variable of the experiment, not a detail.
#
#   Default — `897514728459468821`, Stellar Developers: the official and only
#   Stellar guild (Adam, 2026-09-02, task 0254), the one production gates on,
#   and the one whose screening is MEASURED ON (`MEMBER_VERIFICATION_GATE_
#   ENABLED` in its invite metadata, 2026-09-02 — `npm run
#   discord:verify-guild` prints it). Needs an account that is a member of it;
#   for the `pending: true` arm, an account that joined and has NOT accepted
#   the rules.
#
#   GUILD=1536303837785362432 — the scratch guild (`stellar_test`). You own
#   it, so you can toggle Membership Screening in Server Settings → Members
#   and run this twice to see both arms of `eligibility.rs`'s `match
#   m.pending` on a server whose settings you control. ALREADY RUN,
#   2026-08-27: it answered `pending: false` (present); whether screening was
#   on or off at the time is UNCONFIRMED.
#
# The verdicts below name the landing the callback chose. Since task 0254
# `pending: true` lands on its own `pending_rules` rather than `not_member`.
GUILD="${GUILD:-897514728459468821}"
SECRET_FILE="${SECRET_FILE:-.portal-oauth.json}"
PORT="${PORT:-8080}"
LOG="${LOG:-/tmp/portal-pending-absent-$(date +%Y%m%dT%H%M%S).log}"
TIMEOUT_SECS="${TIMEOUT_SECS:-300}"

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }

[ -f "$SECRET_FILE" ] || { echo "no $SECRET_FILE — see packages/prices-api/README.md §2"; exit 1; }
python3 - "$SECRET_FILE" <<'PY' || exit 1
import json,sys
d=json.load(open(sys.argv[1]))
missing=[k for k in ("client_id","client_secret","redirect_uri","session_signing_key")
         if not d.get(k) or "REPLACE" in str(d[k])]
if missing: sys.exit(f"{sys.argv[1]}: unset or placeholder: {', '.join(missing)}")
if not d["redirect_uri"].startswith("http://localhost:4200/"):
    sys.exit(f"redirect_uri is {d['redirect_uri']} — this script drives the :4200 dev server")
print(f"secret file ok (client_id {d['client_id']}, redirect {d['redirect_uri']})")
PY

say "1/4  stopping any running serve"
OLD=$(pgrep -f 'target/debug/serve' || true)
[ -n "$OLD" ] && { kill $OLD; sleep 1; echo "stopped pid(s): $OLD"; } || echo "none running"

say "2/4  starting serve against guild $GUILD"
echo "log: $LOG"
PORTAL_ENABLED=true \
PORTAL_OAUTH_SECRET_FILE="$SECRET_FILE" \
PORTAL_GUILD_ID="$GUILD" \
PORTAL_MIN_ACCOUNT_AGE_MINUTES="${PORTAL_MIN_ACCOUNT_AGE_MINUTES:-5}" \
PORTAL_FREE_PLAN_ID="${PORTAL_FREE_PLAN_ID:-71t9im}" \
PORT="$PORT" RUST_LOG="${RUST_LOG:-info}" \
  cargo run -q -p prices-api --features local-server --bin serve >"$LOG" 2>&1 &
SERVE=$!
for _ in $(seq 1 60); do
  curl -sf -m 2 "http://localhost:$PORT/api/config" >/dev/null && break
  kill -0 $SERVE 2>/dev/null || { echo "serve died at start:"; tail -20 "$LOG"; exit 1; }
  sleep 1
done
curl -sf -m 2 "http://localhost:$PORT/api/config" >/dev/null || {
  echo "serve never answered /config:"; tail -20 "$LOG"; kill $SERVE; exit 1; }
echo "serve up on :$PORT (pid $SERVE)"

say "3/4  now sign in, once, in the browser"
cat <<EOF

    Make sure the portal dev server is running:  npx nx dev portal
    Then open:   http://localhost:4200/api/login
    Sign in with a Discord account that IS a member of guild $GUILD.

Waiting up to ${TIMEOUT_SECS}s for the callback to reach a verdict…
EOF

DEADLINE=$(( $(date +%s) + TIMEOUT_SECS ))
VERDICT=""
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  if   grep -q 'pending_absent'                        "$LOG"; then VERDICT=absent;  break
  elif grep -q 'invalid_scope'                         "$LOG"; then VERDICT=scope;   break
  elif grep -q 'Unknown Guild (10004)'                 "$LOG"; then VERDICT=guild;   break
  elif grep -q 'membership could not be verified'      "$LOG"; then VERDICT=discord; break
  elif grep -q 'outcome="not_member"\|outcome = "not_member"' "$LOG"; then VERDICT=notmember; break
  elif grep -q 'outcome="pending_rules"\|outcome = "pending_rules"' "$LOG"; then VERDICT=pending; break
  # The success path logs no "signed in" line of its own, so the positive
  # signal is anything that can only happen AFTER membership resolved to
  # `Member`: a key issued, the re-issue cap, or the age refusal.
  elif grep -q 'portal issued an API key\|outcome = "capped"\|outcome = "too_young"' "$LOG"; then VERDICT=present; break
  fi
  sleep 2
done

say "4/4  verdict"
case "$VERDICT" in
  present)
    echo "✅ PRESENT — membership resolved to Member, so the object carried"
    echo "   \`pending: false\`. The gate works as designed and 0189 R1 is closed."
    echo "   Record it in 0189's Step 0 table, dated $(date -u +%Y-%m-%d), with this line:"
    grep -n 'portal issued an API key\|outcome = "capped"\|outcome = "too_young"' "$LOG" | head -2
    ;;
  absent)
    echo "❌ ABSENT — Discord sent no \`pending\` field. EVERY member is refused."
    echo "   This is 0180 item 2, measured. The fix is the one arm in"
    echo "   packages/prices-api/src/portal/eligibility.rs (\`None => Membership::Unknown\`),"
    echo "   in its own commit. Do NOT open the portal (0194) before it lands."
    grep -n 'pending_absent' "$LOG" | head -3
    ;;
  scope)
    echo "⚠️  NOT A MEASUREMENT — Discord refused at the authorize step (invalid_scope)."
    echo "   The registration does not allow \`guilds.members.read\`. Add it in the"
    echo "   Developer Portal (runbook §1 step 3) and run this again."
    ;;
  guild)
    echo "⚠️  NOT A MEASUREMENT — Discord answered Unknown Guild (10004) for $GUILD."
    echo "   Wrong snowflake, or the app cannot see that guild."
    ;;
  discord)
    echo "⚠️  NOT A MEASUREMENT — the membership call failed (rate limit or 5xx)."
    grep -n 'membership could not be verified' "$LOG" | head -3
    echo "   Try again in a few minutes."
    ;;
  notmember)
    echo "⚠️  NOT A MEASUREMENT — that account is not a member of guild $GUILD."
    echo "   Join it, or sign in with an account that is."
    ;;
  pending)
    echo "✅ PRESENT, and TRUE — the object carried \`pending: true\`: on the server,"
    echo "   rules not accepted. This is task 0254's arm, observed on the REST route."
    echo "   Record it in 0254's step 0 with this line, then accept the rules on"
    echo "   Discord and run again for the \`false\` arm:"
    grep -n 'outcome="pending_rules"\|outcome = "pending_rules"' "$LOG" | head -2
    ;;
  *)
    echo "⏳ nothing conclusive within ${TIMEOUT_SECS}s. Read the log yourself:"
    echo "   $LOG"
    ;;
esac

say "clean-up"
echo "serve is still running as pid $SERVE (log: $LOG). Stop it with:  kill $SERVE"
cat <<'EOF'

If a key was issued, it is REAL. List and delete:

  AWS_REGION=eu-central-1 aws apigateway get-api-keys \
    --query 'items[?tags.ManagedBy==`prices-portal`].[id,name,createdDate]' --output table
  AWS_REGION=eu-central-1 aws apigateway delete-api-key --api-key <id>

EOF
