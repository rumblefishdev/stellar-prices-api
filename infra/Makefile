STAGING_APP := node dist/bin/staging.js
PRODUCTION_APP := node dist/bin/production.js
CICD_APP := node dist/bin/cicd.js

build:
	cd .. && npx nx build @rumblefish/stellar-prices-api-aws-cdk

# --- CI/CD ---
# Deployed once per AWS account. Provisions the GitHub Actions OIDC
# provider and per-env deploy roles (staging, production). Role
# ARNs land as CfnOutputs — wire them into GitHub Environment
# secrets as AWS_DEPLOY_ROLE_ARN.

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

# --- Staging ---

synth-staging: build
	npx cdk --app "$(STAGING_APP)" synth

diff-staging: build
	npx cdk --app "$(STAGING_APP)" diff

deploy-staging: build
	npx cdk --app "$(STAGING_APP)" deploy --all --require-approval broadening

destroy-staging: build
	npx cdk --app "$(STAGING_APP)" destroy --all --force

deploy-staging-secrets: build
	npx cdk --app "$(STAGING_APP)" deploy Prices-staging-Secrets --require-approval broadening

destroy-staging-secrets: build
	npx cdk --app "$(STAGING_APP)" destroy Prices-staging-Secrets --force

deploy-staging-compute: build
	npx cdk --app "$(STAGING_APP)" deploy Prices-staging-Compute --require-approval broadening

destroy-staging-compute: build
	npx cdk --app "$(STAGING_APP)" destroy Prices-staging-Compute --force

# --- Production ---

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
