import * as cdk from 'aws-cdk-lib';
import * as events from 'aws-cdk-lib/aws-events';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface EventBridgeStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * EventBridge Scheduler rules for the four periodic prices-api
 * workers (task 0039).
 *
 * Rule shells only — no `target` attached. Task 0039 creates the
 * worker Lambdas (using `createPricesLambdaRole` from
 * `lib/lambda-baseline.ts`) and calls `rule.addTarget(...)` on the
 * properties this stack exposes.
 *
 * The Rollup worker that appeared in the original task 0039 spec
 * is intentionally absent — ADR 0007 §3.4 replaces it with a
 * ClickHouse materialised-view chain (1m → 15m → 1h → 4h → 1d
 * → 1w → 1M). Pre-creating an unused rule would just be a
 * resource for future-0039 to delete.
 */
export class EventBridgeStack extends cdk.Stack {
  public readonly priceUpdaterRule: events.Rule;
  public readonly oracleWatcherRule: events.Rule;
  public readonly assetDiscoveryRule: events.Rule;
  public readonly cleanupRule: events.Rule;

  constructor(scope: Construct, id: string, props: EventBridgeStackProps) {
    super(scope, id, props);

    const { config } = props;
    const env = config.envName;
    const schedules = config.scheduleExpressions;

    this.priceUpdaterRule = new events.Rule(this, 'PriceUpdaterRule', {
      ruleName: `prices-${env}-price-updater`,
      description: `Refreshes current_prices aggregations (${env})`,
      schedule: events.Schedule.expression(schedules.priceUpdater),
    });

    this.oracleWatcherRule = new events.Rule(this, 'OracleWatcherRule', {
      ruleName: `prices-${env}-oracle-watcher`,
      description: `Polls Stellar on-chain oracles (${env})`,
      schedule: events.Schedule.expression(schedules.oracleWatcher),
    });

    this.assetDiscoveryRule = new events.Rule(this, 'AssetDiscoveryRule', {
      ruleName: `prices-${env}-asset-discovery`,
      description: `Periodic asset-registry maintenance (${env})`,
      schedule: events.Schedule.expression(schedules.assetDiscovery),
    });

    this.cleanupRule = new events.Rule(this, 'CleanupRule', {
      ruleName: `prices-${env}-cleanup`,
      description: `Old-data partition drop on prices.* tables (${env})`,
      schedule: events.Schedule.expression(schedules.cleanup),
    });

    new cdk.CfnOutput(this, 'PriceUpdaterRuleArn', {
      value: this.priceUpdaterRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'OracleWatcherRuleArn', {
      value: this.oracleWatcherRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'AssetDiscoveryRuleArn', {
      value: this.assetDiscoveryRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'CleanupRuleArn', {
      value: this.cleanupRule.ruleArn,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', env);
  }
}
