// Config
export type { CicdConfig, EnvironmentConfig } from './lib/types.js';
export { validateConfig } from './lib/types.js';

// Stacks
export { CicdStack } from './lib/stacks/cicd-stack.js';
export type { CicdStackProps } from './lib/stacks/cicd-stack.js';
export { SecretsStack } from './lib/stacks/secrets-stack.js';
export type { SecretsStackProps } from './lib/stacks/secrets-stack.js';
export { ComputeStack } from './lib/stacks/compute-stack.js';
export type { ComputeStackProps } from './lib/stacks/compute-stack.js';
export { ApiGatewayStack } from './lib/stacks/api-gateway-stack.js';
export type { ApiGatewayStackProps } from './lib/stacks/api-gateway-stack.js';
export { EventBridgeStack } from './lib/stacks/eventbridge-stack.js';
export type { EventBridgeStackProps } from './lib/stacks/eventbridge-stack.js';

// Lambda baseline helpers — used by downstream stacks that add
// additional Lambdas (0038, 0039, 0040, 0055).
export {
  baselineLambdaPolicyStatements,
  createPricesLambdaRole,
  lambdaLogGroupName,
  pricesLambdaDefaults,
  PRICES_LAMBDA_LOG_RETENTION,
  PRICES_LAMBDA_LOG_REMOVAL_POLICY,
} from './lib/lambda-baseline.js';
export type { BaselineLambdaContext } from './lib/lambda-baseline.js';
