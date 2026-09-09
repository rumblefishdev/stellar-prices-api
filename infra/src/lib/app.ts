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

  const compute = new ComputeStack(app, `${prefix}-Compute`, { env, config });

  // ApiGatewayStack proxies all /v1 routes to ComputeStack's single axum
  // api-handler Lambda (ADR 0008). Passing the Function in creates the
  // cross-stack dependency (CFN export/import); CDK orders Compute before
  // ApiGateway automatically.
  const apiGateway = new ApiGatewayStack(app, `${prefix}-ApiGateway`, {
    env,
    config,
    apiHandlerFunction: compute.apiHandlerFunction,
    // The role, so ApiGatewayStack can grant the one control-plane action that
    // needs the usage-plan id (task 0187). Same direction as the Function
    // above, so it adds no new dependency and cannot create a cycle.
    apiHandlerRole: compute.apiHandlerRole,
  });

  // No hosting stack for the portal. `PortalHostingStack` (task 0184) — a
  // private bucket and a CloudFront distribution fronting both the bundle
  // and this API — was retired by task 0195 on 2026-09-01: since task 0194
  // the page is served from the block explorer's distribution at
  // `https://sorobanscan.rumblefish.dev/api/` (its bucket, synced by
  // `make -C infra sync-portal-explorer`) and calls this API on
  // `config.apiDomain` directly, so the distribution had become a second,
  // ungated front door to the same portal. Constructing `ApiGatewayStack` is
  // what has the effect; the binding below is now read by nothing, and the
  // `void` is there to say so deliberately rather than to use it. That is the
  // fact worth keeping: nothing imports the stack's exports any more, so it
  // can be destroyed or have its RestApi replaced without tearing anything
  // down first.
  void apiGateway;

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
