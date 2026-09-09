# SCF Milestone 2 — Form Answers (stellar-prices-api)

> Copy the text inside each blockquote into the matching field of the Stellar
> Community Fund **Deliverable Verification** form. Anything in
> `<ANGLE_BRACKETS>` is a placeholder to replace before submitting.
>
> Full evidence and rationale live in the companion document
> [`milestone-2-evidence.md`](./milestone-2-evidence.md), exported to PDF and
> attached in Google Drive next to the video. The wording deviations it declares
> are set out in [`milestone-2-rfp-deviations.md`](./milestone-2-rfp-deviations.md).

---

## Field 1 — Tranche Deliverables

> **Deliverable 2 — Public API** (as originally approved).
>
> Milestone 2 delivers the public read API over the Milestone 1 ingestion
> pipeline: seven endpoint groups on a custom domain, key-gated, cached,
> validated, and verified against the deployed system rather than a staging
> copy.
>
> What is live and verifiable today:
>
> 1. **All seven endpoint groups are deployed and conformant.** An operator-run
>    conformance suite exercises every group across twenty major assets covering
>    all three identifier forms, and validates every response — errors included —
>    against the API's own live OpenAPI document. Latest run on production:
>    **1014 checks pass, 0 fail, 0 skip** — the fourth consecutive zero-failure
>    run, one of which was repeated 29 minutes later with a check-for-check
>    identical verdict.
> 2. **Load tested at the approved target.** 100 requests per second sustained
>    for five minutes on the current-price route: **p95 of 47.09 ms** against a
>    200 ms bar, and **zero errors in 30,001 requests** against a 0.1 % bar. The
>    evidence document states plainly why this pass is narrow and what it does
>    not prove.
> 3. **Response caching is deployed and verified.** Per-endpoint TTLs on the API
>    Gateway stage cache, confirmed against three independent surfaces. Cache
>    hits measure 45 to 53 ms and misses 78 to 145 ms, with no overlap, and
>    expiry is demonstrated on both TTL tiers.
> 4. **The volume-weighted price is verifiable from raw rows.** The published
>    figure was recomputed from 29,108 raw one-minute candles in plain Python,
>    independent of the database's own aggregation: **41 of 41 checks reconciled
>    across four multi-source assets**, worst relative difference 1.4e-11 against
>    a stated tolerance of 1e-9, and volumes exact.
> 5. **Historical depth well beyond the tranche target.** The backfill status
>    endpoint reports earliest available SDEX data of **2015-11-18** against a
>    target of 2022-01-01, reconciled four ways, with candles on **every calendar
>    day** from 2022-01-01 to the present.
> 6. **The full price pipeline is wired.** The §5.5 weighted-price formula with a
>    minimum-volume source threshold, inter-source outlier detection, Aquarius
>    appearing as a named source with real prices and volumes, and input
>    validation returning 400 on invalid input with roughly fifty negative tests
>    running in continuous integration.
> 7. **The three items Milestone 1 deferred are delivered.** The OpenAPI document
>    is served anonymously through the gateway, the CloudWatch dashboard has real
>    data widgets over 50 alarms, and the API edge is settled: a custom domain,
>    CORS preflight on all seven routes verified in a real browser, and a
>    recorded decision against a WAF with four named reversal triggers.
>
> **In-tranche refinements.** Two acceptance criteria are graded against amended
> wording, and both amendments are declared rather than assumed. The load-test
> criterion's target is unchanged, but since a rate-limit change no key on the
> default plan can sustain it, so the run used a purpose-made usage plan and the
> report says which. The cache criterion names an `X-Cache: Hit` header that
> **API Gateway does not emit on any route and cannot be made to emit
> truthfully** — a handler-written header would report a miss on every genuine
> hit, and CloudFront writes a different string, so the literal wording would be
> unmet even after an edge rebuild. It is graded instead on response latency and
> the deployed per-method cache configuration. That is a weaker claim than a
> header would be, and we say so in those words. The reasoning is recorded in an
> accepted ADR with all three alternatives closed.
>
> A third deviation concerns the historical spot-check. The criterion names USDC;
> USDC's series in our store is entirely peg-derived, carries no trades, and
> returns exactly 1.00 through the March 2023 depeg, so it cannot fail the check
> and therefore cannot pass it. We spot-check XLM and yBTC instead, over 28 dates
> rather than the five asked for, against an independent off-Stellar exchange:
> median deviation **0.06 %** and **0.48 %**, with 27 of 28 dates inside 5 % on
> each.
>
> **Full evidence — acceptance-criteria mapping with runnable commands, the
> complete deviation rationale, the limits of each result stated where the
> evidence sits, and an explicit list of what is _not_ claimed:**
> https://drive.google.com/file/d/16wTFhOmJkjLe1ixDXDQbbIDckh0lHlv2/view?usp=sharing

---

## Field 2 — Deliverable Verification - Video

> https://drive.google.com/file/d/1WkC4BG-8vcZrEGqJ1CjPE9iiTasotfOb/view?usp=sharing

---

## Field 3 — Additional Deliverable Verification

> **Evidence package (Google Drive):** https://drive.google.com/drive/folders/1Y0xKwCstQnb0Sn5XNWvuxXkgNpPFx7HC?usp=sharing
>
> The folder holds `milestone-2-evidence.pdf` — the acceptance-criteria
> walkthrough with reproducible commands, dashboard screenshots, the deviation
> rationale and the not-claimed list — and the demo video. Direct links:
>
> - Evidence PDF: https://drive.google.com/file/d/16wTFhOmJkjLe1ixDXDQbbIDckh0lHlv2/view?usp=sharing
> - Demo video: https://drive.google.com/file/d/1WkC4BG-8vcZrEGqJ1CjPE9iiTasotfOb/view?usp=sharing
>
> **Live & anonymous (verify directly in a browser):**
>
> - OpenAPI specification for the whole API:
>   `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`
> - Rendered API reference:
>   `https://sorobanscan.rumblefish.dev/api/docs`
> - API health probe:
>   `https://prices-api.sorobanscan.rumblefish.dev/health`
>
> **Key-gated (API key available to reviewers on request — send the `x-api-key`
> header):**
>
> - Asset list: `GET https://prices-api.sorobanscan.rumblefish.dev/v1/assets`
> - Asset detail: `GET .../v1/assets/native`
> - Current price with per-source breakdown: `GET .../v1/assets/native/price`
> - Source threshold override: `GET .../v1/assets/native/price?min_volume_usd=5000`
> - OHLCV candles: `GET .../v1/assets/native/ohlcv?timeframe=all&granularity=1d`
> - Batch prices: `POST .../v1/prices/batch`
> - Oracle cross-reference: `GET .../v1/oracles/native`
> - Backfill status: `GET .../v1/backfill/status`
>
> **Reproduce the acceptance evidence yourself.** Every criterion in the evidence
> document carries a runnable command. The two that need only an API key:
>
> - Cache behaviour, five requests, no load generation — the recipe is in
>   `milestone-2-evidence.md` §5, AC 3. `min_volume_usd` is part of the cache key
>   on the price route, so any unused value forces a guaranteed miss in one
>   request.
> - Historical spot-check against an independent exchange — the Binance klines
>   URL and the field to compare are in §5, AC 6.
>
> **Source code:**
>
> - Repository: `https://github.com/rumblefishdev/stellar-prices-api`
> - Technical design, including the verbatim Tranche 2 acceptance criteria (§9)
>   and a dated revision history:
>   `.../blob/develop/docs/prices-api-general-overview.md`
> - Declared deviations from the RFP and criteria wording:
>   `.../blob/develop/docs/scf/milestone-2-rfp-deviations.md`
> - ADR 0012 — why no CloudFront and no `X-Cache` header:
>   `.../blob/develop/lore/2-adrs/0012_api-gateway-stage-cache-no-cloudfront-no-x-cache-header.md`
> - ADR 0011 — `base_currency` denominates rather than filters, and the unpriced
>   bucket contract:
>   `.../blob/develop/lore/2-adrs/0011_base-currency-is-a-denomination-not-a-pair-filter.md`
> - REST API (axum): `.../tree/develop/packages/prices-api`
> - Conformance suite: `.../blob/develop/tools/scripts/conformance-0120.mjs`
> - Load-test script: `.../blob/develop/packages/prices-api/loadtest/price_load.js`
>
> **Operational endpoints (private, available on request):** production
> ClickHouse `ch.sorobanscan.rumblefish.dev`, database `prices` (mTLS — client
> certificate issued on request); the `prices-production-overview` CloudWatch
> dashboard and the `prices-production-*` alarms in eu-central-1 (a read-only
> viewer identity can be provisioned for a reviewer).
>
> ⚠️ The `execute-api` hostname cited in the Milestone 1 package was retired on
> 2026-09-02 and now returns 403. The Milestone 1 documents were amended in place
> with dated notes, so every command in both packages remains runnable as
> written.

---

## Field 4 — Support Needed

> —

---

## Pre-submission checklist

- [~] Every Field 3 link **verified present on `develop` 2026-09-09** — both
  ADRs, the deviations document, the general overview, the conformance script
  and the load-test script. ⏳ The submission branch itself is 23 commits
  ahead of `develop` and not yet merged; per project convention the PR stays
  open until the work is confirmed in production.
- [x] **All cited figures re-run 2026-09-09.** Numbers drift: the backfill
      advances, coverage percentages move, and the traded population changes
      between pagination walks — it fell from 5,353 to 3,900 assets in one day.
- [x] Conformance suite re-run on production 2026-09-09 08:12 UTC and §5 AC 1
      updated: **1014 pass, 0 fail, 0 skip**.
- [x] Cache recipe in §5 AC 3 re-run 2026-09-09. The **five-row** table still
      separates cleanly: hits 37-54 ms, misses 282-948 ms. The first ask was a
      cold Lambda start and is reported as such.
- [x] `GET /v1/backfill/status` re-read 2026-09-09 and `earliest_data_available`
      confirmed unchanged at `2015-11-18T03:47:00Z`. All four AC 5 views re-run,
      including the three ClickHouse ones; none moved.
- [x] Dashboard screenshots — **decided: keep the 2026-09-03 captures**
      (commit `a6d3147`, task 0125). They are illustrations of the widgets, not a
      source of any figure, and §7.2 now dates them and explains why their strip
      shows 49 where the text says 50. No re-capture owed.
- [~] `ch-demo-queries.sql` — the queries behind figures the document actually
  pastes were run against production 2026-09-09 (method distribution, the
  per-source 24 h volumes, and AC 5's three database views), and §6.3's
  sources payload was refreshed from them. The remaining queries are
  reproduction aids whose output is not quoted; they were not all re-run.
- [x] `milestone-2-evidence.md` exported: `./build-pdf.sh`
      (already parameterised — it defaults to milestone 2, and `./build-pdf.sh 1`
      still rebuilds the Milestone 1 PDF from its own source). Last build
      2026-09-09: 20 pages, 1.1 MB, all five screenshots embedded.
- [x] PDF uploaded to Google Drive with link-sharing public — **verified
      anonymously 2026-09-09**: the file downloads with no session and is
      byte-identical to the local build (`md5 9c3bb905…`, 1,112,228 bytes,
      20 pages).
- [x] Links placed: the evidence PDF closes Field 1, the folder opens Field 3
      with both direct links beneath it. All three return HTTP 200 anonymously.
- [x] Video uploaded with public sharing; URL in Field 2
      (`stellar-prices-api-milestone-2.mp4`, reachable anonymously).
- [x] All `<ANGLE_BRACKET>` placeholders replaced — none remain outside the
      convention note at the top of this file.
- [x] `curl -sS https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`
      returns **HTTP 200** and the OpenAPI 3.1 document anonymously — verified
      2026-09-09.
- [~] No API key, certificate, or other secret material. The **PDF text and all
  four Markdown deliverables were scanned clean** 2026-09-09 — no AWS access
  keys, no Stellar secret seeds, no private-key blocks, no literal
  `x-api-key` values, and no personal usernames. ⏳ **The video frames and
  the screenshots still need a human pass** — address bar, tab titles,
  terminal scrollback, autocomplete dropdowns.
- [ ] Field 4 — decide between `—` and a specific support request.
- [x] English-only across all four blocks; no personal usernames (scanned
      2026-09-09). Task numbers appear only with their context.
