# Milestone 3 — Deliverable Verification Video Script

> ⚠️ **DRAFT — opened 2026-09-25.**

Target length **about 7 minutes**. Milestone 3 opens the API to developers, so
the video walks through what a developer and a reviewer meet: the portal, the
contract, the tests, the load test, the security review, the operations view
and the repository. As in Milestones 1 and 2, everything on screen is the
production deployment and every command runs live.

Scene 2 doubles as the recorded walk AC 3 asks for: sign-in, key, a `/v1` call,
revocation, each with a timestamp.

---

## Before recording

### Preconditions

- **The release is done.** `develop` is merged to `master` and the first CI run
  on `master` after it is green. Scenes 4 and 7 show `master`.
- **A Discord account for the recording**, not a personal one. It must be older
  than 5 minutes, a member of the Stellar Developers server
  (`discord.gg/stellardev`), and past the server's screening; otherwise the
  portal refuses it.
- **That account has not had a key revoked this month.** Regenerate turns the
  key off at once and issues no replacement until the 1st of the next month.
  Rehearse scene 2 with a different account.
- **Not on a day the history recomputation truncates a rollup tier** (task 0286
  runbook §7a): candles and the dashboard look wrong while a tier rebuilds.

### Windows to have open, in scene order

1. **Browser tab A**: the portal, `https://sorobanscan.rumblefish.dev/prices-api/`,
   signed out, scrolled to the top. Decline non-essential cookies in the consent
   banner before recording.
2. **Terminal A**: `curl` and `jq` ready, with `$API` and `$API_KEY` (the
   reviewer key) **exported in a shell you are not recording**.
3. **Browser tab B**: `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`.
4. **Browser tab C**: the rendered reference, `https://sorobanscan.rumblefish.dev/prices-api/docs`.
5. **Terminal B**: the repository root, with `npm run openapi:lint` already run
   and its summary on screen. The first run builds the extractor and takes a
   few minutes.
6. **Browser tab D**: GitHub Actions, the first `master` run after the release,
   job _Rust (fmt, clippy, test, lambda build)_ expanded.
7. **Editor**: `docs/prices-api-load-test-100rps.md` at "Evidence run and the
   ceiling", and `docs/scf/milestone-3-evidence.md` at AC 6.
8. **Browser tab E**: CloudWatch, dashboard `prices-production-overview` in
   `eu-central-1`, opened by direct URL and zoomed to a readable size.
9. **Browser tab F**: a private window on
   `https://github.com/rumblefishdev/stellar-prices-api`, branch `master`.
10. **Editor**: `docs/scf/milestone-3-rfp-deviations.md`.

### ⚠️ Secrets — read this before you hit record

- **Export** `$API_KEY` **out of frame**, for example
  `export API_KEY=$(cat ~/.prices-reviewer-key)`. Never type a key on camera,
  and never `echo` one.
- **The key issued in scene 2** stays masked on the dashboard. Copy it with the
  Copy button and read it in the terminal with `pbpaste`; it is revoked at the
  end of the scene.
- **The AWS console header shows the account and the signed-in role.** Keep it
  out of frame, or crop it.
- **Check the address bar** before every tab switch, and the terminal
  scrollback before each scene.
- If a secret lands in a frame, revoke or rotate it rather than re-editing the
  video.

### Values to have ready

| Item          | Value                                                                       |
| ------------- | --------------------------------------------------------------------------- |
| API base      | `https://prices-api.sorobanscan.rumblefish.dev`                             |
| Reviewer key  | `$API_KEY`, exported out of frame                                           |
| Portal        | `https://sorobanscan.rumblefish.dev/prices-api/`                            |
| Launch        | 2026-09-23, 09:40 CEST                                                      |
| AC 5          | p95 49.0 ms, 0 errors in 30,001 requests, `prices-production-loadtest-plan` |
| AC 9          | uptime 100.000 %, 0 × 5XX in 65,806 requests, gateway p95 145.2 ms          |
| Alarms        | 83 `prices-production-*`, all OK                                            |
| Earliest data | 2015-11-18                                                                  |
| Dashboard     | `prices-production-overview`, `eu-central-1`                                |
| Repository    | `https://github.com/rumblefishdev/stellar-prices-api`                       |

### What NOT to show

- The CloudWatch dashboard list and the alarm list: they include the block
  explorer's resources, which share the account.
- Secrets Manager, the mTLS certificate files, ClickHouse credentials, the
  Discord application settings.
- The machine running the history recomputation.
- A load-test run. The figures are from 2026-09-18; a run today would measure a
  server that is also running the recomputation.

---

## Scene 1 — Intro and scope (~0:30)

**On screen:** browser tab A, the top of the portal.

> "This is Milestone 3 of the Stellar Prices API, Production Launch and
> Validation, the last tranche. Milestone 1 built the ingestion; Milestone 2 put
> a public API in front of it. Milestone 3 opens it to developers: a
> self-service portal, a published contract, integration tests in CI, a load
> test, a security review, the public repository and a week of monitoring after
> launch. The service went public on 23 September. Everything I show is the
> production deployment."

---

## Scene 2 — The portal, a key, end to end (~1:30)

**On screen:** browser tab A.

> "The onboarding portal is public. Sign-in is through Discord, and eligibility
> is membership in the official Stellar Developers server."

Sign in with the recording account. The dashboard opens with a new key.

> "The key is issued on the free plan: one request a second, a hundred thousand
> a month. The dashboard shows the key, masked, with its plan, its limits and
> its usage."

Click Copy. **On screen:** Terminal A.

```bash
date -u
export NEW_KEY="$(pbpaste)"
curl -sS -H "x-api-key: $NEW_KEY" "$API/v1/assets/native/price" | jq '{price_usd, as_of, price_status}'
```

> "It works at once: the current XLM price, with the time it was computed and
> its status."

**On screen:** browser tab A. Click Regenerate and confirm.

> "Regenerate turns this key off immediately. A replacement can be issued from
> the start of the next quota period, so a leaked key cannot be swapped to
> escape the monthly quota."

**On screen:** Terminal A.

```bash
date -u
curl -sS -o /dev/null -w '%{http_code}\n' -H "x-api-key: $NEW_KEY" "$API/v1/assets/native/price"
```

> "The same request now gets a 403. Sign-in, key, a call with it, revocation,
> each with a timestamp: that is the self-service flow working on production."

If the old key still answers, wait ten seconds and repeat the request before
saying the line.

---

## Scene 3 — The contract, the reference and the live API (~1:00)

**On screen:** Terminal A.

```bash
curl -s "$API/api-docs-json" | jq -r '.openapi, (.paths | keys | length)'
```

> "The API publishes its OpenAPI document without a key. It is OpenAPI 3.1,
> generated from the handler code, so it lists exactly the routes the API
> serves."

**On screen:** Terminal B, the lint summary.

> "CI lints that same document on every Rust change, with Redocly's strict
> ruleset; this is today's run, with no errors."

**On screen:** browser tab C. Expand `GET /v1/assets/{id}/price`.

> "The portal renders it as an API reference, with an example for every
> field. The criterion names IBM's openapi-validator and Swagger UI; we use
> Redocly and the portal's own renderer, and the deviations document says why,
> with IBM's result."

**On screen:** Terminal A.

```bash
curl -sS -o /dev/null -w '%{http_code}\n' -H "x-api-key: $API_KEY" "$API/v1/no-such-route"
```

> "And a route that does not exist answers 404 in the same error envelope as
> every other error."

---

## Scene 4 — Tests and load (~1:00)

**On screen:** browser tab D, the `master` run.

> "Since 22 September, CI starts a ClickHouse database, applies the schema,
> starts the mTLS proxy and runs the integration suite against it. These are the
> tests that used to be skipped for lack of a database, and each of the seven
> API routes has its own. This is the first run on master after the release."

Open the _ClickHouse integration tests_ step and scroll to the summary.

**On screen:** editor, the load-test report at "Evidence run and the ceiling".

> "The load test: a hundred requests a second for five minutes gave a p95 of 49
> milliseconds against a 100 millisecond bar, with no errors in thirty thousand
> requests, on the purpose-built plan the criterion asks us to name. That
> scenario is mostly cache hits. With every request a miss, p95 is 130
> milliseconds from my laptop in Poland and 45 to 90 at the gateway. 500 a
> second held; at 1000 the limit is the ClickHouse server we share with the
> block explorer."

---

## Scene 5 — Security (~0:30)

**On screen:** editor, `milestone-3-evidence.md` at AC 6.

> "The security checklist. ClickHouse is reachable only over mutual TLS through
> Caddy; the client certificates live in Secrets Manager, not in environment
> variables; every invalid input gets a 400. No IAM statement grants every
> action or a whole service. Twenty-two statements use a wildcard resource, each
> for an action AWS gives no resource ARN, and each is listed with what limits
> it."

---

## Scene 6 — Operations (~1:15)

**On screen:** Terminal A.

```bash
curl -sS -H "x-api-key: $API_KEY" "$API/v1/backfill/status" \
  | jq '{realtime_tip_ledger, sdex: (.sdex | {status, earliest_data_available})}'
curl -s 'https://horizon.stellar.org/ledgers?order=desc&limit=1' | jq '._embedded.records[0].sequence'
```

> "The SDEX archive is complete. It reaches back to November 2015, two years
> beyond the January 2018 target. It finished in Tranche 2, so 'running' no
> longer applies; liveness is the ingestion tip, a few ledgers behind Stellar's
> own Horizon, and the freshness alarms."

**On screen:** browser tab E, the dashboard and its alarm strip.

> "The dashboard: API latency and errors, ingestion lag, ClickHouse writes, the
> workers, and an alarm strip with all 83 alarms, all OK. The Stellar team gets
> read access to it on request, per reviewer, through IAM Identity Center."

**On screen:** Terminal B.

```bash
python3 docs/scf/milestone-3-monitoring/report.py | head -20
```

> "The report for the seven days after launch: no 5XX in 65,806 requests,
> uptime 100 percent, gateway p95 145 milliseconds. It recomputes from the raw
> CloudWatch export in the repository, so it runs without AWS access. Two of the
> five metrics the criterion names describe a running backfill; the report
> carries ingestion freshness in their place."

---

## Scene 7 — Repository and deploy (~0:30)

**On screen:** browser tab F.

The public repository, logged out, on `master`. The root `README.md`, then
`infra/README.md` §"Fresh-account deployment", scrolled from its three parts
(the platform, the tenant, this app) to the list of manual steps.

> "The repository is public. The README leads to a fresh-account deployment
> runbook. It has not been run in an empty account, because that needs the
> platform this API runs on first; the claim rests on the runbook, a synth with
> no credentials in CI, and a name-by-name check of the runbook against the
> code."

---

## Scene 8 — Deviations and limitations, wrap-up (~0:35)

**On screen:** editor, `milestone-3-rfp-deviations.md`.

> "Five places where the delivery departs from the wording, each declared with
> its reason: the backfill that finished early, the two report metrics it made
> flat, dashboard access given per reviewer instead of a standing role, the
> linter and the renderer, and the scope of the load-test number. The evidence
> document also lists the known issues and the limitations: the history
> recomputation that finishes after delivery, a slow path we measured but did
> not instrument, and the ceiling of the shared server."

> "Every criterion has evidence a reviewer can check, with the command or the
> report behind it. Thank you."

---

## After recording

- Watch the whole take at full resolution for **secrets** only: the address
  bar, terminal scrollback, tab titles, autocomplete, the console header.
- Write the scene 2 timestamps (sign-in, the `/v1` call, revocation) into
  evidence AC 3 as the recorded walk.
- Upload with public link sharing and paste the URL into Field 2 of
  [`milestone-3-form-answers.md`](milestone-3-form-answers.md).
- If a figure changes between recording and submission, update the evidence
  document rather than re-shooting, and keep the two in agreement.
