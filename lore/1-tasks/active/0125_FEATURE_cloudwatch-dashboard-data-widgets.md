---
id: "0125"
title: "CloudWatch dashboard — replace the empty prices-production-overview scaffold with real data widgets"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0056", "0093", "0121", "0128", "0026"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M2, observability, cloudwatch, dashboard]
milestone: 2
links:
  - "../../../docs/scf/milestone-1-evidence.md"
  - "../../../infra/src/lib/stacks"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "CloudWatch
      dashboard" row of `milestone-1-evidence.md` Table 4 — the M1 submission
      states `prices-production-overview` exists as "a scaffold with no data
      widgets" and explicitly does not offer it as evidence.
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      **Inherited an acceptance criterion from [[0026]] on its archival.**
      0026's "CloudWatch metrics emitted AND visible in the dashboard" AC
      closed as emit-done / display-deferred; the widget half is now owned
      here. That AC previously pointed at task 0056, which is itself
      archived — a dangling reference corrected during the 0026 close-out.
      Concretely: the six `Prices/Enrichment` metrics
      (`EnrichmentRowsEnriched`, `EnrichmentOracleMiss`,
      `EnrichmentRowsRemainingAtVolumeZero`, `EnrichmentRowsRemainingRecent`,
      `EnrichmentPassDurationMs`, `EnrichmentAvgBatchDurationMs`) already
      publish live and must appear as widgets. ⚠️ Note
      `EnrichmentRowsRemainingAtVolumeZero` climbs permanently by design — it
      tracks the exotic-quote no-reference floor, not a backlog — so do not
      render it as a queue depth or alarm on its growth.
  - date: 2026-09-03
    status: active
    who: akot
    note: >
      Activated. Precondition read against the live account: the dashboard is
      still the 811-byte single-TextWidget scaffold (last modified 2026-07-06),
      while `observability-stack.ts` now defines 18 alarms, not the seven this
      task names — the widget set is built against the current alarm set.
      The two source documents disagree on the §9 widget list: the SCF
      submission (`prices-api-design-after-2nd-review.md:881`) says "DB CPU",
      an RDS-era item ADR 0007 removed; the overview says "ClickHouse write
      latency" + "mTLS cert NotAfter". Building to the overview list and
      naming the substitution in the evidence. Of the six §9 topics only
      ClickHouse write latency has no metric today; the rest already publish
      via the probes. `production-soroban-explorer` (same account) is the
      widget template.
---

# CloudWatch dashboard with real data widgets

## Summary

`prices-production-overview` is deployed but empty. The M1 evidence document
says so directly and declines to screenshot it. The seven alarms behind it are
real and fire-tested (task 0056) — it is only the dashboard that is a shell.

Tranche 3 AC 8 eventually requires *"CloudWatch dashboard accessible to Stellar
team (read-only IAM role); all alarms OK"*, so building it in M2 both closes the
M1 promise and de-risks M3.

## Context

§9 Tranche 3 lists the dashboard content as *"API latency, error rate, ingestion
lag, ClickHouse write latency, mTLS cert NotAfter, backfill progress"* — a good
starting widget list.

The useful constraint: the dashboard should make the **M2 acceptance criteria**
observable, not just be a wall of graphs. If a reviewer cannot see p95 latency,
error rate, and cache hit rate on one screen, it does not serve [[0121]] or
[[0122]].

**The hard part is the non-AWS metrics.** API Gateway, Lambda and alarm state
come free from CloudWatch. ClickHouse-side numbers (write latency, query time,
enrichment lag, backfill frontier) live on **Hetzner**, behind mTLS, on a box
BE owns — there is no metric stream into CloudWatch today. Options, cheapest
first:

1. Extend the existing probe Lambdas (`backfill-freshness-probe`,
   `mtls-notafter-probe` — both already scheduled and already speaking mTLS to
   CH) to emit `PutMetricData` custom metrics. Reuses a proven path; adds a
   small per-metric cost.
2. A new dedicated metrics-probe Lambda. Cleaner separation, more moving parts.
3. Skip CH-side metrics on the dashboard and link out. Cheapest, weakest.

**Recommended: (1)** — the probes already run on a schedule, already hold the
certs, and already query the tables the numbers come from.

## Implementation

- Define the widget set against the §9 list plus the M2 ACs:
  - **API** — request count, p50/p95/p99 latency, 4xx/5xx rate, throttles,
    cache hit/miss ratio ([[0122]])
  - **Lambda** — duration, errors, throttles, concurrency, cold starts, per
    function (api-handler and the workers)
  - **Ingestion** — ledger-processor lag (an alarm already exists at 120s),
    SQS depth / DLQ, invocation errors
  - **ClickHouse** — write latency, query latency, enrichment lag, USD-coverage
    percentage (the 0114 metric worth watching permanently)
  - **Backfill** — `earliest_data_available` and `last_push_at` trajectory, so
    the [[0127]] depth milestone is visible over time rather than sampled
  - **Alarm status strip** — all seven alarms, current state
- Emit whatever custom metrics the above needs via the chosen option; keep the
  metric namespace and dimensions consistent with BE's conventions.
- Define the dashboard in **CDK**, not by hand in the console — it must survive
  a redeploy, and Tranche 3 AC 7 requires `cdk deploy` from a clean account to
  reproduce everything.
- Sensible default time range and periods; a dashboard defaulting to 1h hides
  the backfill trajectory, one defaulting to 2 weeks hides a latency spike.
  Consider splitting real-time vs trend rows.
- Provide the read-only IAM role for external (Stellar) access now — M3 needs
  it, and it is a few lines.

## Acceptance Criteria

- [ ] `prices-production-overview` renders real data in every widget; no empty
      panels *(waits on the deploy — every widget is built against a
      metric+dimension pair verified live on 2026-09-03, but "no empty panels"
      is an observation, not a synth property. `ClickHouseWriteLatencyMs` is
      empty by construction until Compute is deployed and the next ledger
      closes.)*
- [x] Every §9-listed topic has a widget: API latency, error rate, ingestion
      lag, ClickHouse write latency, mTLS NotAfter, backfill progress
- [x] Cache hit rate and p95 latency are visible on one screen (serves
      [[0121]] / [[0122]]) — both are in the row-0 acceptance strip
- [x] ClickHouse-side metrics reach CloudWatch by a documented mechanism;
      the chosen option is recorded with its cost — see Implementation Notes
      and Design Decisions below
- [x] Alarm-status widget shows **all** alarms and their current state — the
      live count is **49**, not the seven this task's text names nor the 18 its
      activation note names; the strip is derived from the construct tree, so
      the number maintains itself
- [x] Dashboard is defined in CDK and survives a redeploy — asserted at synth
      by `npm run infra:verify-dashboard`
- [ ] Read-only IAM role for external viewers exists and is documented *(the
      identity is in the template and documented in the runbook below; it waits
      on the operator to create the console login out of band and hand it over.
      Note the deliberate substitution: an IAM **user**, not a role.)*
- [ ] Screenshot captured for [[0128]] — this is the evidence M1 could not give
      *(waits on the deploy; owned by [[0128]])*

## Notes

- Custom metrics are billed per metric per month. Keep the set deliberate; a
  per-asset metric explosion is the easy mistake here.
- [[0093]] (freshness alarms for backfill + live) overlaps on the probe path.
  Whichever lands first should leave the metric-emission hook in place for the
  other.

## Implementation Notes

What shipped, on `feat/0125_cloudwatch-dashboard-data-widgets`, in three commits.

**The metric.** `ClickHouseWriteLatencyMs`, namespace `Prices/Ingest`, unit
Milliseconds, one dimension `Environment`. No per-table, per-source or per-asset
breakdown — one metric, one dimension.

**Where it is timed.** At the two `write_candles` call sites in
`packages/prices-ledger-processor/src/reconcile.rs` (the SDEX flush and the
per-AMM-source loop), not inside `sink/mod.rs`. Timing at the call site leaves
the `CandleSink` trait signature untouched, covers both impls — `ClickHouseSink`
and the in-memory `CountingSink` behind `--dry-run` and `tests/reconcile_e2e.rs`
— without editing either, and keeps every `#[cfg(feature)]` out of the write
path. Samples are recorded only *after* the `?`, so a failed write is never
measured and no error path changed.

**How it is carried out.** A new `WriteLatency` carrier (count / sum / min / max
in ms) hangs off `RunStats` as an `Option`. `Option`, not a flat struct: a run
that persisted nothing must publish **no datapoint**, and a `Default`-derived
`min_ms` of `0.0` would publish a 0 ms minimum on every idle run and poison the
p50 permanently. The `persisted == 0` early return yields `None`; the success
path yields the carrier only when it holds a sample. `record()` seeds both
bounds from the first sample rather than widening against zero.

**Where it is published.** `packages/prices-ledger-processor/src/metrics.rs`,
built to the same shape as `enrichment-worker::metrics`: a pure mapping function
compiled in every build (so it is unit-testable with no AWS SDK) plus a
`#[cfg(feature = "lambda")] publish()` that pulls `aws-sdk-cloudwatch`. The
default build still refuses to resolve the SDK; the `lambda` build resolves it.
`aws-sdk-cloudwatch` was already a workspace dependency, so the lockfile gained
one line and no new package.

The 1+N INSERTs of an invocation fold into **one StatisticSet** (sample count,
sum, minimum, maximum) rather than 1+N separate values, so the per-invocation
spread survives into CloudWatch. `publish()` returns early on an empty slice —
a `PutMetricData` with no data is an API error.

The call sits in the handler's `Ok(stats)` arm, immediately after the
"doorbell processed" log. That placement is the only safe one and the reason
must survive review: `Ok(stats)` is reachable only if the cursor commit at
`reconcile.rs` succeeded, so the rows and the cursor are already durable, and
the `Err(e)` arm — the sole path that pushes a `BatchItemFailure` — cannot be
reached from code in the `Ok` arm. A failed publish logs a warning and the
invocation still returns success.

**IAM.** The ledger-processor role already carried a `cloudwatch:PutMetricData`
grant conditioned on `PricesApi/LedgerProcessor` — a namespace with zero metrics
in the account and exactly one mention in the whole repo. Its condition *value*
was changed to `Prices/Ingest` and the `sid` renamed to match; no second
statement was added. This is a policy diff, not a resource replacement. The
condition value and `METRIC_NAMESPACE` in `metrics.rs` must stay equal: a drift
makes every publish fail with AccessDenied and leaves the widget empty forever
with nothing failing loudly.

**The widget rows** (`observability-stack.ts`, built at the end of the
constructor so the alarm strip can walk a complete construct tree):

- **Row 0, the acceptance strip** — four `SingleValueWidget`s: API p95 latency,
  the 5xx rate as a percentage, the cache-hit ratio as a percentage, and
  ClickHouse write latency p95. Then the alarm strip beneath. This row alone
  answers Tranche 3 AC 8 and the M2 criteria.
- **Row 1, API** — p50/p95/p99 on one graph; requests with 4xx and 5xx; cache
  hits and misses with the ratio on the right axis.
- **Row 2, ingestion** — ingest-queue oldest-message age, DLQ depth,
  ledger-processor duration/errors/concurrency, and `ClickHouseWriteLatencyMs`
  at p50/p95/Maximum.
- **Row 3, ClickHouse and backfill** — a 14-day, 1-hour trend row: free disk
  percentage, rollup lag across all seven tiers, backfill push age, and mTLS
  days-to-expiry.
- **Row 4, workers** — a 7-day, 1-hour trend row: duration, and errors with
  throttles, across the eight scheduled workers that have Lambda metrics.
- **Row 5, enrichment and oracle** — same window; all six `Prices/Enrichment`
  metrics (the criterion inherited from [[0026]]) plus the oracle pair.

Section headers carry the caveats a reader needs: that API Gateway publishes no
`Throttles` metric (429s land in `4XXError`), that the volume-zero count is
permanent by design, and the Decision A substitution.

**The alarm strip** is derived by walking `this.node.findAll()` for
`cloudwatch.Alarm` instances — 40 today — plus the nine per-worker `-errors`
alarms, imported by ARN. No alarm name is written literally anywhere in the
widget code. A `DashboardAlarmCount` output publishes the coverage so it can be
checked mechanically.

**Two new name helpers**, because a drifted physical name in a metric or alarm
reference does not error — the panel just goes quiet: `restApiName` in
`api-gateway-stack.ts` and `workerErrorAlarmName` in `lambda-baseline.ts`. Both
are now used by the resource that creates the name *and* by the dashboard.

**The viewer identity** is an `iam.User` named
`prices-<env>-stellar-viewer` carrying exactly one managed policy,
`CloudWatchReadOnlyAccess`. No password: `iam.User` creates a login profile only
when one is supplied.

**`tools/scripts/verify-dashboard-synth.mjs`** (`npm run infra:verify-dashboard`)
asserts all of it against the synthesized template.

## Design Decisions

### From Plan

1. **Decision A — the named substitution.** The frozen SCF submission's "DB CPU"
   is served by the ClickHouse **host and write-path** metrics —
   `ClickHouseDiskFreePercent` plus `ClickHouseWriteLatencyMs` — because
   **ADR 0007** replaced RDS with the shared Hetzner ClickHouse cluster. Literal
   DB CPU is not readable anyway: the runtime identities hold no grant on the
   ClickHouse system tables, and host CPU of a volume we share at a few percent
   says nothing about our write path. *That sentence is what the evidence file
   needs, verbatim.* It is also on the dashboard itself, in the row-3 header,
   because the reviewer reads the dashboard rather than the evidence file.
2. **One dashboard, not two.** The real-time rows ride the dashboard's default
   3-hour range; the trend rows carry their own 14-day and 7-day windows in
   widget properties. A second `-trends` dashboard would have split the one
   screen AC 8 asks for.
3. **The viewer is an IAM user with `CloudWatchReadOnlyAccess`, not a
   cross-account role.** Tranche 3 AC 8 says "read-only IAM role"; there is no
   external principal to trust, because none is known — SDF has not named the
   account or identity their reviewer will use. A user with console credentials
   satisfies the intent (read-only access to the dashboard) and is the second
   deliberate substitution, to be named in the evidence the same way Decision A
   is. `CloudWatchReadOnlyAccess` rather than the account-wide `ReadOnlyAccess`,
   which would expose Secrets Manager metadata, S3 listings and Lambda
   configuration to an external reviewer.

### Emerged

4. **Decision B2 passed its gate, with the figures.** `AWS/Lambda Duration` for
   `prices-production-ledger-processor` over the 7 days to 2026-09-03: **106 165
   invocations**, average **238.05 ms**, p95 **354.65 ms**, p99 **412.73 ms**,
   max **2 353.11 ms**, against a ledger cadence of roughly 5 seconds. One extra
   HTTPS call of 20–50 ms is far inside that budget, so the real writer is
   instrumented rather than a canary. ⚠️ B2 is a **deviation from this task's own
   recommendation** — the text above recommends option (1), "extend the probes".
   Adam took B2 on 2026-09-03 for signal quality: it measures the real batch
   INSERT that merge pressure and disk stalls hit first, not a one-row canary
   that only answers "is ClickHouse responsive".
5. **The cost, which acceptance criterion 4 asks for:** roughly 450 000
   `PutMetricData` calls per month at $0.01 per 1 000 requests ≈ **$4.50**, plus
   about $0.30 for the custom metric itself. If that ever needs to drop to cents,
   CloudWatch EMF (a metric extracted from a structured log line) is the standard
   alternative — but it has no precedent in this repo, so it is a follow-up, not
   part of 0125.
6. **The timer includes retries, and a reader of the p99 must know it.**
   `ClickHouseSink::write_candles` wraps the write in `retry_with_backoff`, so a
   call-site timer folds the backoff sleeps into the measurement and a retried
   write reads as one long write. That is defensible signal — the write path
   *was* slow — but a single retry inflates a datapoint non-linearly, so the p99
   is not a pure latency distribution.
7. **The alarm count is 49**, not the seven this task's text names nor the 18 its
   activation note names: 40 constructed by ObservabilityStack plus 9 per-worker
   `-errors` alarms owned by EventBridgeStack. Those nine are imported **by ARN**
   rather than by a cross-stack construct reference, so the two stacks stay
   independently deployable — a CFN reference would have coupled their deploys,
   which the stack's own comments defend against everywhere else. The strip walks
   the construct tree, so the number maintains itself.
8. **Two CDK synth traps that changed the implementation.** The Dashboard's
   `periodOverride` defaults to `Auto`, which silently overrides every per-widget
   period — the one-dashboard design only works with it set to `INHERIT`, and
   nothing fails at synth if you forget, so the trend rows would simply have been
   inert. And setting `defaultInterval` together with `start` on the Dashboard
   throws at synth, so the global range is a default interval while the trend
   windows are per-widget. Both are now asserted by the synth script.
9. **The metric is published as raw `Values`, not as a StatisticSet — because
   the dashboard asks it for percentiles.** The first implementation folded the
   1+N INSERT timings into `MetricDatum.statistic_values` (SampleCount / Sum /
   Minimum / Maximum). That shape exists in `aws-sdk-cloudwatch` 1.116.0 and
   compiles, but CloudWatch keeps no distribution behind a statistic set, so
   **`p50`/`p95` cannot be read back from it** — and `p95` is exactly what the
   row-0 acceptance strip and the ingestion trend row query. Both panels would
   have rendered "No data" for ever, with nothing failing at synth, at deploy or
   at render. So `MetricDatum::builder().set_values(Some(values))` carries the
   samples themselves (`_metric_datum.rs:196` in the installed crate; `Counts`
   omitted, which defaults to 1 per value). The batching rationale survives —
   the samples ride in ONE `PutMetricData` call — but `Values` accepts at most
   **150 entries per datum**, so a longer run spills into further datums of the
   same metric inside the same call. The samples are only real writes: an empty
   candle slice short-circuits inside `write_candles` with no round-trip, so it
   is not timed and cannot drag the minimum toward zero. The synth script now
   fails if any `Prices/Ingest` widget asks for a `pNN` stat while
   `metrics.rs` still publishes via `statistic_values(`.
10. **The alarm strip is a `findAll()` walk, and only a comment enforces when it
    runs.** `stackOwnAlarms` is derived by walking the construct tree at a fixed
    point near the end of the constructor, so **every alarm must already have
    been constructed** — an alarm added below that block is silently missing
    from the strip, and nothing errors. The ordering is defended by a comment
    alone. What catches a drop is the synth assertion `DashboardAlarmCount ==
    own alarm resources in the template + 9 imported`: an alarm that falls out
    of the walk still appears as a resource, so the two numbers diverge and the
    check fails. It does not catch a reordering that removes and adds an equal
    number of alarms, which is why the ordering comment stays load-bearing.

## Future Work

Neither is filed yet — both need a `/lore-framework-tasks` action, and both are
out of scope here because this task changes no alarms.

- **The SDEX push-freshness alarm reads a series that has never existed.**
  `prices-production-sdex-push-freshness` watches `Prices/Backfill`
  `PushAgeSeconds` with `Stream=sdex_archive`, but that metric has only ever
  published under `Stream=soroban_amm` — the probe defines both constants and
  only one has ever emitted. The alarm is therefore reading nothing, and **a
  stalled SDEX backfill would not fire it**. It reports OK because missing data
  is non-breaching, which is exactly how this stayed invisible. The dashboard
  widget uses the AMM stream and so is unaffected; the alarm needs its own task.
- **The `cleanup` worker is deployed and dark.** Its EventBridge rule is
  disabled on purpose (it shredded the 0182/0201 repair campaign), so the
  function publishes no Lambda metrics at all and is excluded from the worker
  row for that reason. Whether it should be removed or re-enabled is a decision
  nobody has taken; leaving it deployed and dark is the current default rather
  than a choice.

## PR #280 review (karczuRF, 2026-09-04)

Five findings, all addressed (the CI step landed last, once the push token
carried the `workflow` scope — see WR-01 below). Fixed: the 5xx-rate tile now
`FILL`s its input like the graph beside it (it rendered `--` on a healthy
quiet day — an M2 exhibit); one `aws_config` load shared by the S3 and
CloudWatch clients, with the publish timeouts on the CloudWatch client's own
config so they never touch ledger downloads; the `SCHEDULED_WORKERS` docstring
no longer claims the health alarms derive from it — instead the stack lists
the three workers without duration/no-invocations alarms (`cleanup`,
`asset-discovery`, `supply`) by name and asserts every worker is in exactly one
of the two sets; the misplaced JSDoc above `MIN_ALARMS` moved to the count it
describes. **Future Work:** health alarms for `asset-discovery` and `supply`
are a decision for [[0256]] (asset-discovery) and its own task (supply), not
this one — the `-errors` alarm is their coverage today.

## Deep review 2026-09-04 (pre-merge, cross-file)

`gsd-code-reviewer` at `deep` over the 11 source files of PR #280: 1 critical,
7 warnings, 5 info. All four brief invariants proved clean (no ledger-processor
error path changed; publish only in the `Ok` arm; `lambda` feature gate tight;
name helpers equal their literals). Fixed in one commit:

- **CR-01 (high)** — the viewer carried `CloudWatchReadOnlyAccess`, which also
  grants `logs:FilterLogEvents`/`logs:Get*` and `xray:Get*` on every log group
  and trace in the account shared with the block explorer. Replaced by an
  inline policy of exactly the CloudWatch `Get*`/`List*`/`Describe*` calls the
  dashboard needs; the synth guard now fails on any managed `*ReadOnlyAccess`
  on the viewer and on any non-read action in its inline policy. (The Chatbot
  role in the same stack keeps that managed policy on purpose.)
- **WR-04 (medium)** — the CloudWatch client had no timeout: a stalled
  endpoint would hold the invocation to the 60 s limit, and with reserved
  concurrency 1 that is ingestion lag. Now connect 1 s / attempt 3 s /
  operation 6 s, 2 attempts.
- **WR-01 (medium)** — `verify-dashboard-synth.mjs` claimed to run in CI and
  did not. Now a step after `Synth production app` in `ci.yml`, plus a
  `cargo clippy --no-deps` gate for `prices-ledger-processor` (IN-05). Landed
  in a separate `ci(lore-0125)` commit once the push token carried the
  `workflow` scope GitHub requires for `.github/workflows/` changes.
- **WR-02/03** — the worker list lived in three places by hand. Single
  source: `SCHEDULED_WORKERS` / `SCHEDULE_DISABLED_WORKERS` in
  `lambda-baseline.ts`; `createWorkerLambda` throws on a name not in it, the
  strip import and the workers row derive from it, and the synth guard reads
  the imported count off the body instead of a constant.
- **WR-05/06** — the percentile check can no longer pass vacuously; threshold
  lines are matched to their own widget by title.
- **WR-07** — missing `ENV_NAME` now logs an error at startup (fallback kept:
  ingestion must not stop over telemetry).
- **IN-01/02/03/04/05** — fourth top-row tile on 24 h too; chunking note;
  `--env` validation; `AWS::IAM::AccessKey` guard; the one clippy nit that
  blocked a `--no-deps` gate on `prices-ledger-processor` fixed (the CI gate
  itself waits with WR-01).

Redeploy needed for both stacks: Compute (client timeouts — rebuild the
bootstrap first, runbook step 0) and Observability (viewer policy, tile).

## Operator Runbook

The criteria code cannot close (1, 7 and 8) close here. In order:

0. **Build the ledger-processor binary first.** `cdk synth` does NOT compile
   it: `compute-stack.ts` packages the pre-built
   `target/lambda/prices-ledger-processor/bootstrap` via `Code.fromAsset`
   (see `docs/runbooks/deploy-ledger-processor.md`). A deploy without this
   step ships the **previous** binary with the new IAM — exactly what
   happened on the first deploy of this task (2026-09-03 12:34 UTC: 0 errors,
   0 `publish failed` warnings, and no metric, because the code that
   publishes was never on the Lambda).

   ```bash
   cargo lambda build -p prices-ledger-processor --release --arm64 --features lambda
   grep -c -a 'Prices/Ingest' target/lambda/prices-ledger-processor/bootstrap   # must be > 0
   ```

   The grep is the check that the asset about to be packaged is the new
   code. Then `npm run infra:synth:production` so the asset hash refreshes.
1. Review `npm run infra:diff:production` on **Compute** and **Observability**.
   Compute should show one policy diff (the `PutMetricData` namespace condition
   and its `sid`) plus the ledger-processor code change; Observability should
   show the dashboard body, one new IAM user and two new outputs.
2. Deploy **Compute FIRST**, then Observability. The metric must exist before a
   widget can show it — deploying Observability first leaves the write-latency
   panel empty and makes AC 1 unobservable for no reason.
3. Wait for the next ledger (~5 s) and confirm the ClickHouse write-latency
   panel fills. If it stays empty, check the ledger-processor logs for
   `cloudwatch metric publish failed` — an AccessDenied there means the IAM
   condition value and `METRIC_NAMESPACE` have drifted apart. **No warning
   and no metric** means the deployed binary predates this task: download
   it (`aws lambda get-function … --query Code.Location`) and grep it for
   `Prices/Ingest`; if absent, go back to step 0.
4. Walk every other widget and confirm it shows data rather than "No data".
   That observation is acceptance criterion 1.
5. Capture the screenshot into `docs/scf/screenshots/` for [[0128]] and write
   the evidence entry with the Decision A substitution (verbatim, from Design
   Decisions above), the mechanism and the cost.
6. Create the console login for the viewer user **out of band** — it is
   deliberately not in the template:
   `aws iam create-login-profile --user-name prices-production-stellar-viewer --password '<generated>' --password-reset-required`.
   ⚠️ The account has **no password policy set**, so AWS defaults apply; and this
   will be the **first IAM user in the account**, which is otherwise SSO-only.
   Hand over the credentials, the user name and the dashboard URL.
7. Close the task (`/lore-framework-tasks`), file the two Future Work items, and
   open the PR to `develop`.
