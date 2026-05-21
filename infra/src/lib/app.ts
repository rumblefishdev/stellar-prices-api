import * as cdk from 'aws-cdk-lib';

import { validateConfig, type EnvironmentConfig } from './types.js';
import { SecretsStack } from './stacks/secrets-stack.js';
import { ComputeStack } from './stacks/compute-stack.js';
import { ApiGatewayStack } from './stacks/api-gateway-stack.js';

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

  const secrets = new SecretsStack(app, `${prefix}-Secrets`, { env, config });

  const compute = new ComputeStack(app, `${prefix}-Compute`, {
    env,
    config,
    mtlsCertSecret: secrets.mtlsCertSecret,
    mtlsKeySecret: secrets.mtlsKeySecret,
  });
  compute.addDependency(secrets);

  // ApiGatewayStack is independent of ComputeStack in the skeleton
  // (no Lambda integration yet — task 0040 wires the cross-stack
  // dependency when it attaches the apiHandler Function).
  new ApiGatewayStack(app, `${prefix}-ApiGateway`, { env, config });

  app.synth();
}
