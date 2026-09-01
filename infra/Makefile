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

# EVERY target that synthesizes the production app needs the portal bundle on
# disk, not just the portal's own (task 0185). PortalHostingStack packages
# `web/portal/dist`, and cdk synthesizes the whole app whichever stack you name
# — so an unbuilt frontend fails `synth-production`, `diff-production` and every
# per-stack deploy alike. The cicd app does not include it and keeps using
# `build`.
build-production: build build-portal

synth-production: build-production
	npx cdk --app "$(PRODUCTION_APP)" synth

diff-production: build-production
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
deploy-production: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy --all --require-approval broadening
	$(MAKE) flush-production-cache

destroy-production: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy --all --force

deploy-production-secrets: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Secrets --require-approval broadening

destroy-production-secrets: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Secrets --force

# Ships a new api-handler, so it ships a new OpenAPI document — flush too.
deploy-production-compute: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Compute --require-approval broadening
	$(MAKE) flush-production-cache

destroy-production-compute: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Compute --force

deploy-production-apigateway: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-ApiGateway --require-approval broadening

# ORDERING (task 0184): PortalHosting imports this stack's RestApi id to build
# its execute-api origin, and CloudFormation refuses to delete a stack whose
# exports are in use. So this target now fails with `Export ... cannot be
# deleted as it is in use` until `destroy-production-portal` has run first.
# The same import blocks any change that REPLACES the RestApi, for the same
# reason and with the same remedy.
destroy-production-apigateway: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-ApiGateway --force

# Uploads the portal bundle and invalidates the CloudFront cache as part of the
# deploy (BucketDeployment), so there is no separate `aws s3 sync` step to
# forget. A stale bundle behind a CDN is indistinguishable from a broken deploy.
#
# `build-portal` first, and it is not a convenience (task 0185): this stack
# packages `web/portal/dist` off the disk with no freshness check, so without it
# an unbuilt tree fails synth and — worse — a stale one deploys quietly and
# reports success. That is task 0141's footgun arriving for the frontend.
#
# First deploy of this stack creates a distribution; that took ~4.5 minutes end
# to end, most of it CloudFront propagating. Subsequent deploys are quick.
build-portal:
	cd .. && npx nx build portal

deploy-production-portal: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-PortalHosting --require-approval broadening

destroy-production-portal: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-PortalHosting --force

# The portal on the shared host (task 0194). `https://sorobanscan.rumblefish.dev/api/`
# is served by the block explorer's distribution (EA2TLS5SS5M87) from ITS
# bucket, which nothing in this repo deploys — this sync is the only way a
# build reaches it, and it has been done by hand twice. Built with the API's
# own hostname baked in (`VITE_PORTAL_API_ORIGIN`, see
# `web/portal/src/api-origin.ts`): on that host `/api/*` is a static SPA, so
# the bundle must call the backend on `apiDomain.domainName` rather than
# relatively. `--skip-nx-cache` because the variable is not an Nx input — a
# cached build would hand back whichever variant ran last, and the relative
# one is indistinguishable from this one until a visitor opens the page.
#
# ⚠️ This leaves the ABSOLUTE build in `web/portal/dist`. `deploy-production-portal`
# rebuilds (relative) through `build-portal` first, so our own distribution
# never ships it — but do not `cdk deploy` PortalHosting by hand after this.
#
# The two names are the explorer repo's (`ApiSpaBucket` and the distribution
# of `Explorer-production-Delivery`); override on the command line if that
# stack ever renames them.
EXPLORER_PORTAL_BUCKET ?= production-soroban-explorer-api-spa
EXPLORER_DISTRIBUTION_ID ?= EA2TLS5SS5M87
# `// empty`, so a renamed key yields `https://` rather than `https://null`.
# Either is refused by `check-portal-api-origin` below before a build can bake
# it in; a missing `jq` produces the same empty value and the same refusal.
PORTAL_API_ORIGIN ?= https://$(shell jq -r '.apiDomain.domainName // empty' envs/production.json)

# `$(shell …)` swallows a failed `jq` — binary missing, key renamed — into the
# value, and the value is baked into the bundle: every backend call then goes
# to a host nobody watches, invisible until a visitor opens the page (the same
# failure shape the cache note above describes). Refuse anything that is not
# `https://` followed by a hostname with a dot in it.
check-portal-api-origin:
	@case '$(PORTAL_API_ORIGIN)' in \
	  https://*.*) ;; \
	  *) echo "error: PORTAL_API_ORIGIN is '$(PORTAL_API_ORIGIN)', expected https://<hostname>." >&2; \
	     echo "  → is jq installed, and does envs/production.json still carry apiDomain.domainName?" >&2; \
	     exit 1 ;; \
	esac

build-portal-explorer: check-portal-api-origin
	cd .. && VITE_PORTAL_API_ORIGIN=$(PORTAL_API_ORIGIN) npx nx build portal --skip-nx-cache

# Mirrors the two `BucketDeployment`s in `portal-hosting-stack.ts`, headers
# included. A bare `sync` uploads with no `Cache-Control` at all, so after a
# re-sync a returning visitor keeps booting the previous `index.html` from
# heuristic freshness, and the hashed chunks lose the year-long immutability
# their content-addressed names earn them. Hashed assets go first, so a new
# `index.html` never names a chunk that is not there yet — the order the stack
# enforces with `addDependency`.
#
# No `--delete`: a visitor holding the previous `index.html` still resolves
# its chunk. The invalidation covers the whole prefix because `index.html` is
# the object that changes and the one a browser asks for by name.
sync-portal-explorer: build-portal-explorer
	aws s3 sync ../web/portal/dist s3://$(EXPLORER_PORTAL_BUCKET)/api/ \
	  --exclude '*' --include 'assets/*' \
	  --cache-control 'public, max-age=31536000, immutable'
	aws s3 sync ../web/portal/dist s3://$(EXPLORER_PORTAL_BUCKET)/api/ \
	  --exclude 'assets/*' \
	  --cache-control 'public, max-age=0, must-revalidate'
	aws cloudfront create-invalidation --distribution-id $(EXPLORER_DISTRIBUTION_ID) --paths '/api/*'

deploy-production-eventbridge: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-EventBridge --require-approval broadening

destroy-production-eventbridge: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-EventBridge --force

deploy-production-observability: build-production
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Observability --require-approval broadening

destroy-production-observability: build-production
	npx cdk --app "$(PRODUCTION_APP)" destroy Prices-production-Observability --force
