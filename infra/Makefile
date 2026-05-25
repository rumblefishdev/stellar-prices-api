PRODUCTION_APP := node dist/bin/production.js
CICD_APP := node dist/bin/cicd.js

build:
	cd .. && npx nx build @rumblefish/stellar-prices-api-aws-cdk

# --- CI/CD ---
# Deployed once per AWS account. Provisions the GitHub Actions OIDC
# provider and the production deploy role. Role ARN lands as a
# CfnOutput — wire it into the GitHub Environment secret as
# AWS_DEPLOY_ROLE_ARN.

bootstrap:
	npx cdk bootstrap

synth-cicd: build
	npx cdk --app "$(CICD_APP)" synth

diff-cicd: build
	npx cdk --app "$(CICD_APP)" diff

deploy-cicd: build
	npx cdk --app "$(CICD_APP)" deploy --all --require-approval broadening

destroy-cicd: build
	npx cdk --app "$(CICD_APP)" destroy --all --force

# --- Production ---
# The only AWS environment. Initially deployed with conservative
# test-sized parameters; production sizing is swapped in via
# envs/production.json once the service is exercised in anger.

synth-production: build
	npx cdk --app "$(PRODUCTION_APP)" synth

diff-production: build
	npx cdk --app "$(PRODUCTION_APP)" diff

deploy-production: build
	npx cdk --app "$(PRODUCTION_APP)" deploy --all --require-approval broadening

destroy-production: build
	npx cdk --app "$(PRODUCTION_APP)" destroy --all --force

deploy-production-secrets: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Secrets --require-approval broadening

destroy-production-secrets: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Secrets --force

deploy-production-compute: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Compute --require-approval broadening

destroy-production-compute: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Compute --force

deploy-production-apigateway: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-ApiGateway --require-approval broadening

destroy-production-apigateway: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-ApiGateway --force

deploy-production-eventbridge: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-EventBridge --require-approval broadening

destroy-production-eventbridge: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-EventBridge --force

deploy-production-observability: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Observability --require-approval broadening

destroy-production-observability: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Observability --force
