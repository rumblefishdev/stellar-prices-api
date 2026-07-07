import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import * as cw_actions from 'aws-cdk-lib/aws-cloudwatch-actions';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as subscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

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
  /** SDEX push-freshness alarm (§5.6 / Tranche-1 AC #5). */
  public readonly sdexPushFreshnessAlarm: cloudwatch.Alarm;
  /** mTLS client-cert expiry alarm (§7 / §11.4). */
  public readonly mtlsNotAfterAlarm: cloudwatch.Alarm;
  /** Live ledger-processor ingestion-lag alarm (task 0056 finding B). */
  public readonly ledgerProcessorLagAlarm: cloudwatch.Alarm;
  /** Live ledger-processor invocation-error alarm (task 0056 finding B). */
  public readonly ledgerProcessorErrorAlarm: cloudwatch.Alarm;
  /** Live ledger-processor DLQ-depth alarm (task 0056 finding B). */
  public readonly ledgerProcessorDlqAlarm: cloudwatch.Alarm;

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
    const snsAction = new cw_actions.SnsAction(this.opsAlarmsTopic);

    // The enrichment backlog alarm (task 0026) shipped without an action; wire
    // it to the ops topic now that 0056 owns the alarm-routing.
    this.enrichmentBacklogAlarm.addAlarmAction(snsAction);

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

    // -----------------------------------------------------------------
    // Live ledger-processor health (task 0056 finding B). The core ingestion
    // Lambda shipped unmonitored: `prices.ledger_processor.lag_seconds` existed
    // only as a comment and no alarm watched it. These three alarms cover it
    // from AWS-native metrics, so no custom-metric emission / processor redeploy
    // is required. Metrics are built by deterministic physical name — the
    // function `prices-{env}-ledger-processor`, the queue `prices-ingest-{env}`,
    // and its DLQ `prices-ingest-dlq-{env}` (all owned by ComputeStack) — so
    // ObservabilityStack stays independently deployable with no cross-stack CFN
    // reference, exactly like the ops-topic name-import pattern above.
    const ledgerProcessorFnName = `prices-${config.envName}-ledger-processor`;
    const ingestQueueName = `prices-ingest-${config.envName}`;
    const ingestDlqName = `prices-ingest-dlq-${config.envName}`;

    // Ingestion lag: the ledger-processor emits no lag metric, so we watch the
    // ingest queue's oldest-message age — the honest "processor is falling
    // behind" signal. Ledgers close ~every 5–6 s and a healthy processor drains
    // the doorbell in seconds; an oldest-message age sustained above the
    // threshold (default 60 s) means live ingestion is lagging. Sustained over
    // 3×1-min datapoints so a momentary catch-up spike does not flap.
    this.ledgerProcessorLagAlarm = new cloudwatch.Alarm(
      this,
      'LedgerProcessorLagAlarm',
      {
        alarmName: `prices-${config.envName}-ledger-processor-lag`,
        alarmDescription:
          'The prices-ingest SQS doorbell is backing up (ApproximateAgeOfOldestMessage over the threshold, sustained 3 min): the live ledger-processor is not keeping up with ledger production or has stalled. Threshold is operator-tunable via config.opsAlarms.ledgerProcessorLagSeconds. Check the ledger-processor logs, the mTLS ClickHouse write path, and the DLQ.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/SQS',
          metricName: 'ApproximateAgeOfOldestMessage',
          dimensionsMap: { QueueName: ingestQueueName },
          statistic: 'Maximum',
          period: cdk.Duration.minutes(1),
        }),
        threshold: config.opsAlarms.ledgerProcessorLagSeconds,
        evaluationPeriods: 3,
        datapointsToAlarm: 3,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        // An empty queue publishes no datapoint; missing data is healthy (the
        // processor is caught up), so it must not breach.
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      },
    );
    this.ledgerProcessorLagAlarm.addAlarmAction(snsAction);

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
          dimensionsMap: { QueueName: ingestDlqName },
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

    new cdk.CfnOutput(this, 'OpsAlarmsTopicArn', {
      value: this.opsAlarmsTopic.topicArn,
      description: `Ops-alarms SNS topic ARN for ${config.envName}`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);
  }
}
