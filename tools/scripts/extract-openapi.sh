#!/usr/bin/env bash
#
# Emit the OpenAPI document to target/openapi.json, stamped with the SAME
# `servers` URL the deployed API advertises.
#
# WHY THIS EXISTS
# ---------------
# Task 0124 exposes the spec at `GET /api-docs-json` through API Gateway. The
# lint gate (`npm run openapi:lint`) has to check the document a *reader* gets,
# not a variant of it — otherwise the thing CI blesses and the thing production
# serves are two different files, which is the exact drift 0124 was opened to
# fix.
#
# Two things follow from that:
#
#   1. `API_BASE_URL` is read from `infra/envs/production.json`, the single
#      place the deployed value is configured (it is passed to the api-handler
#      Lambda as `API_BASE_URL` by ComputeStack). Hardcoding it here would make
#      a fourth copy of a URL that already has one home.
#
#   2. `servers` must be present at all. An OpenAPI document with no `servers`
#      is a lint error, and rightly so — it tells a reader nothing about where
#      to send requests. Without the stamp, the extracted document has none.
#
# Override with API_BASE_URL=… to lint a different deployment's document.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
env_file="${repo_root}/infra/envs/production.json"
out="${repo_root}/target/openapi.json"

if [[ -z "${API_BASE_URL:-}" ]]; then
  [[ -f "$env_file" ]] || {
    echo "error: $env_file not found and API_BASE_URL is unset" >&2
    exit 1
  }
  API_BASE_URL="$(node -p "require('${env_file}').apiBaseUrl ?? ''")"
  # An empty value would silently produce a serverless document that fails the
  # lint several steps later with a much less obvious message.
  [[ -n "$API_BASE_URL" ]] || {
    echo "error: apiBaseUrl is missing from $env_file" >&2
    exit 1
  }
fi
export API_BASE_URL

mkdir -p "$(dirname "$out")"
cargo run -q -p prices-api --bin extract_openapi > "$out"
echo "$out (servers: $API_BASE_URL)"
