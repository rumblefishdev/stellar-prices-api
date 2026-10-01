# Runbook — the asset-id migration window (task 0139)

`asset_id` becomes `xxh3(concat(asset_code, ':', issuer_address, ':',
contract_address))`, a `UInt64` that ClickHouse derives from the identity. On
production the old UInt32 counter ids are still in every id-keyed table, and
3,315 of them serve two or three unrelated assets. This runbook moves prod onto
the derived ids in one window:

- `assets` is altered in place (`MODIFY COLUMN … MATERIALIZED`, `MATERIALIZE
  COLUMN`).
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

The operator. The agent holds `dev_read` only and cannot run any step that
writes.

Two shells:

- **Host shell** — the ClickHouse host, as the `default` user over the loopback
  port (connect: [[hetzner-ch-prod-ssh-access]]). The tool runs here because it
  reads `system.view_refreshes`, `system.processes`,
  `system.asynchronous_inserts`, `system.mutations` and `system.query_log`.
  `prices_admin` is denied `system.view_refreshes` (`Code: 497`,
  `0286-candle-definitions-rollout.md` §0), so over mTLS the window gates
  refuse.
- **AWS shell** — a workstation checkout of `develop` at the 0139 merge commit,
  with production AWS credentials. Rules, the event source mapping, deploys and
  CloudWatch.

Set up the host shell once, in `tmux`:

```bash
tmux new -s rekey0139
mkdir -p ~/rekey-0139
read -rs CH_PW
set -o pipefail
rk() {
  CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_USER=default CLICKHOUSE_PASSWORD="$CH_PW" \
    ~/prices-clickhouse-rekey "$@" 2>&1 | tee -a ~/rekey-0139/window.log
}
chq() { docker exec -i app-clickhouse-1 clickhouse-client --multiquery; }
echo "SELECT currentUser(), version()" | chq
```

The password goes in the environment, never in `argv` (`/proc/<pid>/cmdline`
is world-readable). `chq` reads SQL on stdin, the pattern of
`0286-reingest-history.md` §4g. `rk` and `chq` are shell functions: a second
tmux pane needs them defined again.

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
  env -u CARGO_TARGET_DIR cargo build --release -p prices-clickhouse --bin prices-clickhouse-rekey
  env -u CARGO_TARGET_DIR cargo build --release -p sdex-backfill -p events-backfill
  ```

  Copy `target/release/prices-clickhouse-rekey` to the host as
  `~/prices-clickhouse-rekey` (build for the host's architecture, as
  `~/events-backfill` is built today). The new `sdex-backfill` and
  `events-backfill` replace the ones the 0286 orchestrator runs, locally and on
  the host. **Keep the old ones beside them as `*.pre0139`; never overwrite a
  `*.pre0139`:**

  ```bash
  # host shell, for each of events-backfill (and sdex-backfill if it lives there)
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
   record it as the merge commit's first parent: `ROLLBACK_SHA=$(git rev-parse
   <merge>^1)`. Before the merge, it is `origin/develop`. Check it matches
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

Host shell. Writers are running; nothing here touches a live table.

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

Spike 006 estimate, until task 11's rehearsal replaces it: the fill is the long
part, 20–50 min for 1.33 B candle rows (5.8 M rows/s read, 1.0–1.1 M rows/s
insert, measured locally). `EXCHANGE` of 11 tables: milliseconds. With an
online pre-fill, W7 is one probe per partition plus the current month.

<!-- PR8b: gap budget -->

## The window

Times are UTC. Write every recorded value into `~/rekey-0139/window.log` on
the host (the `rk` output already goes there).

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
and partly refilled). Run the same `pgrep` in the host shell. It stays stopped
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

Every line `0` (or `None`: no datapoint). In both shells, nothing manual may
run: `pgrep -af 'coarse-repair|sdex-backfill|events-backfill|pool-registry-seed|prices-clickhouse-init'`
prints nothing.

### W5 — stop the MVs, capture

Host shell:

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
rk capture --execute
```

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

Host shell:

```bash
echo "SELECT count() = uniqExact(asset_id) FROM prices.assets FINAL" | chq   # 1
```

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
jq '.asset_id_uniqueness' ~/rekey-0139/probe.json
aws cloudwatch get-metric-statistics --namespace Prices/Rollup --metric-name AssetIdCollisions \
  --dimensions Name=Environment,Value=production --start-time "$(date -u -d '-10 min' +%FT%TZ)" \
  --end-time "$(date -u +%FT%TZ)" --period 60 --statistics Maximum --query 'Datapoints[].Maximum'
```

The prices are those of W3 (nothing newer is ingested yet). `AssetIdCollisions`
must read `0` (measured as `count() − uniqExact(asset_id)` before the window:
3,321 on 2026-10-01). The probe invocation itself reports an error while the
orphan read refuses on an empty 2-hour window; that is expected until W13.

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

Caught up when every one of the last 5 one-minute maxima is under 120 s (the
`prices-production-ledger-processor-lag` threshold). Record it and compare the
catch-up duration with task 11's prediction:

```bash
CATCHUP_END=$(date -u '+%F %T'); echo "CATCHUP_END=$CATCHUP_END" | tee -a ~/rekey-0139/deployed-before.tsv
```

`AssetIdOrphanCandles` must read 0 once the probe sees live candles again.
From here on, **rollback is forward-fix only**: the old tables lack every row
written since.

<!-- PR8b: W14 -->

### W15 — afterwards

- Adam sends the BE note (draft below) and records the acceptance numbers in
  task 0139.
- The old tables follow "Old tables" below.
- Then task 13 (the colliding history): 0286 phase 3 resumes from the month
  after `PAUSE_MONTH` on the new ids, and the second pass covers every month
  in `prices.rekey_0139_reingest_months` up to and including `PAUSE_MONTH`
  (`0286-reingest-history.md`).

<!-- PR8b: W16 -->

## Rollback

Valid until W13. `rollback` exchanges the 11 tables back with `X__pre0139` (the
UInt64 tables become `X__new` again), swaps `assets` back with
`assets__pre0139` (the altered table becomes `assets__new`), re-creates the
captured MVs and views exactly as captured, and `SYSTEM STOP VIEW`s the MVs. It
refuses if `price_ohlcv_1m` has rows after `last_live_1m_ts`, `oracle_prices`
has rows after the swap, or `assets` gained identities; `--force-lose-post-swap-rows`
overrides that and loses them.

**Before W11** (nothing deployed yet), host shell:

```bash
rk rollback --execute
```

If W9 had run, force an api-handler cold start (its memo may hold new ids; the
`update-function-configuration` line of W11). Then `SYSTEM START VIEW` each of
the seven MVs of W5 and re-enable the writers in W13's order.

**After W11, before W13**:

1. `rk rollback --execute` (host shell).
2. From the rollback worktree (AWS shell):

   ```bash
   cd .claude/worktrees/0139-rollback
   make -C infra deploy-production-compute
   make -C infra deploy-production-eventbridge
   make -C infra deploy-production-observability
   ```

3. Re-check that the ESM is still `Disabled` and the five rules still
   `DISABLED` (the W11 re-check lines). Disable again if needed.
4. Put the `*.pre0139` host binaries back (`cp -p ~/events-backfill.pre0139
   ~/events-backfill`, likewise `sdex-backfill`) and the local ones.
5. Force an api-handler cold start (W11's last line).
6. `SYSTEM START VIEW` for the seven MVs, then re-enable the writers in W13's
   order and watch the catch-up the same way.

**After W13**: forward-fix only.

<!-- PR8b: rollback gap -->

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

> - `asset_id` is now `xxh3(concat(asset_code, ':', issuer_address, ':',
>   contract_address))`, a UInt64 computed by ClickHouse. You can compute it in
>   your own SQL; native XLM is `XLM::`, and case is preserved.
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
