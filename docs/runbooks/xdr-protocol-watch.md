# Runbook — the XDR protocol watch

**Audience:** anyone who gets a notification from the issue
**"stellar-xdr lags the mainnet protocol"**, or who changes the watch itself.
No prior context assumed.

|          |                                                                                                            |
| -------- | ---------------------------------------------------------------------------------------------------------- |
| Workflow | `.github/workflows/xdr-protocol-watch.yml` — daily at 06:17 UTC, plus manual `workflow_dispatch`           |
| Script   | `tools/scripts/verify-xdr-protocol-gap.mjs` (`npm run xdr:watch-protocol-gap`)                             |
| Output   | one tracking issue, titled exactly `stellar-xdr lags the mainnet protocol`                                 |
| Tasks    | 0098 (the watch), 0319 (green while nothing can be done), 0277 (the protocol 28 bump, as a worked example) |

## What it guards against

Our ledger-processor decodes every ledger with the `stellar-xdr` Rust crate. The
crate's **major version is the protocol it understands**: `stellar-xdr` 28
decodes protocol 28. When mainnet votes in a newer protocol and we are still on
the old crate, decoding stops — and **silently**: the SQS queue drains, the DLQ
stays empty, the Lambda logs no error and the queue-age alarm stays green. That
is how protocol 27 froze the live candles for six days (tasks 0091 / 0094).

Nothing in our repository changes when that happens, so a check on push or pull
request cannot see it. The watch runs on a clock instead.

⚠️ **The protocol number is a proxy for the XDR, and protocol 29 broke it**
(task 0325). stellar-core v29 pins the same XDR commit as v28.0.1, so
`stellar-xdr` 28 decodes protocol 29 and no `stellar-xdr` 29 was published.
What stopped ingestion on 2026-10-01 was BE's Galexie: its captive core must
match the protocol whatever the XDR does, and this watch cannot see it. That
stall was caught by the CloudWatch alarms (`production-galexie-ingestion-lag`
after 9 min, `rollup-freshness-1m` after 22 min).

## What it checks

Four readings, compared every run:

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

**The rule: a red run, the failure email and an issue appear only when someone
can do something.** A protocol that is announced but has no crate to bump to is
reported, but stays green.

| Tier         | Condition                                                                                          | Run       | Issue                                                          | Notification                          |
| ------------ | -------------------------------------------------------------------------------------------------- | --------- | -------------------------------------------------------------- | ------------------------------------- |
| **OK**       | pin ≥ core supports                                                                                | green     | an open one is commented "Resolved" and **closed**             | the close comment                     |
| **WAITING**  | pin < core supports or mainnet current, **no** stable `stellar-xdr` for that protocol on crates.io | **green** | **none opened**; an open one is closed as "not actionable yet" | none (only the close comment, if any) |
| **LAGGING**  | pin < core supports, a stable crate for it **is** published                                        | red       | **opened**, body says _bump now_                               | issue opened + GitHub's failure email |
| **BEHIND**   | pin < mainnet current — mainnet already voted — and a stable crate for it **is** published         | red       | opened, or the LAGGING one gets an "Escalated" comment         | yes, once                             |
| **NO CHECK** | Horizon or crates.io could not be read                                                             | red       | **untouched** — nothing is opened, edited or closed            | GitHub's failure email only           |

Rules that are easy to get wrong:

- **WAITING is green on purpose** (task 0319). Before, the run failed every day
  from the moment core supported a new protocol, and the issue asked for a bump
  nobody could make, because no crate existed. Now the report still says WAITING
  on the run summary, but nothing notifies until the crate is published. That
  day the run turns red and the issue opens: that is the notification.
- **WAITING also covers a vote with no crate** (task 0325). Mainnet on a
  protocol with no published `stellar-xdr` is green too: there is nothing to
  bump to, and the protocol may not have changed XDR at all (29 did not). The
  cost: a protocol that does change XDR, voted before its crate ships, stays
  green here. That stall shows as frozen candles, which the
  `rollup-freshness-*` and `ledger-processor-no-invocations` alarms catch.
- **A check that could not complete is red, but opens no issue.** Horizon or
  crates.io being unreachable means nothing was measured. The failure email says
  so; there is no bump to ask for, so the issue is left alone. A watch that
  silently checks nothing is the failure it exists to prevent, which is why the
  run still fails.
- **A pre-release does not count.** `29.0.0-rc.1` is named in the report, but
  the tier stays WAITING until a stable `29.x` exists.
- **Unchanged tier = silent.** The body is rewritten every run (with the fresh
  report and a link to the run), but there is no comment, so the thread is not
  muted by daily noise. GitHub's failure email, however, arrives on every red
  run; it goes to whoever last edited the `cron:` line.

The tier is stored as an HTML comment on the first line of the issue body,
`<!-- xdr-protocol-watch tier=LAGGING -->`. The workflow compares it with the
new tier to decide whether to comment. Don't edit it by hand.

## What to do, per tier

**WAITING** — nothing; you get no notification. The run summary says which
protocol is coming and the newest crate version. Optionally ask BE when they
expect to bump `xdr-parser`.

**NO CHECK** — re-run the workflow later (`gh workflow run … --ref master`). If
it keeps failing, check whether `https://horizon.stellar.org/` or
`https://crates.io/api/v1/crates/stellar-xdr` changed shape.

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

**BEHIND** — mainnet has voted and a crate for its protocol is published. Same
steps as LAGGING, immediately, then verify the live candle frontier moves
(`max(timestamp)` of `price_ohlcv_1m`).

**After any vote, whatever the tier** — check the candle frontier anyway. A
green watch only says the crate question is settled; Galexie can still be
stalled (protocol 29).

## Running it by hand

```bash
# the same check as the schedule (strict: LAGGING, BEHIND and NO CHECK exit 1;
# WAITING exits 0)
npm run xdr:watch-protocol-gap

# the PR-mode check (only BEHIND or a Cargo.toml / Cargo.lock mismatch exits 1)
npm run xdr:verify-protocol-gap

# against a different Horizon or crates.io endpoint
HORIZON_URL=https://horizon-testnet.stellar.org/ npm run xdr:watch-protocol-gap
CRATES_URL=http://127.0.0.1:8765/fake-crate.json npm run xdr:watch-protocol-gap

# every tier against a local mock, no network (CI runs it via `nx run-many -t test`)
node --test tools/scripts/verify-xdr-protocol-gap.test.mjs
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

### Rolling out a workflow change — `master` first

This is how the watch first reached `master` (PR #308, 2026-09-11), and it is
the step owed for PR #369 (task 0319).

⚠️ **Order matters for #369.** Its script change makes WAITING a green run.
`master`'s old workflow reads any green run as "resolved" and would close #336
with the false comment _"Resolved — the pinned `stellar-xdr` is level with
mainnet again."_ So the new workflow goes to `master` **before** #369 merges to
`develop`. The new workflow with the old script is harmless: the old script
still reports LAGGING, and the new workflow keeps #336 as it is.

1. Branch off `master` and copy **only** the workflow file from the PR's branch:

   ```bash
   git fetch origin
   git checkout -b ci/0319_xdr-watch-tiers-on-master origin/master
   git checkout origin/feat/0319_xdr-protocol-watch-says-when-the-bump-is-possible -- .github/workflows/xdr-protocol-watch.yml
   git diff --cached --stat   # must list exactly one file
   git commit -m "ci(lore-0319): bring the xdr watch tiers to master"
   git push -u origin ci/0319_xdr-watch-tiers-on-master
   gh pr create --base master --title "ci(0319): bring the xdr watch tiers to master"
   ```

   Nothing else goes to `master` in this PR: the workflow reads everything else
   from `develop`. (For a later change that is already on `develop`, copy from
   `origin/develop` instead.)

2. Merge that PR, then confirm `master` carries the new file:

   ```bash
   git fetch origin
   git diff origin/master origin/feat/0319_xdr-protocol-watch-says-when-the-bump-is-possible -- .github/workflows/xdr-protocol-watch.yml
   # no output = master runs the new workflow
   ```

3. Only now merge #369 to `develop`.

4. Run the watch once by hand instead of waiting for 06:17 UTC:

   ```bash
   gh workflow run xdr-protocol-watch.yml --ref master
   gh run list --workflow xdr-protocol-watch.yml --limit 1
   ```

5. Check the result while protocol 29 has no crate:
   - the run is **green**, and its summary shows the `WAITING` report with
     `newest on crates.io …`;
   - **#336 is closed** with the comment _"Closed as not actionable yet …"_.
     That single close comment is the last notification until the crate is
     published.

6. From then on you hear from the watch only when there is something to do: a
   **new issue** opens the day a stable `stellar-xdr` for the new protocol is
   published (follow _LAGGING_ above), or it escalates to **BEHIND** if mainnet
   votes first.

If #369 was merged to `develop` before step 2, expect #336 to be closed once
with the misleading "Resolved" comment. Nothing is lost: the watch opens a new
issue when the crate is published.

### If `develop` stops being what ships

The `ref: develop` in the checkout step is deliberate: it measures the branch
that gets deployed. If deploys ever move to `master` (or `master` starts
following `develop` again), change that `ref:` to the branch that ships, in the
same PR that changes the deploy source. Otherwise the watch reads a pin nobody
deploys.
