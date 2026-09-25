# stellar-prices-api

USD prices, OHLCV candles and volumes for Stellar assets, built from the
Stellar DEX (SDEX order book) and the Soroban AMMs (Soroswap, Phoenix, Aquarius,
SushiSwap V3), with a public REST API and a self-service portal for API
keys.

- **API:** `https://prices-api.sorobanscan.rumblefish.dev` (key-gated `/v1/*`;
  the OpenAPI document is public at `/api-docs-json`)
- **Portal (key sign-up):** `https://sorobanscan.rumblefish.dev/api/`
- **Endpoint reference:** [`docs/scf/api-endpoints.md`](docs/scf/api-endpoints.md)
- **Architecture overview:** [`docs/prices-api-general-overview.md`](docs/prices-api-general-overview.md)

## How it runs

Rust Lambdas on AWS (no VPC) read Stellar ledgers from S3, extract trades, and
write candles to ClickHouse over HTTPS with mutual TLS. Scheduled workers
enrich candles with USD values, roll them up to coarser intervals and watch
freshness; one API Lambda behind API Gateway serves the read surface.

This project is a **tenant of the Soroban Block Explorer platform**
([`rumblefishdev/soroban-block-explorer`](https://github.com/rumblefishdev/soroban-block-explorer)).
That platform provides the ledger bucket, the ledger-events SNS topic, and the
ClickHouse server on a Hetzner dedicated machine behind a Caddy mTLS proxy.
This repo owns the `prices` database on that server, its own AWS stacks, and
nothing else.

## Repository layout

| Path             | What                                                                                                                                                            |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `packages/`      | Rust crates: API (`prices-api`), live ingestion (`ledger-processor`, venue extractors), scheduled workers, backfills, and `prices-clickhouse` (schema + client) |
| `infra/`         | AWS CDK app (TypeScript), one stack per file, config in `infra/envs/*.json`                                                                                     |
| `web/portal/`    | The self-service key portal (built here, hosted by the explorer's CDN)                                                                                          |
| `docs/`          | Architecture, runbooks (`docs/runbooks/`), SCF milestone evidence (`docs/scf/`)                                                                                 |
| `tools/scripts/` | Build, verification and operator scripts                                                                                                                        |
| `lore/`          | Task and decision records                                                                                                                                       |

## Local development

```bash
nvm use                                  # Node 22.22.0 (.nvmrc)
npm ci
docker compose up -d clickhouse          # ClickHouse 26.3, same version as production
cargo test --workspace                   # unit tests
npx nx run-many -t test                  # the whole workspace through Nx
```

The ClickHouse-backed integration tests and the schema applier are described in
[`packages/prices-clickhouse/README.md`](packages/prices-clickhouse/README.md).

## Deploy

**[`infra/README.md`](infra/README.md) is the deployment runbook**, including
a deployment into a fresh AWS account. In short:

1. **Platform first.** Deploy the Soroban Block Explorer platform by following
   its own fresh-account runbook. That covers the AWS stacks and the Hetzner
   ClickHouse machine (ordered by hand, then configured with Ansible). The
   platform publishes the five `/platform/<env>/*` SSM parameters this app
   reads.
2. **Tenant on the ClickHouse server.** Issue the two service certificates
   (CNs `prices-ingestion-<env>` and `prices-api-<env>`) and map them to
   `prices_writer` and `prices_reader`. Then apply the `prices` schema.
3. **Secrets and seeds.** Upload the two mTLS bundles and the Discord OAuth
   bundle to Secrets Manager, and seed four SSM parameters.
4. **`cdk deploy`.** Run `npm run infra:bootstrap`, then
   `npm run infra:deploy:production`.

Some steps are manual by design, and `infra/README.md` lists each one. The
Hetzner server is ordered by hand. Client certificates are issued from the
platform's CA. The Discord application is registered by hand. DNS is a hosted
zone you own.
