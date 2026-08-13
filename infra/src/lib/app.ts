import * as cdk from 'aws-cdk-lib';

import { validateConfig, type EnvironmentConfig } from './types.js';
import { SecretsStack } from './stacks/secrets-stack.js';
import { ComputeStack } from './stacks/compute-stack.js';
import { ApiGatewayStack } from './stacks/api-gateway-stack.js';
import { EventBridgeStack } from './stacks/eventbridge-stack.js';
import { ObservabilityStack } from './stacks/observability-stack.js';
import { PortalHostingStack } from './stacks/portal-hosting-stack.js';

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

  const compute = new ComputeStack(app, `${prefix}-Compute`, { env, config });

  // ApiGatewayStack proxies all /v1 routes to ComputeStack's single axum
  // api-handler Lambda (ADR 0008). Passing the Function in creates the
  // cross-stack dependency (CFN export/import); CDK orders Compute before
  // ApiGateway automatically.
  const apiGateway = new ApiGatewayStack(app, `${prefix}-ApiGateway`, {
    env,
    config,
    apiHandlerFunction: compute.apiHandlerFunction,
  });

  // PortalHostingStack fronts both the bundle bucket and the API on one
  // CloudFront distribution, so it needs the REST API id to build the
  // execute-api origin hostname. Passing it in creates the cross-stack
  // dependency, which orders ApiGateway before PortalHosting.
  //
  // The distribution lives in this stack's region like everything else: it
  // needs no us-east-1 certificate, because it has no custom domain until task
  // 0195 adds one (and that certificate is that task's problem).
  new PortalHostingStack(app, `${prefix}-PortalHosting`, {
    env,
    config,
    restApiId: apiGateway.api.restApiId,
  });

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
