#!/usr/bin/env bash
#
# Print the canonical list of Lambda bootstrap assets the CDK app references,
# one crate name per line, sorted.
#
# WHY THIS EXISTS
# ---------------
# The CI Lambda build list, the CI verify list, and the CDK's `Code.fromAsset`
# paths used to be three hand-maintained copies of the same set. They drifted:
# task 0070 hit `CannotFindAsset` at deploy time because CI built 6 of the 9
# assets the production app references, and the "Verify Lambda artifacts" step
# checked only 5 of those 6 (`enrichment-worker` was built and never asserted).
#
# So this derives the list FROM the CDK source — the thing that actually
# decides which assets must exist. Add a Lambda to a stack and CI builds and
# verifies it automatically; forget the crate and the build fails loudly
# instead of the mistake surfacing during an operator's deploy.
#
# The CDK declares each asset dir as a string literal relative to `infra/`:
#
#     process.env['CLEANUP_WORKER_ASSET_DIR'] ?? '../target/lambda/cleanup-worker'
#
# The crate name, the bin name and the asset dir name are all identical for
# every Lambda crate here (each bin is gated behind the crate's `lambda`
# feature), so one name serves as `-p <crate>` and `target/lambda/<name>`.
#
# Usage:  tools/scripts/lambda-assets.sh [repo-root]

set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
src="${root}/infra/src"

if [[ ! -d "$src" ]]; then
  echo "lambda-assets: no such directory: $src" >&2
  exit 1
fi

# Match only the quoted literal form, so prose in comments that happens to
# mention a path cannot inject a name.
mapfile -t assets < <(
  grep -rhoE "'\.\./target/lambda/[a-z0-9-]+'" "$src" \
    | tr -d "'" \
    | sed 's#\.\./target/lambda/##' \
    | sort -u
)

# A refactor that changes how the asset dir is written (a template literal, a
# shared helper, a path built from a variable) would silently produce an empty
# list here — and an empty list makes every downstream check vacuously pass,
# which is precisely the failure this script exists to prevent. Fail instead.
if [[ ${#assets[@]} -eq 0 ]]; then
  echo "lambda-assets: found no '../target/lambda/<name>' literals under $src." >&2
  echo "  The CDK likely changed how asset dirs are declared. Update this" >&2
  echo "  script to match, or CI will stop guarding Lambda assets entirely." >&2
  exit 1
fi

printf '%s\n' "${assets[@]}"
