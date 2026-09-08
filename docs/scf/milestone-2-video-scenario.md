# Milestone 2 — Deliverable Verification Video Script

Target length **6 to 7 minutes**. Milestone 2 is the public API, so the whole
video is a walkthrough of the deployed API through the browser and `curl`.
Milestone 1's video was verifiable because it narrated live URLs rather than
local runs; keep that.

---

## Before recording

### Windows to have open, in scene order

1. **Browser tab A** — `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`,
   already loaded. It is anonymous, so it opens without a key on camera.
2. **Browser tab B** — the rendered API reference at
   `https://sorobanscan.rumblefish.dev/api/docs`.
3. **Terminal A** — `curl` and `jq` ready, with `$API` and `$KEY` **already
   exported in a shell you are not recording**.
4. **Terminal B** — a second shell in the repository root, for the conformance
   suite. Run it once beforehand so you know today's result before the camera is
   on.
5. **Browser tab C** — AWS Console, CloudWatch, the `prices-production-overview`
   dashboard in `eu-central-1`, already loaded and zoomed to a readable size.
6. **Editor** — `milestone-2-rfp-deviations.md` open at §2, for scene 6.

### ⚠️ Secrets — read this before you hit record

- **Export** `$KEY` **out of frame**, or set it from a file:
  `export KEY=$(cat ~/.prices-api-key)`. Never let the key appear in a frame, in
  scrollback, or in a `curl` line typed on camera.
- **Check the browser address bar** before every tab switch. A key pasted as a
  query parameter during rehearsal will still be in the URL history dropdown.
- If a key does land in a frame, **rotate the key** rather than re-editing the
  video. Decide which before uploading.

### Values to have ready

| Item       | Value                                                 |
| ---------- | ----------------------------------------------------- |
| API base   | `https://prices-api.sorobanscan.rumblefish.dev`       |
| API key    | `<API_KEY>` — export out of frame                     |
| Dashboard  | `prices-production-overview`, `eu-central-1`          |
| Repository | `https://github.com/rumblefishdev/stellar-prices-api` |

### What NOT to show

- **Do not show a route you have not re-verified the same day.** The traded
  population moves, and an asset that answered yesterday can 404 today when it
  has not traded in 24 hours. That is documented behaviour, but it is a bad thing
  to discover live.
- **Do not claim a cache hit from a header.** There is no `X-Cache` header on
  this API. Scene 6 exists to explain that honestly; do not undercut it earlier.
- Do not open the mTLS certificate or key files.

---

## Scene 1 — Intro and scope (~0:25)

**On screen:** browser tab B, the rendered API reference.

> "This is Milestone 2 of the Stellar Prices API — the public API tranche. In
> Milestone 1 we built the ingestion pipeline that writes on-chain prices into
> our own database. Milestone 2 puts a public read API in front of it: seven
> endpoint groups, deployed on a custom domain, key-gated, cached and validated.
> Everything I show is the live production deployment."

---

## Scene 2 — The contract is published, and it is generated (~0:40)

**On screen:** browser tab A, `/api-docs-json`.

> "The API publishes its own OpenAPI document, and this route needs no key — you
> can open it right now. It is OpenAPI 3.1, generated from the router in code
> rather than maintained by hand, so it cannot drift from what the API actually
> serves."

Scroll to `paths` and let the seven route groups be visible.

> "This matters for the next few minutes, because our conformance suite validates
> every response against this document rather than against a checked-in copy."

Switch to tab B briefly.

> "The same document is rendered here as a reference — this page is Adam's work,
> and it is the surface most integrators will actually live in rather than the
> raw JSON. It lays the generated specification out to be read: the route groups,
> the schemas, and the field descriptions in the reader's own terms. And because
> it renders that document rather than a copy of it, it inherits the same
> guarantee — it cannot describe a route the API does not serve."

---

## Scene 3 — The endpoints, on real data (~1:30)

**On screen:** Terminal A.

```bash
curl -sS -H "x-api-key: $KEY" "$API/v1/assets?limit=3" | jq '.data[0], .has_more'
curl -sS -H "x-api-key: $KEY" "$API/v1/assets/native/price" | jq
```

> "The current price comes with a per-source breakdown — this is the
> volume-weighted price, and these are the venues behind it. SDEX is the classic
> order book; Soroswap, Aquarius and Phoenix are Soroban AMMs. Aquarius appearing
> here by name with a real price and a real 24-hour volume is one of the tranche's
> named deliverables."

> "Every number is a decimal string rather than a JSON float. That is deliberate —
> a float would silently destroy the low-order digits of our long-tail assets, and
> we document it as a departure from the RFP's wording."

```bash
curl -sS -H "x-api-key: $KEY" "$API/v1/assets/native/price?min_volume_usd=200000" | jq '.sources | keys'
```

> "The minimum-volume threshold is a request parameter too. Raising it drops the
> thinner venues and leaves the one that clears the bar."

```bash
curl -sS -H "x-api-key: $KEY" "$API/v1/assets/native/ohlcv?granularity=1d&timeframe=1y" | jq '.data | length, .data[-1]'
curl -sS -H "x-api-key: $KEY" -X POST "$API/v1/prices/batch" \
  -H 'content-type: application/json' -d '{"assets":["native","USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"]}' | jq
curl -sS -H "x-api-key: $KEY" "$API/v1/oracles/native" | jq
```

> "Candles at a chosen granularity, a batch endpoint for several assets in one
> call, and the oracle cross-reference. We ingest two reference oracles, but
> neither one ever sets a price — prices come from observed trades."

---

## Scene 4 — Input validation (~0:30)

**On screen:** Terminal A.

```bash
curl -sS -o /dev/null -w '%{http_code}\n' -H "x-api-key: $KEY" "$API/v1/assets?limit=9999"
curl -sS -H "x-api-key: $KEY" "$API/v1/assets/not-an-asset/price" | jq
```

> "Invalid input is rejected at the edge with a typed error rather than reaching
> the database. About fifty of these negative cases run in continuous integration
> against a handler wired to panic if it touches the database at all — so a clean
> 400 also proves no query was issued."

---

## Scene 5 — Conformance, on production (~1:00)

**On screen:** Terminal B.

```bash
npm run conformance:0120
```

Let it run for a few seconds, then cut to the finished summary you captured
before recording.

> "This is the acceptance evidence for the first criterion. It exercises all seven
> route groups across twenty major assets, covering all three ways an asset can be
> identified, and validates every response — errors included — against the live
> specification. Today it is 1032 checks passing, none failing, none skipped."

> "Two things worth saying about that number. The check count went **up** when we
> fixed this suite, from 886 to 1032, because we added assertions rather than
> relaxing them. And no production code changed that day — every failure that
> disappeared was a defect in the test, not a fix to the API."

> "It also has to give the same answer twice. Enrichment runs in the background, so
> an earlier version of this suite could pass and fail half an hour apart on
> unchanged code. We ran it twice, 29 minutes apart, and diffed it: 1032 identical
> verdicts while 53 underlying details moved."

---

## Scene 6 — Caching, and the header that does not exist (~1:15)

**On screen:** Terminal A.

> "The tranche asks us to confirm the response cache. The criterion asks for an
> `X-Cache: Hit` header, and I want to be direct about this: **this API emits no
> such header on any route.** API Gateway's stage cache has no hit-or-miss header
> feature. It is not a setting we left switched off."

```bash
U=$(date +%s); P="$API/v1/assets/native/price?min_volume_usd=$U"
srv() { curl -s -o /dev/null -H "x-api-key: $KEY" \
  -w '%{time_starttransfer} %{time_appconnect}' "$1" |
  awk '{printf "%.1f ms\n", ($1-$2)*1000}'; }
srv "$P"; sleep 2; srv "$P"; sleep 9; srv "$P"; sleep 2; srv "$P"
```

> "So we demonstrate it the other way. A first request misses and costs about
> 145 milliseconds. Inside the ten-second window it is served from cache at about 50. After the window it misses again and the cost returns. Hits land between 45
> and 53 milliseconds, misses between 78 and 145, and the two ranges do not
> overlap."

**On screen:** editor, deviations document §2.

> "We could not add the header honestly. Our own handler runs only on a miss, and
> the gateway replays a cached response byte for byte — so a header we wrote would
> report _miss_ on every genuine hit, which is worse than no header at all. And
> CloudFront, the one architecture that emits a truthful one, writes a different
> string, so the literal wording would still be unmet after a week of edge work."

> "So we grade the criterion on latency and the deployed configuration, and we say
> plainly that this is the weaker claim. **A header would be the cache asserting
> itself. Latency is behaviour consistent with a cache.** The reasoning is in an
> accepted architecture decision record, with the alternatives closed and the
> conditions that would reopen them."

---

## Scene 7 — Depth, the load test, and the dashboard (~1:15)

**On screen:** Terminal A.

```bash
curl -sS -H "x-api-key: $KEY" "$API/v1/backfill/status" | jq '.sdex.earliest_data_available'
```

> "The tranche target for historical depth is January 2022. We reach November
> 2015, and we reconcile that four ways rather than trusting the endpoint's own
> stored value — against the candles, against the stored row, and against the
> oldest partition on every tier."

> "On load: 100 requests a second for five minutes gave a p95 of 47 milliseconds
> against a 200 millisecond bar, with zero errors in thirty thousand requests. The
> evidence document explains why that pass is narrower than it looks — it is
> largely the cache answering, and a cache miss on its own costs closer to 200
> milliseconds."

**On screen:** browser tab C, the dashboard.

> "And the CloudWatch dashboard, which Milestone 1 explicitly did not claim
> because it was an empty scaffold. It now carries an acceptance strip, API,
> ingestion, ClickHouse, worker and enrichment panels, and an alarm strip covering
> 50 alarms that derives itself from the infrastructure code."

---

## Scene 8 — What we do not claim, and wrap-up (~0:35)

**On screen:** the evidence document, §8.

> "Finally, the part that made Milestone 1 credible and that we have repeated. The
> evidence document lists what we deliberately do **not** claim, with a destination
> for each: the Swagger interface, the onboarding portal, the security review and
> the public repository are Tranche 3. Cache-miss percentiles could not be measured
> by our method, and we say why. USDC is excluded from the historical spot-check
> because its series is peg-derived and cannot fail the check. And one anomaly on
> two dates in March 2023 is real, reproducible, and we have not yet explained it."

> "All six Tranche 2 criteria are met, two of them against wording we amended in
> the open. Every figure in the package has a command a reviewer can run. Thank
> you."

---

## After recording

- Watch the whole take at full resolution with an eye only on **secrets** — the
  address bar, terminal scrollback, tab titles, and any autocomplete dropdown.
- Confirm no frame shows a route returning an unexpected 404 without the
  narration covering it.
- Upload with public link sharing, and paste the URL into Field 2 of
  [`milestone-2-form-answers.md`](./milestone-2-form-answers.md).
- If any figure changed between recording and submission, update the evidence
  document rather than re-shooting, and make sure the two do not disagree.
