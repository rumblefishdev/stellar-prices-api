#!/usr/bin/env bash
#
# Verify every Lambda asset dir the CDK references maps to a crate that can
# actually produce `target/lambda/<name>/bootstrap`. Build-free: reads
# `cargo metadata`, compiles nothing.
#
# WHY THIS IS NOT A NAME-EXISTS CHECK
# -----------------------------------
# The obvious check — "does a crate with this name exist" — is too weak, and
# was wrong in three separate ways when it was a bare
# `grep -rqx 'name = "<x>"' --include=Cargo.toml packages/`:
#
#   1. grep is section-blind. `[[bin]] name`, `[lib] name` and `[package]`
#      name all match identically, so `enrichment-cli`, `prices-cli`, `serve`
#      and `prices-clickhouse-init` all passed despite none being a package
#      (`cargo lambda build -p enrichment-cli` errors: package not found).
#   2. A pure library passes. `extractors-core` has no `[[bin]]` at all, so
#      `target/lambda/extractors-core/bootstrap` can never exist, yet the
#      name is present in a Cargo.toml.
#   3. A package whose bin is named differently passes. `prices-clickhouse`
#      builds `prices-clickhouse-init`, which lands in the wrong asset dir.
#
# The property that actually matters is not "this crate exists" but "this
# name will produce target/lambda/<name>/bootstrap". That needs all three of:
#
#   - a PACKAGE by that name (so `cargo lambda build -p <name>` resolves),
#   - a BIN target of the same name (so the artifact lands in the asset dir
#     the CDK points at — cargo-lambda keys the output dir off the bin name),
#   - a `lambda` FEATURE on that package (each Lambda bin is gated behind
#     `required-features = ["lambda"]`; without the feature the bin is
#     silently skipped and the build "succeeds" producing nothing).
#
# Usage:  tools/scripts/verify-lambda-assets.sh [repo-root]

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$(cd "${here}/../.." && pwd)}"

# Asset dirs the CDK app references. Exits non-zero on an empty result, so a
# vacuous pass is not reachable through this path.
mapfile -t assets < <("${here}/lambda-assets.sh" "$root")

# Crates that can actually be built into a bootstrap, by the three-way rule
# above. `--no-deps` keeps this to the workspace and compiles nothing;
# `--offline` keeps it off the network (the committed Cargo.lock is enough).
mapfile -t eligible < <(
  cargo metadata --no-deps --offline --format-version 1 --manifest-path "${root}/Cargo.toml" \
    | jq -r '
        .packages[]
        | .name as $n
        | select(.features | has("lambda"))
        | select([.targets[] | select(.kind[] == "bin") | .name] | index($n))
        | $n
      ' \
    | sort -u
)

# The same tripwire lambda-assets.sh has: an empty eligible set would make
# every comparison below fail rather than pass, but a *silently* empty one
# means the metadata query broke (jq shape change, cargo error swallowed) and
# the operator deserves to be told which side is at fault.
if [[ ${#eligible[@]} -eq 0 ]]; then
  echo "verify-lambda-assets: no crate in the workspace declares a 'lambda' feature" >&2
  echo "  with a bin matching its package name. Either the workspace changed" >&2
  echo "  shape or the cargo-metadata query in this script is stale." >&2
  exit 1
fi

echo "=== Lambda assets referenced by the CDK app ==="
missing=0
checked=0
for name in "${assets[@]}"; do
  [[ -z "$name" ]] && continue
  checked=$((checked + 1))
  found=0
  for e in "${eligible[@]}"; do
    [[ "$e" == "$name" ]] && found=1 && break
  done
  if [[ $found -eq 1 ]]; then
    echo "ok: ${name}"
  else
    echo "::error::CDK references target/lambda/${name}, but no workspace package named '${name}' has both a bin of that name and a 'lambda' feature. It cannot produce target/lambda/${name}/bootstrap, so synth will fail with CannotFindAsset."
    missing=1
  fi
done

# A loop that checks nothing exits 0 and reports success. lambda-assets.sh
# already refuses to emit an empty list, but that guarantee does not survive
# being piped through a CI step boundary, so re-assert it where the counting
# actually happens.
if [[ $checked -eq 0 ]]; then
  echo "::error::verify-lambda-assets: checked 0 assets. The asset list reached this script empty; the guard would have passed vacuously." >&2
  exit 1
fi

echo "verified ${checked} Lambda asset(s) against ${#eligible[@]} eligible crate(s)"
exit $missing
