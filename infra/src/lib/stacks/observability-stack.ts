import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import * as cw_actions from 'aws-cdk-lib/aws-cloudwatch-actions';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as subscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface ObservabilityStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * CloudWatch dashboard scaffold for prices-api.
 *
 * Skeleton-only: provisions the empty dashboard with a header
 * TextWidget. Task 0056 attaches the real widgets and alarms:
 *
 * - push-freshness (S3 PutObject → Lambda invocation lag,
 *   alarm > 60s sustained per ADR 0007 / task 0038's
 *   `prices.ledger_processor.lag_seconds` metric)
 * - mTLS NotAfter (cert expiry, alarm 14 days before NotAfter)
 * - Lambda error rate per worker
 * - API Gateway 5xx rate
 * - ClickHouse write latency (custom metric, populated by the
 *   clickhouse-client crate from task 0052)
 *
 * The `dashboard` property is exposed so task 0056 can call
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
   * Alarm on the enrichment backlog (spec §5
   * `EnrichmentRowsRemainingAtVolumeZero`, task 0026). The broader dashboard
   * widget set remains task 0056's; this single alarm ships with 0026 because
   * the metric it watches is emitted by the enrichment worker in the same task.
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

    // Enrichment backlog alarm (task 0026 / spec §5). The worker publishes
    // `EnrichmentRowsRemainingAtVolumeZero` (rows still at volume_quote_usd=0
    // after a pass) under the `Prices/Enrichment` namespace. A sustained high
    // backlog means the pass is not draining — the fingerprint of an
    // oracle↔asset-id mis-reconciliation or missing USDC/USDT/XLM reference
    // assets. Threshold/period are a deliberately conservative scaffold; task
    // 0056 tunes them once real volumes are observed.
    this.enrichmentBacklogAlarm = new cloudwatch.Alarm(
      this,
      'EnrichmentBacklogAlarm',
      {
        alarmName: `prices-${config.envName}-enrichment-backlog`,
        alarmDescription:
          'Enrichment left a large volume_quote_usd=0 backlog across consecutive hourly passes (spec §5 EnrichmentRowsRemainingAtVolumeZero). Sustained high values mean enrichment is not draining — check oracle↔asset-id reconciliation and that USDC/USDT/XLM reference assets exist in prices.assets. Scaffold threshold; tuned in task 0056.',
        metric: new cloudwatch.Metric({
          namespace: 'Prices/Enrichment',
          metricName: 'EnrichmentRowsRemainingAtVolumeZero',
          dimensionsMap: { Environment: config.envName },
          statistic: 'Maximum',
          period: cdk.Duration.hours(1),
        }),
        threshold: 100_000,
        evaluationPeriods: 6,
        datapointsToAlarm: 6,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
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
      topicName: `prices-${config.envName}-ops-alarms`,
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

    new cdk.CfnOutput(this, 'OpsAlarmsTopicArn', {
      value: this.opsAlarmsTopic.topicArn,
      description: `Ops-alarms SNS topic ARN for ${config.envName}`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);
  }
}
