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
#
# Extraction is deliberately PERMISSIVE here — anything up to the closing
# quote — and the name is validated separately below. Extracting with a
# narrow charset instead would make a name the pattern cannot express (an
# underscore, an interpolated `${...}` fragment, a nested path) simply vanish
# from the list. The list would stay non-empty, so the empty-list tripwire
# below would not fire, and CI would guard 8 of 9 assets while reporting
# success. Dropping a name silently is the failure this script exists to
# prevent, so every literal must be accounted for, even the unusable ones.
#
# grep's exit status is captured rather than read through a pipeline: `set -o
# pipefail` does not reach into process substitution, so a genuine grep
# failure (unreadable file, argument-list overflow, a locale error on -E)
# would otherwise be indistinguishable from "no matches" and get reported as
# a CDK refactor that never happened.
raw=""
grep_status=0
raw="$(grep -rhoE "'\.\./target/lambda/[^']*'" "$src")" || grep_status=$?

# grep: 0 = matched, 1 = no match, >1 = actual error.
if [[ $grep_status -gt 1 ]]; then
  echo "lambda-assets: grep failed with status ${grep_status} while scanning $src." >&2
  echo "  This is a tooling/environment failure, not a missing-asset finding." >&2
  exit 1
fi

mapfile -t literals < <(
  printf '%s' "$raw" \
    | tr -d "'" \
    | sed 's#\.\./target/lambda/##' \
    | sed '/^$/d' \
    | sort -u
)

# A refactor that changes how the asset dir is written (a template literal, a
# shared helper, a path built from a variable) would produce an empty list
# here — and an empty list makes every downstream check vacuously pass,
# which is precisely the failure this script exists to prevent. Fail instead.
if [[ ${#literals[@]} -eq 0 ]]; then
  echo "lambda-assets: found no '../target/lambda/<name>' literals under $src." >&2
  echo "  The CDK likely changed how asset dirs are declared. Update this" >&2
  echo "  script to match, or CI will stop guarding Lambda assets entirely." >&2
  exit 1
fi

# Every literal must be a name a downstream consumer can act on: `cargo lambda
# build -p <name>` and a `target/lambda/<name>/bootstrap` path. Cargo permits
# letters, digits, `-` and `_`. Anything else means the CDK is no longer
# declaring a plain literal, and the honest response is to fail and say which
# one rather than quietly shorten the list.
unusable=()
for name in "${literals[@]}"; do
  [[ "$name" =~ ^[A-Za-z0-9_-]+$ ]] || unusable+=("$name")
done

if [[ ${#unusable[@]} -gt 0 ]]; then
  echo "lambda-assets: ${#unusable[@]} asset literal(s) under $src are not usable crate names:" >&2
  printf '  %s\n' "${unusable[@]}" >&2
  echo "  A name must match ^[A-Za-z0-9_-]+\$ to be used as 'cargo lambda build -p <name>'" >&2
  echo "  and as a target/lambda/<name>/bootstrap path. If the CDK now builds asset" >&2
  echo "  dirs dynamically, update this script — do not let the name be skipped." >&2
  exit 1
fi

printf '%s\n' "${literals[@]}"
