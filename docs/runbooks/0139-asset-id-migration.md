# Runbook — the asset-id migration window (task 0139)

`asset_id` becomes `xxh3(concat(asset_code, ':', issuer_address, ':',
contract_address))`, a `UInt64` that ClickHouse derives from the identity. On
production the old UInt32 counter ids are still in every id-keyed table, and
3,315 of them serve two or three unrelated assets. This runbook moves prod onto
the derived ids in one window:

- `assets` is altered in place (`MODIFY COLUMN … MATERIALIZED`,
  `MATERIALIZE COLUMN`).
- The other 11 id-keyed tables are copied into `X__new` tables through the map
  `prices.asset_id_map_0139`, then swapped in with `EXCHANGE TABLES`. The old
  tables stay as `X__pre0139`.
- Rows under colliding ids are blends of two assets and are **not copied**
  (GA2). Their months are re-ingested afterwards (task 13 of plan 261001-fwh,
  `prices.rekey_0139_reingest_months`).
- The 6 rollup MVs and `mv_current_prices` are re-created from their captured
  production text with UInt64 ids (OP1).

Every step is a subcommand of `prices-clickhouse-rekey`
(`packages/prices-clickhouse/src/bin/prices-clickhouse-rekey.rs`). It prints
SQL and writes nothing without `--execute`, logs every write to
`prices.rekey_0139_log`, and is safe to re-run.

## Who runs it, and where

The operator (Adam), from a workstation. The agent holds `dev_read` only and
cannot run any step that writes.

Two shells, both on the workstation:

- **CH shell** — ClickHouse over mTLS with Adam's write certificate
  (`~/.certs/adamkot-write.*`), which authenticates as `dev_shared`, an admin
  user on purpose (task 0258). The tool reads `system.view_refreshes`,
  `system.processes`, `system.asynchronous_inserts`, `system.mutations` and
  `system.query_log`, and runs `SYSTEM STOP/REFRESH VIEW`, `CREATE/DROP VIEW`
  and `EXCHANGE TABLES`. `prices_admin` cannot: it is denied
  `system.view_refreshes` (`Code: 497`, `0286-candle-definitions-rollout.md`
  §0) and holds neither `SYSTEM` nor the view privileges. Checked 2026-10-02:
  `currentUser()` = `dev_shared`, all five `system` tables readable,
  `readonly` = 0. No SSH and no `default` user.
- **AWS shell** — a checkout of `develop` at the 0139 merge commit, with
  production AWS credentials. Rules, the event source mapping, deploys and
  CloudWatch.

Set up the CH shell once, in `tmux`:

```bash
tmux new -s rekey0139
mkdir -p ~/rekey-0139
set -o pipefail
CH=https://ch.sorobanscan.rumblefish.dev/
WCERT=(--cert ~/.certs/adamkot-write.crt --key ~/.certs/adamkot-write.key --cacert ~/prices-mtls/ca.crt)
rk() {
  ~/rekey-0139/prices-clickhouse-rekey "$@" --mtls-domain ch.sorobanscan.rumblefish.dev \
    --mtls-cert ~/.certs/adamkot-write.crt --mtls-key ~/.certs/adamkot-write.key \
    --mtls-ca ~/prices-mtls/ca.crt 2>&1 | tee -a ~/rekey-0139/window.log
}
chq() {
  grep -v '^[[:space:]]*--' \
    | perl -0777 -ne 'for (split /;[ \t]*\n/) { s/^\s+|\s+$//g; s/;$//; print "$_\0" if length }' \
    | while IFS= read -r -d '' q; do
        curl -sS --fail-with-body "${WCERT[@]}" "$@" "$CH" --data-binary "$q" || exit 1
      done
}
echo "SELECT currentUser(), version()" | chq
```

The last line must print `dev_shared`. `chq` reads SQL on stdin and sends one
request per statement (the HTTP interface takes one): it drops full-line `--`
comments and splits on a `;` that ends a line. Its arguments go to `curl`, e.g.
`--url-query "param_x=…"` (curl ≥ 7.87). `rk` and `chq` are shell functions: a
second tmux pane needs them defined again.

Every request goes through the Caddy proxy in front of ClickHouse, the path the
0286 orchestrator's month re-rolls already take. The rehearsal (task 11) runs
the same path, so its fill and mutation times include it. A request the proxy
cuts surfaces as an error: re-run the step (every `rk` step resumes).

Every `rk` line below exits non-zero on a refusal, a failed gate or an error.
**Stop at the first non-zero exit** and read the message.

## Preconditions

- [ ] Plan 261001-fwh task 2 decisions are in task 0139 (GA1, GA2, OP1 and
      variant A of GA4). GA3: one PR, merged on window day, so readers and
      writers deploy together in W11.
- [ ] Task 11's rehearsal numbers are in "Expected duration" below.
- [ ] The 0139 PR is merged to `develop` on window day, before W0. Every
      artefact below is built from that merge commit.
- [ ] Not on the 0236 deploy day (≥ 2026-10-04).
- [ ] 0286 phase 3 is paused at a month boundary (paused by Adam from
      2026-10-02). Record the last finished month here: `PAUSE_MONTH=______`.
- [ ] No deploy of the compute, eventbridge or observability stacks between the
      merge and W11.
- [ ] BE has the heads-up (time of the window, API answers stale for its
      length, `GET /assets` cursors in flight may skip or repeat once).
- [ ] The rollback artefacts below exist and the deployed state is recorded.
- [ ] Built from the merge commit (AWS shell, `CARGO_TARGET_DIR` unset so cargo
      writes `target/` of this checkout, which is what CDK packages):

  ```bash
  git checkout develop && git pull --ff-only
  git rev-parse HEAD                     # the 0139 merge commit: record it
  env -u CARGO_TARGET_DIR make -C infra build-lambdas
  env -u CARGO_TARGET_DIR cargo build --release -p prices-clickhouse --features aws-mtls \
    --bin prices-clickhouse-rekey
  cp target/release/prices-clickhouse-rekey ~/rekey-0139/
  env -u CARGO_TARGET_DIR cargo build --release -p sdex-backfill -p events-backfill
  ```

  The new `sdex-backfill` and `events-backfill` replace the ones the 0286
  orchestrator runs, on `fishuser-hero` and on the CH host (Oskar, before phase
  3 resumes). **Keep the old ones beside them as `*.pre0139`; never overwrite a
  `*.pre0139`:**

  ```bash
  # for each of events-backfill and sdex-backfill, where it lives
  [ -e ~/events-backfill.pre0139 ] || cp -p ~/events-backfill ~/events-backfill.pre0139
  ```

## Rollback artefacts and deployed state (T−1 d, a precondition of W0)

AWS shell. This is what a rollback after W11 redeploys from.

1. Record what production runs now:

   ```bash
   mkdir -p ~/rekey-0139
   for f in ledger-processor api-handler oracle asset-discovery supply enrichment \
            coarse-sweep cleanup backfill-freshness-probe rollup-freshness-probe \
            mtls-notafter-probe coverage-sweep-probe; do
     aws lambda get-function-configuration --function-name "prices-production-$f" \
       --query '[FunctionName,CodeSha256,LastModified,Version]' --output text
   done | tee ~/rekey-0139/deployed-before.tsv
   ESM=$(aws lambda list-event-source-mappings --function-name prices-production-ledger-processor \
     --query 'EventSourceMappings[0].UUID' --output text)
   echo "ESM=$ESM" | tee -a ~/rekey-0139/deployed-before.tsv
   aws lambda get-event-source-mapping --uuid "$ESM" --query State --output text
   RULES="prices-production-oracle-watcher prices-production-asset-discovery \
     prices-production-asset-supply prices-production-enrichment prices-production-coarse-sweep"
   for r in $RULES prices-production-cleanup; do
     printf '%s\t%s\n' "$r" "$(aws events describe-rule --name "$r" --query State --output text)"
   done | tee -a ~/rekey-0139/deployed-before.tsv
   ```

   Expect the ESM `Enabled`, the five rules `ENABLED` and `prices-production-cleanup`
   `DISABLED`.

2. The rollback commit is `develop` just before the 0139 merge. On window day,
   record it as the merge commit's first parent:
   `ROLLBACK_SHA=$(git rev-parse <merge>^1)`. Before the merge, it is `origin/develop`. Check it matches
   production: `make -C infra diff-production` run from it shows no change
   besides Lambda asset hashes.

3. Build the rollback artefacts in their own worktree and target dir, so the
   forward build's `target/lambda` is never overwritten:

   ```bash
   git worktree add .claude/worktrees/0139-rollback "$ROLLBACK_SHA"
   cd .claude/worktrees/0139-rollback && env -u CARGO_TARGET_DIR make -C infra build-lambdas
   ```

   Keep this worktree until W16 passes.

## T−1 d: capture, map dry run, optional pre-fill

CH shell. Writers are running; nothing here touches a live table.

```bash
rk preflight
rk capture --execute
rk map
```

`preflight` must pass: an Atomic database, a 26.3 server, `system.query_log`,
the 12 id tables on UInt32, free disk ≥ 1.2 × the 11 tables. `map` without
`--execute` prints the per-status summary it would write (expect about 3,315
colliding ids).

**Online pre-fill** (if task 11 measured the full fill above ~15 min):

```bash
rk map --execute
rk create --execute
rk fill --execute
```

The window's W7 then re-verifies every partition by fingerprint and refills
only what changed since: the current month, and any partition touched by an id
that became colliding.

## Expected duration

Measured on ch-prod-01, 2026-10-02 (task 11 rehearsal, `prices_r0139`
hardlinked from `prices`, run over mTLS as `dev_shared`, with 0286 phase 3
ingesting for the first ~50 min):

| step                                               | time                                               |
| -------------------------------------------------- | -------------------------------------------------- |
| hardlink copy, 885 partitions                      | ~1 min (9 ms per `ATTACH`)                         |
| `preflight`, `capture`, `map`, `create`            | 1–2 s each                                         |
| `fill`, 11 tables, 784 partitions                  | **2,003 s (33 min)**; 1m alone ~15 min (620M rows) |
| `fill` again with nothing to copy                  | 546 s: every partition is probed again             |
| `check`                                            | **553 s (9 min)**                                  |
| `alter-assets` (632k rows), `swap`, `recreate-mvs` | 3 s, 3 s, 2 s                                      |
| refresh loop + `verify`                            | under a minute; every line `1`                     |
| `gap-backfill`, 3.4 h gap, all six tiers           | 2.2 s                                              |
| `gap-verify`, `rollback`                           | 1 s, 4 s                                           |
| disk while the copy exists                         | −50 GiB (865 → ~815 GiB free)                      |
| API p95 during the run                             | 580–634 ms, baseline 400–630 ms                    |

So W7 is fill + check: about **42 min** cold. With the online pre-fill on T−1 d
(below), W7 is a probe pass plus check: about **18–20 min**. Use the pre-fill.

Two findings, both fixed. A background merge of a `ReplacingMergeTree` source
collapses duplicate keys and changes the raw-row fingerprint `check` compares,
so `check` reported "source changed since fill" on two recent partitions of a
copy nothing wrote to. W5 now stops merges on the 11 sources. And production's
rollup MVs run as `DEFINER = prices_admin`, which cannot read a scratch
database; `capture --rewrite-db` now writes `DEFINER = CURRENT_USER`.

## Gap budget

The writer-stop gap runs from W3 through the end of the catch-up after W13:
W3 → W13, plus the catch-up. `mv_ohlcv_1m_to_15m` re-reads only the last 2 h
of `price_ohlcv_1m` (`rollups.sql`), and catch-up writes 1m rows with their
ledger timestamps, so every catch-up row older than 2 h at the 15m MV's next
refresh is never rolled into 15m, nor into 1h, 4h, 1d, 1w and 1M above it.
Prod has no `mv_reconcile_*` to repair it (0203 is not deployed, and OP1 keeps
it so).

For a window of length W (W3 → W13) and a catch-up speed r (ledger-seconds per
wall-second, task 11), the catch-up takes C = W / (r − 1), and the gap exposed
to the 15m MV is W + C. Above about 1.5 h, the 15m MV alone loses part of it.
**W14 runs whatever the measured gap**: it costs minutes and is the only thing
that makes every tier whole.

Measured catch-up: r ≈ 10.7, from the one backlog in 63 days of
`ApproximateAgeOfOldestMessage` on `prices-ingest-production` (5-minute
maxima): 7.3 h behind at 2026-08-14 09:15 UTC, under 120 s by 09:55. A 2 h
window then catches up in about 12 min, and W + C ≈ 2.2 h, past the 15m MV's
2 h. The gap-backfill time is task 11's to measure.

## The window

Times are UTC. Write every recorded value into `~/rekey-0139/window.log` (the
`rk` output already goes there).

### W0 — mute the alarms the window trips

AWS shell:

```bash
MUTED=(
  prices-production-rollup-freshness-1m prices-production-rollup-freshness-15m
  prices-production-rollup-freshness-1h prices-production-rollup-freshness-4h
  prices-production-rollup-freshness-1d prices-production-rollup-freshness-1w
  prices-production-rollup-freshness-1M prices-production-current-prices-freshness
  prices-production-rollup-mismatch-15m prices-production-rollup-mismatch-1h
  prices-production-rollup-mismatch-4h prices-production-rollup-mismatch-1d
  prices-production-rollup-mismatch-1w prices-production-rollup-mismatch-1M
  prices-production-mv-refresh-disabled prices-production-mv-refresh-waiting
  prices-production-mv-refresh-failing prices-production-mv-drift
  prices-production-ledger-processor-lag prices-production-ledger-processor-no-invocations
  prices-production-sdex-push-freshness prices-production-amm-push-freshness
  prices-production-coverage-sweep-unclassified prices-production-enrichment-backlog
  prices-production-oracle-dark-feed prices-production-oracle-usdc-snapshot-stalled
  prices-production-oracle-no-invocations prices-production-asset-discovery-no-invocations
  prices-production-supply-no-invocations prices-production-enrichment-no-invocations
  prices-production-coarse-sweep-no-invocations
  prices-production-rollup-freshness-probe-errors
)
aws cloudwatch describe-alarms --alarm-names "${MUTED[@]}" --query 'length(MetricAlarms)'
aws cloudwatch disable-alarm-actions --alarm-names "${MUTED[@]}"
```

The count must equal `${#MUTED[@]}` (32); a lower one is a typo. Why each
group:

- rollup freshness, mismatch, MV refresh and drift: the MVs are stopped from
  W5 and dropped and re-created in W10;
- ledger-processor lag and no-invocations, sdex/amm push freshness, coverage
  sweep: no ledger is ingested from W3 to W13;
- oracle, enrichment backlog and the five `-no-invocations`: their rules are
  disabled from W2;
- **`prices-production-rollup-freshness-probe-errors`: the probe's orphan read
  refuses when no `price_ohlcv_1m` candle landed in the last 2 h**
  (`EmptyWindow`, never published as a healthy 0), so its invocations error on
  every tick of a window that long. Its collision read still publishes.

Unmuted at the end of W14. `ch-disk-free`, the DLQ ladder, `api-5xx` and the
usd/zero-invariant ladders stay armed.

### W1 — confirm the 0286 orchestrator is stopped

On `fishuser-hero`, with the flags of the paused run:

```bash
python3 tools/scripts/reingest_0286.py status <the run's flags>
pgrep -af 'reingest_0286|sdex-backfill|events-backfill' || echo "nothing running"
```

No month may be in progress (a half-done month has its `1m` partition dropped
and partly refilled). Oskar runs the same `pgrep` on the CH host. It stays stopped
until W14 is green.

### W2 — disable the scheduled writers

AWS shell:

```bash
for r in $RULES; do aws events disable-rule --name "$r"; done
for r in $RULES prices-production-cleanup; do
  printf '%s\t%s\n' "$r" "$(aws events describe-rule --name "$r" --query State --output text)"
done
```

All six `DISABLED` (`prices-production-cleanup` already was). The probe rules
stay enabled: they only read.

### W3 — stop the live ledger processor

Record the stop time first:

```bash
W3_STOP=$(date -u '+%F %T'); echo "W3_STOP=$W3_STOP" | tee -a ~/rekey-0139/deployed-before.tsv
aws lambda update-event-source-mapping --uuid "$ESM" --no-enabled
aws lambda get-event-source-mapping --uuid "$ESM" --query State --output text
```

Repeat the last line until it prints `Disabled`. **Never set the function's
concurrency to 0**: throttled receives count toward `maxReceiveCount` and push
doorbells to the DLQ. Disabling the mapping loses nothing: the ClickHouse
cursor (task 0064) drives catch-up, and the queue keeps doorbells for 14 days.

### W4 — wait out every writer

Wait at least 10 minutes after W3 (longest worker timeout 5 min, plus async
retries), then:

```bash
for f in ledger-processor oracle asset-discovery supply enrichment coarse-sweep; do
  printf '%s\t%s\n' "$f" "$(aws cloudwatch get-metric-statistics --namespace AWS/Lambda \
    --metric-name Invocations --dimensions Name=FunctionName,Value=prices-production-$f \
    --start-time "$(date -u -d '-10 min' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)" \
    --period 60 --statistics Sum --query 'sum(Datapoints[].Sum)' --output text)"
done
```

Every line `0` (or `None`: no datapoint). Nothing manual may run, here, on
`fishuser-hero` or on the CH host (Oskar):
`pgrep -af 'coarse-repair|sdex-backfill|events-backfill|pool-registry-seed|prices-clickhouse-init'`
prints nothing. W6's `rk swap --check-only` also refuses on any INSERT into
`prices` from anywhere.

### W5 — stop the MVs, capture

CH shell:

```bash
chq <<'SQL'
SYSTEM STOP VIEW prices.mv_ohlcv_1m_to_15m;
SYSTEM STOP VIEW prices.mv_ohlcv_15m_to_1h;
SYSTEM STOP VIEW prices.mv_ohlcv_1h_to_4h;
SYSTEM STOP VIEW prices.mv_ohlcv_4h_to_1d;
SYSTEM STOP VIEW prices.mv_ohlcv_1d_to_1w;
SYSTEM STOP VIEW prices.mv_ohlcv_1d_to_1M;
SYSTEM STOP VIEW prices.mv_current_prices;
SELECT view, status FROM system.view_refreshes WHERE database = 'prices' ORDER BY view;
SQL
for t in price_ohlcv_1m price_ohlcv_15m price_ohlcv_1h price_ohlcv_4h price_ohlcv_1d price_ohlcv_1w \
         price_ohlcv_1M current_prices asset_supply asset_metadata oracle_prices; do
  echo "SYSTEM STOP MERGES prices.$t;"
done | chq && echo "MERGES STOPPED"
rk capture --execute
```

`STOP MERGES` freezes the 11 copy sources until the swap. A merge collapses
duplicate keys and changes the fingerprint `fill` recorded, and `check` then
reports "source changed since fill" for a partition nobody wrote (seen in the
rehearsal). After W9 these tables are the `X__pre0139` copies, and the new
tables merge as usual. `assets` is not stopped: `alter-assets` needs merges.

Every view `Disabled`; repeat the SELECT while one reads `Running`. If any other
view is listed (`mv_reconcile_*` on a target where 0143/0203 were deployed),
stop it too. ⚠️ `SYSTEM STOP VIEW` is lost on a server restart: after one,
re-run this step.

### W6 — the window is quiet

```bash
rk swap --check-only
```

It reads `system.view_refreshes`, `system.processes` and
`system.asynchronous_inserts` for `prices` and exits non-zero while any view is
not `Disabled`, any INSERT into `prices` runs, or any async insert is queued.
It also prints whether the last `check` is green (not yet, before W7).

### W7 — map, fill, check

```bash
rk map --execute
rk create --execute
rk fill --execute
rk check --execute
```

`fill` copies only `mapped` and `sentinel` rows, partition by partition. Each
partition is verified twice: `written_rows` from `system.query_log`, and the
distinct target keys against the source's. `check` must end with a line per
table and exit 0; it rewrites `prices.rekey_0139_reingest_months`.

### W8 — alter `assets`

```bash
rk alter-assets --execute
```

It snapshots `assets__pre0139` by hardlink (`ATTACH PARTITION … FROM`), starts
merges on `assets` (a type change waits forever while they are stopped), then
`MODIFY COLUMN asset_id UInt64 MATERIALIZED …`, `MATERIALIZE COLUMN`, `ADD
CONSTRAINT`. Its wait for `system.mutations` is bounded (`--mutation-timeout`,
default 1800 s). It fails unless every row equals the expression and `assets
FINAL` has one row per id. Between W8 and W9 a join of `assets` against the
candle tables is wrong; go straight on.

### W9 — swap

```bash
rk swap --execute
```

It refuses unless the window is quiet, the last `check` covered all 11 tables
and was green, and `alter-assets` ran. It logs `last_live_1m_ts` (the newest
outgoing `price_ohlcv_1m` row: the start of the writer-stop gap) and the swap
time, then `EXCHANGE TABLES` and renames each `X__new` to `X__pre0139`. A
re-run after a failure resumes.

### W9b — old-id-space backups (GA1)

The `reingest_0286_bak_*` snapshots hold UInt32 ids. They are kept as the
decoder-backed record of phase 3 so far, renamed so the orchestrator never
takes them for its own:

```bash
chq <<'SQL'
SELECT name, total_rows FROM system.tables
WHERE database = 'prices' AND startsWith(name, 'reingest_0286_bak_') ORDER BY name;
SQL
echo "SELECT 'RENAME TABLE prices.' || name || ' TO prices.' || name || '_pre0139;'
FROM system.tables WHERE database = 'prices' AND startsWith(name, 'reingest_0286_bak_')
AND NOT endsWith(name, 'pre0139') FORMAT TSVRaw" | chq > ~/rekey-0139/rename-bak.sql
cat ~/rekey-0139/rename-bak.sql && chq < ~/rekey-0139/rename-bak.sql
echo "SELECT 'ALTER TABLE prices.' || name || ' MODIFY COMMENT ''old id space: pre-0139 UInt32 asset ids, decode with prices.asset_id_map_0139'';'
FROM system.tables WHERE database = 'prices' AND startsWith(name, 'reingest_0286_bak_')
FORMAT TSVRaw" | chq | chq
```

The SELECT before and a re-run after must list the same `total_rows`, under the
new names. `recreate-mvs` (W10) fails its type gate while a
`reingest_0286_bak_*` table without the suffix remains. `rollout_0286_bak_*` and
`price_ohlcv_*_bak` are exempt from that gate and are dropped once their owners
confirm (see "Old tables").

### W10 — re-create the MVs

```bash
rk recreate-mvs --source prod-text --execute
```

OP1: the captured production text, id types rewritten to UInt64, not
`rollups.sql` (which would also ship 0143/0203). The views are re-applied from
their captured text the same way. The type gate then fails if any `asset_id` or
`quote_asset_id` of `prices` is not UInt64 outside `*pre0139`,
`rollout_0286_bak_*` and `price_ohlcv_*_bak` (`drift.rs` cannot see an MV's
declared types). The re-created MVs start refreshing at once.

### W11 — deploy

AWS shell, from the merge commit (the build of the preconditions):

```bash
make -C infra deploy-production-compute
make -C infra deploy-production-eventbridge
make -C infra deploy-production-observability
aws lambda update-function-configuration --function-name prices-production-api-handler \
  --description "0139 cold start $(date -u +%FT%TZ)"
```

After **each** deploy, re-check, because a CDK deploy may restore a state the
CLI changed:

```bash
aws lambda get-event-source-mapping --uuid "$ESM" --query State --output text   # Disabled
for r in $RULES; do aws events describe-rule --name "$r" --query State --output text; done  # DISABLED x5
```

Disable again at once if one came back. The `update-function-configuration`
forces new api-handler containers: the old ones memoise XLM/USDC/USDT ids
(`state.rs`) for their lifetime.

### W12 — verify and smoke-test

CH shell:

```bash
for mv in mv_ohlcv_1m_to_15m mv_ohlcv_15m_to_1h mv_ohlcv_1h_to_4h mv_ohlcv_4h_to_1d \
          mv_ohlcv_1d_to_1w mv_ohlcv_1d_to_1M mv_current_prices; do
  printf 'SYSTEM REFRESH VIEW prices.%s;\nSYSTEM WAIT VIEW prices.%s;\n' "$mv" "$mv"
done | chq
rk verify
```

`verify` runs the type gate, the window post-checks that need no gap (assets
unique, no UInt32 id, `current_price_usd` one row per `current_prices` row,
0129's cross-check), finds no id outside `assets` in any swapped table
(REDSTONE's 0 aside), resolves XLM/USDC in `usd_reference_1h`, and wants a
successful refresh of every MV since the swap (the refresh loop above makes
one; the daily MVs would otherwise wait for their slot).

AWS shell (`PRICES_API_KEY` is an operator key):

```bash
API=https://prices-api.sorobanscan.rumblefish.dev/v1
USDC=USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN
curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/native/price" | jq '{price_usd, method, price_status}'
curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$USDC/price" | jq '{price_usd, method, price_status}'
C=""; for i in 1 2 3; do
  R=$(curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets?limit=200${C:+&cursor=$C}")
  echo "$R" | jq -r '.data[] | [.asset_code, .issuer_address, .contract_address] | @tsv'
  C=$(echo "$R" | jq -r '.cursor // empty'); [ -n "$C" ] || break
done | sort | uniq -d | wc -l                                                       # 0
curl -sS -H "x-api-key: $PRICES_API_KEY" \
  "$API/assets/native/ohlcv?granularity=1h&start=$(date -u -d '-2 day' +%FT%H:00:00Z)&end=$(date -u -d '-1 day' +%FT%H:00:00Z)&base_currency=USD" \
  | jq '.data | length'                                                             # > 0
aws lambda invoke --function-name prices-production-rollup-freshness-probe ~/rekey-0139/probe.json
jq -r '.errorMessage // .' ~/rekey-0139/probe.json                                # the orphan refusal, see below
aws cloudwatch get-metric-statistics --namespace Prices/Rollup --metric-name AssetIdCollisions \
  --dimensions Name=Environment,Value=production --start-time "$(date -u -d '-10 min' +%FT%TZ)" \
  --end-time "$(date -u +%FT%TZ)" --period 60 --statistics Maximum --query 'Datapoints[].Maximum'
```

The prices are those of W3 (nothing newer is ingested yet). `AssetIdCollisions`
must read `0` (measured as `count() − uniqExact(asset_id)` before the window:
3,321 on 2026-10-01). The probe invocation itself returns an error while the
orphan read refuses on an empty 2-hour window, and it fails before it writes its
JSON, so the file holds only that error; that is expected until W13. The
CloudWatch line is the uniqueness check: the probe publishes `AssetIdCollisions`
before the orphan read refuses.

**Any failure here: roll back (below), before W13.**

### W13 — resume the writers and catch up

AWS shell:

```bash
aws lambda update-event-source-mapping --uuid "$ESM" --enabled
CATCHUP_STARTED=$(date -u '+%F %T'); echo "CATCHUP_STARTED=$CATCHUP_STARTED" | tee -a ~/rekey-0139/deployed-before.tsv
for r in $RULES; do aws events enable-rule --name "$r"; done
watch -n 60 "aws cloudwatch get-metric-statistics --namespace AWS/SQS \
  --metric-name ApproximateAgeOfOldestMessage --dimensions Name=QueueName,Value=prices-ingest-production \
  --start-time \$(date -u -d '-6 min' +%FT%TZ) --end-time \$(date -u +%FT%TZ) \
  --period 60 --statistics Maximum --query 'sort_by(Datapoints,&Timestamp)[].Maximum' --output text"
```

Caught up when the newest live candle is within 5 minutes of now:

```bash
echo "SELECT now(), max(timestamp) FROM prices.price_ohlcv_1m WHERE timestamp > now() - INTERVAL 1 DAY AND source = 'sdex'" | chq
```

The SQS age above is a ceiling, not the signal. A message is a doorbell, and
one invocation ingests up to 16 ledgers from the ClickHouse cursor, so the data
catches up before the doorbells drain. On 2026-10-02 the age stood at 1.5 h
while candles were one minute old: BE's Galexie was replaying a backlog of
doorbells. Record it and compare the catch-up duration with task 11's
prediction:

```bash
CATCHUP_END=$(date -u '+%F %T'); echo "CATCHUP_END=$CATCHUP_END" | tee -a ~/rekey-0139/deployed-before.tsv
```

`AssetIdOrphanCandles` must read 0 once the probe sees live candles again.
From here on, **rollback is forward-fix only**: the old tables lack every row
written since.

### W14 — roll the gap into every tier, then the post-checks

CH shell, once W13 recorded `CATCHUP_END`:

```bash
rk gap-backfill --execute
rk gap-verify
```

`gap-backfill` reads `last_live_1m_ts` from the swap's log row and runs the
generator's bounded rollup INSERT (`schema/preroll-live-gap.sql`'s statements)
for 15m, 1h, 4h, 1d, 1w and 1M, in that order, over `[last_live_1m_ts − 2 h,
now)`, on the swapped tables only: colliding blends and orphans stay out, as
the copy left them. It refuses while the newest `price_ohlcv_1m` row is older
than 15 minutes (catch-up not finished), and logs the range, each tier's
`written_rows` and the time taken.

`gap-verify` is read-only: per tier against the tier below, from the gap's
first bucket to the buckets that have settled (closed, then refreshed: 15m
after 16 min, 1h after 31 min, 4h after 1 h 30, 1d after 5 h 15, 1w and 1M
after 28 h 15), every `(asset_id, quote_asset_id, source, bucket)` must agree
on `trade_count` and `volume_base`, then the range totals of the post-checks.
A tier none of whose buckets has settled **fails** rather than passing
unchecked. At W14 it checks 15m and 1h; W16 and W17 check the coarser tiers.
A mismatch: run `rk gap-backfill --execute` once more (it is idempotent)
and `rk gap-verify` again. **A second mismatch stops the window for
investigation**; writers stay on and the fix is forward.

Then the window post-checks (block below; the agent runs the same block
read-only as `dev_read`). Every line must print `1`. Then unmute:

```bash
aws cloudwatch enable-alarm-actions --alarm-names "${MUTED[@]}"
```

The 0286 orchestrator stays stopped until W14 is green.

### W15 — afterwards

- Adam sends the BE note (draft below) and records the acceptance numbers in
  task 0139.
- The old tables follow "Old tables" below.
- Then task 13 (the colliding history): 0286 phase 3 resumes from the month
  after `PAUSE_MONTH` on the new ids, and the second pass covers every month
  in `prices.rekey_0139_reingest_months` up to and including `PAUSE_MONTH`
  ([`0286-reingest-history.md`](0286-reingest-history.md) §10: binaries,
  old snapshots, resume, second pass).

### W16 — the next day

After 05:15 UTC on the day after W14 (1d has settled; 4h long before). CH
shell:

```bash
rk gap-verify --set next-day
```

Then the next-day post-check block below: every line `1`. Then remove the
rollback worktree (`git worktree remove .claude/worktrees/0139-rollback`).

### W17 — the gap's week and month have closed

After 04:15 UTC on the Tuesday after the gap's week ends (UTC Monday-start
weeks) and the 2nd of the month after the gap's month, whichever is later:

```bash
rk gap-verify --set period-close
```

Then the period-close post-check block below: both lines `1`. A `0` before
that time means the bucket has not settled yet, not a lost gap.

## Rollback

Valid until W13. `rollback` exchanges the 11 tables back with `X__pre0139` (the
UInt64 tables become `X__new` again), swaps `assets` back with
`assets__pre0139` (the altered table becomes `assets__new`), re-creates the
captured MVs and views exactly as captured, and `SYSTEM STOP VIEW`s the MVs. It
refuses if `price_ohlcv_1m` has rows after `last_live_1m_ts`, `oracle_prices`
has rows after the swap, or `assets` gained identities; `--force-lose-post-swap-rows`
overrides that and loses them.

**Before W11** (nothing deployed yet), CH shell:

```bash
rk rollback --execute
```

If W9 had run, force an api-handler cold start (its memo may hold new ids; the
`update-function-configuration` line of W11). Then `SYSTEM START MERGES` on the
11 tables of W5 (the restored tables come back with merges stopped),
`SYSTEM START VIEW` each of the seven MVs of W5, and re-enable the writers in
W13's order.

**After W11, before W13**:

1. `rk rollback --execute` (CH shell).
2. From the rollback worktree (AWS shell):

   ```bash
   cd .claude/worktrees/0139-rollback
   make -C infra deploy-production-compute
   make -C infra deploy-production-eventbridge
   make -C infra deploy-production-observability
   ```

3. Re-check that the ESM is still `Disabled` and the five rules still
   `DISABLED` (the W11 re-check lines). Disable again if needed.
4. Put the `*.pre0139` backfill binaries back where they were replaced
   (`cp -p ~/events-backfill.pre0139 ~/events-backfill`, likewise
   `sdex-backfill`).
5. Force an api-handler cold start (W11's last line).
6. `SYSTEM START MERGES` on the 11 tables of W5, `SYSTEM START VIEW` for the
   seven MVs, then re-enable the writers in W13's order and watch the catch-up the same way.

**After W13**: forward-fix only.

**The rollback gap** (both paths above). The restored UInt32 tables and MVs
have the same 2 h 15m lookback, and `gap-backfill` refuses on them. After the
rollback catch-up (the W13 watch, to its own `CATCHUP_END`), pre-roll the gap
with the generated live-gap statements, from the repo, never a copy on the box:

```bash
START_TS=$(date -u -d "$W3_STOP UTC - 2 hour" '+%F %T')   # or last_live_1m_ts - 2 h if W9 ran
chq --url-query "param_start_ts=$START_TS" --url-query "param_end_ts=$CATCHUP_END" \
  < packages/prices-clickhouse/schema/preroll-live-gap.sql && echo "PREROLL OK"
rk gap-verify --schema pre0139 --from "$START_TS" --to "$CATCHUP_END"
```

(Run it from the merge commit's checkout.) `--schema pre0139` makes `gap-verify` run its tier comparisons on the
restored UInt32 tables, read-only.

## Post-checks

Four blocks, rendered from `postcheck_sql` (`src/rekey/gap.rs`) and pinned to
it by a unit test. One `SELECT` per line, each prints `1` on pass. No
`SETTINGS` (`dev_read` is `readonly=1`); gap ranges are hours, and month
queries touch one partition. Run a block with the read certificate
(`0151-zero-invariant-probe-rollout.md`): replace `window` with `next-day` or
`period-close`, or with `month` plus `?param_m=YYYYMM` on the URL.

```bash
B=window   # next-day | period-close | month
sed -n "/<!-- 0139-postcheck:$B -->/,/<!-- \/0139-postcheck:$B -->/p" \
  docs/runbooks/0139-asset-id-migration.md | grep '^SELECT ' | while IFS= read -r q; do
  r=$(printf '%s' "$q" | curl --fail-with-body -sS --cert "$READ_CERT" --key "$READ_KEY" \
    --cacert "$PRICES_CA" "https://ch.sorobanscan.rumblefish.dev/${M:+?param_m=$M}" --data-binary @-)
  printf '%s\t%s\n' "$r" "$q"
done
```

### Window (W14)

<!-- 0139-postcheck:window -->

```sql
SELECT ifNull(count() = uniqExact(asset_id), 0) AS assets_unique FROM prices.assets FINAL
SELECT ifNull(count() = 0, 0) AS no_uint32_ids FROM system.columns WHERE database = 'prices' AND name IN ('asset_id', 'quote_asset_id') AND type NOT IN ('UInt64', 'Nullable(UInt64)') AND NOT (endsWith(table, 'pre0139') OR startsWith(table, 'rollout_0286_bak_') OR match(table, '^price_ohlcv_.+_bak$'))
SELECT ifNull((SELECT count() FROM prices.current_price_usd) = (SELECT count() FROM prices.current_prices FINAL), 0) AS current_price_usd_one_row
SELECT ifNull((SELECT count() FROM prices.price_ohlcv_1h WHERE toYYYYMM(timestamp) BETWEEN 202402 AND 202607 AND volume_quote > 0) = (SELECT count() FROM prices.price_ohlcv_1h AS c INNER JOIN prices.assets AS a FINAL ON a.asset_id = c.quote_asset_id WHERE toYYYYMM(c.timestamp) BETWEEN 202402 AND 202607 AND c.volume_quote > 0), 0) AS cross_check_0129
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 15 MINUTE) < toStartOfInterval(now() - INTERVAL 960 SECOND, INTERVAL 15 MINUTE) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_15m FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 15 MINUTE) AND timestamp < toStartOfInterval(now() - INTERVAL 960 SECOND, INTERVAL 15 MINUTE)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1m FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 15 MINUTE) AND timestamp < toStartOfInterval(now() - INTERVAL 960 SECOND, INTERVAL 15 MINUTE)), 0) AS gap_15m
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 HOUR) < toStartOfInterval(now() - INTERVAL 1860 SECOND, INTERVAL 1 HOUR) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1h FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 HOUR) AND timestamp < toStartOfInterval(now() - INTERVAL 1860 SECOND, INTERVAL 1 HOUR)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_15m FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 HOUR) AND timestamp < toStartOfInterval(now() - INTERVAL 1860 SECOND, INTERVAL 1 HOUR)), 0) AS gap_1h
```

<!-- /0139-postcheck:window -->

### Next day (W16)

<!-- 0139-postcheck:next-day -->

```sql
SELECT ifNull(count() = uniqExact(asset_id), 0) AS assets_unique FROM prices.assets FINAL
SELECT ifNull(count() = 0, 0) AS no_uint32_ids FROM system.columns WHERE database = 'prices' AND name IN ('asset_id', 'quote_asset_id') AND type NOT IN ('UInt64', 'Nullable(UInt64)') AND NOT (endsWith(table, 'pre0139') OR startsWith(table, 'rollout_0286_bak_') OR match(table, '^price_ohlcv_.+_bak$'))
SELECT ifNull((SELECT count() FROM prices.current_price_usd) = (SELECT count() FROM prices.current_prices FINAL), 0) AS current_price_usd_one_row
SELECT ifNull((SELECT count() FROM prices.price_ohlcv_1h WHERE toYYYYMM(timestamp) BETWEEN 202402 AND 202607 AND volume_quote > 0) = (SELECT count() FROM prices.price_ohlcv_1h AS c INNER JOIN prices.assets AS a FINAL ON a.asset_id = c.quote_asset_id WHERE toYYYYMM(c.timestamp) BETWEEN 202402 AND 202607 AND c.volume_quote > 0), 0) AS cross_check_0129
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 15 MINUTE) < toStartOfInterval(now() - INTERVAL 960 SECOND, INTERVAL 15 MINUTE) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_15m FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 15 MINUTE) AND timestamp < toStartOfInterval(now() - INTERVAL 960 SECOND, INTERVAL 15 MINUTE)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1m FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 15 MINUTE) AND timestamp < toStartOfInterval(now() - INTERVAL 960 SECOND, INTERVAL 15 MINUTE)), 0) AS gap_15m
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 HOUR) < toStartOfInterval(now() - INTERVAL 1860 SECOND, INTERVAL 1 HOUR) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1h FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 HOUR) AND timestamp < toStartOfInterval(now() - INTERVAL 1860 SECOND, INTERVAL 1 HOUR)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_15m FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 HOUR) AND timestamp < toStartOfInterval(now() - INTERVAL 1860 SECOND, INTERVAL 1 HOUR)), 0) AS gap_1h
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 4 HOUR) < toStartOfInterval(now() - INTERVAL 5400 SECOND, INTERVAL 4 HOUR) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_4h FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 4 HOUR) AND timestamp < toStartOfInterval(now() - INTERVAL 5400 SECOND, INTERVAL 4 HOUR)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1h FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 4 HOUR) AND timestamp < toStartOfInterval(now() - INTERVAL 5400 SECOND, INTERVAL 4 HOUR)), 0) AS gap_4h
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 DAY) < toStartOfInterval(now() - INTERVAL 18900 SECOND, INTERVAL 1 DAY) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1d FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 DAY) AND timestamp < toStartOfInterval(now() - INTERVAL 18900 SECOND, INTERVAL 1 DAY)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_4h FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 DAY) AND timestamp < toStartOfInterval(now() - INTERVAL 18900 SECOND, INTERVAL 1 DAY)), 0) AS gap_1d
```

<!-- /0139-postcheck:next-day -->

### Period close (W17)

<!-- 0139-postcheck:period-close -->

```sql
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 WEEK) < toStartOfInterval(now() - INTERVAL 101700 SECOND, INTERVAL 1 WEEK) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1w FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 WEEK) AND timestamp < toStartOfInterval(now() - INTERVAL 101700 SECOND, INTERVAL 1 WEEK)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1d FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 WEEK) AND timestamp < toStartOfInterval(now() - INTERVAL 101700 SECOND, INTERVAL 1 WEEK)), 0) AS gap_1w
SELECT ifNull((SELECT count() FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok') > 0 AND toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 MONTH) < toStartOfInterval(now() - INTERVAL 101700 SECOND, INTERVAL 1 MONTH) AND (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1M FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 MONTH) AND timestamp < toStartOfInterval(now() - INTERVAL 101700 SECOND, INTERVAL 1 MONTH)) = (SELECT (sum(trade_count), sum(volume_base)) FROM prices.price_ohlcv_1d FINAL WHERE timestamp >= toStartOfInterval((SELECT range_from FROM prices.rekey_0139_log WHERE step = 'gap-backfill' AND status = 'ok' ORDER BY at DESC LIMIT 1), INTERVAL 1 MONTH) AND timestamp < toStartOfInterval(now() - INTERVAL 101700 SECOND, INTERVAL 1 MONTH)), 0) AS gap_1M
```

<!-- /0139-postcheck:period-close -->

### Month (task 13, `M=YYYYMM`)

<!-- 0139-postcheck:month -->

```sql
SELECT ifNull(count() = uniqExact(asset_id), 0) AS assets_unique FROM prices.assets FINAL
SELECT ifNull((SELECT count() FROM prices.rekey_0139_reingest_months WHERE month = {m:UInt32} AND colliding_rows > 0) = 0 OR (SELECT count() FROM prices.price_ohlcv_1m WHERE toYYYYMM(timestamp) = {m:UInt32} AND (asset_id IN (SELECT new_id FROM prices.asset_id_map_0139 WHERE status = 'colliding') OR quote_asset_id IN (SELECT new_id FROM prices.asset_id_map_0139 WHERE status = 'colliding'))) > 0, 0) AS colliding_restored
SELECT ifNull((SELECT sum(trade_count) FROM prices.price_ohlcv_1d FINAL WHERE toYYYYMM(timestamp) = {m:UInt32}) = (SELECT sum(trade_count) FROM prices.price_ohlcv_1m FINAL WHERE toYYYYMM(timestamp) = {m:UInt32}), 0) AS month_1d_equals_1m
```

<!-- /0139-postcheck:month -->

## Old tables

GA1 (task 0139, "Decision, 2026-10-01"):

- `rollout_0286_bak_*` (0286 phase-1 snapshots) and `price_ohlcv_*_bak` (0136
  recovery backups): drop once their owners (Oskar, 0136) confirm. They are on
  UInt32 ids and cannot be swapped back into a UInt64 table.

  ```bash
  chq <<'SQL'
  SELECT name, total_rows FROM system.tables WHERE database = 'prices'
    AND (startsWith(name, 'rollout_0286_bak_') OR match(name, '^price_ohlcv_.+_bak$')) ORDER BY name;
  SQL
  # then, per table, after confirmation:
  echo "DROP TABLE prices.<name> SYNC" | chq
  ```

- `reingest_0286_bak_*`: renamed in W9b, kept until task 13 ends, decoded by
  `prices.asset_id_map_0139`.

GA2: `X__pre0139` and `assets__pre0139` hold the colliding and orphan rows that
were not copied. They are dropped **as soon as task 13's second pass verifies**
(no extra retention):

```bash
chq <<'SQL'
SELECT name, total_rows FROM system.tables WHERE database = 'prices'
  AND endsWith(name, '__pre0139') ORDER BY name;
SQL
echo "SELECT 'DROP TABLE prices.' || name || ' SYNC;' FROM system.tables
WHERE database = 'prices' AND endsWith(name, '__pre0139') FORMAT TSVRaw" | chq > ~/rekey-0139/drop-pre.sql
cat ~/rekey-0139/drop-pre.sql && chq < ~/rekey-0139/drop-pre.sql
```

Record the `total_rows` of each before the drop. Keep `asset_id_map_0139` while
any old-id-space table remains (`reingest_0286_bak_*_pre0139`).

## BE note (draft; Adam sends)

> - `asset_id` is now
>   `xxh3(concat(asset_code, ':', issuer_address, ':', contract_address))`, a
>   UInt64 computed by ClickHouse. You can compute it in your own SQL; native XLM is `XLM::`, and case is preserved.
> - `current_price_usd` no longer fans out: one row per `current_prices` row,
>   and its columns are unchanged.
> - ClickHouse JSON output quotes UInt64 by default
>   (`output_format_json_quote_64bit_integers`).
> - Orders that broke ties on `asset_id` change: the `/assets` secondary sort,
>   and `argMaxIf` on `quote_asset_id`.
> - A `GET /assets` cursor walk in flight across the window may skip or repeat
>   rows once.
> - The 3,315 formerly colliding ids carried blends of two assets. Their
>   history is not copied; it returns month by month as the 0286 backfill and
>   its second pass re-ingest those months, under each asset's own id.

## Local development

`init.sql` uses `CREATE TABLE … IF NOT EXISTS`, so an existing local volume
keeps its UInt32 tables. Recreate it once:

```bash
docker compose down -v && docker compose up -d clickhouse
```
