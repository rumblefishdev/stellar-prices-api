#!/usr/bin/env bash
#
# Stand a reverse proxy in front of the local ClickHouse, so tests can exercise
# the PRODUCTION shape (task 0281).
#
# WHY THIS EXISTS
# ---------------
# `execution_bound_error_it` asserts that a failed statement reaches the caller
# carrying ClickHouse's own message. Straight to ClickHouse it always does —
# even with the defect present — because the crate's LZ4 fallback fires. Put a
# proxy in the path and the decode succeeds with zero bytes instead, the
# fallback never runs, and the message becomes "". Production always has Caddy
# in front of ClickHouse, so without a proxy the test is vacuous.
#
#   scripts/ch-proxy-0281.sh up
#   CLICKHOUSE_PROXY_URL=http://localhost:8124 \
#     cargo test -p prices-clickhouse --test execution_bound_error_it -- --ignored
#   scripts/ch-proxy-0281.sh down
set -euo pipefail

name=ch-proxy-0281
conf="$(mktemp -d)/Caddyfile"

case "${1:-up}" in
  up)
    # Mirrors the transport block BE runs in front of ch-prod-01.
    cat > "$conf" <<'EOF'
{
	admin off
	auto_https off
}

:8124 {
	reverse_proxy localhost:8123 {
		transport http {
			dial_timeout 10s
			response_header_timeout 7200s
			read_timeout 7200s
			write_timeout 7200s
		}
	}
}
EOF
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker run -d --name "$name" --network host \
      -v "$conf":/etc/caddy/Caddyfile:ro caddy:2 >/dev/null
    until curl -sS --max-time 2 "http://localhost:8124/?query=SELECT%201" >/dev/null 2>&1; do
      sleep 1
    done
    echo "proxy up: http://localhost:8124 -> clickhouse :8123"
    ;;
  down)
    docker rm -f "$name" >/dev/null 2>&1 || true
    echo "proxy down"
    ;;
  *)
    echo "usage: $0 [up|down]" >&2
    exit 1
    ;;
esac
