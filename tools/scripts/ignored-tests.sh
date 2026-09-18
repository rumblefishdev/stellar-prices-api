#!/usr/bin/env bash
#
# The derived inventory of `#[ignore]`d integration tests, and the one command
# that runs the ones CI must run (task 0275).
#
# WHY THIS EXISTS
# ---------------
# Until task 0275 no ClickHouse integration test ran in CI. 229 of them sat
# behind `#[ignore]`, a guard that could not fail the build. Task 0215 asked
# for a test "so a silently-narrowed pivot set fails the suite"; the test was
# written, and the suite could not fail. A guard that cannot fail the build is
# documentation.
#
# So the set CI runs is DERIVED from the source, never hand-listed — the same
# reasoning as lambda-assets.sh (task 0077, where three hand-kept copies of one
# list drifted twice). Add an ignored ClickHouse test and CI runs it; rename a
# target and CI goes red by name.
#
# THE CLOSED VOCABULARY
# ---------------------
# Every `#[ignore]` under packages/*/tests/*_it.rs carries a reason that starts
# with one of three class prefixes:
#
#   requires ClickHouse      (CH)   run by CI on every Rust PR, by this script
#   requires public network  (NET)  recorded, never run by CI
#   requires production      (PROD) recorded, never run by CI
#
# A bare `#[ignore]` or any other reason is unclassifiable: CI cannot know
# whether to run it, so it FAILS the build instead of being quietly skipped —
# that quiet skip is exactly how 229 tests went unrun. An `#[ignore]` anywhere
# else under packages/ fails too, because only `*_it.rs` targets are derived.
#
# NET and PROD are recorded but never run: third-party uptime (Soroban RPC,
# Horizon, Reflector) and production state must not gate a PR — a developer
# can fix neither (Adam, 2026-09-18).
#
# One target holds one class (D2). `cargo test --test X -- --ignored` runs
# every ignored test in the binary, so a mixed file would arm its network test
# alongside its ClickHouse test. For the same reason a CH target may not share
# its NAME with any other `_it` target: `--test NAME` selects every target of
# that name across the workspace.
#
# Usage:  tools/scripts/ignored-tests.sh <subcommand> [repo-root]
#
#   check      classify every #[ignore]; exit non-zero on any violation
#   targets    one `-p <package> --test <target>` line per CH target, sorted
#   expect     CH_TESTS= CH_TARGETS= NET_TESTS= PROD_TESTS=, derived
#   image-tag  the ClickHouse version pinned in docker-compose.yml
#
# `repo-root` defaults to this checkout; the guard's own tests
# (ignored-tests.test.mjs) point it at throwaway fixture trees.

set -euo pipefail
# Command substitutions must fail the script too, not just return empty.
shopt -s inherit_errexit

CH_PREFIX='requires ClickHouse'
NET_PREFIX='requires public network'
PROD_PREFIX='requires production'

default_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

die() {
  echo "ignored-tests: $*" >&2
  exit 1
}

usage() {
  echo "usage: $0 {check|targets|expect|image-tag} [repo-root]" >&2
  exit 2
}

# derive ROOT — prints one record per `_it.rs` target that carries at least one
# `#[ignore]`:  <class> TAB <relative path> TAB <number of ignored tests>
# and fails, listing every violation, if the tree breaks a rule above.
derive() {
  local root="$1"
  [[ -d "${root}/packages" ]] || die "no such directory: ${root}/packages"

  local it_files=()
  local f
  shopt -s nullglob
  for f in "${root}"/packages/*/tests/*_it.rs; do
    it_files+=("${f#"${root}/"}")
  done
  shopt -u nullglob

  # Anchored, so `//!` prose that mentions the attribute cannot inject a match.
  # grep's status is captured rather than piped, as in lambda-assets.sh: a
  # genuine grep failure must not read as "no ignored tests".
  local raw="" grep_status=0
  raw="$(grep -rnE --include='*.rs' --exclude-dir=target \
    '^[[:space:]]*#\[ignore' "${root}/packages")" || grep_status=$?
  if [[ $grep_status -gt 1 ]]; then
    die "grep failed with status ${grep_status} while scanning ${root}/packages — a tooling failure, not a finding."
  fi

  local out
  out="$(
    {
      for f in "${it_files[@]}"; do printf 'F\t%s\n' "$f"; done
      if [[ -n "$raw" ]]; then
        printf '%s\n' "$raw" | sed "s#^${root}/##" | sort -t: -k1,1 -k2,2n | sed 's/^/G\t/'
      fi
    } | awk -v CH="$CH_PREFIX" -v NET="$NET_PREFIX" -v PROD="$PROD_PREFIX" '
      function has_prefix(s, p) {
        return index(s, p) == 1 && \
          (length(s) == length(p) || substr(s, length(p) + 1, 1) !~ /[A-Za-z0-9]/)
      }
      function base(p) { sub(/^.*\//, "", p); sub(/\.rs$/, "", p); return p }
      BEGIN { FS = "\t"; nerr = 0 }
      $1 == "F" { files[$2] = 1; order[++nfiles] = $2; next }
      $1 == "G" {
        s = substr($0, 3)
        i = index(s, ":"); rel = substr(s, 1, i - 1); s = substr(s, i + 1)
        i = index(s, ":"); ln = substr(s, 1, i - 1); txt = substr(s, i + 1)
        if (!(rel in files)) {
          err[++nerr] = rel ":" ln ": #[ignore] outside packages/*/tests/*_it.rs — only those targets are derived, so this test would never run anywhere"
          next
        }
        sub(/^[[:space:]]+/, "", txt); sub(/[[:space:]]+$/, "", txt)
        if (txt == "#[ignore]") {
          err[++nerr] = rel ":" ln ": bare #[ignore] — give it a reason starting with \"" CH "\", \"" NET "\" or \"" PROD "\""
          next
        }
        if (txt !~ /^#\[ignore = "[^"]*"\]$/) {
          err[++nerr] = rel ":" ln ": unparseable #[ignore] attribute: " txt
          next
        }
        reason = txt; sub(/^#\[ignore = "/, "", reason); sub(/"\]$/, "", reason)
        if (has_prefix(reason, CH)) cls = "CH"
        else if (has_prefix(reason, NET)) cls = "NET"
        else if (has_prefix(reason, PROD)) cls = "PROD"
        else {
          err[++nerr] = rel ":" ln ": unknown #[ignore] reason \"" reason "\" — it must start with \"" CH "\", \"" NET "\" or \"" PROD "\""
          next
        }
        if (!((rel, cls) in cnt)) { classes[rel] = classes[rel] (classes[rel] == "" ? "" : " ") cls }
        cnt[rel, cls]++
        next
      }
      END {
        nch = 0
        for (k = 1; k <= nfiles; k++) {
          rel = order[k]
          if (!(rel in classes)) continue
          n = split(classes[rel], cs, " ")
          if (n > 1) {
            d = ""
            for (j = 1; j <= n; j++) d = d (j > 1 ? ", " : "") cs[j] " (" cnt[rel, cs[j]] ")"
            err[++nerr] = rel ": mixes classes " d " — one class per target; move the non-ClickHouse tests into their own _it.rs"
            continue
          }
          cls = cs[1]
          if (cls == "CH") { nch++; chname[base(rel)] = chname[base(rel)] " " rel }
          rec[++nrec] = cls "\t" rel "\t" cnt[rel, cls]
        }
        # A CH target may not share its name with any non-CH `_it` target:
        # `cargo test --workspace --test NAME` selects both.
        for (k = 1; k <= nfiles; k++) {
          rel = order[k]; b = base(rel)
          if (!(b in chname)) continue
          if (rel in classes && classes[rel] == "CH") continue
          what = (rel in classes) ? classes[rel] : "no #[ignore]"
          err[++nerr] = rel ": name collision — target \"" b "\" (" what ") shares its name with ClickHouse target(s)" chname[b] "; `--test " b "` would select both. Rename one."
        }
        if (nerr == 0 && nch == 0) {
          err[++nerr] = "found no ClickHouse-class #[ignore] under packages/*/tests/*_it.rs (" nfiles " _it.rs file(s) scanned). An empty inventory makes every downstream check pass vacuously — refusing."
        }
        if (nerr > 0) {
          for (k = 1; k <= nerr; k++) print "E\t" err[k]
          exit
        }
        for (k = 1; k <= nrec; k++) print "R\t" rec[k]
      }
    '
  )"

  if grep -q '^E' <<<"$out"; then
    local n
    n="$(grep -c '^E' <<<"$out")"
    echo "ignored-tests: ${n} violation(s) of the #[ignore] inventory rules:" >&2
    grep '^E' <<<"$out" | cut -f2- | sed 's/^/  /' >&2
    exit 1
  fi
  grep '^R' <<<"$out" | cut -f2-
}

# package_name ROOT CRATE_DIR — the `[package] name` from the crate's
# Cargo.toml. Never the directory name: the two differ in general.
package_name() {
  local manifest="${1}/${2}/Cargo.toml" name
  [[ -f "$manifest" ]] || die "no Cargo.toml for ${2} (expected ${manifest})"
  name="$(awk '
    /^[[:space:]]*\[/ { sec = $0; gsub(/[[:space:]]/, "", sec); next }
    sec == "[package]" && /^[[:space:]]*name[[:space:]]*=/ {
      v = $0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/".*$/, "", v); print v; exit
    }' "$manifest")"
  [[ "$name" =~ ^[A-Za-z0-9_-]+$ ]] || die "could not read a usable [package] name from ${manifest} (got \"${name}\")"
  printf '%s\n' "$name"
}

cmd_targets() {
  local root="$1" recs cls rel n dir lines=()
  recs="$(derive "$root")"
  while IFS=$'\t' read -r cls rel n; do
    [[ "$cls" == CH ]] || continue
    dir="${rel%/tests/*}"
    lines+=("-p $(package_name "$root" "$dir") --test $(basename "$rel" .rs)")
  done <<<"$recs"
  printf '%s\n' "${lines[@]}" | sort
}

cmd_expect() {
  local root="$1" recs cls rel n
  local ch_tests=0 ch_targets=0 net_tests=0 prod_tests=0
  recs="$(derive "$root")"
  while IFS=$'\t' read -r cls rel n; do
    case "$cls" in
      CH) ch_tests=$((ch_tests + n)); ch_targets=$((ch_targets + 1)) ;;
      NET) net_tests=$((net_tests + n)) ;;
      PROD) prod_tests=$((prod_tests + n)) ;;
    esac
  done <<<"$recs"
  printf 'CH_TESTS=%s\nCH_TARGETS=%s\nNET_TESTS=%s\nPROD_TESTS=%s\n' \
    "$ch_tests" "$ch_targets" "$net_tests" "$prod_tests"
}

# The ONE ClickHouse pin (docker-compose.yml). CI and the workstation both
# assert the server they test against is this version, so a pin bump and the
# server it describes cannot drift apart unnoticed.
cmd_image_tag() {
  local compose="${1}/docker-compose.yml" tags
  [[ -f "$compose" ]] || die "no such file: ${compose}"
  tags="$(sed -nE 's#^[[:space:]]*image:[[:space:]]*["'\'']?clickhouse/clickhouse-server:([^"'\''[:space:]]+).*$#\1#p' "$compose")"
  if [[ -z "$tags" ]]; then
    die "no 'image: clickhouse/clickhouse-server:<version>' line in ${compose} — the pin CI asserts against is gone or renamed."
  fi
  if [[ "$(wc -l <<<"$tags")" -ne 1 ]]; then
    die "more than one clickhouse/clickhouse-server image in ${compose}: $(tr '\n' ' ' <<<"$tags")— there must be one pin."
  fi
  [[ "$tags" =~ ^[0-9]+(\.[0-9]+)+$ ]] ||
    die "clickhouse/clickhouse-server tag \"${tags}\" in ${compose} is not an exact version — pin the version production runs."
  printf '%s\n' "$tags"
}

sub="${1:-}"
[[ -n "$sub" ]] || usage
root="${2:-$default_root}"

case "$sub" in
  check)
    counts="$(cmd_expect "$root")"
    eval "$counts"
    echo "ignored-tests: inventory OK — ${CH_TESTS} ClickHouse test(s) in ${CH_TARGETS} target(s); ${NET_TESTS} network and ${PROD_TESTS} production test(s) recorded, not run."
    ;;
  targets) cmd_targets "$root" ;;
  expect) cmd_expect "$root" ;;
  image-tag) cmd_image_tag "$root" ;;
  *) usage ;;
esac
