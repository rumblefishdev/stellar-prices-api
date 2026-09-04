import * as cdk from 'aws-cdk-lib';
import * as chatbot from 'aws-cdk-lib/aws-chatbot';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import * as cw_actions from 'aws-cdk-lib/aws-cloudwatch-actions';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as subscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import {
  workerErrorAlarmName,
  workerFunctionName,
  SCHEDULED_WORKERS,
  SCHEDULE_DISABLED_WORKERS,
} from '../lambda-baseline.js';
import {
  ingestDlqName,
  ingestQueueName,
  ledgerProcessorFunctionName,
} from './compute-stack.js';
import { restApiName } from './api-gateway-stack.js';

/**
 * Physical name of the Tranche-1 ops-notification SNS topic (task 0056).
 * Single source of truth: `ObservabilityStack` creates the topic under this
 * name, and `EventBridgeStack` imports it by the same deterministic name to
 * wire its probe `-errors` alarms — no cross-stack CFN reference (and thus no
 * deploy-ordering coupling) between the two stacks.
 */
export function opsAlarmsTopicName(envName: string): string {
  return `prices-${envName}-ops-alarms`;
}

export interface ObservabilityStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * The platform-metric alarms this module adds per scheduled worker (task 0112).
 *
 * Deliberately does NOT include an invocation-errors alarm: `createWorkerLambda`
 * already creates `prices-{env}-{name}-errors` for every worker. That alarm was
 * never the gap — the gap was that most workers never passed
 * `errorAlarmActions`, leaving it inert. Adding a second alarm here would
 * collide on `alarmName` and mask the real defect instead of fixing it.
 */
export interface WorkerHealthAlarms {
  /** Duration approaching the configured timeout — warns BEFORE it becomes errors. */
  readonly duration: cloudwatch.Alarm;
  /** Zero invocations — the worker is not running at all. */
  readonly noInvocations: cloudwatch.Alarm;
}

/** Inputs for {@link addWorkerHealthAlarms}. */
interface WorkerHealthAlarmProps {
  /** Worker name as passed to `createWorkerLambda` (e.g. `enrichment`). */
  readonly name: string;
  /** CDK construct-id prefix (e.g. `Enrichment`). */
  readonly idPrefix: string;
  /** Deployed Lambda function name — `prices-${env}-${name}`. */
  readonly functionName: string;
  /** The worker's configured Lambda timeout. */
  readonly timeout: cdk.Duration;
  /** How often EventBridge invokes it — sets the evaluation windows. */
  readonly cadence: cdk.Duration;
  /** Appended to each alarm description: what breaks when this worker stops. */
  readonly impact: string;
}

/**
 * Create the platform-metric health alarms for one scheduled worker.
 *
 * WHY THIS EXISTS (task 0112)
 * ---------------------------
 * `prices-production-enrichment` failed 72/72 invocations per day for four
 * consecutive days (2026-07-14 → 07-17), `Duration.Maximum` pinned at the 300 s
 * wall, and nobody was told. It was found by hand six days after it recovered.
 *
 * Two separate defects produced that silence, and they need different fixes:
 *
 * 1. **The errors alarm existed but was inert.** `createWorkerLambda` creates
 *    `prices-{env}-{name}-errors` for every worker, but only the two probes
 *    passed `errorAlarmActions`. Enrichment's alarm almost certainly went to
 *    ALARM on 07-14 and notified no one. Fixed by wiring the action — NOT by
 *    adding another alarm here, which would collide on `alarmName`.
 *
 * 2. **The progress alarm cannot see a dead worker.** `enrichmentBacklogAlarm`
 *    reads `Prices/Enrichment` metrics the worker publishes at the END of a
 *    pass. A worker killed by its timeout never reaches that publish call, so
 *    it emits nothing — and `treatMissingData: NOT_BREACHING` scores "nothing"
 *    as healthy. It can detect "ran and made no progress", never "did not run".
 *    The same defect applies to `sdexPushFreshnessAlarm` (Prices/Backfill) and
 *    `mtlsNotAfterAlarm` (Prices/Mtls).
 *
 * This function adds the two alarms nothing else covers, both on `AWS/Lambda`
 * metrics the platform emits whether or not our code survives:
 *
 * - **duration** — the one that would have PREVENTED the outage rather than
 *   reported it. Enrichment's batch cost climbed for days before crossing the
 *   wall, so a threshold at 80% of the timeout converts a silent degradation
 *   into days of warning.
 * - **noInvocations** — catches a disabled or deleted schedule rule, which the
 *   errors alarm cannot see (no invocations means no error datapoints either).
 *
 * The pattern is not new: the ledger-processor's no-invocations alarm below
 * already does exactly this, `treatMissingData: BREACHING` and all. It simply
 * was never applied to the scheduled workers.
 */
function addWorkerHealthAlarms(
  scope: Construct,
  envName: string,
  snsAction: cw_actions.SnsAction,
  props: WorkerHealthAlarmProps,
): WorkerHealthAlarms {
  const { name, idPrefix, functionName, timeout, cadence, impact } = props;

  const metric = (
    metricName: string,
    statistic: string,
    period: cdk.Duration,
  ) =>
    new cloudwatch.Metric({
      namespace: 'AWS/Lambda',
      metricName,
      dimensionsMap: { FunctionName: functionName },
      statistic,
      period,
    });

  // 80% of the timeout. A worker creeping toward its limit is the leading
  // indicator; once it crosses, every run fails and the errors alarm is
  // reporting an outage that already started.
  const durationThresholdMs = Math.floor(timeout.toMilliseconds() * 0.8);
  const duration = new cloudwatch.Alarm(
    scope,
    `${idPrefix}WorkerDurationAlarm`,
    {
      alarmName: `prices-${envName}-${name}-duration-near-timeout`,
      alarmDescription: `The ${name} worker is running at ≥80% of its ${timeout.toHumanString()} Lambda timeout (Duration.Maximum ≥ ${durationThresholdMs} ms for two consecutive periods). It has not failed yet, but it is trending at the wall and will start timing out. ${impact} Investigate before it becomes an outage — this is the warning enrichment did not have in 2026-07.`,
      metric: metric('Duration', 'Maximum', cadence),
      threshold: durationThresholdMs,
      evaluationPeriods: 2,
      datapointsToAlarm: 2,
      comparisonOperator:
        cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    },
  );
  duration.addAlarmAction(snsAction);
  duration.addOkAction(snsAction);

  // Three cadences of total silence: the schedule rule was disabled, deleted,
  // or is failing to invoke. Lambda publishes NO Invocations datapoint for a
  // period with zero invocations, so a LESS_THAN threshold on the raw metric
  // would never evaluate.
  //
  // `FILL(invocations, 0)` (task 0222) turns that absence into real zero
  // datapoints, so the alarm evaluates ordinary data and `treatMissingData` is
  // no longer load-bearing. This is a FIX, not a tidy-up: on 2026-08-25 the
  // coarse sweep was held silent for three full hourly periods and the
  // BREACHING-only form did NOT fire — verified against the metric, with the
  // alarm's configuration confirmed correct at the time. Why it did not fire is
  // still unexplained; this removes the dependency on that behaviour instead of
  // tuning around it.
  //
  // Two properties were verified with get-metric-data before relying on this,
  // both against the 2026-08-25 window itself:
  //
  //   1. FILL extends past the LAST datapoint, not merely between two (window
  //      ending inside the gap: raw 3 points, filled 6). An alarm's window
  //      always ends at ~now, so a real halt is always a TRAILING gap.
  //   2. FILL needs NO anchor at all. Over 09:00-12:00, which contains not one
  //      raw datapoint, `raw` returned 0 values and `FILL` returned 3 zeros.
  //
  // (2) is the load-bearing one and is easy to miss: the alarm's own evaluation
  // range during a sustained halt contains no raw data whatsoever, so a FILL
  // that required a datapoint to anchor to would have been a no-op in the exact
  // case this exists for — while still passing a test whose window happened to
  // include healthy periods. Raised in review on PR #247 and settled by
  // measurement, not by argument.
  //
  // Expressed as three periods of one cadence rather than one period of three
  // cadences, which would be equivalent here but exceeds CloudWatch's 86400 s
  // maximum alarm period for the daily mtls probe (3 × 1 day = 259200 s). That
  // is a DEPLOY-time validation failure, not a synth-time one, so it would have
  // passed every local check and failed in CloudFormation.
  //
  // Three periods (not one) also absorbs a deploy window or a delayed datapoint.
  const noInvocations = new cloudwatch.Alarm(
    scope,
    `${idPrefix}WorkerNoInvocationsAlarm`,
    {
      alarmName: `prices-${envName}-${name}-no-invocations`,
      alarmDescription: `The ${name} worker recorded zero invocations for three consecutive ${cadence.toHumanString()} periods despite being scheduled at that cadence: the EventBridge rule is disabled or deleted, or invocation is failing before the function runs. ${impact} Fires on the ABSENCE of activity, which neither the -errors alarm (no invocations means no error datapoints) nor any custom-metric alarm can do.`,
      metric: new cloudwatch.MathExpression({
        expression: 'FILL(invocations, 0)',
        usingMetrics: { invocations: metric('Invocations', 'Sum', cadence) },
        period: cadence,
        label: 'InvocationsFilled',
      }),
      threshold: 1,
      evaluationPeriods: 3,
      datapointsToAlarm: 3,
      comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
      // Retained as a backstop, no longer the mechanism. FILL yields zeros only
      // where the metric has published at some point; a function that has NEVER
      // been invoked still produces no series at all, and BREACHING is the right
      // answer there too — a worker that has never run is also a halt.
      treatMissingData: cloudwatch.TreatMissingData.BREACHING,
    },
  );
  noInvocations.addAlarmAction(snsAction);
  noInvocations.addOkAction(snsAction);

  return { duration, noInvocations };
}

/**
 * CloudWatch dashboard scaffold for prices-api.
 *
 * The dashboard itself is skeleton-only (a header TextWidget); the real
 * widget set is still task 0056's dashboard work. The alarms, however, are
 * live here:
 *
 * - SDEX push-freshness (Prices/Backfill `PushAgeSeconds`, §5.6 / AC #5)
 * - mTLS NotAfter (Prices/Mtls `MinDaysToNotAfter`, §7 / §11.4)
 * - enrichment-stall (Prices/Enrichment progress signal, task 0026)
 * - live ledger-processor lag / errors / DLQ (AWS-native metrics, finding B):
 *   the core ingestion Lambda's `lag_seconds > 60s` intent (ADR 0007 /
 *   task 0038) is realised via the ingest queue's oldest-message age, since
 *   the processor emits no custom lag metric.
 *
 * Still deferred to the dashboard-widget half of 0056: API Gateway 5xx rate
 * and ClickHouse write latency.
 *
 * The `dashboard` property is exposed so the widget work can call
 * `addWidgets(...)` without cross-stack imports.
 *
 * The log-group naming convention is intentionally NOT redefined
 * here — it lives in `lib/lambda-baseline.ts` (`lambdaLogGroupName`)
 * alongside the per-Lambda role helpers. ObservabilityStack consumes
 * those names when alarms attach to log-group metric filters in
 * 0056.
 */
export class ObservabilityStack extends cdk.Stack {
  public readonly dashboard: cloudwatch.Dashboard;
  /**
   * Enrichment stall alarm (progress-based: `EnrichmentRowsEnriched` = 0 while
   * the recency-bounded `EnrichmentRowsRemainingRecent` > 0, task 0026). The
   * broader dashboard widget set remains task 0056's; this single alarm ships
   * with 0026 because the metrics it watches are emitted by the enrichment
   * worker in the same task.
   */
  public readonly enrichmentBacklogAlarm: cloudwatch.Alarm;
  /**
   * Tranche-1 ops-notification SNS topic (task 0056). All prices-api ops alarms
   * publish here; operators subscribe (email/Slack/PagerDuty) directly in SNS
   * without a redeploy. One address can be seeded via
   * `config.opsAlarms.notificationEmail`.
   */
  public readonly opsAlarmsTopic: sns.Topic;
  /**
   * AWS Chatbot Slack subscription for the ops-alarms topic (task 0056).
   * Created only when `config.opsAlarms.slack` is set; undefined otherwise.
   */
  public readonly opsAlarmsSlackChannel?: chatbot.SlackChannelConfiguration;
  /** SDEX push-freshness alarm (§5.6 / Tranche-1 AC #5). */
  public readonly sdexPushFreshnessAlarm: cloudwatch.Alarm;
  /** mTLS client-cert expiry alarm (§7 / §11.4). */
  public readonly mtlsNotAfterAlarm: cloudwatch.Alarm;
  /**
   * One rollup-staleness alarm per OHLCV granularity, keyed by table name
   * (task 0137). Keyed rather than a flat list so a caller can assert on a
   * specific tier without depending on declaration order.
   */
  public readonly rollupFreshnessAlarms: Record<string, cloudwatch.Alarm>;
  /** ClickHouse host free-space alarm (task 0204, gap 1). */
  public readonly chDiskFreeAlarm: cloudwatch.Alarm;
  /** Live ledger-processor ingestion-lag alarm (task 0056 finding B). */
  public readonly ledgerProcessorLagAlarm: cloudwatch.Alarm;
  /** Live ledger-processor invocation-error alarm (task 0056 finding B). */
  public readonly ledgerProcessorErrorAlarm: cloudwatch.Alarm;
  /** Live ledger-processor DLQ-depth alarm (task 0056 finding B). Rung 1. */
  public readonly ledgerProcessorDlqAlarm: cloudwatch.Alarm;
  /**
   * Escalation rungs above {@link ledgerProcessorDlqAlarm}, keyed by depth as a
   * string (task 0204, gap 2). Each depth is a separate alarm so a growing DLQ
   * keeps producing Slack messages instead of latching silently at `>= 1`.
   */
  public readonly ledgerProcessorDlqEscalationAlarms: Record<
    string,
    cloudwatch.Alarm
  >;
  /**
   * USDT-quoted candles valued as if the $1 peg still held (task 0204, gap 4),
   * keyed by the count that rung fires at. Correctness, not liveness — every
   * other alarm in this stack scores this data perfectly healthy.
   */
  public readonly usdPegAppliedAlarms: Record<string, cloudwatch.Alarm>;
  /**
   * USDT-quoted candles left at `close_usd = 0` past the enrichment grace
   * period (task 0204, gap 4), keyed by count. The inverse direction, added
   * because task 0182's own repair produced it.
   */
  public readonly usdStrandedAlarms: Record<string, cloudwatch.Alarm>;
  /**
   * A rollup MV that has lost `APPEND` (task 0204, gap 3) — history destroyed
   * on every refresh. Separate from {@link mvDriftAlarm} because this is the
   * only drift severity that compounds while nobody looks.
   */
  public readonly mvDriftCriticalAlarm: cloudwatch.Alarm;
  /** A rollup MV whose definition no longer matches `rollups.sql` (gap 3). */
  public readonly mvDriftAlarm: cloudwatch.Alarm;
  /** The drift check could not see the schema at all (gap 3) — likely a grant. */
  public readonly mvDriftUnreadableAlarm: cloudwatch.Alarm;
  /** Live ledger-processor total-halt alarm — zero invocations (finding B / halt gap). */
  public readonly ledgerProcessorNoInvocationsAlarm: cloudwatch.Alarm;
  /**
   * The oracle poll feed wrote nothing for 30 minutes (task 0231). The symptom
   * alarm: it catches every cause, including ones not yet imagined.
   */
  public readonly oracleDarkFeedAlarm: cloudwatch.Alarm;
  /**
   * A Reflector reading was refused by the 0227 plausibility guard (task 0231).
   * The cause alarm: it names *why* the feed is going dark.
   */
  public readonly oracleTimestampRejectedAlarm: cloudwatch.Alarm;
  /**
   * Platform-metric health alarms for the scheduled workers whose only other
   * alarm reads a metric the worker itself publishes (task 0112). Keyed by
   * worker name: `errors`, `duration`, `noInvocations`.
   */
  public readonly workerHealthAlarms: Record<string, WorkerHealthAlarms>;

  constructor(scope: Construct, id: string, props: ObservabilityStackProps) {
    super(scope, id, props);

    const { config } = props;

    // Construct id and `dashboardName` are load-bearing and must not change: a
    // rename replaces the CFN resource and changes the console URL handed to the
    // Stellar reviewer (Tranche 3 AC 8). The widget set itself is built at the
    // END of this constructor — it derives its alarm strip by walking the
    // construct tree, so it has to run after the last alarm exists.
    this.dashboard = new cloudwatch.Dashboard(this, 'OverviewDashboard', {
      dashboardName: `prices-${config.envName}-overview`,
      // Global range for the real-time rows. NOT `start`: setting both throws
      // at synth (`BothPropertiesDefaultIntervalStart`). Trend rows carry their
      // own longer window per widget instead.
      defaultInterval: cdk.Duration.hours(3),
      // Without this the per-widget periods below are silently overridden by
      // CloudWatch's Auto period — the whole trend-row design goes inert and
      // nothing fails at synth. Default is AUTO; it must be INHERIT.
      periodOverride: cloudwatch.PeriodOverride.INHERIT,
    });

    // Enrichment stall alarm (task 0026 / spec §5, re-designed in 0056). Fires
    // on *lack of progress*, not absolute backlog: a pass that enriched zero
    // rows (`EnrichmentRowsEnriched < 1`) while a *recent* volume_quote_usd=0
    // backlog still exists (`EnrichmentRowsRemainingRecent > 0`), sustained
    // across 3 consecutive hourly passes. That is the fingerprint of a genuine
    // stall (oracle↔asset-id mis-reconciliation, missing USDC/USDT/XLM reference
    // assets) — enrichment is *stuck*, not merely behind.
    //
    // Why not the old `backlog Maximum > 100_000` scaffold: it latched with no
    // path back to OK. (a) A legitimate multi-million-row post-backfill drain
    // sits above any absolute threshold for hours while enriching fine; (b) the
    // permanent exotic-quote floor (quote ∉ {USDC,USDT,XLM}, no oracle) never
    // drains by design, so once it exceeds the threshold the alarm is stuck in
    // ALARM forever. The progress-based signal instead clears the moment a pass
    // enriches ≥1 row again, so a draining catch-up and a steady-state floor
    // both read OK while a true stall still fires.
    //
    // The backlog term is `EnrichmentRowsRemainingRecent`, the worker's
    // recency-bounded volume-zero count (candles within `ENRICH_RECENT_WINDOW_S`,
    // default 4h, of the CH clock). It excludes the permanent deep-history
    // exotic-quote floor, so the earlier residual — an idle env with a nonzero
    // floor and no new enrichable rows tripping the alarm — is closed: an idle
    // env produces no fresh candles, so `EnrichmentRowsRemainingRecent` reads 0
    // and the alarm stays OK (task 0026 finding #5, resolved worker-side).
    //
    // The window must be >= this alarm's 3h sustain (evaluationPeriods ×
    // datapointsToAlarm × 1h). Otherwise a genuinely stuck *fresh* candle ages
    // out of the window before it can breach 3 consecutive hourly datapoints, so
    // a real stall in a low-cadence env would never page (task 0026 finding #1).
    // 4h ≥ 3h keeps a fresh stuck candle counted across all 3 datapoints; the
    // deep-history floor (years old) stays excluded, so the idle-env guarantee
    // above holds. If you raise datapointsToAlarm/evaluationPeriods, raise
    // ENRICH_RECENT_WINDOW_S to match.
    const enrichedPerHour = new cloudwatch.Metric({
      namespace: 'Prices/Enrichment',
      metricName: 'EnrichmentRowsEnriched',
      dimensionsMap: { Environment: config.envName },
      statistic: 'Sum',
      period: cdk.Duration.hours(1),
    });
    const backlogPerHour = new cloudwatch.Metric({
      namespace: 'Prices/Enrichment',
      metricName: 'EnrichmentRowsRemainingRecent',
      dimensionsMap: { Environment: config.envName },
      statistic: 'Maximum',
      period: cdk.Duration.hours(1),
    });
    this.enrichmentBacklogAlarm = new cloudwatch.Alarm(
      this,
      'EnrichmentBacklogAlarm',
      {
        alarmName: `prices-${config.envName}-enrichment-backlog`,
        alarmDescription:
          'Enrichment made no progress (EnrichmentRowsEnriched = 0) while a recent volume_quote_usd=0 backlog remained (EnrichmentRowsRemainingRecent > 0) across 3 consecutive hourly passes — enrichment is stalled, not merely behind. Check oracle↔asset-id reconciliation and that USDC/USDT/XLM reference assets exist in prices.assets. Progress-based (0056): clears when a pass enriches ≥1 row, so it does not latch on catch-up drains; the recency-bounded backlog excludes the permanent exotic-quote floor, so an idle env stays OK.',
        metric: new cloudwatch.MathExpression({
          // 1 when a pass enriched nothing AND a backlog remains, else 0.
          // Comparison operators yield per-datapoint 0/1 series in CW metric
          // math; the product is the logical AND.
          expression: '(enriched < 1) * (backlog > 0)',
          usingMetrics: { enriched: enrichedPerHour, backlog: backlogPerHour },
          period: cdk.Duration.hours(1),
          label: 'EnrichmentStalledWithBacklog',
        }),
        threshold: 1,
        evaluationPeriods: 3,
        datapointsToAlarm: 3,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );

    new cdk.CfnOutput(this, 'EnrichmentBacklogAlarmName', {
      value: this.enrichmentBacklogAlarm.alarmName,
      description: `Enrichment backlog alarm for ${config.envName}`,
    });

    new cdk.CfnOutput(this, 'DashboardName', {
      value: this.dashboard.dashboardName,
      description: `CloudWatch dashboard name for ${config.envName}`,
    });

    // -----------------------------------------------------------------
    // Tranche-1 ops alarms (task 0056): SDEX push-freshness + mTLS NotAfter,
    // both routed to a shared SNS topic.
    // -----------------------------------------------------------------
    this.opsAlarmsTopic = new sns.Topic(this, 'OpsAlarmsTopic', {
      topicName: opsAlarmsTopicName(config.envName),
      displayName: `prices-api ${config.envName} ops alarms`,
    });
    if (config.opsAlarms.notificationEmail) {
      this.opsAlarmsTopic.addSubscription(
        new subscriptions.EmailSubscription(config.opsAlarms.notificationEmail),
      );
    }

    // AWS Chatbot → Slack (task 0056). When `opsAlarms.slack` is configured we
    // route the ops-alarms topic to a Slack channel, matching how BE delivers
    // its own CloudWatch alarms (its `${env}-soroban-explorer-alarms` topic →
    // Chatbot → Slack) — the team's real ops surface, no email/mailing list.
    // Reuses BE's channel: the workspace/channel IDs are read at deploy from the
    // SSM params `opsAlarms.slack.{workspaceIdSsmParam,channelIdSsmParam}` (kept
    // out of this public repo), which point at BE's existing
    // `/soroban-explorer/{env}/slack-*` values in the shared account. The Slack
    // workspace is already authorized in AWS Chatbot for BE, so no extra console
    // step is needed here — the named SSM params just have to exist before
    // deploying Observability. Omit the config to leave the topic
    // subscriber-less (managed manually in SNS).
    if (config.opsAlarms.slack) {
      const slackWorkspaceId = ssm.StringParameter.valueForStringParameter(
        this,
        config.opsAlarms.slack.workspaceIdSsmParam,
      );
      const slackChannelId = ssm.StringParameter.valueForStringParameter(
        this,
        config.opsAlarms.slack.channelIdSsmParam,
      );
      this.opsAlarmsSlackChannel = new chatbot.SlackChannelConfiguration(
        this,
        'OpsAlarmsSlackChannel',
        {
          slackChannelConfigurationName: opsAlarmsTopicName(config.envName),
          slackWorkspaceId,
          slackChannelId,
          notificationTopics: [this.opsAlarmsTopic],
          // Chatbot defaults to LoggingLevel NONE, which makes the last hop of
          // the alerting path unobservable: CloudWatch reports "Successfully
          // executed action <sns-arn>" whether or not Chatbot then renders
          // anything in Slack, and with logging off there is no record either
          // way. During the 0112 fire-test three SNS publishes succeeded and
          // produced no Slack message, and the reason could not be determined
          // because this was NONE. An alerting path we cannot audit is the same
          // class of problem as the alarm that fired into an empty action list.
          loggingLevel: chatbot.LoggingLevel.ERROR,
          role: new iam.Role(this, 'OpsAlarmsChatbotRole', {
            assumedBy: new iam.ServicePrincipal('chatbot.amazonaws.com'),
            managedPolicies: [
              iam.ManagedPolicy.fromAwsManagedPolicyName(
                'CloudWatchReadOnlyAccess',
              ),
            ],
          }),
        },
      );
    }

    const snsAction = new cw_actions.SnsAction(this.opsAlarmsTopic);

    // Every alarm gets both an ALARM and an OK action on the ops topic, so the
    // Slack channel sees the recovery as well as the breach (task 0056).
    // The enrichment backlog alarm (task 0026) shipped without an action; wire
    // it to the ops topic now that 0056 owns the alarm-routing.
    this.enrichmentBacklogAlarm.addAlarmAction(snsAction);
    this.enrichmentBacklogAlarm.addOkAction(snsAction);

    // -----------------------------------------------------------------
    // Oracle poll feed — dark-feed + timestamp-rejection alarms (task 0231).
    //
    // Task 0227 made the worker REFUSE an implausible Reflector timestamp
    // instead of writing it. Correct, and only half the job: the rejection logs
    // at `error` and `run_oracle` still returns Ok, so the Lambda is green. If
    // Reflector changes `lastprice`'s unit again, every reading is refused —
    // correctly — and the poll feed goes 100% dark behind a healthy Lambda.
    // Five months of silently WRONG rows would become an indefinite silently
    // ABSENT feed. These two alarms are what make that visible.
    //
    // The pair is deliberate: `-dark-feed` is the SYMPTOM (catches any cause,
    // including causes nobody has thought of), `-timestamp-rejected` is the
    // CAUSE (fires sooner, and says which one). Neither replaces the other.
    //
    // Both read `Prices/Oracle`, published by the worker at the end of every
    // pass. The `Environment` dimension is part of the metric's identity: it
    // must match the `ENV_NAME` the Lambda runs with, or these alarms watch a
    // series that does not exist and CloudWatch reports 0 datapoints rather
    // than an error — which is how task 0204 shipped 10 of 13 alarms blind.
    // -----------------------------------------------------------------

    // Each bucket must span at least one scheduled pass, or an empty bucket is
    // schedule jitter rather than a dark feed. Production polls every 5 minutes
    // (`infra/envs/production.json` → `scheduleExpressions.oracleWatcher`), so a 10-minute
    // bucket holds ~2 passes and cannot be emptied by jitter alone.
    const darkFeedBucketMinutes = 10;
    const oracleCadenceMinutes = rateExpressionMinutes(
      config.scheduleExpressions.oracleWatcher,
    );
    if (
      oracleCadenceMinutes !== undefined &&
      oracleCadenceMinutes * 2 > darkFeedBucketMinutes
    ) {
      // Fail at SYNTH, loudly. A cadence that does not fit twice in the bucket
      // makes an empty bucket reachable by ordinary schedule jitter, and the
      // alarm then false-fires forever on a healthy feed — the one outcome
      // worse than no alarm, because it trains people to ignore it.
      //
      // The 2x is the real invariant, not a safety margin: at exactly one pass
      // per bucket, two passes drifting into the same bucket leave the next one
      // empty. Enforced as stated, so the check and its message agree.
      throw new Error(
        `oracle-watcher cadence ${config.scheduleExpressions.oracleWatcher} (${oracleCadenceMinutes} min) ` +
          `does not fit twice in the ${darkFeedBucketMinutes}-minute dark-feed bucket; raise ` +
          `darkFeedBucketMinutes to at least 2x the cadence in observability-stack.ts, ` +
          `or the alarm will false-fire on a healthy feed.`,
      );
    }

    // FILL(written, 0) rather than the raw metric. The worker publishes an
    // explicit 0 on a pass that wrote nothing, so raw data normally exists —
    // but a pass that is invoked and then dies hard (OOM, timeout, or a killed
    // container) never reaches its publish and emits NO datapoint. That is a
    // genuine dark feed the `-no-invocations` alarm cannot see, because the
    // invocations are happening. FILL turns that absence into breaching zeros
    // (task 0222); see the `FILL(invocations, 0)` block above for the two
    // properties that were verified by measurement before relying on it.
    //
    // Three periods, never 1/1: the trailing bucket is always partially filled
    // and reads low, so a single-datapoint alarm would fire on a healthy feed
    // every time it evaluated. Three consecutive fully-dark buckets is 30
    // minutes, which also absorbs a deploy window.
    this.oracleDarkFeedAlarm = new cloudwatch.Alarm(
      this,
      'OracleDarkFeedAlarm',
      {
        alarmName: `prices-${config.envName}-oracle-dark-feed`,
        alarmDescription: `The Reflector oracle poll wrote ZERO rows to prices.oracle_prices for 30 minutes (OracleRowsWritten = 0 across 3 consecutive 10-minute buckets) while the Lambda kept reporting success. The feed is dark. Most likely: Reflector changed lastprice units or started returning 0, so the task 0227 plausibility guard is correctly refusing every reading — check prices-${config.envName}-oracle-timestamp-rejected, which names that cause directly, and the worker ERROR logs for "reflector timestamp rejected" with the raw value. Otherwise: Soroban RPC unreachable, or the contract moved. If oracle_prices IS gaining rows, suspect the metric path rather than the feed — the publish only logs a warning on failure, so a mis-scoped namespace grant, or deploying Observability ahead of EventBridge, reads identically here. Non-critical (§2.2): the feed degrades to last-known value, but close_usd enrichment stops advancing, and 0227 showed this failing silently for five months.`,
        metric: new cloudwatch.MathExpression({
          expression: 'FILL(written, 0)',
          usingMetrics: {
            written: new cloudwatch.Metric({
              namespace: 'Prices/Oracle',
              metricName: 'OracleRowsWritten',
              dimensionsMap: { Environment: config.envName },
              statistic: 'Sum',
              period: cdk.Duration.minutes(darkFeedBucketMinutes),
            }),
          },
          period: cdk.Duration.minutes(darkFeedBucketMinutes),
          label: 'OracleRowsWrittenFilled',
        }),
        threshold: 1,
        evaluationPeriods: 3,
        datapointsToAlarm: 3,
        comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
        // Backstop, not the mechanism — FILL yields zeros only where the metric
        // has published at some point. A worker that has NEVER published still
        // produces no series, and BREACHING is right there too: a feed that has
        // never written a row is also dark. An environment with the schedule
        // disabled is covered by prices-{env}-oracle-no-invocations instead.
        treatMissingData: cloudwatch.TreatMissingData.BREACHING,
      },
    );
    this.oracleDarkFeedAlarm.addAlarmAction(snsAction);
    this.oracleDarkFeedAlarm.addOkAction(snsAction);

    // The cause alarm. Fires on a SINGLE rejected reading, at 1/1, and that is
    // deliberate on both counts:
    //
    //   * No FILL, so 1/1 is safe here. The 1/1 hazard is a partially-filled
    //     trailing bucket reading LOW; this alarm compares `>= 1`, so a partial
    //     bucket can only under-report and delay the alarm by one period. It
    //     can never fabricate one.
    //   * One rejection is already signal. The 0227 guard decides from the
    //     magnitude of the reading, and Reflector's real seconds-valued
    //     timestamps sit nowhere near the threshold — the dead zone between the
    //     two units is checked by a unit test. So a rejection is not a near-miss
    //     that might drift back; it means a reading arrived in a shape this feed
    //     has never produced. Waiting for a second one only buys 5 minutes of
    //     silence.
    //
    // Missing data is NOT_BREACHING: this metric is only published by a pass
    // that completed, and "no passes are completing" is the dark-feed alarm's
    // job, not this one's. That is also what keeps it OK on an idle environment.
    this.oracleTimestampRejectedAlarm = new cloudwatch.Alarm(
      this,
      'OracleTimestampRejectedAlarm',
      {
        alarmName: `prices-${config.envName}-oracle-timestamp-rejected`,
        alarmDescription: `The oracle worker REFUSED a Reflector reading whose timestamp failed the task 0227 plausibility guard (OracleTimestampRejected >= 1). The row was not written — this is the guard working, not data loss — but it means Reflector sent a timestamp in a shape this feed has never produced, most likely a units change on lastprice (0227 was seconds being divided by 1000 and landing every row in 1970). The raw value is in the worker logs: filter /aws/lambda/prices-${config.envName}-oracle for the ERROR "reflector timestamp rejected; row not written", whose fields carry symbol, raw and error. If every symbol is being refused the feed is now dark and prices-${config.envName}-oracle-dark-feed follows within 30 minutes. Fix the unit handling in reflector_timestamp_to_epoch_seconds; do NOT widen the guard to let the readings through.`,
        metric: new cloudwatch.Metric({
          namespace: 'Prices/Oracle',
          metricName: 'OracleTimestampRejected',
          dimensionsMap: { Environment: config.envName },
          statistic: 'Sum',
          period: cdk.Duration.minutes(5),
        }),
        threshold: 1,
        evaluationPeriods: 1,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.oracleTimestampRejectedAlarm.addAlarmAction(snsAction);
    this.oracleTimestampRejectedAlarm.addOkAction(snsAction);

    new cdk.CfnOutput(this, 'OracleDarkFeedAlarmName', {
      value: this.oracleDarkFeedAlarm.alarmName,
      description: `Oracle dark-feed alarm for ${config.envName}`,
    });
    new cdk.CfnOutput(this, 'OracleTimestampRejectedAlarmName', {
      value: this.oracleTimestampRejectedAlarm.alarmName,
      description: `Oracle timestamp-rejection alarm for ${config.envName}`,
    });

    // SDEX push freshness (§5.6 / Tranche-1 AC #5). The backfill-freshness-probe
    // republishes `sdex_archive`'s push age (seconds) as Prices/Backfill
    // PushAgeSeconds; alarm once it exceeds the operator-tuned threshold. The
    // probe keeps publishing a *rising* age when pushes stop, so a stale metric
    // is itself the signal — missing data is left non-breaching (probe-down is
    // covered by the probe's own error alarm).
    this.sdexPushFreshnessAlarm = new cloudwatch.Alarm(
      this,
      'SdexPushFreshnessAlarm',
      {
        alarmName: `prices-${config.envName}-sdex-push-freshness`,
        alarmDescription:
          'sdex_archive.last_push_at has aged past the Tranche-1 freshness threshold (a scheduled sdex-cloud-push cycle was skipped, or the push pipeline is stalled). Threshold is operator-tunable via config.opsAlarms.sdexPushFreshnessSeconds.',
        metric: new cloudwatch.Metric({
          namespace: 'Prices/Backfill',
          metricName: 'PushAgeSeconds',
          dimensionsMap: {
            Environment: config.envName,
            Stream: 'sdex_archive',
          },
          statistic: 'Maximum',
          period: cdk.Duration.minutes(15),
        }),
        threshold: config.opsAlarms.sdexPushFreshnessSeconds,
        evaluationPeriods: 1,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.sdexPushFreshnessAlarm.addAlarmAction(snsAction);
    this.sdexPushFreshnessAlarm.addOkAction(snsAction);

    // Rollup freshness, one alarm per OHLCV granularity (task 0137).
    //
    // Task 0136 froze `price_ohlcv_15m` through `_1M` for NINE DAYS and nothing
    // alarmed: eight of the nine refreshable MVs reported `status = Scheduled`
    // with an empty `exception` every cycle, because rolling up nothing is not
    // a failure. The freeze surfaced only by accident, when task 0072's rollout
    // check noticed `change_7d_pct` was 0 for every asset. These alarms watch
    // the DATA — how old each tier's newest bucket is — rather than the MV's
    // exit status, which is the only signal that could have caught it.
    //
    // One alarm per tier rather than one aggregate, because WHICH tier is stale
    // is the diagnosis: in 0136 the break was at `mv_ohlcv_1m_to_15m` and every
    // coarser tier merely inherited it. A single alarm over all seven would say
    // "rollups are stale" and lose the fact that `1m` was healthy — which is
    // exactly what localises the fault.
    //
    // Missing data is NOT_BREACHING: the probe publishes no datum for a tier
    // with zero rows (a freshly-provisioned environment), and probe-down is
    // covered by the probe's own `-errors` alarm plus its worker-health pair
    // below. A stalled tier keeps publishing a RISING lag, so a stale metric is
    // itself the signal.
    this.rollupFreshnessAlarms = Object.fromEntries(
      Object.entries(config.opsAlarms.rollupLagSeconds).map(
        ([table, threshold]) => {
          // `price_ohlcv_15m` → `PriceOhlcv15m`, a stable CFN logical id.
          const idSuffix = table
            .split('_')
            .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
            .join('');
          const alarm = new cloudwatch.Alarm(
            this,
            `RollupFreshness${idSuffix}Alarm`,
            {
              alarmName: `prices-${config.envName}-rollup-freshness-${table.replace('price_ohlcv_', '')}`,
              alarmDescription: `${table} has not received a new bucket within its staleness bound (${threshold}s): the rollup chain feeding it is stalled, or upstream ingestion has halted. A rollup MV that reads stale input still reports success, so this measures the data, not the MV (task 0136/0137). Threshold is operator-tunable via config.opsAlarms.rollupLagSeconds.${table}. Check the finer tiers first — the finest stale tier is the fault, the coarser ones inherit it.`,
              metric: new cloudwatch.Metric({
                namespace: 'Prices/Rollup',
                metricName: 'RollupLagSeconds',
                dimensionsMap: {
                  Environment: config.envName,
                  Table: table,
                },
                statistic: 'Maximum',
                period: cdk.Duration.minutes(15),
              }),
              threshold,
              // M-of-N (1 of 2) rather than 1-of-1, so a single missed publish
              // cannot flip the alarm back to OK.
              //
              // The period equals the probe cadence, so with 1-of-1 any probe
              // outage — bad grant, CH unreachable, sustained throttle — makes
              // every datum go missing, NOT_BREACHING scores that healthy, and
              // ALL SEVEN tier alarms send an OK action. That is seven
              // "recovered" messages into Slack for tiers that are still frozen,
              // arriving before the probe's own `-errors` alarm fires. Requiring
              // only 1 breaching datapoint out of 2 keeps a real breach latched
              // across one missed cycle while still alarming on the first bad
              // reading. (`sdexPushFreshnessAlarm` has the same 1-of-1 shape;
              // the difference here is that the blast radius is 7×.)
              evaluationPeriods: 2,
              datapointsToAlarm: 1,
              comparisonOperator:
                cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
              treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
            },
          );
          alarm.addAlarmAction(snsAction);
          alarm.addOkAction(snsAction);
          return [table, alarm];
        },
      ),
    );

    // ClickHouse host free space (task 0204, gap 1). The 2026-08-13 stall ran
    // 11.5 h and was found by reading Lambda panic logs — `asset-discovery`,
    // `supply` and `ledger-processor` all failing with CH `Code: 243` — because
    // nothing watched the disk itself.
    //
    // ⚠️ The volume is SHARED with the block-explorer team and we are 3.3% of
    // it, so this alarm cannot prevent the condition and we cannot free
    // meaningful space when it fires. Its entire value is warning time, which
    // is why the threshold is a generous percentage rather than a last-ditch
    // one; see config.opsAlarms.chDiskFreePercent for the arithmetic.
    //
    // Published by the rollup-freshness-probe (every 15 min) rather than a
    // probe of its own, and into the existing Prices/Rollup namespace, so this
    // change stays inside THIS stack. A new namespace would need the probe
    // role's PutMetricData condition widened in eventbridge-stack.ts, which is
    // where `CleanupRule` lives — and every deploy of that stack can silently
    // re-enable `prices-{env}-cleanup`, which CDK asserts is ENABLED while the
    // live rule is DISABLED. Cleanup running during the 0182/0201 repair
    // campaign shreds that campaign's output. Not a hazard worth taking on for
    // a namespace label.
    //
    // Missing data is NOT_BREACHING for the same reason as the rollup alarms:
    // probe-down is covered by the probe's own `-errors` alarm and its
    // worker-health pair, and an unreadable capacity fails the invocation
    // rather than publishing a misleading zero. M-of-N (1 of 2) so a single
    // missed publish cannot flip a real breach back to OK.
    this.chDiskFreeAlarm = new cloudwatch.Alarm(this, 'ChDiskFreeAlarm', {
      alarmName: `prices-${config.envName}-ch-disk-free`,
      alarmDescription: `Free space on the ClickHouse host's filesystem has dropped below ${config.opsAlarms.chDiskFreePercent}% (task 0204). ⚠️ The volume is SHARED with the block-explorer team and we are ~3.3% of it — deleting prices data will NOT recover a meaningful amount, so escalate to BE rather than starting a cleanup. On 2026-08-13 this condition stalled ingestion for 11.5 h and surfaced only as ClickHouse Code: 243 panics in asset-discovery, supply and ledger-processor. ⛔ Do NOT enable the cleanup worker as a remedy while a repair/backfill campaign is running (task 0200). Threshold is operator-tunable via config.opsAlarms.chDiskFreePercent.`,
      metric: new cloudwatch.Metric({
        namespace: 'Prices/Rollup',
        metricName: 'ClickHouseDiskFreePercent',
        dimensionsMap: { Environment: config.envName },
        statistic: 'Minimum',
        period: cdk.Duration.minutes(15),
      }),
      threshold: config.opsAlarms.chDiskFreePercent,
      evaluationPeriods: 2,
      datapointsToAlarm: 1,
      comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });
    this.chDiskFreeAlarm.addAlarmAction(snsAction);
    this.chDiskFreeAlarm.addOkAction(snsAction);

    // mTLS cert expiry (§7 / §11.4). The mtls-notafter-probe publishes the
    // minimum days-to-NotAfter across the ingestion + api client certs; alarm
    // when it drops below the threshold (30 days default). Fires on an expired
    // cert too (the metric goes negative, still < threshold).
    this.mtlsNotAfterAlarm = new cloudwatch.Alarm(this, 'MtlsNotAfterAlarm', {
      alarmName: `prices-${config.envName}-mtls-notafter`,
      alarmDescription:
        'An mTLS client cert is within the Tranche-1 expiry window (days-to-NotAfter below config.opsAlarms.mtlsNotAfterDaysThreshold). Re-issue + upload the bundle before it expires or ClickHouse mTLS auth breaks.',
      metric: new cloudwatch.Metric({
        namespace: 'Prices/Mtls',
        metricName: 'MinDaysToNotAfter',
        dimensionsMap: { Environment: config.envName },
        statistic: 'Minimum',
        period: cdk.Duration.days(1),
      }),
      threshold: config.opsAlarms.mtlsNotAfterDaysThreshold,
      evaluationPeriods: 1,
      datapointsToAlarm: 1,
      comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });
    this.mtlsNotAfterAlarm.addAlarmAction(snsAction);
    this.mtlsNotAfterAlarm.addOkAction(snsAction);

    // -----------------------------------------------------------------
    // Live ledger-processor health (task 0056 finding B). The core ingestion
    // Lambda shipped unmonitored: `prices.ledger_processor.lag_seconds` existed
    // only as a comment and no alarm watched it. These three alarms cover it
    // from AWS-native metrics, so no custom-metric emission / processor redeploy
    // is required. Metric dimensions are built from ComputeStack's own name
    // helpers (the function, the ingest queue, and its DLQ) — a shared single
    // source of truth, so a rename there flows here automatically instead of
    // silently leaving the alarm on a non-existent metric. Imported by name (not
    // as CDK objects), so ObservabilityStack stays independently deployable with
    // no cross-stack CFN reference, exactly like the ops-topic pattern above.
    const ledgerProcessorFnName = ledgerProcessorFunctionName(config.envName);
    const ingestQueue = ingestQueueName(config.envName);
    const ingestDlq = ingestDlqName(config.envName);

    // Ingestion lag: the ledger-processor emits no lag metric, so we watch the
    // ingest queue's oldest-message age — the honest "processor is falling
    // behind" signal. Ledgers close ~every 5–6 s and a healthy processor drains
    // the doorbell in seconds; an oldest-message age sustained above the
    // threshold (default 120 s) means live ingestion is lagging.
    //
    // Sustained over 5×1-min datapoints (not 3) so a routine deploy / cold start
    // / brief mTLS reconnect — during which the oldest enqueued doorbell ages a
    // few minutes while the processor is briefly paused, then drains — does not
    // false-page. A real stall keeps the age climbing well past 5 min. Note the
    // Maximum statistic means a single old message pins the datapoint, so the
    // sustain (not the per-datapoint value) is what suppresses catch-up flap.
    this.ledgerProcessorLagAlarm = new cloudwatch.Alarm(
      this,
      'LedgerProcessorLagAlarm',
      {
        alarmName: `prices-${config.envName}-ledger-processor-lag`,
        alarmDescription:
          'The prices-ingest SQS doorbell is backing up (ApproximateAgeOfOldestMessage over the threshold, sustained 5 min): the live ledger-processor is not keeping up with ledger production or has stalled. Threshold is operator-tunable via config.opsAlarms.ledgerProcessorLagSeconds. Check the ledger-processor logs, the mTLS ClickHouse write path, and the DLQ.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/SQS',
          metricName: 'ApproximateAgeOfOldestMessage',
          dimensionsMap: { QueueName: ingestQueue },
          statistic: 'Maximum',
          period: cdk.Duration.minutes(1),
        }),
        threshold: config.opsAlarms.ledgerProcessorLagSeconds,
        evaluationPeriods: 5,
        datapointsToAlarm: 5,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        // SQS reports 0 (not missing) on an empty-but-active queue → 0 <
        // threshold → OK. Should the metric go truly absent, missing = caught up
        // (a real halt is caught by the separate no-invocations alarm below), so
        // it must not breach.
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.ledgerProcessorLagAlarm.addAlarmAction(snsAction);
    this.ledgerProcessorLagAlarm.addOkAction(snsAction);

    // Hard errors: the ledger-processor Lambda throwing (a crash, not a graceful
    // per-item batch failure). Any invocation error over 5 min pages.
    this.ledgerProcessorErrorAlarm = new cloudwatch.Alarm(
      this,
      'LedgerProcessorErrorAlarm',
      {
        alarmName: `prices-${config.envName}-ledger-processor-errors`,
        alarmDescription:
          'The live ledger-processor Lambda is throwing invocation errors (AWS/Lambda Errors ≥ 1 over 5 min). Distinct from a poison-pill doorbell (see the DLQ alarm): this is the handler crashing. Check the ledger-processor logs.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/Lambda',
          metricName: 'Errors',
          dimensionsMap: { FunctionName: ledgerProcessorFnName },
          statistic: 'Sum',
          period: cdk.Duration.minutes(5),
        }),
        threshold: 1,
        evaluationPeriods: 1,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.ledgerProcessorErrorAlarm.addAlarmAction(snsAction);
    this.ledgerProcessorErrorAlarm.addOkAction(snsAction);

    // Poison-pill / permanent-failure doorbells: under reportBatchItemFailures a
    // handler that keeps failing one item re-drives it (no Lambda Error) until
    // maxReceiveCount, then it lands in the DLQ — a dropped ledger = a data gap.
    // Any message in the DLQ pages; this is the failure the Errors alarm cannot
    // see. treatMissingData BREACHING would false-fire while SQS reports no
    // datapoint on an empty DLQ, so keep it NOT_BREACHING and watch the count.
    this.ledgerProcessorDlqAlarm = new cloudwatch.Alarm(
      this,
      'LedgerProcessorDlqAlarm',
      {
        alarmName: `prices-${config.envName}-ledger-processor-dlq`,
        alarmDescription:
          'A ledger doorbell exhausted its SQS retries and landed in the prices-ingest DLQ (ApproximateNumberOfMessagesVisible ≥ 1): a ledger the live processor could not process = a candle gap. Inspect the DLQ message, fix the cause, and redrive. This is rung 1 of an escalating ladder (task 0204) — if the DLQ keeps filling, the -dlq-N alarms fire in turn, so ONE message here means one message, not necessarily one message for long.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/SQS',
          metricName: 'ApproximateNumberOfMessagesVisible',
          dimensionsMap: { QueueName: ingestDlq },
          statistic: 'Maximum',
          period: cdk.Duration.minutes(5),
        }),
        threshold: 1,
        evaluationPeriods: 1,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.ledgerProcessorDlqAlarm.addAlarmAction(snsAction);
    this.ledgerProcessorDlqAlarm.addOkAction(snsAction);

    // DLQ escalation ladder (task 0204, gap 2). On 2026-08-13 Slack carried
    // exactly one line — `ApproximateNumberOfMessagesVisible >= 1` — while the
    // DLQ grew to 91 overnight. Nobody reading the channel could tell 1 from 91.
    //
    // ⚠️ The cause is structural, not a bad threshold: CloudWatch notifies on a
    // state TRANSITION. Once the rung-1 alarm above is latched in ALARM it says
    // nothing further, however far the queue climbs. No threshold on a single
    // alarm fixes that. Additional rungs do: each is its own alarm with its own
    // transition, so a growing DLQ crosses a new one and sends a new message.
    //
    // ⚠️ Every rung MUST keep its OK action. A rung with no way back to OK
    // latches permanently on first breach and is then silent for every
    // subsequent incident — it would reproduce the very defect this closes, one
    // level up. The cost is that a redrive to empty sends one OK per rung; that
    // noise is deliberate and much cheaper than a silent ladder.
    //
    // Rung 1 stays exactly as declared above — same logical id, same alarm name
    // — so this change is purely additive and cannot replace the alarm the team
    // already watches.
    this.ledgerProcessorDlqEscalationAlarms = Object.fromEntries(
      config.opsAlarms.dlqEscalationDepths.map((depth) => {
        const alarm = new cloudwatch.Alarm(
          this,
          `LedgerProcessorDlqAlarmDepth${depth}`,
          {
            alarmName: `prices-${config.envName}-ledger-processor-dlq-${depth}`,
            alarmDescription: `The prices-ingest DLQ has reached ${depth} messages — ${depth} ledgers the live processor could not handle, i.e. ${depth} candle gaps. Escalation rung above prices-${config.envName}-ledger-processor-dlq (task 0204): a lone poison pill does not reach this depth, so treat it as systemic — check the ledger-processor logs, ClickHouse reachability and disk headroom (a full shared volume put 91 messages here on 2026-08-13) before redriving. Rungs are operator-tunable via config.opsAlarms.dlqEscalationDepths.`,
            metric: new cloudwatch.Metric({
              namespace: 'AWS/SQS',
              metricName: 'ApproximateNumberOfMessagesVisible',
              dimensionsMap: { QueueName: ingestDlq },
              statistic: 'Maximum',
              period: cdk.Duration.minutes(5),
            }),
            threshold: depth,
            evaluationPeriods: 1,
            datapointsToAlarm: 1,
            comparisonOperator:
              cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
            // Same reasoning as rung 1: SQS publishes no datapoint for an empty
            // DLQ, so BREACHING would false-fire on a healthy queue.
            treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
          },
        );
        alarm.addAlarmAction(snsAction);
        alarm.addOkAction(snsAction);
        return [String(depth), alarm];
      }),
    );

    // USD-value correctness on the USDT quote leg (task 0204, gap 4).
    //
    // ⚠️ This is the only alarm in this stack that watches whether the data is
    // RIGHT rather than whether it is ARRIVING. A close_usd that is fresh,
    // present and wrong is invisible to all seven rollup alarms, to the disk
    // alarm and to the DLQ ladder — every one of them reads healthy while the
    // numbers are ~7.4x too high, which is the state prod sat in from the June
    // 2022 depeg until 2026-08-18 (tasks 0172, 0182).
    //
    // ⚠️ Two directions, and the second is not symmetry for its own sake.
    // peg-applied is the original defect; stranded (close_usd = 0 on a candle
    // with a representable close) is what task 0182's REPAIR produced on
    // 2026-08-19 — 157 candles zeroed with nothing to refill them. A check for
    // only the first direction would have passed while that damage stood.
    //
    // ⚠️ A ladder rather than one alarm, for the gap 2 reason: a wrong
    // close_usd is a STANDING condition, and CloudWatch notifies on a state
    // TRANSITION, so a single alarm latches and goes quiet however far the
    // population climbs. Unlike MV drift (gap 3), which is binary and has no
    // way out of this, a count of wrong candles has DEPTH — a regressed writer
    // keeps adding to it — so gap 2's ladder transfers directly. Every rung
    // keeps its OK action for the same reason gap 2's rungs do: a rung with no
    // path back to OK latches permanently on first breach and is then silent
    // for every later incident.
    //
    // The probe scopes both counts to the USDT quote leg and bounds each to a
    // rolling window. Both are load-bearing — exotic-quoted zeros are by
    // design (~74M rows), and an unbounded scan every 15 min is task 0111's
    // outage wearing a health check's clothes. See
    // packages/rollup-freshness-probe/src/usd_sanity.rs.
    //
    // ⚠️ THE TWO DIRECTIONS READ DIFFERENT TIERS AND DIFFERENT WINDOWS (task
    // 0213). `peg_applied` reads `price_ohlcv_1m` over 48 h — the tier
    // enrichment WRITES; `stranded` reads `price_ohlcv_1h` over 7 days, where a
    // zero rolls up faithfully and the 48 h grace matches BE's loss window.
    // Reading one tier for both is what made the peg direction publish a
    // confident 0 over 1,564,045 wrong rows.
    //
    // 🔴 A GREEN `usd-peg-applied` IS NOT EVIDENCE THE USDT LEG IS HEALTHY.
    // Measured on prod 2026-08-20 inside the 48 h window: 684 rows scanned, ALL
    // 684 at close_usd = 0, zero peg-valued and zero correctly priced. The leg
    // has been unpriced since 2026-08-13 (task 0209), so this direction has
    // nothing to judge and reads 0 for want of input. `usd-stranded` is what
    // carries that condition, and it is latched. Never read one without the
    // other.
    //
    // ⚠️ An earlier version of this comment said the peg ladder must not be
    // deployed before 0212/0209 because it would ship permanently breached.
    // The measurement above falsifies that — the 1.5 M peg population sits at
    // timestamps <= 2026-08-13, entirely outside the window. That figure
    // applies to an unbounded repoint, not to the query that ships; do not
    // re-size these rungs against it.
    const usdSanityRungs = (
      metricName: string,
      idPrefix: string,
      alarmSuffix: string,
      describe: (count: number) => string,
    ): Record<string, cloudwatch.Alarm> =>
      Object.fromEntries(
        config.opsAlarms.usdSanityEscalationCounts.map((count) => {
          const alarm = new cloudwatch.Alarm(this, `${idPrefix}${count}`, {
            alarmName: `prices-${config.envName}-${alarmSuffix}-${count}`,
            alarmDescription: describe(count),
            metric: new cloudwatch.Metric({
              namespace: 'Prices/Rollup',
              metricName,
              dimensionsMap: { Environment: config.envName },
              statistic: 'Maximum',
              period: cdk.Duration.minutes(15),
            }),
            threshold: count,
            evaluationPeriods: 2,
            datapointsToAlarm: 1,
            comparisonOperator:
              cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
            treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
          });
          alarm.addAlarmAction(snsAction);
          alarm.addOkAction(snsAction);
          return [String(count), alarm];
        }),
      );

    this.usdPegAppliedAlarms = usdSanityRungs(
      'UsdtPegAppliedCandles',
      'UsdPegAppliedAlarmCount',
      'usd-peg-applied',
      (count) =>
        `${count} or more USDT-quoted price_ohlcv_1m candles written in the last 48 h carry a close_usd within 2% of their close — valued as if USDT were still pegged at $1. USDT depegged in June 2022 and trades at ~0.13-0.15 (task 0172), so these values are roughly 7.4x too high. Something is applying the peg path to the USDT leg again: check the enrichment tiers (USDT must be a PIVOT reference, never a peg member) and prices.oracle_prices for rows mis-attributed to the USDT identity — that is how tasks 0196 and 0168 reintroduced this WITHOUT touching the writer. VERIFY ON _1m, NEVER a coarse tier: task 0182 repaired the coarse tables directly, so _1h reads clean over a broken _1m (tasks 0212, 0213). Find what is writing them before re-running any repair. NOTE this metric reads 0 whenever the leg is unpriced, which it has been since 2026-08-13 - a green ladder is NOT proof of correct USD valuation, check prices-{env}-usd-stranded too. Rungs tunable via config.opsAlarms.usdSanityEscalationCounts.`,
    );

    this.usdStrandedAlarms = usdSanityRungs(
      'UsdtStrandedCandles',
      'UsdStrandedAlarmCount',
      'usd-stranded',
      (count) =>
        `${count} or more USDT-quoted candles are still at close_usd = 0 more than 48 h after being written, despite a close large enough to price. A zero is indistinguishable from "no data" at ~130 unguarded argMax(close_usd, ...) sites (task 0145), and BE render an empty "--" TVL when nothing priced within 48 h, so this is a value the consumer has already LOST, not one that is merely late. KNOWN CAUSE as of 2026-08-20: the USDT pivot has NEVER priced a price_ohlcv_1m row (measured pivot_written = 0 against 1,564,045 peg-written), so this leg has been dark since 2026-08-13 and this alarm stays latched until that is fixed - tasks 0209 (root cause) and 0212 (the peg-valued rows). Verify on _1m, NEVER on a coarse tier: task 0182 repaired the coarse tables directly, so _1h reads clean over a broken _1m. Do NOT re-run a reset repair - 0182 own reset CREATED 157 stranded candles. Rungs tunable via config.opsAlarms.usdSanityEscalationCounts.`,
    );

    // Materialized-view drift, on a schedule (task 0204, gap 3). Task 0142 built
    // `prices-clickhouse-drift` and NOTHING RAN IT — a check nobody runs is a
    // check that does not exist. It covers a condition no other alarm here can
    // see: task 0137 watches whether the rollups PRODUCE data, and a drifted MV
    // does that perfectly well while producing the wrong numbers.
    //
    // ⚠️ Two severities, deliberately not one alarm. The CLI collapses
    // everything to `exit 1`, which throws away the distinction that decides who
    // gets woken — an MV that lost APPEND is destroying history on every refresh
    // (the task 0095 data loss), while a definition mismatch is wrong but static.
    //
    // ⚠️ These fire ONCE and latch, unlike the DLQ ladder above, and that is a
    // decision rather than an oversight (operator, 2026-08-19). Gap 2 needed a
    // ladder because a DLQ GROWS while the alarm is quiet — 1 became 91
    // overnight. Drift does not grow: one drifted MV stays one drifted MV until
    // a person fixes it, so a latched alarm costs "somebody may forget", not
    // "we are blind to an escalation". The exception is the critical severity,
    // which does compound — which is exactly why it has its own alarm and its
    // own urgency rather than being buried in the ordinary count.
    //
    // ⚠️ treatMissingData: MISSING, NOT the NOT_BREACHING used everywhere else
    // in this stack, and the difference is load-bearing. Every alarm here has an
    // OK action, so under NOT_BREACHING two consecutive missing datapoints would
    // transition a latched ALARM back to OK and post an explicit "resolved"
    // message to Slack — while the MV was still drifted and nobody had touched
    // it. That is a stronger version of the 2026-08-13 false-recovery signal
    // this whole task was filed over: the lag alarm returned to OK truthfully
    // but for the wrong reason, and the operator read it as fixed. MISSING
    // retains the last state across a gap instead, so a dead probe cannot
    // announce a repair that did not happen. Nothing is lost by it — a probe
    // that stops publishing is already covered by its own `-errors` alarm
    // (addWorkerHealthAlarms), which is the correct signal for that condition.
    // The liveness alarms above keep NOT_BREACHING deliberately: for those,
    // "no data" genuinely is the absence of a breach.
    this.mvDriftCriticalAlarm = new cloudwatch.Alarm(
      this,
      'MvDriftCriticalAlarm',
      {
        alarmName: `prices-${config.envName}-mv-drift-critical`,
        alarmDescription: `A rollup materialized view is live WITHOUT the APPEND refresh mode. It atomically REPLACES its whole target table on every refresh, and because these MVs carry a bounded "WHERE timestamp >= now() - <window>", each tick overwrites the coarse table with only the recent window — deleting pre-rolled history permanently. This is the task 0090/0095 data loss, and every refresh makes it worse, so treat it as an emergency: check schema/rollups.sql against the live definition (packages/prices-clickhouse bin prices-clickhouse-drift prints the diff), re-create the MV WITH APPEND, then assess what history was lost. ⚠️ This alarm fires once and stays latched — it will NOT re-notify while the condition persists, so do not treat silence as resolution.`,
        metric: new cloudwatch.Metric({
          namespace: 'Prices/Rollup',
          metricName: 'MvDriftCritical',
          dimensionsMap: { Environment: config.envName },
          statistic: 'Maximum',
          period: cdk.Duration.minutes(15),
        }),
        threshold: 1,
        evaluationPeriods: 2,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.MISSING,
      },
    );
    this.mvDriftCriticalAlarm.addAlarmAction(snsAction);
    this.mvDriftCriticalAlarm.addOkAction(snsAction);

    this.mvDriftAlarm = new cloudwatch.Alarm(this, 'MvDriftAlarm', {
      alarmName: `prices-${config.envName}-mv-drift`,
      alarmDescription: `A rollup materialized view no longer matches schema/rollups.sql — its definition drifted, it is missing, it could not be fingerprinted, or an undeclared MV is writing into a table it does not own. Data keeps flowing and looks healthy; it is simply being built from the wrong definition, which no other alarm can see. NOT an emergency (nothing is being destroyed) but it does not fix itself: run the prices-clickhouse-drift binary for the field-level diff, and note that re-applying rollups.sql will report success and change nothing on an MV that already exists. ⚠️ Fires once and stays latched by design (task 0204 gap 3) — drift does not grow, so silence here means "still wrong", never "resolved".`,
      metric: new cloudwatch.Metric({
        namespace: 'Prices/Rollup',
        metricName: 'MvDriftCount',
        dimensionsMap: { Environment: config.envName },
        statistic: 'Maximum',
        period: cdk.Duration.minutes(15),
      }),
      threshold: 1,
      evaluationPeriods: 2,
      datapointsToAlarm: 1,
      comparisonOperator:
        cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
      treatMissingData: cloudwatch.TreatMissingData.MISSING,
    });
    this.mvDriftAlarm.addAlarmAction(snsAction);
    this.mvDriftAlarm.addOkAction(snsAction);

    // ⚠️ The false page this exists to prevent. `system.tables` is filtered by
    // grant, not denied, so a narrowed grant makes every MV report "missing" —
    // identical in shape to the entire rollup chain having been dropped.
    // Publishing MvDriftCount = 6 in that state would page at maximum urgency
    // with the wrong diagnosis. The probe suppresses the counts when it can see
    // NO prices objects at all and raises this instead.
    this.mvDriftUnreadableAlarm = new cloudwatch.Alarm(
      this,
      'MvDriftUnreadableAlarm',
      {
        alarmName: `prices-${config.envName}-mv-drift-unreadable`,
        alarmDescription: `The MV drift check ran but could see NO objects in the prices database, so its drift counts are meaningless and have been suppressed. This is far more likely a narrowed ClickHouse grant than the rollup chain having been deleted — system.tables is filtered by grant rather than denied, so a permissions change makes every MV look missing. Check the probe's mTLS identity and its SELECT grant on prices.* FIRST. ⚠️ If the grant is intact, then the objects really are gone and this is a catastrophe: escalate immediately and do NOT re-apply rollups.sql before understanding what happened.`,
        metric: new cloudwatch.Metric({
          namespace: 'Prices/Rollup',
          metricName: 'MvDriftUnreadable',
          dimensionsMap: { Environment: config.envName },
          statistic: 'Maximum',
          period: cdk.Duration.minutes(15),
        }),
        threshold: 1,
        evaluationPeriods: 2,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.MISSING,
      },
    );
    this.mvDriftUnreadableAlarm.addAlarmAction(snsAction);
    this.mvDriftUnreadableAlarm.addOkAction(snsAction);

    // Total ingestion halt: the lag / errors / DLQ alarms above all key on the
    // *presence* of enqueued or failed messages, so a producer-side stop (BE's
    // S3→SNS→SQS delivery halts, the subscription is deleted, or upstream simply
    // stops publishing) is invisible to them — the queue drains to empty, the
    // Lambda is never invoked, and all three sit OK while live candles silently
    // stop. This alarm closes that blind spot from the consumer side: if the
    // ledger-processor records zero `Invocations` for two 15-min periods it has
    // received no
    // doorbells at all. Pubnet closes a ledger every ~5–6 s, so a healthy
    // processor is invoked near-continuously and a 15-min silence is a genuine
    // outage, never normal idle.
    //
    // `FILL(invocations, 0)` (task 0222), same as the scheduled workers above.
    // Lambda publishes NO `Invocations` datapoint for a period with zero
    // invocations; FILL turns that absence into real zeros so the alarm
    // evaluates ordinary data instead of depending on missing-data semantics.
    //
    // ⚠️ Note this alarm was NOT observed failing. The measured failure was at
    // `Period=3600` (the coarse sweep, 2026-08-25); temporary alarms at 60 s and
    // 300 s both breached correctly, and this one's 900 s was never tested. It is
    // converted anyway because FILL makes the question irrelevant rather than
    // betting that this period happens to be unaffected — and because this is
    // the only alarm watching a total ingestion halt from the consumer side.
    this.ledgerProcessorNoInvocationsAlarm = new cloudwatch.Alarm(
      this,
      'LedgerProcessorNoInvocationsAlarm',
      {
        alarmName: `prices-${config.envName}-ledger-processor-no-invocations`,
        alarmDescription:
          'The live ledger-processor recorded zero invocations for two consecutive 15-min periods: no ledger doorbells are arriving (upstream S3→SNS→SQS delivery stopped, the subscription was removed, or the producer halted). Live ingestion is stalled at the source and candles are silently frozen. Check the SNS subscription on prices-ingest, BE ledger publication, and the ingest queue. Unlike the lag/errors/DLQ alarms this fires on the ABSENCE of throughput.',
        metric: new cloudwatch.MathExpression({
          expression: 'FILL(invocations, 0)',
          usingMetrics: {
            invocations: new cloudwatch.Metric({
              namespace: 'AWS/Lambda',
              metricName: 'Invocations',
              dimensionsMap: { FunctionName: ledgerProcessorFnName },
              statistic: 'Sum',
              period: cdk.Duration.minutes(15),
            }),
          },
          period: cdk.Duration.minutes(15),
          label: 'InvocationsFilled',
        }),
        threshold: 1,
        // 2 of 2, not 1 of 1 (task 0222, raised in review on PR #247). A single
        // filled zero must not be able to breach this alarm on its own.
        //
        // The exposure is not new — `1/1` with BREACHING already meant one late
        // `Invocations` datapoint was a breach. What changed is that FILL
        // evaluates promptly on ordinary data, whereas the slow missing-data
        // evaluation this task exists to remove was probably ALSO what kept the
        // latent flap quiet. Removing the lag can convert a dormant risk into a
        // live one, on the alarm that pages "live ingestion is stalled at the
        // source" with both alarm and OK actions on the ops topic.
        //
        // Cost: a genuine halt is detected in 30 min rather than 15. Accepted —
        // a missed 15 minutes on a halt is recoverable, whereas a flapping
        // top-severity page teaches people to ignore the channel, which is the
        // failure already sitting on prices-production-enrichment-errors
        // (task 0214, in ALARM since 2026-07-27). The 3-of-3 alarms in
        // addWorkerHealthAlarms already have this insulation; this hand-rolled
        // one did not.
        evaluationPeriods: 2,
        datapointsToAlarm: 2,
        comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
        // Backstop only, no longer the mechanism: FILL supplies zeros for empty
        // periods — verified with no anchor datapoint at all — so BREACHING now
        // only covers a processor that has never published a series.
        treatMissingData: cloudwatch.TreatMissingData.BREACHING,
      },
    );
    this.ledgerProcessorNoInvocationsAlarm.addAlarmAction(snsAction);
    this.ledgerProcessorNoInvocationsAlarm.addOkAction(snsAction);

    // Platform-metric health alarms for the scheduled workers (task 0112).
    //
    // These three workers each had exactly ONE alarm, and each of those reads a
    // custom metric the worker publishes only if it survives to the end of a
    // pass — so none of them could detect the worker dying. See
    // `addWorkerHealthAlarms` for the full reasoning and the incident that
    // exposed it.
    //
    // Function names come from `workerFunctionName`, the same helper
    // `createWorkerLambda` uses, so a rename cannot leave an alarm pointing at
    // a name that no longer exists — which would not error, it would just
    // never fire.
    //
    // Timeouts and cadences ARE duplicated from eventbridge-stack.ts /
    // production.json, because ObservabilityStack does not receive the
    // functions as props. That drift is real but benign: a stale timeout only
    // mis-tunes the 80% threshold, and a stale cadence only widens the
    // detection window. Neither can silently disarm an alarm. Threading the
    // functions through was judged not worth the stack coupling — see
    // §Design Decisions in task 0112.
    const workerHealth: Array<WorkerHealthAlarmProps> = [
      {
        name: 'enrichment',
        idPrefix: 'Enrichment',
        functionName: workerFunctionName(config.envName, 'enrichment'),
        timeout: cdk.Duration.minutes(5),
        cadence: cdk.Duration.hours(1),
        impact:
          'USD columns (close_usd / volume_quote_usd) stop being filled in, so new candles serve as 0 to the API.',
      },
      {
        // Task 0218. This worker is the whole reason the -no-invocations alarm
        // matters here: the sweep previously ran as a stage inside the
        // enrichment handler, where "never reached" was indistinguishable from
        // "ran and found nothing" because neither emitted anything. As its own
        // function, a run that does not happen is zero invocations, and the
        // FILL-to-zero alarm above turns that silence into a page.
        name: 'coarse-sweep',
        idPrefix: 'CoarseSweep',
        functionName: workerFunctionName(config.envName, 'coarse-sweep'),
        timeout: cdk.Duration.minutes(5),
        cadence: cdk.Duration.hours(1),
        impact:
          'The coarse OHLCV rollups (_15m … _1M) stop having close_usd / volume_quote_usd filled in, so long-range charts serve 0 for any candle the rollup MVs captured before enrichment reached it. This ran silently unfixed for months (task 0218).',
      },
      {
        // Task 0231. The oracle worker only qualifies for this list now: until
        // it published `Prices/Oracle`, it had no custom-metric alarm that could
        // go dark, so task 0112 left it out. Its two new alarms both read
        // metrics the worker itself publishes, which is exactly the dependency
        // this list exists to backstop — if the function stops being invoked,
        // -dark-feed still fires (missing data is BREACHING there), but only
        // -no-invocations says the schedule is the reason rather than Reflector.
        name: 'oracle',
        idPrefix: 'Oracle',
        functionName: workerFunctionName(config.envName, 'oracle'),
        timeout: cdk.Duration.minutes(2),
        cadence: cdk.Duration.minutes(5),
        impact:
          'The Reflector poll stops entirely, so prices.oracle_prices gains no new rows and close_usd enrichment degrades to the last known value (non-critical per §2.2). Task 0227 showed this path can be wrong — and now silent — for five months without anyone noticing.',
      },
      {
        name: 'backfill-freshness-probe',
        idPrefix: 'BackfillFreshness',
        functionName: workerFunctionName(
          config.envName,
          'backfill-freshness-probe',
        ),
        timeout: cdk.Duration.minutes(1),
        cadence: cdk.Duration.minutes(15),
        impact:
          'The sdex-push-freshness alarm goes dark: it reads Prices/Backfill PushAgeSeconds, which only this probe publishes, so a stalled backfill would stop being reported rather than reported as stalled.',
      },
      {
        name: 'rollup-freshness-probe',
        idPrefix: 'RollupFreshness',
        functionName: workerFunctionName(
          config.envName,
          'rollup-freshness-probe',
        ),
        timeout: cdk.Duration.minutes(1),
        cadence: cdk.Duration.minutes(15),
        impact:
          'Every rollup-freshness alarm goes dark: they read Prices/Rollup RollupLagSeconds, which only this probe publishes, so a frozen rollup chain would stop being reported rather than reported as frozen — the exact nine-day blind spot of task 0136. Since task 0204 the ClickHouse free-space alarm rides on the same probe, so it goes dark too: a filling shared volume would also stop being reported.',
      },
      {
        name: 'mtls-notafter-probe',
        idPrefix: 'MtlsNotAfter',
        functionName: workerFunctionName(config.envName, 'mtls-notafter-probe'),
        timeout: cdk.Duration.minutes(1),
        cadence: cdk.Duration.days(1),
        impact:
          'Client-certificate expiry monitoring goes dark: the mtls-notafter alarm reads Prices/Mtls MinDaysToNotAfter, which only this probe publishes. An expired cert breaks ALL ClickHouse ingestion, so silence here is the most expensive of the three.',
      },
    ];

    this.workerHealthAlarms = Object.fromEntries(
      workerHealth.map((w) => [
        w.name,
        addWorkerHealthAlarms(this, config.envName, snsAction, w),
      ]),
    );

    new cdk.CfnOutput(this, 'OpsAlarmsTopicArn', {
      value: this.opsAlarmsTopic.topicArn,
      description: `Ops-alarms SNS topic ARN for ${config.envName}`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);

    // =================================================================
    // Overview dashboard widgets (task 0125).
    //
    // Built HERE, at the end of the constructor, on purpose: the alarm strip
    // below is derived by walking this stack's construct tree, so every alarm
    // must already exist. Moving this block earlier silently shrinks the strip.
    //
    // Every widget is built against a metric+dimension pair verified to carry
    // live data in this account on 2026-09-03. The one exception is
    // `ClickHouseWriteLatencyMs`, which the ledger-processor starts publishing
    // only once ComputeStack is deployed — hence the deploy order Compute first,
    // then Observability.
    // =================================================================
    const apiName = restApiName(config.envName);
    const apiDims = { ApiName: apiName, Stage: config.envName };
    const envDims = { Environment: config.envName };

    const apiMetric = (
      metricName: string,
      statistic: string,
      label: string,
    ): cloudwatch.Metric =>
      new cloudwatch.Metric({
        namespace: 'AWS/ApiGateway',
        metricName,
        dimensionsMap: apiDims,
        statistic,
        label,
        period: cdk.Duration.minutes(5),
      });

    const chWriteLatency = (statistic: string): cloudwatch.Metric =>
      new cloudwatch.Metric({
        namespace: 'Prices/Ingest',
        metricName: 'ClickHouseWriteLatencyMs',
        dimensionsMap: envDims,
        statistic,
        label: statistic,
        period: cdk.Duration.minutes(5),
      });

    // 5xx as a percentage of all requests. `Count` is never zero in production
    // (108 655 requests on 2026-09-02), so the division is safe; metric math
    // yields no datapoint rather than an error if it ever were.
    const errorRatePct = new cloudwatch.MathExpression({
      expression: '100 * (e5xx / requests)',
      usingMetrics: {
        e5xx: apiMetric('5XXError', 'Sum', '5xx'),
        requests: apiMetric('Count', 'Sum', 'requests'),
      },
      period: cdk.Duration.minutes(5),
      label: '5xx error rate (%)',
    });

    // A quiet hour publishes no 5xx datapoint at all, which the graph draws as
    // a gap. For an error count a gap means zero, so say zero.
    const fiveXxFilled = new cloudwatch.MathExpression({
      expression: 'FILL(e5xx, 0)',
      usingMetrics: { e5xx: apiMetric('5XXError', 'Sum', '5xx') },
      period: cdk.Duration.minutes(5),
      label: '5xx',
    });

    const cacheHitRatioPct = new cloudwatch.MathExpression({
      expression: '100 * (hits / (hits + misses))',
      usingMetrics: {
        hits: apiMetric('CacheHitCount', 'Sum', 'hits'),
        misses: apiMetric('CacheMissCount', 'Sum', 'misses'),
      },
      period: cdk.Duration.minutes(5),
      label: 'Cache hit ratio (%)',
    });

    // -----------------------------------------------------------------
    // The alarm strip — DERIVED, never a list of names.
    //
    // Walking the construct tree covers every alarm this stack builds and needs
    // no maintenance when one is added or removed. Collected BEFORE the imports
    // below so an imported alarm can never be double-counted.
    // -----------------------------------------------------------------
    const stackOwnAlarms = this.node
      .findAll()
      .filter((c): c is cloudwatch.Alarm => c instanceof cloudwatch.Alarm);

    // The remaining alarms in the account are the per-worker `-errors` alarms
    // created inside `createWorkerLambda` and owned by EventBridgeStack, which
    // never re-exports them. Imported BY ARN rather than by a cross-stack
    // construct reference: a CFN reference would make Observability depend on
    // EventBridge and destroy the independent deployability this stack defends
    // everywhere else. The name comes from `workerErrorAlarmName`, the same
    // helper that creates them.
    const importedWorkerErrorAlarms = SCHEDULED_WORKERS.map((name) => {
      const idPrefix = name
        .split('-')
        .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
        .join('');
      return cloudwatch.Alarm.fromAlarmArn(
        this,
        `ImportedWorkerErrorAlarm${idPrefix}`,
        cdk.Stack.of(this).formatArn({
          service: 'cloudwatch',
          resource: 'alarm',
          resourceName: workerErrorAlarmName(config.envName, name),
          arnFormat: cdk.ArnFormat.COLON_RESOURCE_NAME,
        }),
      );
    });

    const alarmStripCount =
      stackOwnAlarms.length + importedWorkerErrorAlarms.length;

    // Operationally useful, and the only cheap way to prove mechanically that
    // the strip is complete and derived rather than typed out — the synth
    // assertion compares it against the template's own alarm-resource count.
    new cdk.CfnOutput(this, 'DashboardAlarmCount', {
      value: String(alarmStripCount),
      description: `Alarms covered by the ${config.envName} overview dashboard alarm strip`,
    });

    // -----------------------------------------------------------------
    // Row 0 — the acceptance strip. This row alone must answer SCF Tranche 3
    // AC 8 and the M2 acceptance criteria (p95 latency, error rate, cache hit
    // rate on one screen), so it is first and both of its physical rows sit
    // above the fold.
    // -----------------------------------------------------------------
    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          `# prices-api — ${config.envName}`,
          '',
          'Top line: API latency, error rate, cache hit ratio and ClickHouse write latency over the last 24 hours, and the current state of every alarm.',
        ].join('\n'),
        width: 24,
        height: 2,
      }),
    );

    this.dashboard.addWidgets(
      new cloudwatch.SingleValueWidget({
        title: 'API latency p95 (ms) — last 24h',
        metrics: [apiMetric('Latency', 'p95', 'p95')],
        width: 6,
        height: 5,
        start: '-P1D',
        setPeriodToTimeRange: true,
      }),
      new cloudwatch.SingleValueWidget({
        title: '5xx error rate (%) — last 24h',
        metrics: [errorRatePct],
        width: 6,
        height: 5,
        start: '-P1D',
        setPeriodToTimeRange: true,
      }),
      new cloudwatch.SingleValueWidget({
        title: 'Cache hit ratio (%) — last 24h',
        metrics: [cacheHitRatioPct],
        width: 6,
        height: 5,
        start: '-P1D',
        setPeriodToTimeRange: true,
      }),
      new cloudwatch.SingleValueWidget({
        title: 'ClickHouse write latency p95 (ms) — last 24h',
        metrics: [chWriteLatency('p95')],
        width: 6,
        height: 5,
        start: '-P1D',
        setPeriodToTimeRange: true,
      }),
    );

    // Two widgets rather than one so the cross-stack import is visible to a
    // reader: what this stack owns, and what it borrows.
    this.dashboard.addWidgets(
      new cloudwatch.AlarmStatusWidget({
        title: `Alarms owned by ObservabilityStack (${stackOwnAlarms.length})`,
        alarms: stackOwnAlarms,
        width: 16,
        height: 6,
      }),
      new cloudwatch.AlarmStatusWidget({
        title: `Worker -errors alarms, imported from EventBridgeStack (${importedWorkerErrorAlarms.length})`,
        alarms: importedWorkerErrorAlarms,
        width: 8,
        height: 6,
      }),
    );

    // -----------------------------------------------------------------
    // Row 1 — API.
    // -----------------------------------------------------------------
    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          '## API',
          '',
          'Public API traffic on API Gateway: request latency (p50 / p95 / p99), request count with 4xx and 5xx errors (rate-limit 429s are counted under 4xx), and response-cache hits, misses and hit ratio.',
        ].join('\n'),
        width: 24,
        height: 3,
      }),
    );

    this.dashboard.addWidgets(
      new cloudwatch.GraphWidget({
        title: 'Latency p50 / p95 / p99 (ms)',
        left: [
          apiMetric('Latency', 'p50', 'p50'),
          apiMetric('Latency', 'p95', 'p95'),
          apiMetric('Latency', 'p99', 'p99'),
        ],
        width: 8,
        height: 6,
      }),
      new cloudwatch.GraphWidget({
        title: 'Requests, 4xx and 5xx',
        left: [apiMetric('Count', 'Sum', 'requests')],
        right: [
          apiMetric('4XXError', 'Sum', '4xx (includes 429 throttles)'),
          fiveXxFilled,
        ],
        width: 8,
        height: 6,
      }),
      new cloudwatch.GraphWidget({
        title: 'Cache hits / misses and ratio',
        left: [
          apiMetric('CacheHitCount', 'Sum', 'hits'),
          apiMetric('CacheMissCount', 'Sum', 'misses'),
        ],
        right: [cacheHitRatioPct],
        width: 8,
        height: 6,
      }),
    );

    // -----------------------------------------------------------------
    // Row 2 — ingestion.
    // -----------------------------------------------------------------
    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          '## Ingestion',
          '',
          'Live ledger ingestion: age of the oldest ledger notification waiting in the ingest queue, dead-letter queue depth, ledger-processor duration / errors / concurrency, and how long each candle INSERT into ClickHouse takes (p50, p95, max). A retried write is measured as one long write.',
        ].join('\n'),
        width: 24,
        height: 3,
      }),
    );

    this.dashboard.addWidgets(
      new cloudwatch.GraphWidget({
        title: 'Ingest queue — oldest message age (s)',
        left: [
          new cloudwatch.Metric({
            namespace: 'AWS/SQS',
            metricName: 'ApproximateAgeOfOldestMessage',
            dimensionsMap: { QueueName: ingestQueue },
            statistic: 'Maximum',
            label: 'oldest doorbell age',
            period: cdk.Duration.minutes(1),
          }),
        ],
        // The same number the ledger-processor-lag alarm fires on, read from
        // the same config key, so the line and the alarm cannot drift apart.
        leftAnnotations: [
          {
            value: config.opsAlarms.ledgerProcessorLagSeconds,
            label: `alarm at ${config.opsAlarms.ledgerProcessorLagSeconds}s`,
            color: '#d13212',
          },
        ],
        width: 6,
        height: 6,
      }),
      new cloudwatch.GraphWidget({
        title: 'Ingest DLQ — messages visible',
        // SQS stops publishing for a queue that has been idle for hours, so an
        // empty DLQ would show as "No data". For a dead-letter queue no data
        // means empty, so draw the zero.
        left: [
          new cloudwatch.MathExpression({
            expression: 'FILL(dlq, 0)',
            usingMetrics: {
              dlq: new cloudwatch.Metric({
                namespace: 'AWS/SQS',
                metricName: 'ApproximateNumberOfMessagesVisible',
                dimensionsMap: { QueueName: ingestDlq },
                statistic: 'Maximum',
                period: cdk.Duration.minutes(1),
              }),
            },
            period: cdk.Duration.minutes(1),
            label: 'DLQ depth',
          }),
        ],
        width: 6,
        height: 6,
      }),
      new cloudwatch.GraphWidget({
        title: 'Ledger-processor — duration, errors, concurrency',
        left: [
          new cloudwatch.Metric({
            namespace: 'AWS/Lambda',
            metricName: 'Duration',
            dimensionsMap: { FunctionName: ledgerProcessorFnName },
            statistic: 'p95',
            label: 'duration p95 (ms)',
            period: cdk.Duration.minutes(5),
          }),
        ],
        right: [
          new cloudwatch.Metric({
            namespace: 'AWS/Lambda',
            metricName: 'Errors',
            dimensionsMap: { FunctionName: ledgerProcessorFnName },
            statistic: 'Sum',
            label: 'errors',
            period: cdk.Duration.minutes(5),
          }),
          new cloudwatch.Metric({
            namespace: 'AWS/Lambda',
            metricName: 'ConcurrentExecutions',
            dimensionsMap: { FunctionName: ledgerProcessorFnName },
            statistic: 'Maximum',
            label: 'concurrent executions',
            period: cdk.Duration.minutes(5),
          }),
        ],
        width: 6,
        height: 6,
      }),
      new cloudwatch.GraphWidget({
        title: 'ClickHouse candle-write latency (ms)',
        left: [
          chWriteLatency('p50'),
          chWriteLatency('p95'),
          chWriteLatency('Maximum'),
        ],
        width: 6,
        height: 6,
      }),
    );

    // -----------------------------------------------------------------
    // Row 3 — ClickHouse and backfill. A TREND row: every widget carries its
    // own 14-day window and 1-hour period rather than moving the dashboard's
    // global range, because the backfill trajectory is invisible at 1 hour and
    // a latency spike is invisible at 2 weeks. `periodOverride: INHERIT` on the
    // Dashboard above is what makes these take effect.
    // -----------------------------------------------------------------
    const trend14d = { start: '-P14D', period: cdk.Duration.hours(1) };
    // The seven OHLCV tiers, taken from the same config the per-tier freshness
    // alarms are built from, so a tier added there appears here too. Note
    // `price_ohlcv_1m` and `price_ohlcv_1M` differ only in case.
    const rollupTables = Object.keys(config.opsAlarms.rollupLagSeconds);

    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          '## ClickHouse and backfill — 14-day trend',
          '',
          'The ClickHouse host and the data behind the API: free disk on the host, how far each OHLCV rollup table lags behind the 1-minute source, time since the last backfill push for the soroban_amm stream, and days left on the mTLS client certificate.',
        ].join('\n'),
        width: 24,
        height: 3,
      }),
    );

    this.dashboard.addWidgets(
      new cloudwatch.GraphWidget({
        title: 'ClickHouse host — free disk (%)',
        left: [
          new cloudwatch.Metric({
            namespace: 'Prices/Rollup',
            metricName: 'ClickHouseDiskFreePercent',
            dimensionsMap: envDims,
            statistic: 'Minimum',
            label: 'free disk (%)',
          }),
        ],
        // Same key the ch-disk-free alarm reads.
        leftAnnotations: [
          {
            value: config.opsAlarms.chDiskFreePercent,
            label: `alarm below ${config.opsAlarms.chDiskFreePercent}%`,
            color: '#d13212',
            fill: cloudwatch.Shading.BELOW,
          },
        ],
        width: 6,
        height: 6,
        ...trend14d,
      }),
      new cloudwatch.GraphWidget({
        title: 'Rollup lag per table (s)',
        left: rollupTables.map(
          (table) =>
            new cloudwatch.Metric({
              namespace: 'Prices/Rollup',
              metricName: 'RollupLagSeconds',
              dimensionsMap: { ...envDims, Table: table },
              statistic: 'Maximum',
              label: table,
            }),
        ),
        width: 6,
        height: 6,
        ...trend14d,
      }),
      new cloudwatch.GraphWidget({
        title: 'Backfill push age (s) — soroban_amm',
        left: [
          new cloudwatch.Metric({
            namespace: 'Prices/Backfill',
            metricName: 'PushAgeSeconds',
            // `soroban_amm` is the ONLY stream dimension that has ever
            // published. The probe defines an archive stream constant too, but
            // it has never emitted a datapoint, so a widget on it would render
            // empty and fail AC 1.
            dimensionsMap: { ...envDims, Stream: 'soroban_amm' },
            statistic: 'Maximum',
            label: 'push age',
          }),
        ],
        width: 6,
        height: 6,
        ...trend14d,
      }),
      new cloudwatch.SingleValueWidget({
        // A single-value tile has no annotation line, so the threshold the
        // mtls-notafter alarm fires on goes into the title, from the same key.
        title: `mTLS client cert — days to NotAfter (alarm below ${config.opsAlarms.mtlsNotAfterDaysThreshold})`,
        // `MinDaysToNotAfter` carries the `Environment` dimension ONLY. The
        // per-role series is a different metric name; adding a Role dimension
        // here yields nothing.
        metrics: [
          new cloudwatch.Metric({
            namespace: 'Prices/Mtls',
            metricName: 'MinDaysToNotAfter',
            dimensionsMap: envDims,
            statistic: 'Minimum',
            label: 'days to expiry',
          }),
        ],
        width: 6,
        height: 6,
        ...trend14d,
      }),
    );

    // -----------------------------------------------------------------
    // Row 4 — scheduled workers. A TREND row (7 days, 1-hour period): these run
    // hourly to daily and would show three sparse points on the 3-hour range.
    //
    // The `cleanup` worker is EXCLUDED: its EventBridge rule is disabled on
    // purpose, so it emits no `AWS/Lambda` metrics at all and would be a
    // permanently empty series. It still appears in the alarm strip above.
    // -----------------------------------------------------------------
    const trend7d = { start: '-P7D', period: cdk.Duration.hours(1) };
    const dashboardWorkers = SCHEDULED_WORKERS.filter(
      (name) => !SCHEDULE_DISABLED_WORKERS.includes(name),
    );
    const workerMetric = (
      metricName: string,
      statistic: string,
      suffix = '',
    ): cloudwatch.Metric[] =>
      dashboardWorkers.map(
        (name) =>
          new cloudwatch.Metric({
            namespace: 'AWS/Lambda',
            metricName,
            dimensionsMap: {
              FunctionName: workerFunctionName(config.envName, name),
            },
            statistic,
            label: `${name}${suffix}`,
          }),
      );

    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          '## Scheduled workers — 7-day trend',
          '',
          'Duration (p95) and error / throttle counts for every scheduled worker Lambda: asset discovery, supply, oracle, enrichment, coarse sweep and the three probes. The `cleanup` worker is not charted — its schedule is disabled, so it publishes no metrics.',
        ].join('\n'),
        width: 24,
        height: 3,
      }),
    );

    this.dashboard.addWidgets(
      new cloudwatch.GraphWidget({
        title: 'Worker duration p95 (ms)',
        left: workerMetric('Duration', 'p95'),
        width: 12,
        height: 6,
        ...trend7d,
      }),
      new cloudwatch.GraphWidget({
        title: 'Worker errors and throttles',
        left: workerMetric('Errors', 'Sum'),
        right: workerMetric('Throttles', 'Sum', ' (throttles)'),
        width: 12,
        height: 6,
        ...trend7d,
      }),
    );

    // -----------------------------------------------------------------
    // Row 5 — enrichment and oracle. Same 7-day trend window.
    // -----------------------------------------------------------------
    const enrichmentMetric = (
      metricName: string,
      statistic: string,
      label: string,
    ): cloudwatch.Metric =>
      new cloudwatch.Metric({
        namespace: 'Prices/Enrichment',
        metricName,
        dimensionsMap: envDims,
        statistic,
        label,
      });

    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          '## Enrichment and oracle — 7-day trend',
          '',
          'USD enrichment of candles and the Reflector oracle feed: rows enriched per pass, oracle misses and the recent backlog, pass and batch duration, the volume-zero floor (candles with no USD reference — this number only ever grows by design and is not a backlog), and oracle rows written vs. timestamps rejected.',
        ].join('\n'),
        width: 24,
        height: 3,
      }),
    );

    this.dashboard.addWidgets(
      new cloudwatch.GraphWidget({
        title: 'Enrichment progress',
        left: [
          enrichmentMetric('EnrichmentRowsEnriched', 'Sum', 'rows enriched'),
          enrichmentMetric('EnrichmentOracleMiss', 'Sum', 'oracle misses'),
        ],
        right: [
          enrichmentMetric(
            'EnrichmentRowsRemainingRecent',
            'Maximum',
            'recent backlog',
          ),
        ],
        width: 8,
        height: 6,
        ...trend7d,
      }),
      new cloudwatch.GraphWidget({
        title: 'Enrichment pass and batch duration (ms)',
        left: [
          enrichmentMetric(
            'EnrichmentPassDurationMs',
            'Average',
            'pass duration',
          ),
          enrichmentMetric(
            'EnrichmentAvgBatchDurationMs',
            'Average',
            'avg batch duration',
          ),
        ],
        width: 8,
        height: 6,
        ...trend7d,
      }),
      new cloudwatch.GraphWidget({
        title: 'Volume-zero floor (grows by design, not a backlog)',
        left: [
          enrichmentMetric(
            'EnrichmentRowsRemainingAtVolumeZero',
            'Maximum',
            'exotic-quote no-reference floor',
          ),
        ],
        width: 4,
        height: 6,
        ...trend7d,
      }),
      new cloudwatch.GraphWidget({
        title: 'Oracle feed',
        left: [
          new cloudwatch.Metric({
            namespace: 'Prices/Oracle',
            metricName: 'OracleRowsWritten',
            dimensionsMap: envDims,
            statistic: 'Sum',
            label: 'rows written',
          }),
        ],
        right: [
          new cloudwatch.Metric({
            namespace: 'Prices/Oracle',
            metricName: 'OracleTimestampRejected',
            dimensionsMap: envDims,
            statistic: 'Sum',
            label: 'timestamps rejected',
          }),
        ],
        width: 4,
        height: 6,
        ...trend7d,
      }),
    );

    // -----------------------------------------------------------------
    // Read-only viewer identity for the Stellar reviewer (Tranche 3 AC 8).
    //
    // An IAM *user*, not a cross-account role: there is no external principal
    // to trust — none is known. A named substitution, like Decision A above.
    //
    // A hand-written read policy, not `CloudWatchReadOnlyAccess`: that managed
    // policy also grants `logs:FilterLogEvents` / `logs:Get*` and `xray:Get*`
    // on every log group and trace in the account — which is shared with the
    // block explorer. The dashboard has no log widgets, so the viewer needs
    // exactly the dashboard, metric and alarm read calls below and nothing
    // that can read log contents (task 0125 deep review CR-01). These
    // CloudWatch read actions do not support resource-level scoping.
    //
    // NO password is passed: `iam.User` creates a login profile only when one is
    // supplied, and a password in the template is precisely what is being
    // avoided. Creating the console login is an out-of-band operator step
    // (`aws iam create-login-profile --password-reset-required`).
    // -----------------------------------------------------------------
    const stellarViewer = new iam.User(this, 'StellarDashboardViewer', {
      userName: `prices-${config.envName}-stellar-viewer`,
    });
    stellarViewer.attachInlinePolicy(
      new iam.Policy(this, 'StellarDashboardViewerPolicy', {
        policyName: `prices-${config.envName}-dashboard-read`,
        statements: [
          new iam.PolicyStatement({
            sid: 'ReadDashboardsMetricsAndAlarms',
            actions: [
              'cloudwatch:GetDashboard',
              'cloudwatch:ListDashboards',
              'cloudwatch:GetMetricData',
              'cloudwatch:GetMetricStatistics',
              'cloudwatch:ListMetrics',
              'cloudwatch:GetMetricWidgetImage',
              'cloudwatch:DescribeAlarms',
              'cloudwatch:DescribeAlarmHistory',
              'cloudwatch:DescribeAlarmsForMetric',
            ],
            resources: ['*'],
          }),
        ],
      }),
    );

    new cdk.CfnOutput(this, 'StellarViewerUserName', {
      value: stellarViewer.userName,
      description: `Read-only CloudWatch viewer IAM user for ${config.envName} (console login is created out of band)`,
    });

    assertAlarmDescriptionsFitCloudWatch(this);
  }
}

/**
 * Cadence of an EventBridge schedule expression, in minutes, or `undefined`
 * for a `cron(...)` expression whose cadence cannot be read off the string.
 *
 * ⚠️ Throws on a `rate(...)` it cannot parse rather than returning `undefined`.
 * That distinction is the whole point: silently treating an unreadable *rate*
 * as "unknown cadence" is how a guard passes a schedule it was written to
 * catch. `rate(1 hour)` is not hypothetical — it is the idiom already used for
 * `assetSupply`, `assetDiscovery` and `enrichment` in the same
 * `production.json`, so it is exactly how someone would slow a poll down. An
 * earlier version of this matched only `minutes?` and would have let every
 * hour- and day-valued rate through as if it were a cron.
 *
 * EventBridge `rate()` accepts minute(s), hour(s) and day(s) only.
 */
function rateExpressionMinutes(expression: string): number | undefined {
  const trimmed = expression.trim();
  if (!trimmed.startsWith('rate(')) return undefined;
  const parsed = /^rate\((\d+)\s+(minute|hour|day)s?\)$/.exec(trimmed);
  if (!parsed) {
    throw new Error(
      `unreadable EventBridge rate expression "${expression}"; expected ` +
        `rate(N minutes|hours|days). Fix the schedule, or the cadence-dependent ` +
        `alarm guards in observability-stack.ts cannot check it.`,
    );
  }
  const value = Number(parsed[1]);
  switch (parsed[2]) {
    case 'hour':
      return value * 60;
    case 'day':
      return value * 60 * 24;
    default:
      return value;
  }
}

/**
 * CloudWatch caps `AlarmDescription` at 1024 characters, and **nothing local
 * enforces it**: `cdk synth` renders an over-long description happily, the
 * template is valid CloudFormation, and the request is only rejected by the
 * CloudWatch API mid-deploy — after some alarms in the stack have already been
 * created (task 0204, 2026-08-20; three `usd-stranded` rungs at ~1250 chars).
 *
 * ⚠️ The alarms in this stack carry deliberately long, runbook-style
 * descriptions, because an operator reading Slack at 03:00 has nothing else.
 * That is worth keeping — but it means this ceiling will be hit again, and a
 * failure discovered at deploy time is the most expensive place to discover it.
 *
 * So the check runs at synth: a walk of the construct tree that throws with the
 * offending alarm and its length. It reads the resolved CloudFormation property
 * rather than the constructor argument, so descriptions built from tokens or
 * `Fn::Join` are measured as CloudWatch will actually see them.
 */
function assertAlarmDescriptionsFitCloudWatch(scope: Construct): void {
  const MAX = 1024;
  const tooLong = scope.node
    .findAll()
    .filter((c): c is cloudwatch.Alarm => c instanceof cloudwatch.Alarm)
    .map((alarm) => {
      const cfn = alarm.node.defaultChild as cloudwatch.CfnAlarm;
      const description = cdk.Stack.of(alarm).resolve(
        cfn.alarmDescription,
      ) as unknown;
      const length = typeof description === 'string' ? description.length : 0;
      return { name: cfn.alarmName, length };
    })
    .filter((a) => a.length > MAX);

  if (tooLong.length > 0) {
    const detail = tooLong
      .map((a) => `  ${String(a.name)} — ${a.length} chars`)
      .join('\n');
    throw new Error(
      `${tooLong.length} CloudWatch alarm description(s) exceed the ${MAX}-character ` +
        `API limit and would fail mid-deploy:\n${detail}\n` +
        'Shorten the description(s); the limit is enforced by CloudWatch, not by synth.',
    );
  }
}
