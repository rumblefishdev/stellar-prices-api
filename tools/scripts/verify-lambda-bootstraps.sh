#!/usr/bin/env bash
#
# Verify that every Lambda asset the CDK app references is sitting under
# `target/lambda/<name>/bootstrap` as something Lambda can actually run: an
# executable aarch64 ELF, and not the same file as any other asset.
#
# WHY "IT EXISTS" IS NOT ENOUGH (task 0141)
# -----------------------------------------
# `Code.fromAsset` zips whatever is in the directory. On 2026-08-12 all eleven
# bootstraps on a developer machine were byte-identical 10-byte files holding
# `#!/bin/sh`; they existed, they were executable, `cdk diff` showed a clean
# S3Key change, and the deploy would have replaced every Lambda in production
# with a shell stub. So this checks what the file IS:
#
#   - ELF magic, so a stub or a text placeholder is refused;
#   - `e_machine` = aarch64, so a host build (`cargo build` without
#     `cargo lambda … --arm64`) is refused — every function here is ARM_64;
#   - no two assets share a sha256, so one binary copied over the rest is
#     refused even though each copy is a perfectly good ELF.
#
# It also refuses to run while any `*_ASSET_DIR` override the CDK reads is set.
# Each asset dir is `process.env['X_ASSET_DIR'] ?? '../target/lambda/x'`; with
# the variable exported, `Code.fromAsset` zips THAT directory, and a "verified"
# line about `target/lambda/` would be a statement about files nobody ships.
#
# This says nothing about whether the binary matches the tree. That is
# `build-lambda-assets.sh`'s job, and it does it by building: cargo is the
# freshness check. This runs after it, on what the build left behind.
#
# Usage:  tools/scripts/verify-lambda-bootstraps.sh [repo-root]

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$(cd "${here}/../.." && pwd)}"

# The names the CDK reads, from the CDK source, like the asset list itself.
overridden=0
while IFS= read -r var; do
  [[ -z "$var" ]] && continue
  # `??` falls through on undefined only, so an EMPTY variable still overrides.
  if [[ -v "$var" ]]; then
    echo "::error::${var} is set ('${!var}'): CDK would package that directory, not target/lambda/. Unset it and deploy what this script builds and checks." >&2
    overridden=1
  fi
done < <(grep -rhoE "process\.env\['[A-Z0-9_]+_ASSET_DIR'\]" "${root}/infra/src" | grep -oE "[A-Z0-9_]+_ASSET_DIR" | sort -u)
if [[ $overridden -ne 0 ]]; then
  exit 1
fi

# A process substitution's exit status is DISCARDED, even under `set -e`, so
# lambda-assets.sh refusing an empty list does not stop this script. What stops
# a vacuous pass is the `checked -eq 0` test after the loop — keep it.
mapfile -t assets < <("${here}/lambda-assets.sh" "$root")

# Bytes [offset, offset+count) of a file as lowercase hex, no separators.
hex_at() {
  od -An -v -tx1 -j "$2" -N "$3" "$1" | tr -d ' \n'
}

echo "=== Lambda bootstrap binaries ==="
failed=0
checked=0
declare -A seen=()
for name in "${assets[@]}"; do
  [[ -z "$name" ]] && continue
  checked=$((checked + 1))
  bin="${root}/target/lambda/${name}/bootstrap"

  if [[ ! -f "$bin" ]]; then
    echo "::error::${name}: missing Lambda bootstrap: ${bin}" >&2
    failed=1
    continue
  fi
  if [[ ! -x "$bin" ]]; then
    echo "::error::${name}: bootstrap is not executable: ${bin}" >&2
    failed=1
    continue
  fi
  if [[ "$(hex_at "$bin" 0 4)" != "7f454c46" ]]; then
    echo "::error::${name}: bootstrap is not an ELF binary ($(stat --format='%s' "$bin") bytes): ${bin}" >&2
    failed=1
    continue
  fi
  # e_machine, little-endian at offset 18: 0x00b7 is EM_AARCH64.
  if [[ "$(hex_at "$bin" 18 2)" != "b700" ]]; then
    echo "::error::${name}: bootstrap is an ELF but not aarch64 — built without 'cargo lambda build --arm64'?: ${bin}" >&2
    failed=1
    continue
  fi

  sum="$(sha256sum "$bin" | cut -d' ' -f1)"
  if [[ -n "${seen[$sum]:-}" ]]; then
    echo "::error::${name}: bootstrap is byte-identical to ${seen[$sum]}'s — two Lambdas cannot share one binary" >&2
    failed=1
    continue
  fi
  seen[$sum]="$name"
  echo "${sum}  ${name}  $(stat --format='%s bytes' "$bin")"
done

# A loop over an empty list exits 0 having asserted nothing, which reads as
# success. This is the ONLY thing between an empty asset list and a pass (see
# the process-substitution note above).
if [[ $checked -eq 0 ]]; then
  echo "::error::verified 0 Lambda bootstraps; the guard passed vacuously" >&2
  exit 1
fi

if [[ $failed -ne 0 ]]; then
  echo "verify-lambda-bootstraps: refusing — see the errors above. Rebuild with tools/scripts/build-lambda-assets.sh" >&2
  exit 1
fi

echo "verified ${checked} Lambda bootstrap(s)"
