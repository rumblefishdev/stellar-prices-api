# Rolling out the zero-invariant probe and its alarms (task 0151, ADR 0292)

Deploys one new probe check and one new CloudWatch alarm ladder:

| Piece                                                       | Stack                             | Deploy target                          |
| ----------------------------------------------------------- | --------------------------------- | -------------------------------------- |
| the `zero-invariants` check inside `rollup-freshness-probe` | `Prices-production-EventBridge`   | `make deploy-production-eventbridge`   |
| the `prices-production-zero-invariant-*` ladder             | `Prices-production-Observability` | `make deploy-production-observability` |

The check asserts three things about STORED rows, on `price_ohlcv_1m`, over a
48-hour window (`packages/rollup-freshness-probe/src/zero_invariants.rs`):

1. `pf_trade_count = 0 ⇒ close = 0` — a candle that formed no price carries none.
2. `close_usd > 0 ⇒ close > 0` — a USD close is `rate × close`, so it cannot
   exist without one.
3. `pf_trade_count > 0 ⇒ close > 0` — a candle that claims price-forming fills
   carries the price they formed. This is the one that catches a statement that
   **omits `pf_trade_count`**: the column takes its `DEFAULT (trade_count)`, so a
   dust-only minute is stored as `close = 0, pf_trade_count = 5`, which neither
   of the first two can see.

Tests pin the writers we know about. This pins the data, which is what catches
the writer nobody listed — **at the live tip only**. The window is on the
candle's BUCKET time, not on when the row was written (the table has no
insert-time column). A backfill, a pre-roll script or a hand-run `INSERT … SELECT`
into buckets older than 48 hours is **not** covered; a historical rewrite such as
0286 phase 3 needs its own assertion over the partitions it touched.

## 1. Hard precondition: 0286's schema step must already be on production

> ⛔ **Do not deploy either stack until `price_ohlcv_1m` on production has the
> `pf_trade_count` column.** As of this writing task 0286 is **merged (`d11d20a`)
> and NOT deployed**, so the precondition is **not** yet met.

This is an ordering rule, and it is worth knowing exactly what breaks if it is
skipped — it is less than it sounds, and still not acceptable:

- `zero_invariant_query()` names `pf_trade_count`. On a ClickHouse without that
  column the read does not return a bad number — it **errors**.
- The probe is built so that one check's failure never suppresses another
  (`main.rs`, the comment opening the handler): the failed read is recorded in
  `failures`, **every other check still runs and publishes normally**, and only
  at the end does a non-empty `failures` make the invocation return an error.
- That error trips the probe's own invocation-errors alarm — the **dead-probe
  signal** ("the per-tier rollup lag metric may be stale, blinding every rollup
  freshness alarm").

So deploying the probe early does **not** take the other checks down. What it
does is make the dead-probe alarm fire, and stay fired, every 15 minutes, for a
probe that is in fact alive. A dead-probe alarm that is permanently red cannot
tell anyone when the probe really dies — the false-page failure mode, on the one
alarm that watches the watcher.

The alarm stack alone is harmless before the probe (the metric simply never
arrives, and the ladder treats missing data as `NOT_BREACHING`), but there is no
reason to split them: deploy both, after 0286.

**Confirm the precondition** (read-only — see "Where these commands run"
below):

```sql
SELECT count() FROM system.columns
WHERE database = 'prices' AND table = 'price_ohlcv_1m' AND name = 'pf_trade_count';
```

`1` means 0286's schema step has run. `0` means **stop**.

## 2. Pre-deploy read: the invariants must already hold

Run the assertion by hand before you deploy the thing that alarms on it. The
expected result is **`violations = 0`** with a `scanned` well above zero.

> ⚠️ The probe's own query names its table **unqualified**
> (`FROM price_ohlcv_1m FINAL`) because the Lambda binds its client to `prices`.
> A manual run has no such binding, so the copy below is qualified. Keep it so.

```sql
SELECT countIf((pf_trade_count = 0 AND close != 0)
            OR (close_usd > 0 AND close = 0)
            OR (pf_trade_count > 0 AND close = 0)) AS violations,
       count() AS scanned
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 172800 SECOND;
```

A `scanned` of 0 is **not** a pass. The probe refuses an empty scan rather than
publishing a healthy zero (`SanityRefusal::EmptyScan`), precisely because the
alarm is `NOT_BREACHING` on missing data and a query that matched nothing must
not read as a clean bill of health. If `scanned` is 0, ingestion is the problem —
investigate that first.

### Expect a non-zero count right after 0286 — and wait it out, do not repair it

0286 rolls out schema first and **ingest last**. In between, the old ingest keeps
writing rows without the pf columns, so they take `pf_trade_count = trade_count`.
Where such a row's price underflowed to `close = 0`, it breaks invariant 3 — not
because a writer regressed, but because the writer that fixes it is not deployed
yet. Those rows age out of the window by themselves: if section 2 reads non-zero
and every offending row (section 3's query) predates 0286's ingest step, wait
until that step is 48 hours old and read again. Deploy on a `0`, not before.

### Cost of the scan — measure it, this is the second way to lose the probe

The window is 48 hours, but `timestamp` is the **fourth** column of the table's
sort key (`asset_id, quote_asset_id, source, timestamp`), so ClickHouse prunes
to the monthly **partition**, not to 48 hours, and then reads it with `FINAL` —
on the largest table, every 15 minutes. Unlike the two USD-sanity scans it has
no quote-leg predicate to narrow it. Nothing here has been measured on
production: the local ClickHouse holds a handful of rows.

A scan that ClickHouse refuses or times out behaves exactly like the missing
column in section 1: the other checks publish normally, the invocation errors,
and the dead-probe alarm goes red for a probe that is alive. A **hard Lambda
timeout** (300 seconds for the whole invocation) is the worse case — it loses
whatever has not been published yet — and that is why this check runs LAST:
every other datum, the MV-drift one included, is out before it starts, so a
runaway scan can only cost itself.

So take the timing from the same manual run (append `FORMAT JSON` and read
`statistics.elapsed`, or time the `curl`):

| Manual run                                  | Meaning                                                                                                                               |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| a few seconds                               | deploy                                                                                                                                |
| tens of seconds, or `MEMORY_LIMIT_EXCEEDED` | **do not deploy.** Record the numbers on lore task 0151 and narrow the query first (a shorter window, or a partition-aligned `FINAL`) |

Re-take the number in the first week of a month and in the last: the partition
being read is a day old in one case and thirty in the other.

## Where these commands run

⚠️ **Read this before section 1 or 2.** Both reads are plain SQL, and both name
their table as `prices.…` on purpose, so nothing depends on a default database.
A client pointed at `localhost:8123` reads the **local dev ClickHouse** and
returns a clean result that says nothing whatever about production — check the
host in the command before you read the answer.

Prod's HTTP endpoint (`ch.sorobanscan.rumblefish.dev`) is mTLS-only behind Caddy
(task 0276). Run the reads from a workstation with your **personal read
certificate**, which maps to the read-only ClickHouse user `dev_read`:

```bash
curl --fail-with-body -sS \
  --cert   "$READ_CERT" \
  --key    "$READ_KEY" \
  --cacert "$PRICES_CA" \
  https://ch.sorobanscan.rumblefish.dev/ \
  --data-binary @query.sql
```

Put the SQL of section 1 or 2 into `query.sql` first. No password is involved:
the certificate is the credential, so nothing secret reaches `argv`. Do **not**
use a write certificate (`dev_shared`, `prices_writer`) for a read — `dev_read`
cannot change anything by construction, and that is the point of having it.

`dev_read` is capped at **3.73 GiB per query**. If section 2's scan is refused
with `MEMORY_LIMIT_EXCEEDED`, that is a finding about the query's cost, not an
obstacle to route around — see "Cost of the scan" in section 2.

## 3. If the count is not zero

**A WRITER did this. Find it before repairing anything.** Repairing the rows
first destroys the evidence and the writer just re-creates them.

- The usual cause is a statement that **omits `pf_trade_count`** and takes its
  `DEFAULT (trade_count)`. The Rust writer is name-routed, so an omitted column
  is silent (ADR 0287).
- Check, in this order: recent `INSERT … SELECT` run by hand, the pre-roll
  scripts, and the enrichment statements — against the inventory at
  `docs/database-schema/close-usd-zero-guardrails.md`.
- Locate them:

  ```sql
  SELECT timestamp, asset_id, quote_asset_id, source, close, close_usd,
         trade_count, pf_trade_count, version
  FROM prices.price_ohlcv_1m FINAL
  WHERE timestamp >= now() - INTERVAL 172800 SECOND
    AND ((pf_trade_count = 0 AND close != 0) OR (close_usd > 0 AND close = 0)
      OR (pf_trade_count > 0 AND close = 0))
  ORDER BY timestamp DESC LIMIT 100;
  ```

> ⛔ **Do not deploy the ladder while the count is breached.** The first rung is
> `1`, so any violation at all pages immediately — you would be deploying a
> guaranteed page.

## 4. Deploy

`make diff-production` first and **read it for removals**, not just additions:
`--require-approval broadening` prompts on IAM/security-group widening only, so
a deploy can destroy a live resource without asking (`infra/Makefile`).

```bash
make diff-production
make deploy-production-eventbridge     # the probe Lambda + its schedule
make deploy-production-observability   # the alarm ladder
```

## 5. Alarms to expect

Three rungs, reusing the existing `opsAlarms.usdSanityEscalationCounts` ladder
(`[1, 100, 10000]` at `infra/envs/production.json`) — no new config key:

- `prices-production-zero-invariant-1`
- `prices-production-zero-invariant-100`
- `prices-production-zero-invariant-10000`

| Property           | Value                                             |
| ------------------ | ------------------------------------------------- |
| namespace / metric | `Prices/Rollup` / `CandleZeroInvariantViolations` |
| dimension          | `Environment=production`                          |
| statistic / period | `Maximum` over 15 minutes                         |
| evaluation periods | 2, with 1 datapoint to alarm                      |
| comparison         | `>=` threshold                                    |
| missing data       | `NOT_BREACHING`                                   |

The healthy reading is **exactly 0**, so the first rung is the one that matters
and the other two only say how fast a regressed writer is growing the count.

`NOT_BREACHING` on missing data is also why the probe refuses an empty scan
instead of publishing a zero: if it published one, a query that matched nothing
would be indistinguishable from a clean tier.

## 6. Verification

- The probe's structured log line carries `zero_invariant_violations` and
  `zero_invariant_scanned` — check `scanned` is non-zero on the first tick after
  the deploy.
- `checks_failed` on that same line must be `0`. Read it **first**: a failed
  read logs `zero_invariant_scanned = 0` too, exactly like an empty table, and
  only `checks_failed` tells the two apart.
- **The metric actually arrives.** Do NOT take the alarms reading `OK` as
  evidence: the ladder is `NOT_BREACHING` on missing data, so all three settle in
  `OK` within two periods even if the probe never publishes a single datapoint —
  a step that passes either way proves nothing. Ask CloudWatch for the datum:

  ```bash
  aws cloudwatch get-metric-statistics \
    --namespace Prices/Rollup --metric-name CandleZeroInvariantViolations \
    --dimensions Name=Environment,Value=production \
    --statistics Maximum --period 900 \
    --start-time "$(date -u -d '-1 hour' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)"
  ```

  At least one datapoint, with `Maximum` = `0`. An empty `Datapoints` list means
  the check is not publishing — read the probe's log line for `checks_failed`.

## Rollback

> ⛔ **Never `make destroy-production-observability`.** That target is
> `cdk destroy Prices-production-Observability --force`: it deletes the WHOLE
> stack — every rollup-freshness tier, the DLQ ladder, disk headroom, MV drift,
> `current_prices` age, both USD-sanity ladders, the SNS wiring and the
> dashboard — with no confirmation prompt. It does not "remove the ladder"; it
> removes production's alarms.

To remove **only** this ladder, revert the commit that added the
`zeroInvariantAlarms` block to `infra/src/lib/stacks/observability-stack.ts`,
then:

```bash
make diff-production                    # expect exactly three alarms removed, nothing else
make deploy-production-observability
```

If the diff shows anything beyond the three `prices-production-zero-invariant-*`
alarms, stop.

The probe check is not independently toggleable: rolling it back means reverting
its wiring in `packages/rollup-freshness-probe/src/main.rs` and redeploying the
EventBridge stack. Remove the check **before** the ladder, or the ladder just
goes quiet on missing data. If the check is failing because the precondition was
not met, the fix is to complete 0286's schema step, not to remove the check.

## Related

- [`0286-candle-definitions-rollout.md`](0286-candle-definitions-rollout.md) —
  phase 1, whose schema step is this runbook's hard precondition.
- [`0142-rollup-mv-reapply.md`](0142-rollup-mv-reapply.md) — the host-loopback
  route, which is for the drift BINARY (it reads `Config::from_env()`). Its
  environment block does nothing for plain SQL; do not copy it here.
- `packages/rollup-freshness-probe/src/zero_invariants.rs` — the query, the
  metric and the empty-scan refusal.
- `docs/database-schema/close-usd-zero-guardrails.md` — the guardrail inventory
  a violation is triaged against.
- ADR 0292 §6; lore task 0151.
