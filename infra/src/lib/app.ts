import * as cdk from 'aws-cdk-lib';

import { validateConfig, type EnvironmentConfig } from './types.js';
import { SecretsStack } from './stacks/secrets-stack.js';
import { ComputeStack } from './stacks/compute-stack.js';
import { ApiGatewayStack } from './stacks/api-gateway-stack.js';
import { EventBridgeStack } from './stacks/eventbridge-stack.js';
import { ObservabilityStack } from './stacks/observability-stack.js';

export interface CreateAppOptions {
  readonly config: EnvironmentConfig;
}

export function createApp({ config }: CreateAppOptions): void {
  validateConfig(config);

  const app = new cdk.App();

  const env: cdk.Environment = {
    account: process.env['CDK_DEFAULT_ACCOUNT'],
    region: config.awsRegion,
  };

  const prefix = `Prices-${config.envName}`;

  // SecretsStack only publishes the mTLS bundle secret NAMES to SSM — it does
  // not create the secrets (operator-issued out-of-band; BE-mirroring). So
  // ComputeStack derives its own secret names from the shared `mtlsSecretName`
  // helper and needs no cross-stack reference / dependency on SecretsStack.
  new SecretsStack(app, `${prefix}-Secrets`, { env, config });

  new ComputeStack(app, `${prefix}-Compute`, { env, config });

  // ApiGatewayStack is independent of ComputeStack in the skeleton
  // (no Lambda integration yet — task 0040 wires the cross-stack
  // dependency when it attaches the apiHandler Function).
  new ApiGatewayStack(app, `${prefix}-ApiGateway`, { env, config });

  // EventBridgeStack is independent of ComputeStack in the skeleton
  // (no Lambda targets yet — task 0039 wires the cross-stack
  // dependency when it attaches its four worker Lambdas via
  // rule.addTarget()).
  new EventBridgeStack(app, `${prefix}-EventBridge`, { env, config });

  // ObservabilityStack is independent of every other stack at the
  // skeleton stage. Task 0056 attaches widgets/alarms that reference
  // ComputeStack log groups and ApiGatewayStack metrics; the
  // cross-stack dependency arrives then.
  new ObservabilityStack(app, `${prefix}-Observability`, { env, config });

  app.synth();
}
