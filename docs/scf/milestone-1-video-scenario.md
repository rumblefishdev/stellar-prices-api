# Milestone 1 — Deliverable Verification Video Script

Purpose: record the SCF deliverable-verification video for Milestone 1 of the
Stellar Prices API: **Infrastructure & Real-time Ingestion**.

**Target length: 3–4 minutes.** Scene timings below add up to ~3:30. If you run
long, Scene 5 is the one to trim.

## Before recording

### Windows to have open (in this order — the scene order)

1. **Slides/editor** — `architecture.png` open full-screen.
2. **Terminal A** — in `infra/`, ready to run `make synth-production` and the
   grep. Run it once beforehand so the build is warm and the take is fast.
3. **Terminal B** — `clickhouse-client` connected to production over mTLS,
   with [`ch-demo-queries.sql`](./ch-demo-queries.sql) open in a scratch buffer
   to paste from. **Run every query once before recording** — you want to know
   the outputs are good before the camera is on, and you will paste the same
   outputs into the evidence PDF.
4. **Terminal C** — `curl` + `jq` ready, `$API` and `$KEY` already exported
   (see the secrets warning below).
5. **Browser** — AWS Console → CloudWatch → Alarms, filtered to
   `prices-production`, in eu-central-1.
6. **Browser tab** — the Slack channel showing the alarm notification from the
   task 0056 fire-test.

### ⚠️ Secrets — read this before you hit record

- **Export `$KEY` in a shell you are not recording**, or set it from a file:
  `export KEY=$(cat ~/.prices-api-key)`. Never let the key appear in a frame,
  in shell history that scrolls past, or in a `curl` line you type on camera.
- **Do not open the mTLS certificate or key files on camera.** Connect
  Terminal B before recording.
- If a key does land in a frame, it is easier to **rotate the key** than to
  re-edit the video. Decide which you are doing before uploading.

### Values to have ready

| Item       | Value                                                                  |
| ---------- | ---------------------------------------------------------------------- |
| API base   | `https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production` |
| API key    | `<API_KEY>` — export out of frame                                      |
| ClickHouse | `ch.sorobanscan.rumblefish.dev`, database `prices`                     |
| Repository | `https://github.com/rumblefishdev/stellar-prices-api`                  |
| Region     | `eu-central-1`                                                         |

### What NOT to show

- **The CloudWatch dashboard** (`prices-production-overview`) — it is a
  scaffold with no data widgets. The alarms are the real evidence. Showing an
  empty dashboard actively hurts the submission.
- Any of the six non-Tranche-1 API routes unless you have re-verified them the
  same day. Tranche 1 only requires `/v1/backfill/status`.

---

## Scene 1 — Intro and scope (~0:20)

SHOW: `architecture.png`.

SAY:

> Hi, I'm from Rumble Fish. This video verifies Milestone 1 of the Stellar
> Prices API: Infrastructure and Real-time Ingestion.
>
> We observe every closed ledger on Stellar mainnet, decode the XDR, and extract
> the trades that happened on chain — SDEX order-book trades, and Soroban AMM
> swaps from Soroswap, Aquarius, and Phoenix — into per-source one-minute OHLCV
> candles. Our prices are derived from observed on-chain trades; there is no
> third-party price API on this path.

---

## Scene 2 — Architecture and the shared platform (~0:35)

SHOW: `architecture.png`, tracing the flow left to right as you talk.

SAY:

> Here is the shape. In green is infrastructure the Soroban Block Explorer
> already funds and operates: Galexie streaming mainnet ledgers into an S3
> bucket. We joined that pipeline as a second tenant — the bucket fans out to an
> SNS topic, and we own our own queue on it.
>
> In amber is what this project owns: the queue, the Rust ledger processor, the
> scheduled workers, and the API. The queue message is only a doorbell — the
> processor reads the ledger XDR from S3 by sequence number, so a duplicate or
> out-of-order notification can't corrupt a candle.
>
> In red is our data store: a dedicated `prices` database inside the Block
> Explorer's Hetzner ClickHouse cluster, reached over mutual TLS.

---

## Scene 3 — The scope refinement (~0:30)

SHOW: the design doc's Revision History table (`docs/prices-api-general-overview.md`),
then ADR 0007 in the repo.

SAY:

> The approved plan specified PostgreSQL on AWS RDS. We ship ClickHouse on
> Hetzner instead — the deliverable, live mainnet price ingestion into our own
> store, is unchanged.
>
> ClickHouse fits: every read this API serves is a time-range scan over a few
> columns, its ReplacingMergeTree makes ledger replay idempotent by
> construction, and its materialised views do our rollups — which let us delete
> two Lambdas from the design. Dropping RDS also dropped the VPC and the NAT
> Gateway that Lambda-to-RDS connectivity needed.
>
> This is ADR 0007, accepted in May after a cross-team agreement with the Block
> Explorer team. Every refinement in this milestone is catalogued in the design
> document's revision history.

---

## Scene 4 — Infrastructure as code, and the negative (~0:30)

SHOW: Terminal A. Run:

```bash
cd infra && make synth-production
grep -RE '"AWS::RDS::|"AWS::EC2::VPC"|"AWS::EC2::NatGateway"' cdk.out/*.template.json
```

SAY:

> Acceptance criterion one: `cdk deploy` from a clean account produces the full
> stack with no manual steps, and the synth output contains no RDS, no VPC, and
> no NAT Gateway.
>
> That's the synth of all five production stacks, and here's the grep for those
> three resource types across every template: nothing. You can verify that
> negative yourself from the public repo.

---

## Scene 5 — The schema and the live data (~1:00)

SHOW: Terminal B. Run queries (1), (2), (3), (4), (5) from `ch-demo-queries.sql`.

SAY (over query 1 and 2):

> Criterion two: the schema on ClickHouse matches the design. Here are the
> tables in the `prices` database — the candle tables at every granularity, the
> asset registry, oracle prices, backfill progress, plus the read views.
>
> You'll see the six rollup materialised views here — `mv_ohlcv_1m_to_15m` up
> through `mv_ohlcv_1w_to_1M`. These replaced two Lambdas from the original
> design; the rollups run inside the database, in append mode.
>
> And here's the one-minute candle table. Note the engine: ReplacingMergeTree,
> versioned on a number derived from the ledger sequence, ordered by asset,
> quote asset, source, and timestamp. That's the idempotency guarantee —
> replaying a ledger re-inserts rows that collapse away.

SAY (over queries 3, 4, 5):

> Criterion three: after 24 hours of live operation, do we have continuous
> one-minute candles for at least 20 major assets, with no gaps bigger than two
> candles?
>
> Distinct assets with candles in the last 24 hours — over the bar.
>
> Here's the per-asset breakdown for the majors the criterion names — XLM, USDC,
> EURC, AQUA, BTC, ETH — with the largest gap in each series. XLM and AQUA on the
> order book are minute-continuous: a largest gap of one minute across the whole
> day.
>
> And here are the candles broken out by source — the classic SDEX order book,
> and the Soroban AMM extractors decoding swap events. Both halves of the
> pipeline working.

---

## Scene 6 — The Tranche 1 API endpoint (~0:35)

SHOW: Terminal C.

```bash
curl -sS -o /dev/null -w '%{http_code}\n' "$API/v1/backfill/status"                    # 403
curl -sS -H "x-api-key: $KEY" "$API/v1/backfill/status" | jq .                         # 200 + payload
curl -sS "$API/health"                                                                 # keyless probe
```

SAY:

> Criterion four: the backfill status endpoint is live. This is the one API
> endpoint Milestone 1 requires — the full public API surface is Tranche 2.
>
> Without a key: 403 — access control is enforced at the gateway. With the key:
> the dual-stream progress payload — the SDEX archive stream and the Soroban AMM
> stream, each with its status, current and target ledger, progress percentage,
> and the timestamp of its last push.
>
> For the depth of history — criterion six — here's the daily table directly:
> every source goes back to roughly Soroban activation, around 880 days,
> comfortably past the six-month bar. I use the daily table because the
> one-minute table is a seven-day transient feeder; the coarse tables, hourly
> through monthly, are kept forever.

---

## Scene 7 — Alarms and wrap-up (~0:35)

SHOW: CloudWatch → Alarms filtered to `prices-production`, all in OK. Then the
Slack channel with the fire-test notification.

**Do not show the dashboard.**

SAY:

> Criterion five: the freshness alarm fires when a backfill push cycle is
> skipped. Here's the production alarm set — fourteen alarms, all currently OK.
> Seven of them, including this milestone's freshness alarm, route through SNS
> and AWS Chatbot into a Slack channel.
>
> The criterion's alarm is backfill push freshness. We didn't just deploy it —
> we fire-tested it against real metrics, drove it into ALARM, and watched it
> recover. Here's the notification it produced in Slack.
>
> That's Milestone 1: infrastructure as code with no RDS, VPC, or NAT; the
> schema live on ClickHouse; 24 hours of continuous candles across more than 20
> assets from real on-chain trades; the status endpoint live and
> access-controlled; six months of history; and a fire-tested alarm set. Thanks
> for reviewing.

---

## After recording

- [ ] Watch it back at 1.5× **specifically checking for a leaked API key or
      certificate path in any frame.**
- [ ] Confirm total length is 3–4 minutes.
- [ ] Confirm the dashboard never appears.
- [ ] Upload with sharing set to public / "anyone with the link".
- [ ] Paste the URL into Field 2 of
      [`milestone-1-form-answers.md`](./milestone-1-form-answers.md).
- [ ] Paste the same query outputs you showed on camera into
      `milestone-1-evidence.md`, so the video and the PDF agree.
