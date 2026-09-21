# Coverage sweep: triage and rollout (task 0100)

The coverage sweep is **layer 3 of 3** of the coverage model:

1. **Layer 1** is `prices.pool_registry` itself: the pools we index.
2. **Layer 2** is `prices-<env>-ledger-processor-unregistered-pool` (task 0291).
   It covers trades the live processor dropped from a contract shaped like a
   venue we already index (Aquarius, Soroswap, Phoenix) that is missing from
   the registry.
3. **Layer 3** is this sweep. Once a week it looks at **every** contract that
   emitted swap- or trade-shaped Soroban events over the trailing ~14 days and
   reports those in neither `prices.pool_registry` nor the committed
   allow-list. That residual may be a venue we do not index at all. SushiSwap
   V3 was one: it traded unseen for months (task 0290).

| Piece                                                              | Stack                             | Deploy target                          |
| ------------------------------------------------------------------ | --------------------------------- | -------------------------------------- |
| Lambda `prices-<env>-coverage-sweep-probe` + rule of the same name | `Prices-production-EventBridge`   | `make deploy-production-eventbridge`   |
| its `prices-<env>-coverage-sweep-probe-errors` alarm               | `Prices-production-EventBridge`   | (same)                                 |
| alarm `prices-<env>-coverage-sweep-unclassified`                   | `Prices-production-Observability` | `make deploy-production-observability` |

- Code: `packages/coverage-sweep-probe/`.
- Allow-list: `packages/coverage-sweep-probe/allowlist.toml`. It is compiled
  into the binary.

⚠ **The sweep reports only. It never registers anything.** A contract leaves
the residual only when a human does one of two things:

- registers it in the registry, with an extractor;
- adds it to the allow-list, with a reason and a task.

Auto-registering a router would double-count volume, because a router's swap
event wraps the pool-level trade we already index.

## 1. Owner and cadence (AC8)

**OWNER: TBD — to be agreed with the team (AC8 open)**

- **Cadence:** every **Monday 05:17 UTC** (`cron(17 5 ? * MON *)`). Each run
  covers `[max_ledger − 221,178, max_ledger]` inclusive, i.e. 221,179 ledgers
  (≈ 14 days, measured 2026-09-21). With a
  14-day window on a weekly run, every week is seen twice, so one failed run
  loses nothing.
- **Service level:** triage a non-zero residual before the next Monday run.
- **Reminder:** while the alarm stays latched, the daily stuck-alarm digest
  (task 0214) re-lists it on the ops channel every day.

## 2. Reading the alarms

### `prices-<env>-coverage-sweep-unclassified`

- **Metric:** `Prices/Coverage` `UnclassifiedSwapEvents`, dimension
  `Environment`. It is the sum of swap/trade-shaped events over the trailing
  14 days, from contracts on no list. `UnclassifiedSwapContracts` (the count of
  those contracts) is published beside it, and nothing alarms on it.
- **Published only when non-zero.** The alarm condition is `Sum >= 1` over
  1-day periods, 1 of 7, with `treatMissingData: NOT_BREACHING`.
- **One datapoint a week holds the alarm in ALARM for about 7 days.**
  7 × 86,400 s = 604,800 s is CloudWatch's maximum evaluation span.
- **Weekly boundary:** the old datapoint can leave the window just as the next
  run publishes. While a residual persists, that can produce an OK→ALARM
  pair of notifications around Monday 05:17 UTC. This is expected, not a flap
  to fix — and **an OK on this alarm is never, by itself, "resolved"**.
- **OK means one of three things:** the last run found nothing, the weekly
  boundary above, **or nothing ran** (the alarm goes back to OK 7 days after
  its last datapoint, whatever the cause). Rule out the last with the
  `-errors` alarm and the `coverage sweep complete` log line below.

### `prices-<env>-coverage-sweep-probe-errors`

The run did not complete. Causes, most likely first:

- **Code 497 `ACCESS_DENIED`:** BE has not yet granted `prices_writer` SELECT
  on `default.soroban_events` / `default.soroban_contracts` (§4.1). Every run
  fails until they do. That is intended: a swallowed error would publish
  nothing and read green.
- **`TIMEOUT_EXCEEDED`:** the client bounds each of the run's two statements
  at 50 s (`SWEEP_MAX_EXECUTION_SECS`; 2 × 50 s < the 120 s Lambda timeout).
  The measured run takes 9.6 s.
- **A Lambda timeout** (`Task timed out` in the log, no ClickHouse code):
  counted in `Errors` like the rest, but with no cause attached.
- **An allow-list that fails validation at cold start.** This should be
  impossible after `cargo test`, but it fails Init if it happens.
- **A `PutMetricData` failure.** It is propagated on purpose.

This alarm is 1-day periods, **1 of 7**: a failed Monday run keeps it in
ALARM until the next Monday run can clear it.

There is **no liveness alarm**. A `-no-invocations` alarm needs three
cadences, which is 21 days, and CloudWatch evaluates at most 7. To confirm the
probe ran, look for the Monday INFO line **`coverage sweep complete`** in
`/aws/lambda/prices-<env>-coverage-sweep-probe`. It is logged on every run,
including the ones that find nothing. It carries these fields:

- `max_ledger`, `lo`, `hi`
- `rows`, `allowlisted`, `unclassified`, `unclassified_events`
- `unmatched_allowlist`

## 3. Triage

Run a CloudWatch Logs Insights query over
`/aws/lambda/prices-<env>-coverage-sweep-probe` for the last 7 days:

```text
fields @timestamp, fields.contract, fields.wasm, fields.events, fields.txs,
       fields.first_ledger, fields.last_ledger, fields.top_action, fields.top_shape
| filter fields.message = "unclassified swap emitter"
| sort fields.events desc
```

The query assumes `init_tracing`'s JSON layout, with structured fields under
`fields`. If your log group flattens them, drop the `fields.` prefix.

There is one WARN line per contract. Each contract ends in exactly one of
three outcomes:

- **(a) A pool of a venue we already index.** Register it
  (`docs/runbooks/seed-pool-registry.md`) and reprice its range. Layer 2 may
  have fired for it too.
- **(b) A venue we do not index.** Open a lore task (task 0290 is the
  precedent). Meanwhile, add a `[[wasm]]` entry for the family with
  `until = "<that task>"`, and remove the entry when that task registers the
  family. A `[[wasm]]` entry without `until` fails validation on purpose: a
  permanent family-wide ignore would hide the next pool of a real venue.
- **(c) A router, aggregator or other non-AMM contract.** Add a permanent
  `[[contract]]` entry with `reason` and `task`. Never register it.

Also look at:

- **`unresolved:<id>` rows.** BE's `soroban_contracts` has no row for that
  surrogate id, which is usually BE lag. Check it again next week, and ask BE
  if it persists.
- **`unmatched_allowlist`** on the INFO line. It lists entries that matched
  nothing this run. Those are stale entries and candidates for pruning,
  especially an `until` entry whose task has shipped.

**Editing the allow-list** is a PR against
`packages/coverage-sweep-probe/allowlist.toml`:

- `cargo test -p coverage-sweep-probe` validates it: reason and task on every
  entry, `until` on every `[[wasm]]` entry, valid `C…` strkeys, 64-lowercase-hex
  hashes, no duplicates, no unknown keys.
- The list is compiled into the binary, so an edit **ships only with an
  EventBridge deploy** (§4.4).

> ⚠ **Phase-1 note: the first run WILL alarm.** The residual measured on
> 2026-09-21 was deliberately left off the allow-list, because classifying it
> is phase 1 of task 0100. It is:
>
> - `8abc2891…`, a `POOL`/`swap` Comet-style shape with 730 events, the
>   largest candidate;
> - an 11-family tail of 1–63 events each: `AtomicSwapV2`,
>   `NewBuyInitiatedTrade`, `SoroswapAggregator`, `lifi_swap`,
>   `swap_executed`, `swap_exact`, and others (the baseline-residual
>   table in task 0100).
>
> That ALARM is the end-to-end proof (§4.5), not a fault.

## 4. Rollout (deploy-gated)

Each step names who runs it. **Status (2026-09-21):** §4.1 and §4.2 are
done — BE applied the two grants the same day (0477-style), and every §4.2
check passed as `prices_writer`: `SHOW GRANTS` lists the two new lines,
both tables return `0`, `default.transactions` still returns Code 497, and
`max(ledger_sequence)` reads 47 rows from part metadata (assumption A3
holds). The full sweep statement as `prices_writer` over the 14-day window
took ~16 s end to end. `coverageSweepEnabled` is therefore `true`. §4.4
onwards has not been run.

### 4.1 The BE request (Adam sends it to BE)

Ready-to-send text:

> **prices-api task 0100: two read grants for `prices_writer`**
>
> Please add exactly these two lines to the existing `<grants>` block of
> `prices_writer` in `crates/db-clickhouse/users.d/services.xml`:
>
> ```xml
> <query>GRANT SELECT ON default.soroban_events</query>
> <query>GRANT SELECT ON default.soroban_contracts</query>
> ```
>
> Scope: **those two tables only, not `default.*`**. `prices.pool_registry`
> is already covered by the existing `prices.*` grant. There is no new user,
> certificate, CN-map entry, profile or quota.
>
> Cost per run, measured 2026-09-21: 218.5 M rows / 46.6 GB read, 9.6 s,
> 141 MB memory. It runs **once a week**, Monday 05:17 UTC, one statement at a
> time (reserved concurrency 1, no retries). Each statement is bounded
> client-side at 50 s (`max_execution_time`).
>
> Why: layer-3 coverage (prices-api task 0100). A weekly sweep of swap-shaped
> events for venues we do not index, after SushiSwap V3 traded unseen for
> months (task 0290). The probe only reads.

**How BE applies it: NOT a plain `--tags app` deploy.** The precedent is BE
task **0477** (`soroban-block-explorer`
`lore/1-tasks/archive/0477_OPS_prices-writer-monitoring-grants.md`, commit
`4212937e`, PR #399 → release #402), which added `system.mutations` /
`system.view_refreshes` to the same user. `services.xml` is bind-mounted into
the ClickHouse container **as a single file**. Ansible writes it by
write-and-rename (a new inode), so the container keeps reading the old one and
**a deploy alone silently applies nothing**. A SQL `GRANT` is impossible too,
because `users_xml` storage is read-only. 0477 therefore:

1. committed the change in the BE repo (`services.xml` and the
   `prices_writer` row of `docs/architecture/security/clickhouse-rbac.md`);
2. on the box, `diff`-ed the live file against the new one (only the added
   lines may differ), kept a backup, and overwrote the mounted file **in place**
   (`cat new > services.xml`, which keeps the inode; not `sed -i`, `scp` or an
   editor), checking `stat -c %i` before and after;
3. checked the config-reload line in the ClickHouse log and
   `SHOW GRANTS FOR prices_writer`;
4. merged, so the repo matches the box byte for byte and the next
   `--tags app` is a no-op.

Rollback is the same in-place overwrite with the backup, then a revert.
Whether the inode trap still exists is BE's to confirm (0477 left the Ansible
fix unowned).

### 4.2 Read-only verification after BE deploys (Adam)

Run these with the **`prices_writer` certificate** over mTLS, in the curl shape
of `docs/runbooks/0151-zero-invariant-probe-rollout.md` ("Where these commands
run"), but with the `prices_writer` cert and key in place of `dev_read`. Run
**SELECT/SHOW only**. Put one statement per request in `query.sql`.

| Statement                                                                          | Expected                                                                      |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `SHOW GRANTS`                                                                      | the existing lines **plus** the two `GRANT SELECT ON default.soroban_*` lines |
| `SELECT count() FROM default.soroban_contracts WHERE 0`                            | `0`, **not** Code 497                                                         |
| `SELECT count() FROM default.soroban_events WHERE 0`                               | `0`, **not** Code 497                                                         |
| `SELECT count() FROM default.transactions WHERE 0` (scope check: a third BE table) | **still Code 497**                                                            |
| `SELECT max(ledger_sequence) FROM default.soroban_events`                          | the chain tip; read `read_rows` in the `X-ClickHouse-Summary` header          |

The last row is the cost check for the probe's first statement. It is expected
to be answered from part metadata, with a small `read_rows`. If it reads
billions of rows, record that in task 0100 **before** deploying (RESEARCH
assumption A3), and bound it.

### 4.3 The trade-off of decision D5, option A

The probe reuses the ingestion identity (`prices_writer`) rather than a new
one. The cost of that:

- **Wider read access.** The ledger processor and every scheduled worker, about
  10 Lambdas, use the ingestion certificate. All of them can now read these two
  BE tables. The tables hold public chain data, and the grant is read-only.
- **Shared quota.** The weekly scan counts against the `prices_write` quota,
  whose reads are unlimited.

A **dedicated read-only identity** is a later clean-up under **task 0258**. On
our side only the coverage probe's `mtlsSecretName` argument in
`infra/src/lib/stacks/eventbridge-stack.ts` changes. BE's side is a new user
and a CN-map entry.

### 4.4 Deploy (Adam only; the agent never deploys)

Preconditions:

- `coverageSweepEnabled` in `infra/envs/production.json` is `true` (§4.2
  passed on 2026-09-21). It exists so the rule can be switched off durably:
  with `false` the Lambda, the rule (DISABLED) and both alarms still deploy,
  but nothing invokes the probe. Use `false` whenever the grants are not
  verified live (a new environment, a BE change that drops them).
- `make -C infra diff-production` has been read **in full, for removals too**.
  `--require-approval broadening` prompts only on IAM/security-group widening,
  so it does not prompt on removals.

⚠ **Deploying `Prices-production-EventBridge` ships every other undeployed
EventBridge change on develop.** As of 2026-09-21 that includes 0151's
zero-invariant probe and the 0286 rollout ordering, and each carries its own
preconditions. Check them before you deploy.

⚠ **Deploying `Prices-production-Observability` likewise ships every other
undeployed Observability change on develop** (other tasks' alarms and their
actions). Read its part of the diff with the same care; it is not only this
task's alarm.

The CleanupRule hazard (task 0200) is guarded at synth by
`assertCleanupRuleStaysDisabled`, but still read the diff for it.

Order: **EventBridge before Observability, or in the same session.** An alarm
on a metric nobody publishes reads OK under NOT_BREACHING.

```bash
export CARGO_BUILD_JOBS=4               # build-lambdas is a full cargo build; this desktop has OOM'd on it
make -C infra diff-production
make -C infra deploy-production-eventbridge      # Lambda + weekly rule + -errors alarm
make -C infra deploy-production-observability    # the unclassified alarm
```

### 4.5 First run and the deliberate proof (AC5)

Invoke the probe once by hand rather than waiting for Monday:

```bash
aws lambda invoke --function-name prices-production-coverage-sweep-probe /dev/stdout
```

Expect:

- one WARN `unclassified swap emitter` line per contract of the unlisted
  residual (§3 phase-1 note). A local dry run of the probe's code on
  2026-09-21 (window 64,322,610–64,543,788) gave **13 contracts / 911
  events**, led by `CAS3FL6T…` (`8abc2891`, `POOL / swap`, 727 events), and
  no unmatched allow-list entry;
- the INFO `coverage sweep complete` summary;
- `prices-production-coverage-sweep-unclassified` going to **ALARM** on the ops
  topic within the evaluation window.

That ALARM is the end-to-end proof. Record it, with the contract count and
event sum, in task 0100.

### 4.6 Rollback

- Stop the runs: set `coverageSweepEnabled` to `false` and redeploy
  EventBridge. `aws events disable-rule --name prices-production-coverage-sweep-probe`
  works at once, but the next EventBridge deploy re-enables the rule while the
  flag is `true`; follow it with the flag change.
- Or remove the probe completely: revert the task-0100 commits and redeploy
  both stacks.
- BE's two grants can stay, because they are read-only.

## 5. Back-test (AC7: manual, read-only, `dev_read`)

This shows that the sweep would have caught SushiSwap V3 in April 2026.

1. In a scratch directory (**never commit the output**), render the statement:

   ```bash
   cargo run -q -p coverage-sweep-probe --example render_sweep_sql > query.sql
   ```

2. POST it with the **`dev_read`** certificate, in the 0151 runbook's curl
   shape. Pass the bounds as typed parameters:

   ```bash
   curl --fail-with-body -sS --cert "$READ_CERT" --key "$READ_KEY" --cacert "$PRICES_CA" \
     "https://ch.sorobanscan.rumblefish.dev/?param_lo=61926675&param_hi=62147853&default_format=TSVWithNames" \
     --data-binary @query.sql
   ```

   - `lo = 61,926,675` is the first ledger of 2026-04-02 (from task 0286's
     local backfill).
   - `hi = lo + 221,178` (inclusive, so 221,179 ledgers — the probe's window).

3. **Expected:** the `003710b3…` family (SushiSwap V3 pools, task 0290)
   appears among the rows. The SQL does not apply the allow-list; the probe
   subtracts it in Rust. The Rust subtraction of that family is pinned by
   `coverage_sweep_it::the_sushiswap_family_is_reported_without_its_wasm_entry`.
4. Record the contracts and events in task 0100.

**Run on 2026-09-21** with the probe's own code (`run_sweep`'s SQL and
`partition`) as `prices_writer`, window 61,926,675–62,147,853: with the two
`[[wasm]]` entries removed, **32 SushiSwap V3 contracts / 3,125 events**
come out unclassified, the largest pool (`CCR2CH4G…`, 2,829 events) first of
41 rows. With the embedded list they are all subtracted. AC7 holds.

The same window surfaced the **Soroswap factory** (`CA4HEQTL…`,
`SoroswapFactory / new_pair`): its topic contains `swap`, so every new
Soroswap pair would have paged. It is now a permanent `[[contract]]` entry.
Other April-only candidates for phase 1: `Swap` (`d4b4976b…`, 462 events),
`SwappedToVUsd` (`a757a1ed…`, 355) and `tokens_swapped_event`.

## 6. Related

- Tasks:
  - 0100 (this sweep)
  - 0290 (SushiSwap V3)
  - 0291 (layer 2)
  - 0285 (registry vs. what trades)
  - 0087 (Aquarius router)
  - 0258 (dedicated read-only identity)
  - 0200 (CleanupRule hazard)
  - 0214 (stuck-alarm digest)
  - 0275 (`#[ignore]` vocabulary)
- Runbook: `docs/runbooks/seed-pool-registry.md`
