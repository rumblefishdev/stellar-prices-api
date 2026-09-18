#!/usr/bin/env bash
#
# Build every Lambda bootstrap the CDK app packages, then verify what the build
# left under `target/lambda/`. The ONE invocation, shared by CI and by every
# `make deploy-production*` target that can ship a Lambda.
#
# WHY THE DEPLOY BUILDS INSTEAD OF CHECKING (task 0141)
# -----------------------------------------------------
# `make deploy-production-compute` used to compile the CDK TypeScript only and
# zip whatever was already in `target/lambda/`. On 2026-08-03 that shipped a
# stale `prices-api` to production behind a clean-looking diff and a green
# deploy. The fix is not a staleness detector: "is this binary older than its
# inputs?" needs the input set, and the input set is not the crate's directory
# (path dependencies, `Cargo.lock`, `include_str!` reaching into
# `docs/runbooks/`). Cargo already knows the input set exactly. So ask cargo:
# building is a no-op when the artifacts are current and the correct repair
# when they are not.
#
# Each Lambda bin is gated behind its crate's `lambda` feature
# (`required-features = ["lambda"]`), so `--features lambda` and an explicit
# `-p` per crate are BOTH required — a bare `cargo lambda build` silently skips
# these bins. The list comes from `lambda-assets.sh`, which derives it from the
# CDK source, so it cannot drift from what the app references (task 0077).
#
# On an x86 workstation this is a cross-compile (cargo-lambda drives zig); CI
# runs it on a native ARM runner. Use the toolchain pair CI pins (see the
# `rust` job in ci.yml): rustc >= 1.98 fails every aarch64 link under zig with
# "unsupported linker arg". That failure is loud — it cannot ship anything.
#
# Usage:  tools/scripts/build-lambda-assets.sh [repo-root]

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$(cd "${here}/../.." && pwd)}"

# The verifier below and `Code.fromAsset` both read `<root>/target/lambda/`.
# With `CARGO_TARGET_DIR` or a `build.target-dir` in some cargo config, cargo
# writes the fresh bootstraps somewhere else, and the weeks-old ones still
# sitting under `<root>/target/` — perfectly good aarch64 ELFs — would be
# verified and shipped. "Cargo is the freshness check" only holds if the files
# checked are the files cargo just wrote, so refuse before building anything.
target_dir="$(cargo metadata --no-deps --offline --format-version 1 --manifest-path "${root}/Cargo.toml" | jq -r '.target_directory')"
if [[ "$(realpath -m "$target_dir")" != "$(realpath -m "${root}/target")" ]]; then
  echo "::error::cargo's target dir is '${target_dir}', but CDK packages '${root}/target/lambda/'. Unset CARGO_TARGET_DIR / build.target-dir for this build." >&2
  exit 1
fi

args=()
while IFS= read -r name; do
  [[ -z "$name" ]] && continue
  args+=(-p "$name")
done < <("${here}/lambda-assets.sh" "$root")

if [[ ${#args[@]} -eq 0 ]]; then
  echo "::error::resolved zero Lambda assets; refusing to run a no-op build" >&2
  exit 1
fi

# What is about to be built, in the operator's scrollback next to the deploy.
# A dirty tree is reported, not refused: whether to ship one is the operator's
# call, but it must not be possible to do it without having been told.
provenance=""
if git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
  provenance="$(git -C "$root" rev-parse --abbrev-ref HEAD) @ $(git -C "$root" describe --always --dirty)"
  echo "building Lambdas from ${provenance}"
fi

echo "cargo lambda build --release --arm64 --features lambda ${args[*]}"
(cd "$root" && cargo lambda build --release --arm64 --features lambda "${args[@]}")

"${here}/verify-lambda-bootstraps.sh" "$root"

# Again, because the first copy is minutes of cargo output up the scrollback and
# this one sits directly above `cdk deploy`.
if [[ -n "$provenance" ]]; then
  echo "Lambdas built from ${provenance}"
fi
