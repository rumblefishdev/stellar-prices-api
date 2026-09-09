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
> `<DRIVE_FOLDER_LINK>`

---

## Field 2 — Deliverable Verification - Video

> `<VIDEO_LINK>`

---

## Field 3 — Additional Deliverable Verification

> **Evidence package (Google Drive):** `<DRIVE_FOLDER_LINK>` — contains
> `milestone-2-evidence.pdf` (acceptance-criteria walkthrough with reproducible
> commands, dashboard screenshots, the deviation rationale, and the
> not-claimed list) and the demo video.
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

- [ ] `develop` merged up to date and every Field 3 link resolves in an
      incognito window (the anonymous ones without a key).
- [ ] **All cited figures re-run within days of submission.** Numbers drift: the
      backfill advances, coverage percentages move, and the traded population
      changes between pagination walks.
- [ ] Conformance suite re-run on production and the pass/fail/skip counts in
      `milestone-2-evidence.md` §5 AC 1 updated: `npm run conformance:0120`.
- [ ] Cache recipe in §5 AC 3 re-run; the six-row latency table still separates
      cleanly.
- [ ] `GET /v1/backfill/status` re-read and `earliest_data_available` in §5 AC 5
      confirmed unchanged.
- [ ] Dashboard screenshots current — `screenshots/ac8-dashboard-*.png` were
      captured **2026-09-03** (commit `a6d3147`, task 0125) and are byte-identical
      in the tree today; they have never been re-taken. Their alarm strip shows
      **49** alarms against §7.2's text, which says **50**. Re-capture before
      submission, or correct the count.
- [ ] `ch-demo-queries.sql` run against production and any pasted output in the
      evidence document refreshed.
- [ ] `milestone-2-evidence.md` finalised and exported: `./build-pdf.sh`
      (retarget it from the Milestone 1 source first).
- [ ] PDF uploaded to a Google Drive folder with link-sharing set to "anyone with
      the link can view".
- [ ] Drive folder link copied into the Field 1 closer **and** the Field 3
      opener (replace both `<DRIVE_FOLDER_LINK>` placeholders).
- [ ] Video uploaded with public sharing; URL pasted into Field 2.
- [ ] All `<ANGLE_BRACKET>` placeholders in this file replaced.
- [ ] `curl -sS https://prices-api.sorobanscan.rumblefish.dev/api-docs-json | head`
      returns the specification anonymously.
- [ ] No API key, certificate, or other secret material visible in the PDF, the
      screenshots, or any video frame — including the browser address bar and
      any terminal scrollback.
- [ ] Field 4 — decide between `—` and a specific support request.
- [ ] English-only across all four blocks; no internal slang, no task numbers
      without context, no personal usernames.
