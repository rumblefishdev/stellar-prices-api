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
import { workerFunctionName } from '../lambda-baseline.js';
import {
  ingestDlqName,
  ingestQueueName,
  ledgerProcessorFunctionName,
} from './compute-stack.js';

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
  // or is failing to invoke. `treatMissingData: BREACHING` is load-bearing —
  // Lambda publishes NO Invocations datapoint for a period with zero
  // invocations, so a LESS_THAN threshold alone would never evaluate.
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
      metric: metric('Invocations', 'Sum', cadence),
      threshold: 1,
      evaluationPeriods: 3,
      datapointsToAlarm: 3,
      comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
      // Missing = zero invocations = the halt we are looking for.
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

  /** Write-amplification guardrail (task 0133). */
  public readonly writeAmplificationAlarm: cloudwatch.Alarm;
  /** Live ledger-processor ingestion-lag alarm (task 0056 finding B). */
  public readonly ledgerProcessorLagAlarm: cloudwatch.Alarm;
  /** Live ledger-processor invocation-error alarm (task 0056 finding B). */
  public readonly ledgerProcessorErrorAlarm: cloudwatch.Alarm;
  /** Live ledger-processor DLQ-depth alarm (task 0056 finding B). */
  public readonly ledgerProcessorDlqAlarm: cloudwatch.Alarm;
  /** Live ledger-processor total-halt alarm — zero invocations (finding B / halt gap). */
  public readonly ledgerProcessorNoInvocationsAlarm: cloudwatch.Alarm;
  /**
   * Platform-metric health alarms for the scheduled workers whose only other
   * alarm reads a metric the worker itself publishes (task 0112). Keyed by
   * worker name: `errors`, `duration`, `noInvocations`.
   */
  public readonly workerHealthAlarms: Record<string, WorkerHealthAlarms>;

  constructor(scope: Construct, id: string, props: ObservabilityStackProps) {
    super(scope, id, props);

    const { config } = props;

    this.dashboard = new cloudwatch.Dashboard(this, 'OverviewDashboard', {
      dashboardName: `prices-${config.envName}-overview`,
    });

    this.dashboard.addWidgets(
      new cloudwatch.TextWidget({
        markdown: [
          `# prices-api / ${config.envName}`,
          '',
          'Scaffold dashboard. Widgets and alarms land in task 0056.',
          '',
          'See `infra/src/lib/stacks/observability-stack.ts` for the planned alarm set.',
        ].join('\n'),
        width: 24,
        height: 4,
      }),
    );

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

    // Write amplification (task 0133 — the guardrail for the 0132 egress bug).
    // The write-amplification-probe publishes the max rows-written-per-hour
    // across all prices tables as Prices/Ingest MaxRowsWrittenPerHour; alarm
    // when it exceeds the operator-tuned threshold (well above the busiest legit
    // table, far below a 0132-class ~130M/hour runaway). A quiet hour publishes a
    // real 0 (healthy), so missing data is non-breaching — probe-down is covered
    // by the probe's own error alarm.
    this.writeAmplificationAlarm = new cloudwatch.Alarm(
      this,
      'WriteAmplificationAlarm',
      {
        alarmName: `prices-${config.envName}-write-amplification`,
        alarmDescription:
          'A prices table is being written far more than any legitimate table (rows-written-per-hour above config.opsAlarms.writeAmplificationRowsPerHour). Likely a write-amplification regression like task 0132 (full-registry re-emit). Check system.part_log per table to find the offender.',
        metric: new cloudwatch.Metric({
          namespace: 'Prices/Ingest',
          metricName: 'MaxRowsWrittenPerHour',
          dimensionsMap: { Environment: config.envName },
          statistic: 'Maximum',
          period: cdk.Duration.hours(1),
        }),
        threshold: config.opsAlarms.writeAmplificationRowsPerHour,
        evaluationPeriods: 1,
        datapointsToAlarm: 1,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.writeAmplificationAlarm.addAlarmAction(snsAction);
    this.writeAmplificationAlarm.addOkAction(snsAction);

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
          'A ledger doorbell exhausted its SQS retries and landed in the prices-ingest DLQ (ApproximateNumberOfMessagesVisible ≥ 1): a ledger the live processor could not process = a candle gap. Inspect the DLQ message, fix the cause, and redrive.',
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

    // Total ingestion halt: the lag / errors / DLQ alarms above all key on the
    // *presence* of enqueued or failed messages, so a producer-side stop (BE's
    // S3→SNS→SQS delivery halts, the subscription is deleted, or upstream simply
    // stops publishing) is invisible to them — the queue drains to empty, the
    // Lambda is never invoked, and all three sit OK while live candles silently
    // stop. This alarm closes that blind spot from the consumer side: if the
    // ledger-processor records zero `Invocations` for 15 min it has received no
    // doorbells at all. Pubnet closes a ledger every ~5–6 s, so a healthy
    // processor is invoked near-continuously and a 15-min silence is a genuine
    // outage, never normal idle. `treatMissingData: BREACHING` is load-bearing:
    // Lambda publishes NO `Invocations` datapoint for a period with zero
    // invocations, so "missing" *is* the halt signal (a LESS_THAN threshold
    // alone would never evaluate).
    this.ledgerProcessorNoInvocationsAlarm = new cloudwatch.Alarm(
      this,
      'LedgerProcessorNoInvocationsAlarm',
      {
        alarmName: `prices-${config.envName}-ledger-processor-no-invocations`,
        alarmDescription:
          'The live ledger-processor recorded zero invocations for 15 min: no ledger doorbells are arriving (upstream S3→SNS→SQS delivery stopped, the subscription was removed, or the producer halted). Live ingestion is stalled at the source and candles are silently frozen. Check the SNS subscription on prices-ingest, BE ledger publication, and the ingest queue. Unlike the lag/errors/DLQ alarms this fires on the ABSENCE of throughput.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/Lambda',
          metricName: 'Invocations',
          dimensionsMap: { FunctionName: ledgerProcessorFnName },
          statistic: 'Sum',
          period: cdk.Duration.minutes(15),
        }),
        threshold: 1,
        evaluationPeriods: 1,
        datapointsToAlarm: 1,
        comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
        // Missing = zero invocations = the halt we are looking for.
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
  }
}
