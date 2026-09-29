# Runbook — the XDR protocol watch

**Audience:** anyone who gets a notification from the issue
**"stellar-xdr lags the mainnet protocol"**, or who changes the watch itself.
No prior context assumed.

|          |                                                                                                  |
| -------- | ------------------------------------------------------------------------------------------------ |
| Workflow | `.github/workflows/xdr-protocol-watch.yml` — daily at 06:17 UTC, plus manual `workflow_dispatch` |
| Script   | `tools/scripts/verify-xdr-protocol-gap.mjs` (`npm run xdr:watch-protocol-gap`)                   |
| Output   | one tracking issue, titled exactly `stellar-xdr lags the mainnet protocol`                       |
| Tasks    | 0098 (the watch), 0319 (the WAITING tier), 0277 (the protocol 28 bump, as a worked example)      |

## What it guards against

Our ledger-processor decodes every ledger with the `stellar-xdr` Rust crate. The
crate's **major version is the protocol it understands**: `stellar-xdr` 28
decodes protocol 28. When mainnet votes in a newer protocol and we are still on
the old crate, decoding stops — and **silently**: the SQS queue drains, the DLQ
stays empty, the Lambda logs no error and the queue-age alarm stays green. That
is how protocol 27 froze the live candles for six days (tasks 0091 / 0094).

Nothing in our repository changes when that happens, so a check on push or pull
request cannot see it. The watch runs on a clock instead.

## What it checks

Three readings, compared every run:

| Reading             | Source                                                                                                                           |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| **our pin**         | `stellar-xdr` in `[workspace.dependencies]` of `Cargo.toml` (and `Cargo.lock` must agree, with only one major resolved)          |
| **mainnet current** | `current_protocol_version` from `https://horizon.stellar.org/`                                                                   |
| **core supports**   | `core_supported_protocol_version` from the same page — rises **weeks before** the vote, once validators run the new stellar-core |
| **published crate** | `max_stable_version` from `https://crates.io/api/v1/crates/stellar-xdr` — only read when our pin lags `core supports`            |

Why the crate is read separately: stellar-core builds its XDR straight from the
`stellar/stellar-xdr` git repo, so SDF runs a new protocol **before** the Rust
crate for it is published. On 2026-09-29 Horizon reported core 29 while the
newest crate was 28.0.1 — there was nothing to bump to yet.

## The tiers

| Tier        | Condition                                                                                    | Run   | Issue                                              | Notifies?                                               |
| ----------- | -------------------------------------------------------------------------------------------- | ----- | -------------------------------------------------- | ------------------------------------------------------- |
| **OK**      | pin ≥ core supports                                                                          | green | commented "Resolved" and **closed**                | yes (close comment)                                     |
| **WAITING** | pin < core supports, **no** stable `stellar-xdr` for that protocol on crates.io              | red   | opened or kept open, body says _nothing to do yet_ | only when the issue is first opened                     |
| **LAGGING** | pin < core supports, a stable crate for it **is** published (or crates.io could not be read) | red   | body says _bump now_                               | yes — once, on the move into LAGGING ("Now actionable") |
| **BEHIND**  | pin < mainnet current — mainnet already voted                                                | red   | body says _bump and deploy now_                    | yes — once, on the move into BEHIND ("Escalated")       |

Rules that are easy to get wrong:

- **WAITING still fails the run.** The issue stays open exactly as long as the
  run fails, and it should stay open through the wait. GitHub's own
  "scheduled workflow failed" email therefore arrives daily while WAITING; that
  email goes to whoever last edited the `cron:` line.
- **An unreadable crates.io is never WAITING.** The script reports LAGGING with
  `crates.io unknown (…)`, so an unknown answer never reads as "nothing to do".
  The workflow then **keeps an issue that was WAITING at WAITING** — otherwise it
  would post a false "Now actionable", and the day the crate really appears
  would find the tier already at LAGGING and stay silent.
- **A pre-release does not count.** `29.0.0-rc.1` is named in the report, but
  the tier stays WAITING until a stable `29.x` exists.
- **Unchanged tier = silent.** The body is rewritten every run (with the fresh
  report and a link to the run), but there is no comment, so the thread is not
  muted by daily noise.
- **An unreachable Horizon fails the run.** A watch that silently checks nothing
  is the failure it exists to prevent.

The tier is stored as an HTML comment on the first line of the issue body,
`<!-- xdr-protocol-watch tier=WAITING -->`. The workflow compares it with the
new tier to decide whether to comment. Don't edit it by hand.

## What to do, per tier

**WAITING** — nothing. Read the report to see which protocol is coming and the
newest crate version. Optionally ask BE when they expect to bump `xdr-parser`.

**LAGGING** — the bump is possible now, and mainnet has not voted yet:

1. Ask BE to bump `xdr-parser` first. `prices-ingest-core/src/decode.rs` takes
   `LedgerCloseMeta` across that crate boundary, so both sides must move to the
   same major; the script fails on two majors in `Cargo.lock`.
2. Check the new crate's XDR, not only its number: the XDR commit the crate pins
   must be the one core uses for that protocol (on 2026-09-29 rs-stellar-xdr
   `main` carried protocol-29 XDR under the label `28.0.0`).
3. Bump `stellar-xdr` in `Cargo.toml`, open the PR, and **deploy** it before the
   vote — a merged bump does nothing until the binary ships
   (`docs/runbooks/deploy-ledger-processor.md`; 0091 was merged and prod stayed
   frozen until 0094 shipped it). Task 0277 is the protocol 28 bump, done this
   way.

**BEHIND** — mainnet has voted and we can't decode new ledgers. Same steps as
LAGGING, immediately, then verify the live candle frontier moves
(`max(timestamp)` of `price_ohlcv_1m`).

## Running it by hand

```bash
# the same check as the schedule (strict: WAITING / LAGGING exit 1)
npm run xdr:watch-protocol-gap

# the PR-mode check (LAGGING and WAITING are only notices, exit 0)
npm run xdr:verify-protocol-gap

# against a different Horizon or crates.io endpoint
HORIZON_URL=https://horizon-testnet.stellar.org/ npm run xdr:watch-protocol-gap
CRATES_URL=http://127.0.0.1:8765/fake-crate.json npm run xdr:watch-protocol-gap
```

Trigger the workflow itself (it runs from `master`, see below):

```bash
gh workflow run xdr-protocol-watch.yml --ref master
gh run list --workflow xdr-protocol-watch.yml --limit 3
```

## ⚠️ Which branch runs what

A `schedule` trigger only fires from the repository's **default branch, which is
`master`**. `master` is not a release branch that follows `develop`; it is
updated only by deliberate PRs, and on 2026-09-29 it was 729 commits behind.
So the watch is split across two branches:

| Part                                                | Comes from                                         | Why                                                                    |
| --------------------------------------------------- | -------------------------------------------------- | ---------------------------------------------------------------------- |
| the **workflow file** (tiers, issue text, comments) | `master`                                           | a schedule only runs the default branch's copy                         |
| the **script** and **`Cargo.toml`**                 | `develop` (`actions/checkout` with `ref: develop`) | `develop` is what gets deployed, so its pin is the one worth measuring |

Consequences:

- A change to **the script** takes effect on the next run after it merges to
  `develop`.
- A change to **the workflow file** takes effect only once it reaches `master`.
  Merging it to `develop` changes nothing that runs.

### After a workflow change is merged to `develop` — bring it to `master`

This is how the watch first reached `master` (PR #308, 2026-09-11), and it is
the step owed after PR #369 (task 0319):

1. From an up-to-date `develop`, branch off `master` and copy only the workflow
   file across:

   ```bash
   git fetch origin
   git checkout -b ci/0319_xdr-watch-tiers-on-master origin/master
   git checkout origin/develop -- .github/workflows/xdr-protocol-watch.yml
   git diff --cached --stat   # must list exactly one file
   git commit -m "ci(lore-0319): bring the xdr watch tiers to master"
   git push -u origin ci/0319_xdr-watch-tiers-on-master
   gh pr create --base master --title "ci(0319): bring the xdr watch tiers to master"
   ```

   Nothing else goes to `master` in this PR — the workflow reads everything else
   from `develop`.

2. After it merges, confirm `master` carries the new file:

   ```bash
   git fetch origin
   git diff origin/master origin/develop -- .github/workflows/xdr-protocol-watch.yml
   # no output = master runs the same workflow as develop
   ```

3. Run it once by hand instead of waiting for 06:17 UTC:

   ```bash
   gh workflow run xdr-protocol-watch.yml --ref master
   ```

4. Check the result on the tracking issue (#336 while protocol 29 is pending):
   - the first line of the body reads `<!-- xdr-protocol-watch tier=WAITING -->`
     while crates.io has no stable `stellar-xdr` 29;
   - the body shows **"Nothing to do yet."** and the report's
     `newest on crates.io …` value;
   - there is **no new comment** (a move from LAGGING to WAITING is silent).

5. From then on, the next notification on that issue is either
   **"📦 Now actionable"** (a stable crate was published — follow _LAGGING_
   above) or **"⚠️ Escalated to BEHIND"** (mainnet voted first).

Until step 1 is done, `master`'s older workflow still runs. It treats WAITING
as LAGGING — the issue stays open and its report says WAITING, but its "What to
do" paragraph still asks for the bump, and publishing the crate posts no comment.

### If `develop` stops being what ships

The `ref: develop` in the checkout step is deliberate: it measures the branch
that gets deployed. If deploys ever move to `master` (or `master` starts
following `develop` again), change that `ref:` to the branch that ships, in the
same PR that changes the deploy source. Otherwise the watch reads a pin nobody
deploys.
