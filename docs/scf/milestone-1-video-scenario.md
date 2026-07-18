# Milestone 1 — Deliverable Verification Video Script

Purpose: record the SCF deliverable-verification video for Milestone 1 of the
Stellar Prices API: **Infrastructure & Real-time Ingestion**.

**Target length: 5–7 minutes.** Scene timings below add up to ~6:00, leaving
headroom. If you run long, Scene 7 is the one to trim.

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
  same day. Tranche 1 only requires `/v1/backfill/status`; claiming more than
  you demo is the one thing that can turn a good submission bad.

---

## Scene 1 — Intro and scope (~0:30)

SHOW: `architecture.png`.

SAY:

> Hi, I'm from Rumble Fish. This video verifies Milestone 1 of the Stellar
> Prices API: Infrastructure and Real-time Ingestion.
>
> The short version of what this milestone delivers: we observe every closed
> ledger on Stellar mainnet, decode the XDR, and extract the trades that
> actually happened on chain — classic SDEX order-book trades, and Soroban AMM
> swaps from Soroswap, Aquarius, and Phoenix. Those become per-source
> one-minute OHLCV candles in our own database.
>
> The important part is the provenance: our prices are derived from observed
> on-chain trades. There is no third-party price API anywhere on this path.

---

## Scene 2 — Architecture and the shared platform (~0:50)

SHOW: `architecture.png`, tracing the flow left to right as you talk.

SAY:

> Here is the shape. On the left, in green, is infrastructure the Soroban Block
> Explorer already funds and operates: Galexie streaming mainnet ledgers into
> an S3 bucket. Rather than run a second Captive Core, we joined that pipeline
> as a second tenant — the bucket fans out to an SNS topic, and we own our own
> queue on it. If our processor fails, it cannot back up theirs.
>
> In amber is what this project owns: the queue, the Rust ledger processor, the
> scheduled workers, and the API. One detail that matters for correctness — the
> queue message is only a doorbell. The processor reads the ledger XDR from S3
> by sequence number and never parses the message body, so a duplicate or
> out-of-order notification can't corrupt a candle.
>
> On the right, in red, is our data store: a dedicated `prices` database inside
> the Block Explorer's Hetzner ClickHouse cluster, reached over mutual TLS.
>
> And that is the one thing I want to be upfront about, because it differs from
> the approved plan.

---

## Scene 3 — The scope refinement, stated plainly (~0:50)

SHOW: the design doc's Revision History table (`docs/prices-api-general-overview.md`),
then ADR 0007 in the repo.

SAY:

> The approved Milestone 1 plan specified PostgreSQL on AWS RDS as our
> datastore. We ship ClickHouse on Hetzner instead. The deliverable — live
> mainnet price ingestion into our own store — is unchanged, but I want to
> explain why we changed the engine, and show you that we didn't do it quietly.
>
> The driver was fit, not cost. Every read this API serves is a time-range scan
> over a few columns, which is what a columnar engine is built for.
> ClickHouse's ReplacingMergeTree makes ledger replay idempotent by
> construction, so a retried invocation can't double-count a trade — in
> Postgres that's an application-level upsert on every write. And its
> materialised views do our rollups, which let us delete two Lambdas from the
> design — I'll show you those views running in a moment.
>
> Let me be precise rather than dramatic about cost, and about one thing we got
> wrong. The RDS line item was only about twelve dollars a month, because our
> data volume is small — that alone wouldn't justify changing engines. The real
> saving is that dropping RDS also dropped the VPC and the NAT Gateway that
> Lambda-to-RDS connectivity needed.
>
> On compression: it's a real advantage at large data scale, and it's part of
> why the Soroban Block Explorer runs its own platform on ClickHouse at Hetzner
> in the first place. For our own volumes it matters too, if less dramatically
> than we first estimated — we projected under a gigabyte a year, and the
> measured schema came in at a few gigabytes, around 2.6× compression. Smaller
> than the headline number, but still a genuine saving on a shared cluster, and
> it's documented in the evidence package.
>
> This is ADR 0007, accepted in May, after a cross-team agreement with the
> Block Explorer team. Every refinement in this milestone has an ADR like this
> one, and they're all catalogued in the design document's revision history.
> The written evidence package walks through each one.

---

## Scene 4 — Infrastructure as code, and the negative (~0:45)

SHOW: Terminal A. Run:

```bash
cd infra && make synth-production
grep -RE '"AWS::RDS::|"AWS::EC2::VPC"|"AWS::EC2::NatGateway"' cdk.out/*.template.json
```

SAY:

> Acceptance criterion one: `cdk deploy` from a clean account produces the full
> stack with no manual steps, and — this is the falsifiable part — the synth
> output contains no RDS, no VPC, and no NAT Gateway.
>
> That's the synth of all five production stacks. And here's the grep for those
> three resource types across every template: nothing. You can verify that
> negative yourself from the public repo.
>
> One honest caveat, which is also in the written evidence: two prerequisites
> are deliberately manual, one-time operator actions — issuing our mTLS client
> certificate from the Block Explorer's CA, and provisioning our database and
> user on the shared cluster. We can't automate those from our CDK app without
> handing it someone else's CA private key, and we're not going to do that.

---

## Scene 5 — The schema and the live data (~1:30)

SHOW: Terminal B. Run queries (1), (2), (3), (4), (5) from `ch-demo-queries.sql`.

SAY (over query 1 and 2):

> Criterion two: the schema on ClickHouse matches the design. Here are the
> tables in the `prices` database — the candle tables at every granularity, the
> asset registry, oracle prices, backfill progress — plus the read views.
>
> You'll see the six rollup materialised views right here in the list —
> `mv_ohlcv_1m_to_15m` up through `mv_ohlcv_1w_to_1M`. These are what replaced
> two Lambdas from the original design; the rollups run inside the database.
> I want to be upfront about one thing in their history: they originally ran in
> replace mode, and a bounded refresh in that mode overwrote coarse history the
> backfill had pre-rolled — a real incident, not a hypothetical. We caught it,
> and we recreated them in append mode, which inserts each refresh window
> instead of replacing the whole table. So they roll live candles forward now
> without touching history — you'll see the coarse tips tracking the live
> frontier in a moment. It's all in section 6 of the evidence document.
>
> And here's the one-minute candle table itself. Note the engine:
> ReplacingMergeTree, versioned on a number we derive from the ledger sequence,
> ordered by asset, quote asset, source, and timestamp. That's the idempotency
> guarantee I mentioned — replaying a ledger re-inserts rows that collapse
> away.

SAY (over queries 3, 4, 5):

> Criterion three is the real test: after 24 hours of live operation, do we
> have continuous one-minute candles for at least 20 major assets, with no gaps
> bigger than two candles?
>
> Distinct assets with candles in the last 24 hours — that's over the bar.
>
> Here's the per-asset breakdown for the majors the criterion names — XLM,
> USDC, EURC, AQUA, BTC, ETH — with the largest gap in each series. Two of these
> are worth pointing at: XLM and AQUA on the order book are minute-continuous,
> a largest gap of one minute across the whole day — that's the indexer catching
> every trading minute. The thinner majors show larger gaps, and here's the
> honest part: a one-minute candle only exists if a trade happened in that
> minute, so on a thin pair a gap means a quiet market, not a broken indexer.
> That's exactly why the criterion names liquid assets — and it's why our actual
> liveness alarm doesn't measure gaps at all. One detail you'll notice in the
> query: we pin each ticker to its canonical asset first, because on Stellar an
> asset code like "BTC" isn't unique — anyone can issue one — and we don't want
> an illiquid look-alike standing in for the real market. I'll show you what the
> liveness alarm measures in a moment.
>
> And here are the candles broken out by source. That's both halves of the
> pipeline working: the classic SDEX order book, and the Soroban AMM extractors
> decoding swap events from the venues.

---

## Scene 6 — The Tranche 1 API endpoint (~0:50)

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
> First, without a key: 403. Access control is enforced at the gateway, with a
> usage plan and a per-key quota behind it.
>
> With the key: the dual-stream progress payload — the SDEX archive stream and
> the Soroban AMM stream, each with its status, its current and target ledger,
> a progress percentage, and the timestamp of its last push. And
> `earliest_data_available` — that's criterion six, roughly six months of
> history. One honest clarification: that field is the earliest ledger available
> _to backfill_ — the floor of the public archive, back in 2015 — not the
> earliest candle we've ingested. For the depth we actually hold, I'll show you
> the daily table directly. I'm using the daily table deliberately: the
> one-minute table is a transient feeder on a seven-day retention, so it only
> ever holds the last few days. The coarse tables — hourly through monthly — are
> kept forever, and they're the store of record. On depth we're comfortably past
> the bar, about five times over: every source — SDEX and the AMM venues — goes
> back to roughly Soroban activation in February 2024, around 880 days. The SDEX
> pre-Soroban tail is still backfilling toward that 2015 archive floor.
>
> One difference from the approved wording worth flagging: the original
> criterion described a tip-backward backfill with the ledger number counting
> down. Ours runs full-chain with forward discovery, so it counts up. The
> property being tested — that progress moves monotonically and is visible
> through the API — holds either way. Only the sign changed.

---

## Scene 7 — Alarms (~0:50)

SHOW: CloudWatch → Alarms filtered to `prices-production`, all in OK. Then the
Slack channel with the fire-test notification.

**Do not show the dashboard.**

SAY:

> Criterion five: the freshness alarm fires when a backfill push cycle is
> skipped. Here's the production alarm set — fourteen alarms, all currently OK.
> Seven of them, including this milestone's freshness alarm, route through SNS
> and AWS Chatbot into a Slack channel; the rest are per-worker error catchers.
>
> The criterion's alarm is this one: backfill push freshness. We didn't just
> deploy it — we fire-tested it against real metrics, drove it into ALARM, and
> watched it recover. Here's the notification it produced in Slack.
>
> Two of these alarms are worth thirty seconds, because they're the kind of
> thing you only learn in production. This one — "no invocations" — exists
> because every other alarm here keys on messages being present. Lag, errors,
> DLQ depth: if the upstream stops publishing entirely, all of them read
> healthy while ingestion is quietly dead. Alarming on silence is the only
> thing that catches that.
>
> And the enrichment alarm is progress-based rather than threshold-based,
> because an absolute-backlog alarm latched permanently on a floor of candles
> that can never be enriched. Alarming on lack of progress instead of presence
> of backlog is what made it actionable.

---

## Scene 8 — What we're not claiming, and wrap-up (~0:45)

SHOW: the evidence PDF's section 6 table ("What is deliberately not claimed"),
then the repository.

SAY:

> Before I close, here's what this milestone is _not_ claiming, because I'd
> rather you hear it from me than find it.
>
> The CloudWatch dashboard is still a scaffold — the alarms are real and
> tested, the dashboard isn't, and it's Tranche 2 work. The full public API
> surface is deployed but Tranche 2 scope; Milestone 1 verifies the one status
> endpoint I showed you. The historical backfill is a long-running operator
> job: Milestone 1 asks for about six months of depth, we're past that, and
> full-chain coverage back to genesis continues past this milestone. The rollup
> materialised views are live in append mode, as I showed you — they roll the
> coarse tables forward automatically; only cadence tuning against production
> merge load is tracked as follow-up.
>
> All of that is written down in section 6 of the evidence document, alongside
> the full acceptance-criteria walkthrough, every query I just ran, and the
> rationale for each refinement with its ADR.
>
> So: infrastructure as code with no RDS, VPC, or NAT; the schema live on
> ClickHouse; 24 hours of continuous candles across more than 20 assets from
> real on-chain trades; the status endpoint live and access-controlled; six
> months of history; and a fire-tested alarm set. That's Milestone 1. Thanks
> for reviewing.

---

## After recording

- [ ] Watch it back at 1.5× **specifically checking for a leaked API key or
      certificate path in any frame.**
- [ ] Confirm total length is 5–7 minutes.
- [ ] Confirm the dashboard never appears.
- [ ] Upload with sharing set to public / "anyone with the link".
- [ ] Paste the URL into Field 2 of
      [`milestone-1-form-answers.md`](./milestone-1-form-answers.md).
- [ ] Paste the same query outputs you showed on camera into
      `milestone-1-evidence.md`, so the video and the PDF agree.
