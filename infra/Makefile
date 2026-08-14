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

# The /api-docs-json stage-cache entry has a 3600s TTL, which is only safe
# because the cache is dropped when a new build ships (task 0124). Without this
# the gateway keeps serving the PREVIOUS deployment's OpenAPI document for up to
# an hour after the deploy, to every partner generating a client from it.
#
# Flushes the whole stage cache, not one route — API Gateway has no per-route
# flush. Harmless: every other TTL here is 10-60s on data that self-corrects.
#
# Run after any deploy that can change the served document, i.e. anything that
# ships a new api-handler.
flush-production-cache:
	aws apigateway flush-stage-cache \
		--rest-api-id "$$(aws ssm get-parameter \
			--name /prices/production/api-gateway-id \
			--query Parameter.Value --output text)" \
		--stage-name production
	@echo "production stage cache flushed"

# `--require-approval broadening` prompts on IAM/security-group widening ONLY. It
# does not prompt on resource deletion or replacement, so a deploy can destroy a
# live resource without asking — and an API key destroyed this way takes its value
# with it, cutting off every holder the moment the deploy succeeds. Task 0157
# rotated `prices-production-partner-key` exactly this way, deliberately.
#
# So `make diff-production` before any production deploy, and read it for removals
# rather than skimming it for additions. This applies to every deploy target below
# too; it lives here because this is the one that deploys all stacks at once.
deploy-production: build
	npx cdk --app "$(PRODUCTION_APP)" deploy --all --require-approval broadening
	$(MAKE) flush-production-cache

destroy-production: build
	npx cdk --app "$(PRODUCTION_APP)" destroy --all --force

deploy-production-secrets: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Secrets --require-approval broadening

destroy-production-secrets: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Secrets --force

# Ships a new api-handler, so it ships a new OpenAPI document — flush too.
deploy-production-compute: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Compute --require-approval broadening
	$(MAKE) flush-production-cache

destroy-production-compute: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Compute --force

deploy-production-apigateway: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-ApiGateway --require-approval broadening

# ORDERING (task 0184): PortalHosting imports this stack's RestApi id to build
# its execute-api origin, and CloudFormation refuses to delete a stack whose
# exports are in use. So this target now fails with `Export ... cannot be
# deleted as it is in use` until `destroy-production-portal` has run first.
# The same import blocks any change that REPLACES the RestApi, for the same
# reason and with the same remedy.
destroy-production-apigateway: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-ApiGateway --force

# Uploads the portal bundle and invalidates the CloudFront cache as part of the
# deploy (BucketDeployment), so there is no separate `aws s3 sync` step to
# forget. A stale bundle behind a CDN is indistinguishable from a broken deploy.
#
# First deploy of this stack creates a distribution; that took ~4.5 minutes end
# to end, most of it CloudFront propagating. Subsequent deploys are quick.
deploy-production-portal: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-PortalHosting --require-approval broadening

destroy-production-portal: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-PortalHosting --force

deploy-production-eventbridge: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-EventBridge --require-approval broadening

destroy-production-eventbridge: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-EventBridge --force

deploy-production-observability: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Observability --require-approval broadening

destroy-production-observability: build
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Observability --force
