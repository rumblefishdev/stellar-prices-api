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
# HOW THE RUN IS COUNTED (D4)
# ---------------------------
# `run` is ONE cargo invocation over the derived target names, then two
# assertions on its log: the summed `passed` equals the number of CH-class
# `#[ignore]` attributes, AND the number of `test result:` lines equals the
# number of CH targets. The second catches a binary killed by a signal or the
# OOM killer, which prints no summary at all and would otherwise read as
# inventory drift. Any `failed` is red. A renamed or deleted target is a hard
# named cargo error before any test runs.
#
# `--workspace --test NAME`, not `-p PKG --test NAME` pairs: cargo treats `-p`
# and `--test` as a cross product, not pairs, and `--workspace --test seed_it`
# selects both crates' `seed_it` (proved by the CH_TARGETS line count). Never
# `--test '*_it'`: that glob arms the NET/PROD targets too.
#
# --test-threads=1 IS DELIBERATE — do not "optimise" it away (D9, Adam,
# 2026-09-18). Several targets write the shared `prices` database rather than
# a scratch one: `rollup_freshness_it` went 7 of 25 red in parallel and 25/25
# serially; `symbol_queue_it`'s
# `a_resolved_symbol_leaves_the_queue_even_after_failures` was red 1 run in 2
# in parallel, green 3/3 serially. `supply_it`, `progress_it`, `freshness_it`
# and `usd_rate_population_it` (which TRUNCATEs usd_rate / oracle_prices /
# assets) share it too. Measured cost: ~109 s of test time vs ~50 s.
#
# NEVER RUN TWO OF THESE AGAINST ONE SERVER AT ONCE. Across targets the run is
# safe only because cargo runs test binaries one after another inside ONE
# invocation. Two invocations — two CI jobs, or a developer while CI runs —
# truncate each other's tables. No lock is taken: the real risk is two
# machines against one server, which no local lock can see.
#
# Usage:  tools/scripts/ignored-tests.sh [subcommand] [repo-root]
#
#   run        (default) check, preflight, the counted cargo run, assert
#   check      classify every #[ignore]; exit non-zero on any violation
#   targets    one `-p <package> --test <target>` line per CH target, sorted
#   expect     CH_TESTS= CH_TARGETS= NET_TESTS= PROD_TESTS=, derived
#   image-tag  the ClickHouse version pinned in docker-compose.yml
#   preflight  wait for $CLICKHOUSE_URL, assert version() == image-tag and
#              timezone() == UTC
#   assert LOG [repo-root]   judge a saved cargo log against `expect`
#
# Locally (ClickHouse per docker-compose.yml, or any server of that version):
#
#   scripts/ch-proxy-0281.sh up        # execution_bound_error_it needs it
#   CLICKHOUSE_URL=http://localhost:8123 \
#   CLICKHOUSE_PROXY_URL=http://localhost:8124 \
#     tools/scripts/ignored-tests.sh
#
# The schema must exist first (`cargo run -q -p prices-clickhouse --bin
# prices-clickhouse-init -- --rollups`, idempotent); CI does exactly this.
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
  echo "usage: $0 [run|check|targets|expect|image-tag|preflight] [repo-root] | assert LOG [repo-root]" >&2
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

ch_query() {
  local url="$1" sql="$2" auth=()
  [[ -n "${CLICKHOUSE_USER:-}" ]] && auth+=(-H "X-ClickHouse-User: ${CLICKHOUSE_USER}")
  [[ -n "${CLICKHOUSE_PASSWORD:-}" ]] && auth+=(-H "X-ClickHouse-Key: ${CLICKHOUSE_PASSWORD}")
  curl -fsS --max-time 5 "${auth[@]}" --data-binary "$sql" "${url}/"
}

# The server under test must be the pinned build, in UTC — `prices-api` ITs
# compare literal timestamps and read empty data under any other zone.
cmd_preflight() {
  local root="$1" url="${CLICKHOUSE_URL:-http://localhost:8123}" want attempt=0 version tz
  want="$(cmd_image_tag "$root")"
  echo "ignored-tests: preflight against ${url} (expecting ClickHouse ${want}, UTC)"
  # This retry IS the readiness check — do not replace it with a sleep.
  # `docker compose up --wait` reports healthy on an in-container healthcheck
  # that can reach the image's TEMPORARY initdb server (bound to the
  # container's loopback), and right after initdb that server is killed and
  # nothing listens until the real one starts. Only a query from the host
  # proves the port the tests use is being served.
  until ch_query "$url" 'SELECT 1' >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [[ $attempt -ge 60 ]]; then
      echo "ignored-tests: ${url} did not answer 'SELECT 1' after 60 attempts:" >&2
      ch_query "$url" 'SELECT 1' >&2 || true
      exit 1
    fi
    sleep 1
  done
  version="$(ch_query "$url" 'SELECT version()')"
  tz="$(ch_query "$url" 'SELECT timezone()')"
  echo "ignored-tests: server version() = ${version}, timezone() = ${tz}"
  [[ "$version" == "$want" ]] ||
    die "server is ClickHouse ${version}, but docker-compose.yml pins ${want} — the tests must run against the build production runs."
  [[ "$tz" == UTC ]] ||
    die "server timezone() is ${tz}, not UTC — production is UTC and the ITs compare literal timestamps."
}

# Judge a saved cargo log against the derived inventory.
cmd_assert() {
  local log="$1" root="$2" counts clean summaries n_lines=0 passed=0 failed=0 p f errors=()
  [[ -f "$log" ]] || die "no such log: ${log}"
  counts="$(cmd_expect "$root")"
  local CH_TESTS CH_TARGETS NET_TESTS PROD_TESTS
  eval "$counts"
  # Tolerate colour even though `run` asks cargo for none: the rust job sets
  # CARGO_TERM_COLOR=always job-wide.
  clean="$(sed -E $'s/\x1b\\[[0-9;]*[A-Za-z]//g' "$log")"
  if grep -qE '^[[:space:]]*Doc-tests ' <<<"$clean"; then
    die "the log contains a Doc-tests run — a --test-filtered run builds none, so this is not the log of \`run\` and its summaries cannot be counted against the inventory."
  fi
  summaries="$(grep -E '^[[:space:]]*test result: ' <<<"$clean" || true)"
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    p="$(sed -nE 's/.* ([0-9]+) passed;.*/\1/p' <<<"$line")"
    f="$(sed -nE 's/.* ([0-9]+) failed;.*/\1/p' <<<"$line")"
    [[ -n "$p" && -n "$f" ]] || die "unparseable summary line: ${line}"
    n_lines=$((n_lines + 1))
    passed=$((passed + p))
    failed=$((failed + f))
  done <<<"$summaries"

  if [[ $failed -gt 0 ]]; then
    errors+=("${failed} failed — the failures are named in the log above.")
  fi
  if [[ $n_lines -lt $CH_TARGETS ]]; then
    errors+=("only ${n_lines} of ${CH_TARGETS} ClickHouse targets printed a 'test result:' line; $((CH_TARGETS - n_lines)) produced no summary at all (killed by a signal, the OOM killer, or a crash before libtest reported). Look at the log, not the inventory.")
  elif [[ $n_lines -gt $CH_TARGETS ]]; then
    errors+=("${n_lines} 'test result:' lines for ${CH_TARGETS} ClickHouse targets — the run selected targets the inventory does not list.")
  elif [[ $passed -ne $CH_TESTS ]]; then
    errors+=("count mismatch: expected ${CH_TESTS} passed ClickHouse tests (derived from the #[ignore] inventory), got ${passed}.")
  fi
  if [[ ${#errors[@]} -gt 0 ]]; then
    echo "ignored-tests: the ClickHouse integration run is RED:" >&2
    printf '  %s\n' "${errors[@]}" >&2
    exit 1
  fi
  echo "ignored-tests: ${passed} passed, ${failed} failed over ${n_lines} target summaries — matches the derived inventory (${CH_TESTS} tests, ${CH_TARGETS} targets)."
}

cmd_run() {
  local root="$1" counts recs names=() name log status=0
  counts="$(cmd_expect "$root")"
  local CH_TESTS CH_TARGETS NET_TESTS PROD_TESTS
  eval "$counts"
  if [[ -z "${CLICKHOUSE_PROXY_URL:-}" ]]; then
    die "CLICKHOUSE_PROXY_URL is unset. execution_bound_error_it is armed and needs a reverse proxy in front of ClickHouse: run \`scripts/ch-proxy-0281.sh up\` and set CLICKHOUSE_PROXY_URL=http://localhost:8124."
  fi
  cmd_preflight "$root"

  recs="$(cmd_targets "$root")"
  echo "ignored-tests: ${CH_TESTS} ClickHouse test(s) in ${CH_TARGETS} target(s):"
  while IFS= read -r name; do echo "  ${name}"; done <<<"$recs"
  # `--test NAME` once per distinct name; a name shared by two crates selects
  # both (the per-crate list above is the attribution cargo's own Running
  # lines cannot give — both seed_it binaries log identically).
  while IFS= read -r name; do
    names+=(--test "$name")
  done < <(sed -E 's/.* --test //' <<<"$recs" | sort -u)

  log="${IGNORED_TESTS_LOG:-$(mktemp "${TMPDIR:-/tmp}/ignored-tests.XXXXXX.log")}"
  # Absolute before the subshell: `tee` runs after `cd "$root"` and
  # `cmd_assert` reads from here, so a relative path would name two files.
  [[ "$log" == /* ]] || log="${PWD}/${log}"
  echo "ignored-tests: log: ${log}"
  echo "ignored-tests: cargo test --workspace ${names[*]} --no-fail-fast -- --ignored --test-threads=1"
  (
    cd "$root"
    set -o pipefail
    CARGO_TERM_COLOR=never cargo test --workspace "${names[@]}" --no-fail-fast \
      -- --ignored --test-threads=1 2>&1 | tee "$log"
  ) || status=$?
  # Judge the log even when cargo failed, so the verdict names what went wrong.
  cmd_assert "$log" "$root"
  [[ $status -eq 0 ]] || die "cargo exited ${status} although the counts matched."
}

sub="${1:-run}"
if [[ "$sub" == assert ]]; then
  [[ -n "${2:-}" ]] || usage
  cmd_assert "$2" "${3:-$default_root}"
  exit 0
fi
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
  preflight) cmd_preflight "$root" ;;
  run) cmd_run "$root" ;;
  *) usage ;;
esac
