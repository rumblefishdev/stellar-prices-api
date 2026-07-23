# Runbook — repair missing USD values in the coarse OHLCV tables (task 0114)

Fills `close_usd` / `volume_quote_usd` in the coarse forever-tables
(`price_ohlcv_1h`, `_4h`, `_1d`, `_1w`, `_1M`) for the Soroban era, where the
rollup path captured pre-enrichment zeros and never revisited them. Hard
100%-zero block is **2025-02 → 2026-02**; the full repair span is **2024-02 →
present**.

The tool is `enrichment-worker`'s `coarse-repair` binary. It is **additive-only**
(re-inserts corrected rows the `ReplacingMergeTree` collapses by higher
`version`) and **partition-bounded** (one month at a time, so cost is independent
of table size).

It can also `FREEZE` each partition before touching it, but **in practice the
operator takes the snapshots** — `prices_writer` cannot hold the required
privilege (precondition 7 / Step 3b). Either way, every partition is snapshotted
before it is written.

> ⚠️ For 2025-02 → 2026-02 the `price_ohlcv_1m` source was dropped by cleanup on
> 2026-07-18. The coarse tables are the **sole surviving copy**. NEVER truncate or
> rebuild them — this tool never does; do not substitute the pre-roll's
> clean-slate path. **Never run without a verified snapshot**, whoever took it.

---

## Which tables

Only the **forever-tables**: `price_ohlcv_1h`, `_4h`, `_1d`, `_1w`, `_1M`.
`price_ohlcv_15m` has a 30-day retention (cleanup drops it), so it holds no deep
history to repair, and its recent 30 days are enriched live. Run the tool once
per forever-table.

## Preconditions

| #   | Check                                                                                                                                                                                                                           | How                                                   |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| 1   | **prices-api owner sign-off** — heavy RMT rewrites on the shared `ch-prod-01`.                                                                                                                                                  | —                                                     |
| 2   | **Low-traffic window** picked.                                                                                                                                                                                                  | Same courtesy as the pre-roll runbook.                |
| 3   | **Not colliding with the 0088 pre-roll.** The tail backfill is pre-Soroban (≤2020); this repair is 2024-02+. Disjoint partitions — safe to run concurrently, but confirm 0088's step-3 pre-roll is not writing the same months. | [[continue-soroban-backfill]]                         |
| 4   | **Disk headroom on the CH host** for `FREEZE` snapshots (hardlinks under `shadow/`, cheap, but non-zero). Pure SQL — **no container restart**.                                                                                  | `ssh … 'df -h /var/lib/docker'`                       |
| 5   | **mTLS writer certs present** on the operator box.                                                                                                                                                                              | `$HOME/prices-mtls/prices_writer.{crt,key}`, `ca.crt` |
| 6   | Version-arithmetic + additive-only already proven by test (`ch_enrich_it.rs`).                                                                                                                                                  | no action                                             |
| 7   | **`ALTER FREEZE PARTITION` — decide the snapshot path.** `prices_writer` does **not** hold it, and cannot be granted it (see below), so the tool's built-in `FREEZE` fails. Use the operator pre-freeze path in Step 3b.        | `SHOW GRANTS FOR CURRENT_USER`                        |

> **Snapshots must be taken by the CH admin, not by the tool** (learned the hard
> way on the 2026-07-23 prod run). `coarse-repair` connects as `prices_writer`,
> which holds only `SELECT, INSERT, ALTER DELETE, OPTIMIZE`. Its `FREEZE` is
> refused with `ACCESS_DENIED` — and the clickhouse driver surfaces that as a
> bare `Clickhouse(BadResponse(""))` with no message, so replay the statement
> over `curl` to see the real error.
>
> `GRANT ALTER FREEZE PARTITION ON prices.* TO prices_writer` does **not** fix
> it: `prices_writer` is defined in `users.xml`, which is read-only storage, so
> the grant fails with `ACCESS_STORAGE_READONLY` **even as the container's
> superuser**. Nobody can grant it at runtime. Editing the XML on a shared prod
> cluster is not worth it for a one-off repair — take the snapshots as admin
> instead (Step 3b) and run with `--skip-snapshot`.

---

## Step 0 — land the code

The `coarse-repair` bin and the partition-bounding change must be merged and built
from a real branch — do not run prod off an uncommitted working tree.

```bash
# from the repo, on the branch carrying the 0114 code
/branch            # feat branch off the 0114 work
/pr                # open PR → develop; CI must be green
# merge after review, then pull on the operator box
```

## Step 1 — build the tool on the operator box

```bash
cd ~/stellar-prices-api && git checkout develop && git pull --ff-only
# aws-mtls is REQUIRED for --transport hetzner
cargo build --release -p enrichment-worker --features aws-mtls --bin coarse-repair
```

Binary lands at `./target/release/coarse-repair`.

Point at the writer certs (same env as sdex-backfill):

```bash
export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt
```

## Step 2 — DRY RUN first (writes nothing)

Preview the months-with-zeros for each table. This only reads.

```bash
for TBL in price_ohlcv_1h price_ohlcv_4h price_ohlcv_1d price_ohlcv_1w price_ohlcv_1M; do
  echo "===== $TBL ====="
  ./target/release/coarse-repair \
    --transport hetzner --table "$TBL" \
    --start-month 202402 --end-month 202607 \
    --dry-run
done
```

Expect a per-month table showing `zeros_before` counts, heaviest across
2024-02 → 2026-02. `enriched = 0`, nothing written.

## Step 3 — record the baseline (before)

Hand this to the prod CH shell and **keep the output** — it is the before/after
evidence the ACs require. Repeat per table (`1h` shown).

`FINAL` is mandatory — these are `ReplacingMergeTree` tables and an
un-collapsed read counts superseded rows, inflating `pct_zero`.

```bash
cat > /tmp/0114_cov.sql <<'SQL'
SELECT toYYYYMM(timestamp) AS month,
       count() AS rows,
       countIf(close_usd = 0) AS zero_usd,
       round(100 * zero_usd / rows, 1) AS pct_zero
FROM prices.price_ohlcv_1h FINAL
WHERE volume_quote > 0 AND toYYYYMM(timestamp) BETWEEN 202402 AND 202607
GROUP BY month ORDER BY month FORMAT PrettyCompact;
SQL

ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec -i app-clickhouse-1 clickhouse-client' < /tmp/0114_cov.sql
```

## Step 3b — snapshot every affected partition (as CH admin)

Because of precondition 7 the operator takes the snapshots, not the tool. Freeze
**before** repairing, and use the tool's own naming so the rollback and cleanup
steps below apply unchanged.

```bash
cat > /tmp/0114_freeze.sh <<'EOF'
#!/bin/sh
# Freeze every active partition of $1 in the repair span. Idempotent-ish: an
# already-frozen partition errors with DIRECTORY_ALREADY_EXISTS, which is
# reported as `already-frozen` rather than silently counted as success — a
# re-freeze would otherwise overwrite a pre-repair snapshot with a post-repair
# one, destroying the rollback point.
TBL="$1"
clickhouse-client -q "SELECT DISTINCT partition FROM system.parts
  WHERE database='prices' AND table='$TBL' AND active
    AND toUInt32(partition) BETWEEN 202402 AND 202607
  ORDER BY partition FORMAT TabSeparated" |
while read p; do
  NAME="repair_0114_prices_${TBL}_${p}"
  if clickhouse-client -q "ALTER TABLE prices.$TBL FREEZE PARTITION $p WITH NAME '$NAME'" 2>/tmp/freeze_err; then
    echo "frozen $TBL $p"
  elif grep -q DIRECTORY_ALREADY_EXISTS /tmp/freeze_err; then
    echo "already-frozen $TBL $p (pre-existing snapshot KEPT)"
  else
    echo "FAILED $TBL $p: $(cat /tmp/freeze_err)" >&2
  fi
done
EOF

scp -i ~/.ssh/sorban-prod_ed25519 /tmp/0114_freeze.sh deploy@168.119.73.161:/tmp/
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker cp /tmp/0114_freeze.sh app-clickhouse-1:/tmp/ && docker exec app-clickhouse-1 sh /tmp/0114_freeze.sh price_ohlcv_1h' \
  | tee /tmp/0114_freeze_1h.log
```

Verify the count matches the months the dry run reported, and that the snapshots
hold real data, **before** any write:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec app-clickhouse-1 ls /var/lib/clickhouse/shadow/ | grep -c repair_0114_prices_price_ohlcv_1h_'
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  'docker exec app-clickhouse-1 du -sh /var/lib/clickhouse/shadow/'
```

A near-zero `du` means the partitions were not captured — stop.

## Step 4 — the real run (snapshot ON), one table at a time

Start with `price_ohlcv_1h` (the table BE consumes). Review its result before
moving to the next table.

Run under `tmux` — the full `1h` span measured **~7 min/month**, so budget 3–4 h.

```bash
tmux new -s repair0114
./target/release/coarse-repair \
  --transport hetzner --table price_ohlcv_1h \
  --start-month 202402 --end-month 202607 \
  --skip-snapshot 2>&1 | tee /tmp/0114_run_1h.log
```

> `--skip-snapshot` is correct **only because Step 3b already took the
> snapshots**. The tool prints a loud warning saying partitions have no backup;
> that warning is false here and the `ls`/`du` output from Step 3b is the
> evidence. Without Step 3b the warning is exactly as serious as it sounds — this
> span is the sole surviving copy.

Keep the default batch size. `--batch-size 100000` cuts per-batch re-scans but
raises per-query memory on a shared cluster (the concern tracked by task 0113).

The summary prints per month: `zeros_before | enriched | zeros_after |
snapshot`. `zeros_after` settling to a small residual is expected — those are
exotic quotes with no USD path (the genuine `no_reference` floor), not a failure.

### Log lines that look like failures but are not

Do **not** abort the run on these. They are written for the live 1m pass, where
they do signal a real fault; on the historical coarse repair they are normal.

| Log line                                                                                               | Why it is expected here                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `enrichment pass enriched 0 rows despite a non-empty backlog — check oracle↔asset-id reconciliation …` | Emitted for any month that is **entirely exotic quotes**. The preceding `peg-pivot tier made no progress — remaining candles have no USD reference (exotic quotes)` is the accurate description. 202402 is wholly like this (~620 k rows, `enriched 0`). |
| `oracle tier drained — handing remaining candles to peg-pivot tier`, `remaining` unchanged             | `prices.oracle_prices` starts **2025-09**, so the oracle tier no-ops across most of the span **by design**. The peg-pivot / stablecoin-direct tiers do the work.                                                                                         |
| `snapshot DISABLED for this partition` (per month)                                                     | Correct under the Step 3b path — the operator already froze it. Verify once via `ls`/`du`, then ignore the repetition.                                                                                                                                   |

**These are the signals that DO mean stop:**

- A month reporting **exactly 200,000** enriched — `one_shot` did not take
  effect; `coarse-repair.rs` hard-codes `max_batches: 20`, which at the default
  10 k batch caps a run at 200 k rows. Not a data limit.
- `enriched 0` on a month known to hold **peg- or pivot-reachable** rows (any
  month 2025-01 onward holds 150 k–340 k USDC-quoted rows) — that is the silent
  no-op this repair exists to avoid.
- A `FreezeDenied` error — snapshots are missing; see precondition 7.

### Timing

Measured 2026-07-23 on `price_ohlcv_1h`: **~7 min per month** at ~2.8 M
candidates, so the 30-month span runs **4–5 h**. Cost scales with candidate
count, not partition size. The dry run's scan rate (~12 M rows/s) does **not**
predict this — it measures enumeration only, and underestimated the run by ~2×.

**Merge pressure outlives the process.** Re-inserted RMT rows keep collapsing in
background merges after the tool exits. `zeros_after` reads `FINAL` so it is
correct immediately, but the shared host keeps working for a while.

Then repeat for `_4h`, `_1d`, `_1w`, `_1M`, one at a time.

## Step 5 — verify (after)

**a) Coverage dropped.** Re-run the Step-3 query. `pct_zero` for 2024-02 → 2026-02
should fall from 86–100% to the exotic-only floor.

**b) Check the reference, not the value ceiling.** The correct test is whether
the implied USD reference is right for the era — **not** whether every
`close_usd` looks sane. Some inputs are junk and the repair faithfully
multiplies them (see the dust-trade note below), so a value ceiling will always
"fail" on data the repair cannot fix.

```sql
SELECT b.asset_code AS base, q.asset_code AS quote,
       round(toFloat64(p.close_usd) / nullIf(toFloat64(p.close), 0), 8) AS implied_ref_usd,
       count() AS rows
FROM prices.price_ohlcv_1h p FINAL
JOIN prices.assets b FINAL ON b.asset_id = p.asset_id
JOIN prices.assets q FINAL ON q.asset_id = p.quote_asset_id
WHERE toYYYYMM(p.timestamp) = <month> AND p.close_usd > 0
GROUP BY base, quote, implied_ref_usd ORDER BY rows DESC LIMIT 20
```

`implied_ref_usd` must be **1.0 exactly** for USDC/USDT-quoted rows (the
stablecoin-direct 1:1 cast) and the **real XLM/USD price for that month** for
XLM-quoted rows. On the 2026-07-23 pilot over 202502 it was 0.312–0.408, which
is correct for February 2025 — the strongest evidence the pivot tier works.

> **Dust-trade candles — expected, not a repair defect.** A tail of absurd
> `close_usd` values survives the repair (202502: 40 rows > $1M, max $29.6M,
> 0.004%). Every one is a single dust trade — `trade_count = 1`, a few XLM of
> volume, a nonsense unit price like 94,810,046 XLM/token — multiplied by a
> _correct_ reference. Live-enriched data has the same tail (`price_ohlcv_1h`
> 202607, written by the live path: max $24.0M, 0.0015%), so this predates the
> repair and is inherited from source candles. `volume_quote_usd` is unaffected
> (those rows carry ~$3 of volume), so BE's volume analytics do not see it.
> Tracked separately as a data-quality task.

**c) Spot-check liquid pairs against an independent source.** Pick a couple of
well-known pairs and eyeball `close_usd` against a known market price for that
month (e.g. CoinGecko historical). It should be in the right ballpark.

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query=\"
     SELECT a.asset_code, toDate(timestamp) AS d,
            round(toFloat64(close), 6) AS close_quote,
            round(toFloat64(close_usd), 6) AS close_usd
     FROM prices.price_ohlcv_1d p
     JOIN prices.assets a ON a.asset_id = p.asset_id
     WHERE p.timestamp BETWEEN toDateTime('2025-06-01') AND toDateTime('2025-06-02')
       AND p.close_usd > 0
     ORDER BY p.volume_quote DESC LIMIT 10 FORMAT PrettyCompact\""
```

## Step 6 — schedule the recurring guard

The one-off historical repair and the ongoing guard are the same job. Once the
historical run is verified, schedule a periodic pass over a **recent** window so
any rows the rollup path freezes going forward get mopped up — turning permanent
corruption into temporary lag.

- Cron target: run `coarse-repair` per forever-table over the trailing ~2 months
  (`--start-month`/`--end-month` = last two `YYYYMM`).
- Snapshot can be OFF for the recent window (1m still holds it and can rebuild),
  but leave it ON if unsure.
- Wire it like the enrichment Lambda's EventBridge schedule (infra), or as an
  operator cron on a schedule box. (Infra wiring is a follow-up — prepare-only.)

## Step 7 — clean up the snapshots

Once satisfied the repair is good and you no longer need the rollback point,
release the frozen snapshots (frees the `shadow/` hardlinks). The names are in
the Step-4 summary output (`repair_0114_prices_<table>_<month>`).

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query=\"
     SYSTEM UNFREEZE WITH NAME 'repair_0114_prices_price_ohlcv_1h_202502'\""
# repeat per snapshot name reported by the run.
```

---

## Rollback — if a month's repair looks wrong

Each repaired partition has a frozen "before" snapshot. To revert one month:

1. Locate the snapshot on the CH host: it is under
   `/var/lib/clickhouse/shadow/repair_0114_prices_<table>_<month>/`.
2. Detach the current (repaired) partition, then attach the frozen copy back:

```bash
# on the CH host, via the prod client. <month> is the numeric YYYYMM.
ALTER TABLE prices.<table> DETACH PARTITION <month>;
ALTER TABLE prices.<table> ATTACH PARTITION <month>
  FROM '/var/lib/clickhouse/shadow/repair_0114_prices_<table>_<month>/';
```

3. Verify the partition’s USD columns match the pre-repair state, then
   `SYSTEM UNFREEZE WITH NAME …`.

> The additive design means a bad run does **not** destroy the pre-repair rows at
> write time (the frozen snapshot is untouched hardlinks), so this restore is
> always available until you unfreeze. Automating this as a `--revert` flag is a
> tracked follow-up.

## Notes

- The repair reuses the exact enrichment tiers (`ch_enrich.rs`): USDC/USDT → ×$1,
  XLM → ×(XLM/USDC), exotic → left zero by design.
- The pivot tier computes its XLM/USDC reference from the **same coarse table**,
  forward-filling from earlier months, so a month's first buckets keep a valid
  anchor even when bounded to one partition.
- All figures are `FINAL`-collapsed reads; do not compare without `FINAL`.
