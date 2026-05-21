import * as cdk from 'aws-cdk-lib';

import { validateConfig, type EnvironmentConfig } from './types.js';
import { SecretsStack } from './stacks/secrets-stack.js';

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

  new SecretsStack(app, `${prefix}-Secrets`, { env, config });

  app.synth();
}
