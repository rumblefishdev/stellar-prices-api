import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
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

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);
  }
}
