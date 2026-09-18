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
# The test is ARMED in CI, not excluded (task 0275): the rust job runs
# `scripts/ch-proxy-0281.sh up` and then tools/scripts/ignored-tests.sh with
# CLICKHOUSE_PROXY_URL=http://localhost:8124.
#
# ONE Caddyfile serves every environment. Its upstream and listen port are
# Caddy parse-time placeholders with the CI values as defaults:
#
#   CH_UPSTREAM    (default localhost:8123)  the ClickHouse HTTP endpoint
#   CH_PROXY_PORT  (default 8124)            the port the proxy listens on
#
# CI / docker (`--network host`, ClickHouse from docker-compose.yml on :8123):
#
#   scripts/ch-proxy-0281.sh up
#   CLICKHOUSE_PROXY_URL=http://localhost:8124 tools/scripts/ignored-tests.sh run
#   scripts/ch-proxy-0281.sh down
#
# Workstation without docker, ClickHouse on another port (a static `caddy`
# binary from the GitHub release is enough — no root):
#
#   scripts/ch-proxy-0281.sh caddyfile > /tmp/Caddyfile
#   CH_UPSTREAM=localhost:18123 CH_PROXY_PORT=18124 \
#     caddy run --config /tmp/Caddyfile --adapter caddyfile
#
# Subcommands: up | down | caddyfile (print the Caddyfile to stdout).
set -euo pipefail

name=ch-proxy-0281
# Pinned: the floating `caddy:2` tag was re-pushed on 2026-09-18. Its arm64
# digest was identical to 2.11.4's that day, so the pin changed nothing but
# the ability of the proxy under an armed test to change without a commit.
image=caddy:2.11.4
port="${CH_PROXY_PORT:-8124}"
# ~60 s: in CI an unbounded wait would burn the whole job timeout on a proxy
# that never starts, instead of failing with its log.
max_attempts=60

caddyfile() {
  # Mirrors the transport block BE runs in front of ch-prod-01.
  cat <<'EOF'
{
	admin off
	auto_https off
}

:{$CH_PROXY_PORT:8124} {
	reverse_proxy {$CH_UPSTREAM:localhost:8123} {
		transport http {
			dial_timeout 10s
			response_header_timeout 7200s
			read_timeout 7200s
			write_timeout 7200s
		}
	}
}
EOF
}

case "${1:-up}" in
  up)
    conf_dir="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/ch-proxy-0281.XXXXXX")"
    conf="${conf_dir}/Caddyfile"
    caddyfile >"$conf"
    # The caddy image runs as root, but a 0700 dir owned by another uid is
    # still a surprise worth avoiding on a shared daemon.
    chmod 755 "$conf_dir"
    echo "Caddyfile: ${conf}"
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker run -d --name "$name" --network host \
      -e CH_UPSTREAM -e CH_PROXY_PORT \
      -v "$conf":/etc/caddy/Caddyfile:ro "$image" >/dev/null
    attempt=0
    until curl -sS --max-time 2 "http://localhost:${port}/?query=SELECT%201" >/dev/null 2>&1; do
      attempt=$((attempt + 1))
      if [[ $attempt -ge $max_attempts ]]; then
        echo "ch-proxy-0281: proxy on :${port} did not answer SELECT 1 after ${max_attempts} attempts." >&2
        echo "  Caddyfile: ${conf}" >&2
        echo "  --- docker logs ${name} ---" >&2
        docker logs "$name" >&2 || true
        exit 1
      fi
      sleep 1
    done
    echo "proxy up: http://localhost:${port} -> clickhouse ${CH_UPSTREAM:-localhost:8123} (${image})"
    ;;
  down)
    docker rm -f "$name" >/dev/null 2>&1 || true
    echo "proxy down"
    ;;
  caddyfile)
    caddyfile
    ;;
  *)
    echo "usage: $0 [up|down|caddyfile]" >&2
    exit 1
    ;;
esac
