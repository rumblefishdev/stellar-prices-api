# SCF Milestone 1 — Form Answers (stellar-prices-api)

> Copy the text inside each blockquote into the matching field of the Stellar
> Community Fund **Deliverable Verification** form. Anything in
> `<ANGLE_BRACKETS>` is a placeholder to replace before submitting.
>
> Full evidence and rationale live in the companion document
> [`milestone-1-evidence.md`](./milestone-1-evidence.md), exported to PDF and
> attached in Google Drive next to the video.

---

## Field 1 — Tranche Deliverables

> **Deliverable 1 — Infrastructure & Real-time Ingestion** (as originally
> approved).
>
> Milestone 1 delivers an end-to-end Stellar mainnet price-ingestion pipeline
> writing into our own database, with no third-party price API on the read
> path, plus the supporting cloud infrastructure, alarms, and the progress API
> that later tranches build on.
>
> What is live and verifiable today:
>
> 1. **Prices Ledger Processor Lambda** (Rust, `prices-production-ledger-processor`)
>    is subscribed to the ledger-bucket SNS fan-out, decodes each ledger's XDR,
>    and writes typed per-source 1-minute OHLCV candles into our `prices`
>    database over HTTPS-mTLS.
> 2. **Prices come from observed on-chain trades, not from an oracle feed.**
>    We extract classic SDEX order-book trades plus Soroban AMM swaps from
>    Soroswap, Aquarius, and Phoenix (ScVals decoded via `stellar-xdr`). The
>    Reflector SEP-40 oracle is ingested for reference and USD conversion only;
>    it never sets a price.
> 3. **24 hours of continuous 1-minute candles** for more than 20 major assets
>    (XLM, USDC, EURC, AQUA, BTC, ETH among them). The deepest order-book
>    markets — XLM and AQUA on SDEX — are minute-continuous (largest gap 1
>    minute); gaps on thinner majors reflect genuine market quiet, since a
>    candle exists only when a trade occurs in that minute.
> 4. **Coarser granularities are derived in the database, not in application
>    code.** A ClickHouse materialised-view chain rolls
>    1m → 15m → 1h → 4h → 1d → 1w → 1M and maintains `current_prices`, which
>    removed two Lambdas from the original design. The six rollup MVs run in
>    APPEND mode (a replace-mode incident that overwrote pre-rolled history was
>    caught and fixed), so they roll live candles forward without clobbering
>    history; the coarse tables track the live frontier automatically and are
>    correct and verified.
> 5. **`GET /v1/backfill/status` is live** behind API Gateway with key-based
>    access control, a usage plan, and stage caching, returning dual-stream
>    (SDEX + Soroban AMM) historical-backfill progress.
> 6. **Infrastructure as code:** five AWS CDK stacks deployed with
>    `make deploy-production-*`. The synth output contains no RDS instance, no
>    VPC, and no NAT Gateway.
> 7. **Monitoring:** seven production CloudWatch alarms routed through SNS and
>    AWS Chatbot to a dedicated Slack channel, covering backfill-push
>    freshness, mTLS certificate expiry, ingestion lag, processor errors, DLQ
>    depth, ingestion silence, and enrichment progress. The alarm set has been
>    fire-tested against real metrics and is healthy.
>
> **In-tranche scope refinements.** The approved plan specified PostgreSQL on
> AWS RDS as the primary datastore; the delivered system writes to a dedicated
> `prices` database inside the Soroban Block Explorer's existing Hetzner
> ClickHouse cluster. The deliverable scope is unchanged. The driver was fit
> rather than cost: every read we serve is a time-range column scan,
> ClickHouse's `ReplacingMergeTree` makes ledger replay idempotent by
> construction, and its materialised views let us delete two Lambdas from the
> design (those MVs run in APPEND mode —
> see the evidence document). The change also removed the VPC and NAT Gateway that Lambda-to-RDS
> connectivity required, and let us join a cluster the Block Explorer already
> operates rather than run our own. The historical backfill likewise moved
> from a continuous ECS Fargate task to an operator-run Rust CLI, cutting
> backfill compute cost by ~95%. Each refinement is recorded in an accepted ADR
> (0007, 0005, 0001, 0009, 0008) and catalogued in the design document's
> revision history.
>
> **Full evidence — acceptance-criteria mapping, live ClickHouse query output,
> architecture diagram, ADR references, the complete refinement rationale, and
> an explicit statement of what is _not_ claimed for this milestone:**
> `<DRIVE_LINK_TO_milestone-1-evidence.pdf>`

---

## Field 2 — Deliverable Verification - Video

> `<VIDEO_URL>`

---

## Field 3 — Additional Deliverable Verification

> **Evidence package (Google Drive):** `<DRIVE_FOLDER_LINK>` — contains
> `milestone-1-evidence.pdf` (full acceptance-criteria walkthrough,
> architecture diagram, current ClickHouse query outputs, AWS screenshots,
> refinement rationale) and the demo video.
>
> **Live & anonymous (verify directly in a browser):**
>
> - API health probe:
>   `https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/health`
> - OpenAPI specification:
>   `https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/api-docs-json`
>
> **Key-gated (API key available to reviewers on request):**
>
> - Milestone 1 acceptance-criteria endpoint:
>   `GET https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/v1/backfill/status`
>   with header `x-api-key`.
>
> **Source code (public):**
>
> - Repository: `https://github.com/rumblefishdev/stellar-prices-api`
> - Technical design, including the verbatim Tranche 1 acceptance criteria (§9)
>   and a dated revision history of every scope refinement:
>   `https://github.com/rumblefishdev/stellar-prices-api/blob/develop/docs/prices-api-general-overview.md`
> - ADR 0007 — ClickHouse on Hetzner as the live data sink (the RDS pivot):
>   `https://github.com/rumblefishdev/stellar-prices-api/blob/develop/lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md`
> - Ingestion pipeline shared by the live Lambda and the backfill CLI:
>   `https://github.com/rumblefishdev/stellar-prices-api/tree/develop/packages/prices-ingest-core`
> - AWS CDK application:
>   `https://github.com/rumblefishdev/stellar-prices-api/tree/develop/infra`
>
> **Operational endpoints (private, available on request):** production
> ClickHouse `ch.sorobanscan.rumblefish.dev`, database `prices` (mTLS — client
> certificate issued on request), and the `prices-production-*` CloudWatch
> alarms in eu-central-1 (read-only IAM access can be provisioned for a
> reviewer AWS account).
>
> Tranche 1 deliverables are infrastructure and data. The public API surface
> and its credentials are Tranche 2 scope; the one Tranche 1 API endpoint
> (`/v1/backfill/status`) is demonstrated in the video and its key is available
> on request.

---

## Field 4 — Support Needed

> —

---

## Pre-submission checklist

- [ ] `master` merged up to date and all Field 3 links resolve anonymously in an
      incognito window.
- [ ] Operator has run [`ch-demo-queries.sql`](./ch-demo-queries.sql) against
      production and pasted every output into `milestone-1-evidence.md`
      (replacing the `<TODO: paste output>` markers).
- [ ] Screenshots captured for every `<TODO: screenshot>` marker (CDK synth
      grep, CloudWatch alarms in OK state, Slack alarm notification).
- [ ] `architecture.png` rendered from `architecture.mmd`:
      `npx -y @mermaid-js/mermaid-cli -i architecture.mmd -o architecture.png -s 3`
- [ ] AC 3 verified live: ≥ 20 assets with 24 h of candles; deepest markets
      (XLM, AQUA on SDEX) minute-continuous (largest gap 1 min), thinner majors'
      gaps consistent with market quiet.
- [ ] AC 6 verified live: `earliest_data_available` ≈ 6 months back.
- [ ] Re-verify the six non-AC API routes respond before publication, or leave
      them unclaimed — Field 1 deliberately claims only `/v1/backfill/status`.
- [ ] `milestone-1-evidence.md` finalised and exported: `./build-pdf.sh`.
- [ ] PDF uploaded to a Google Drive folder with link-sharing set to
      "anyone with the link can view".
- [ ] Drive folder link copied into Field 1 closer **and** Field 3 opener
      (replace both `<DRIVE_*>` placeholders).
- [ ] Video uploaded with public sharing; URL pasted into Field 2.
- [ ] All `<ANGLE_BRACKET>` placeholders in this file replaced.
- [ ] `curl -sS https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/health`
      returns 200.
- [ ] No API key, certificate, or other secret material visible in the PDF, the
      screenshots, or any video frame.
- [ ] Field 4 — decide between `—` and a specific support request.
- [ ] English-only across all four blocks (no Polish, no internal slang, no
      personal usernames).
