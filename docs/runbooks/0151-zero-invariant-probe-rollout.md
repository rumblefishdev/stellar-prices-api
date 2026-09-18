# Rolling out the zero-invariant probe and its alarms (task 0151, ADR 0292)

Deploys one new probe check and one new CloudWatch alarm ladder:

| Piece                                                       | Stack                             | Deploy target                          |
| ----------------------------------------------------------- | --------------------------------- | -------------------------------------- |
| the `zero-invariants` check inside `rollup-freshness-probe` | `Prices-production-EventBridge`   | `make deploy-production-eventbridge`   |
| the `prices-production-zero-invariant-*` ladder             | `Prices-production-Observability` | `make deploy-production-observability` |

The check asserts two things about STORED rows, on `price_ohlcv_1m`, over a
48-hour window (`packages/rollup-freshness-probe/src/zero_invariants.rs`):

1. `pf_trade_count = 0 ⇒ close = 0` — a candle that formed no price carries none.
2. `close_usd > 0 ⇒ close > 0` — a USD close is `rate × close`, so it cannot
   exist without one.

Tests pin the writers we know about. This pins the data, which is what catches
the writer nobody listed.

## 1. Hard precondition: 0286's schema step must already be on production

> ⛔ **Do not deploy either stack until `price_ohlcv_1m` on production has the
> `pf_trade_count` column.** As of this writing task 0286 is **merged (`d11d20a`)
> and NOT deployed**, so the precondition is **not** yet met.

This is an ordering rule, not a preference, and the mechanism is worth
understanding before you weigh skipping it:

- `zero_invariant_query()` names `pf_trade_count`. On a ClickHouse without that
  column the read does not return a bad number — it **errors**.
- A failed read is pushed onto `failures`
  (`main.rs`, `failures.push(format!("zero-invariants read: {e}"))`).
- At the end of the invocation, **a non-empty `failures` fails the WHOLE probe
  run** (`main.rs`, `if !failures.is_empty() { return Err(...) }`).

So deploying the probe early does not merely leave one check dark. It turns
**every probe tick red** and takes the other checks down with it — rollup
freshness, current-prices age, disk headroom, the USD-sanity counts and the
MV-drift check all run in the same invocation. You would lose five working
alarms to gain one that cannot work yet.

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
            OR (close_usd > 0 AND close = 0)) AS violations,
       count() AS scanned
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 172800 SECOND;
```

A `scanned` of 0 is **not** a pass. The probe refuses an empty scan rather than
publishing a healthy zero (`SanityRefusal::EmptyScan`), precisely because the
alarm is `NOT_BREACHING` on missing data and a query that matched nothing must
not read as a clean bill of health. If `scanned` is 0, ingestion is the problem —
investigate that first.

### Cost of the scan — measure it, this is the second way to lose the probe

The window is 48 hours, but `timestamp` is the **fourth** column of the table's
sort key (`asset_id, quote_asset_id, source, timestamp`), so ClickHouse prunes
to the monthly **partition**, not to 48 hours, and then reads it with `FINAL` —
on the largest table, every 15 minutes. Unlike the two USD-sanity scans it has
no quote-leg predicate to narrow it. Nothing here has been measured on
production: the local ClickHouse holds a handful of rows.

A scan that times out fails exactly like the missing column in section 1 — the
read lands in `failures` and the **whole invocation** goes red. And the check
runs before the MV-drift check inside one 300-second Lambda, so a slow scan
costs that check its turn even when it does finish.

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
    AND ((pf_trade_count = 0 AND close != 0) OR (close_usd > 0 AND close = 0))
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
- All three alarms should settle in `OK` (not `INSUFFICIENT_DATA`) within two
  15-minute periods.

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
