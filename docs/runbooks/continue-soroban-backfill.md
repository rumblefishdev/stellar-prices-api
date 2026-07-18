# Runbook — Continue the Soroban historical backfill (chunk by chunk)

**Audience:** anyone (incl. the BE team) running this for the first time. No prior
context assumed. Follow the steps top to bottom.

## What this does

We are backfilling historical price candles (`prices.*` on the Hetzner
ClickHouse) by replaying Stellar ledgers from the public archive. The whole job
is too big for one run, so we do it in **chunks**: each run indexes a slice of
ledgers, writes candles directly to Hetzner over mTLS, and records where it got
to. You just repeat: _find where the last run ended → run the next slice →
verify → repeat_, until the whole range is done.

The tool is the `sdex-backfill` binary in this repo. In `combined` mode it
extracts **SDEX trades + Soroban AMM swaps + oracle prices** from a single
download of each ledger.

> **⚠ Writing `1m` is only half the job — the `1m` table is NOT durable.**
> `prices.price_ohlcv_1m` is a **transient 7-day feeder**: the nightly cleanup
> worker drops its month-partitions, and the live rollup MVs only aggregate the
> last 2 h (`now() - INTERVAL 2 HOUR`), so they **ignore backfilled rows**. The
> store of record is the coarse forever-tables (`1h/4h/1d/1w/1M`), and for
> **historical** data those are filled by an explicit **pre-roll** (§9), never
> the live MVs. **A backfilled range isn't "done" until it has been pre-rolled**
> — otherwise its candles are silently dropped at the next 02:00 UTC cleanup.
> Background + root-cause: task 0090 and
> `docs/runbooks/fix-backfill-history-loss-and-rerun.md`.

### Where we are (update this line as you go)

- **Done:** `[50457424, 51007999]` (the first tranche + first forward chunk).
- **Next combined chunk starts at:** `51008000`.
- **Remaining combined range:** up to `63352611` (the "floor" — see §7).
- **After that:** one `sdex-only` run over the pre-Soroban tail `[1, 50457423]`.

> The authoritative "where did it end" is always the live number in ClickHouse
> (Step 3) — the line above is just a hint.

---

## 0. Prerequisites

- **A Linux machine to run on.** Strongly prefer an **AWS EC2 instance in
  `us-east-2`** — see §1 (it makes the download ~20–50× faster).
- **Rust** (stable, via [rustup](https://rustup.rs)) — to build the binary.
- **AWS CLI v2** — used to download ledgers (`aws s3 sync`).
- **`jq`** — to split the mTLS bundle JSON into files.
- **Disk:** each 5-partition chunk downloads ~60 GB of ledgers, but the scratch
  folder is cleaned per-partition as it goes, so **~20 GB free** is enough. (One
  64k-ledger partition is ~12 GB.)
- **Access to the prices AWS account** (`750702271865`) with permission to read
  the writer secret in Step 2. If you don't have it, ask the prices-api owner to
  grant `secretsmanager:GetSecretValue` on the secret named in Step 2, or to hand
  you the three PEM files directly.
- **SSH access to the prod ClickHouse host** (to check progress) — key
  `~/.ssh/sorban-prod_ed25519`, host `deploy@168.119.73.161`.

---

## 1. Where to run it — pick the fastest region

The public ledger archive lives in the S3 bucket **`aws-public-blockchain`,
which is in AWS region `us-east-2` (Ohio)**. Download bandwidth is the entire
bottleneck of this job (each ledger is ~180 KB and there are millions of them).

- **Fastest by far:** run on an **EC2 instance in `us-east-2`**. Same-region S3
  reads are hundreds of MB/s and incur no cross-region data-transfer cost. A
  chunk that takes ~3.4 h on a home connection can finish in minutes there.
- **Slow:** running from a laptop / office network pulls the data across the
  internet at ~5 MB/s. Fine for one or two chunks; painful for the whole chain
  (the remaining combined range alone is ~2.3 TB).

Everything else (writing candles to Hetzner in Germany, fetching the secret from
`eu-central-1`) is tiny and location-independent — only the S3 download speed
matters, so **co-locate with the bucket in `us-east-2`.**

If you launch an EC2 box: Amazon Linux or Ubuntu, an instance with good network
(e.g. `m6i.large`+), a ~40 GB disk, and install `rustup`, `awscli`, `jq`, `git`.

---

## 2. Get AWS access and the mTLS write credential

There are **two** AWS interactions, and they are different:

1. **Downloading ledgers** — the archive bucket is public, so this needs **no
   credentials at all**; the tool passes `--no-sign-request`. You do not sign in
   for this.
2. **Fetching the write credential** — one-time, to get the certificate the
   backfill uses to authenticate to the Hetzner ClickHouse. This needs AWS
   credentials for the prices account.

### 2a. Sign in to the prices AWS account (one-time, for the secret only)

Use whatever your team uses for account `750702271865`. With AWS SSO that is:

```bash
aws configure sso            # first time only; set the start URL + region
aws sso login --profile soroban-explorer
```

(`soroban-explorer` is the conventional profile name — use yours if different.)

### 2b. Fetch the writer mTLS bundle → three PEM files

The backfill authenticates as the `prices_writer` ClickHouse user. Its
certificate/key/CA are stored as one JSON secret. Pull it and split it into
three files (the commands below **never print** the key to your terminal):

```bash
mkdir -p ~/prices-mtls && cd ~/prices-mtls

aws secretsmanager get-secret-value \
  --profile soroban-explorer \
  --region eu-central-1 \
  --secret-id prices/production/clickhouse-mtls-prices-ingestion-production \
  --query SecretString --output text > bundle.json

jq -r .cert bundle.json > prices_writer.crt
jq -r .key  bundle.json > prices_writer.key
jq -r .ca   bundle.json > ca.crt

chmod 600 prices_writer.key      # lock down the private key
rm bundle.json                   # don't leave the combined secret lying around
```

> **Secret hygiene:** `prices_writer.key` is a private key. Never `cat`, echo,
> commit, or copy it anywhere. It only ever needs to sit in `~/prices-mtls` on
> the run box. Delete the folder when the whole backfill is finished.

Keep these three paths handy — you'll point the tool at them in Step 5.

---

## 3. Build the tool

```bash
git clone <repo-url> stellar-prices-api    # or: cd into your existing checkout
cd stellar-prices-api
git checkout develop && git pull --ff-only

# The `aws-mtls` feature is REQUIRED to write to Hetzner.
cargo build --release -p sdex-backfill --features aws-mtls
```

The binary lands at `./target/release/sdex-backfill`.

---

## 4. Find where the last run ended

Ask the database for the forward watermark (read-only):

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query='SELECT task_name, current_ledger, status FROM prices.backfill_progress FINAL FORMAT PrettyCompact'"
```

Look at the **`soroban_amm`** row. Its `current_ledger` is the last ledger done.
**Your next chunk starts at `current_ledger + 1`.**
(The `sdex_archive` row showing `paused` at `50457424` is expected — ignore it
until you reach the pre-Soroban tail in §8.)

---

## 5. Run the next chunk

**Pick the range.** Chunks are sized in whole 64,000-ledger partitions. Five
partitions = 320,000 ledgers ≈ 3–4 h on a home line, minutes on a us-east-2 EC2.

- `START` = `soroban_amm.current_ledger + 1` (from Step 4).
- `END` = `START + 320000 - 1` (5 partitions), **but never past the floor
  `63352611`** — see §7.
- `TIP` = the current live chain tip (so the progress % is meaningful):

```bash
curl -s 'https://horizon.stellar.org/' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["core_latest_ledger"])'
```

**Point at your cert files and run inside `tmux`** (so an SSH drop can't kill
the run — see §11 on why finishing matters):

```bash
tmux new -s backfill        # detach later with Ctrl-b then d; reattach: tmux attach -t backfill

export CH_DOMAIN=ch.sorobanscan.rumblefish.dev
export MTLS_CERT_PATH=$HOME/prices-mtls/prices_writer.crt
export MTLS_KEY_PATH=$HOME/prices-mtls/prices_writer.key
export MTLS_CA_PATH=$HOME/prices-mtls/ca.crt

./target/release/sdex-backfill \
  --mode combined \
  --start 51008000 \
  --end   51327999 \
  --tip   <TIP> \
  --transport hetzner \
  --verbose
```

(Replace `51008000 / 51327999` with your `START / END`, and `<TIP>` with the
number from above.) A healthy start prints `pre-flight: all checks passed` and
`backfill starting … to_process: 5`. Then it downloads and indexes one partition
at a time, printing `partition indexing complete` with counts after each. When
the whole chunk is done you'll see `=== sdex-backfill complete ===`.

> One **WARN** at startup — _"combined mode starts after activation…"_ — is
> **normal and expected** for every continuation chunk. It just means earlier
> pools are resolved from the saved registry, which is exactly what we want.

---

## 6. Verify the chunk

Run these (read-only) after `=== sdex-backfill complete ===`. Substitute your
chunk's `START`/`END`.

```bash
# a) progress advanced to your END:
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query='SELECT task_name, current_ledger, status FROM prices.backfill_progress FINAL FORMAT PrettyCompact'"

# b) all ledgers written (expect END-START+1, e.g. 320000; a few short = archive tail-lag, fine):
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query='SELECT count() FROM prices.backfill_sdex_ledgers WHERE sequence BETWEEN 51008000 AND 51327999'"

# c) candles per source in the range:
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query='SELECT source, count() FROM prices.price_ohlcv_1m FINAL WHERE toUInt64(version) DIV 1000 BETWEEN 51008000 AND 51327999 GROUP BY source FORMAT PrettyCompact'"

# d) any pools we could not resolve this chunk:
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query='SELECT * FROM prices.unresolved_pools FINAL FORMAT PrettyCompact'"
```

What "good" looks like: `soroban_amm.current_ledger` = your `END`; the ledger
count matches; `sdex` candles present. In the **early** Soroban era (below
~ledger 62,000,000) it is **normal** to see **no `aquarius`/`phoenix` rows and
zero oracle rows** — those AMMs/oracles weren't active yet; they appear in later
chunks. Rows in `unresolved_pools` with a `first_ledger` **below your chunk's
`START`** are old leftovers from earlier runs, not from this chunk.

---

## 7. Repeat — and the "floor" you must not cross

Go back to Step 4 and do the next chunk: new `START` = `END + 1`, new `END` =
`START + 320000 - 1`, fresh `TIP`. Keep stepping forward.

**The floor.** The live ingestion service already writes candles from about
ledger **`63352612`** onward. The backfill must stop **below** that, or the two
would fight over the same minutes and undercount. So the **last** combined chunk
ends at exactly **`--end 63352611`** (do not go higher). All earlier chunks are
well below it, so you only think about the floor on the final chunk.

> Because the backfill now writes early `sdex` candles too, you can no longer
> read the live floor with a simple `min()` over `sdex`. Before the **final**
> chunk only, re-confirm it from a source the backfill hasn't reached yet, e.g.
> `SELECT min(toUInt64(version) DIV 1000) FROM prices.price_ohlcv_1m FINAL WHERE source='aquarius'` — it never decreases, so `63352611` stays a safe ceiling meanwhile.

Rough remaining size (home-line pace): the combined range to the floor is
~2.3 TB / ~5–6 days of continuous download; on a us-east-2 EC2 it's a small
fraction of that.

---

## 8. Final step — the pre-Soroban tail

Once a chunk reaches the floor and the combined range is done, run the classic
pre-Soroban ledgers **once** (SDEX only — there were no AMM pools before Soroban):

```bash
TIP=$(curl -s 'https://horizon.stellar.org/' | python3 -c 'import sys,json;print(json.load(sys.stdin)["core_latest_ledger"])')

./target/release/sdex-backfill \
  --mode sdex-only \
  --start 1 \
  --end   50457423 \
  --tip   "$TIP" \
  --transport hetzner \
  --verbose
```

This is the largest range (~50M ledgers) — definitely one for a us-east-2 EC2.
It walks `sdex_archive` down to genesis; when it finishes, the backfill is
complete.

---

## 9. Pre-roll to the coarse forever-tables (REQUIRED for durability)

Once the `1m` candles for the range you care about are written (a completed
chunk, or the whole range), you **must** roll them up into the coarse
forever-tables — otherwise the cleanup worker drops them (see the ⚠ box at the
top). The live rollup MVs won't do it: they only look at the last 2 h, so
historical rows need the **full-range pre-roll**, `schema/preroll.sql`.

What it does: re-aggregates `1m → 15m → 1h → 4h → 1d → 1w → 1M` over the **whole**
range, producing exactly one production-correct row per
`(bucket, asset, quote, source)`. It is **idempotent** — the coarse tables are
`ReplacingMergeTree(version)`, so a second run collapses duplicates. Six
`INSERT … SELECT` statements, run in order (`1m`→`15m` first, each granularity
feeds the next).

> **⚠ Heavy aggregation on the SHARED `ch-prod-01`.** This scans the entire `1m`
> history in one GROUP BY. Before running: get **prices-api owner sign-off**, pick
> a **low-traffic window**, and pass the **spill-to-disk** settings below so it
> can't OOM the shared node (it does **not** restart the container — pure SQL —
> but it competes for RAM/CPU with the BE tenant). See
> [[flag-container-restarts]] / [[feedback-prepare-not-deploy]].

**Optional — clean rebuild.** For a from-scratch coarse build (recommended the
first time, since prod's coarse tables are currently near-empty), truncate them
first. Safe: the pre-roll rebuilds every row from `1m`.

```bash
# OPTIONAL clean-slate — only if you want a deterministic full rebuild:
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --multiquery --query='
     TRUNCATE TABLE prices.price_ohlcv_15m;
     TRUNCATE TABLE prices.price_ohlcv_1h;
     TRUNCATE TABLE prices.price_ohlcv_4h;
     TRUNCATE TABLE prices.price_ohlcv_1d;
     TRUNCATE TABLE prices.price_ohlcv_1w;
     TRUNCATE TABLE prices.price_ohlcv_1M;'"
```

**Apply the pre-roll** (pipe the repo's `preroll.sql` into the prod client; the
spill settings bound memory on the shared node):

```bash
cat packages/prices-clickhouse/schema/preroll.sql \
| ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
    "docker exec -i app-clickhouse-1 clickhouse-client --multiquery \
       --max_bytes_before_external_group_by=20000000000 \
       --max_memory_usage=40000000000"
```

**Verify** the coarse tables now hold the range (read-only):

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client --query=\"
     SELECT '1h' g, source, min(timestamp) oldest, max(timestamp) newest, count() rows
       FROM prices.price_ohlcv_1h FINAL GROUP BY source
     UNION ALL
     SELECT '1d', source, min(timestamp), max(timestamp), count()
       FROM prices.price_ohlcv_1d FINAL GROUP BY source
     ORDER BY g, source FORMAT PrettyCompact\""
```

Good = `1h`/`1d` span your backfilled range per source (e.g. sdex `2024-02 →`
whatever the newest `1m` is). Empty coarse tables after this = the pre-roll
didn't run — do not re-enable cleanup (§10).

---

## 10. Cleanup coordination — re-enable ONLY after pre-roll

The nightly cleanup rule (`prices-production-cleanup`, EventBridge
`cron(0 2 * * *)` → the `prices-production-cleanup` Lambda) drops `1m` partitions
older than 7 days. It is currently **disabled** to protect the un-rolled `1m`
history while this backfill runs. **Do not re-enable it until §9's pre-roll has
captured the history into the coarse tables** — otherwise the `1m` partitions
drop at the next 02:00 UTC and the history is lost again (the exact 0090 bug).

Confirm the current state first (read-only; profile `soroban-explorer`, region
`eu-central-1`):

```bash
aws events describe-rule --name prices-production-cleanup \
  --query State --output text            # expect: DISABLED
```

Re-enable **only after** §9 pre-roll is verified — this is a prod mutation, so
**owner sign-off** and the standard approval gate apply:

```bash
aws events enable-rule --name prices-production-cleanup
aws events describe-rule --name prices-production-cleanup \
  --query State --output text            # now: ENABLED
```

Order that must hold: **write `1m` → pre-roll (§9) → verify coarse → re-enable
cleanup (§10).** Never re-enable before the coarse tables are populated.

---

## 11. If you need to stop and re-run

**Stopping is safe.** You can `Ctrl-C` the run (or the machine can reboot, or the
network can drop) at any time without corrupting anything. Everything already
written stays valid: candles use a `ReplacingMergeTree`, so re-writing the same
ledger just replaces it — no duplicates, no double-counting. Completed ledgers
are recorded **per partition**, so the archive is never re-downloaded for work
already done.

**To resume,** just do Step 4 again — read `soroban_amm.current_ledger`, set
`--start = current_ledger + 1`, and launch the next chunk as usual. (You can also
simply re-run the _same_ `--start/--end` you were on; already-finished ledgers
are skipped automatically. Either works.)

**The one caveat — let each chunk finish.** The discovered pool registry is saved
to the database only at the _successful end_ of a chunk. If you kill a chunk
partway through, the AMM pools it discovered in that chunk aren't saved, and
because those ledgers are now marked done they won't be re-scanned on resume — so
a few pools created in that interrupted chunk can end up unregistered, and their
swaps get recorded in `prices.unresolved_pools` (visible, **not** silently lost).
In the early Soroban era this is a non-issue (no AMM activity yet). If it ever
happens in the AMM-active era and you want those pools back, the clean fix is to
run the pool-registry seeder once (`docs/runbooks/seed-pool-registry.md`) to
repopulate `prices.pool_registry`, then continue — no need to redo the whole
backfill. The simplest habit that avoids all of this: size chunks so each one
finishes in a sitting (5 partitions ≈ 3–4 h on a home line, far less on EC2) and
let it run to `=== sdex-backfill complete ===`.

---

## Quick reference

| Thing                        | Value                                                                                                                       |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Ledger bucket                | `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` — region **us-east-2**, anonymous (`--no-sign-request`)           |
| Writer secret                | `prices/production/clickhouse-mtls-prices-ingestion-production` (region `eu-central-1`, acct `750702271865`)                |
| CH domain                    | `ch.sorobanscan.rumblefish.dev`                                                                                             |
| Prod CH shell                | `ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161` → `docker exec -i app-clickhouse-1 clickhouse-client --query='…'` |
| Activation ledger            | `50457424` (split between `combined` and `sdex-only`)                                                                       |
| Combined floor (`--end` max) | `63352611` (live starts at `63352612`)                                                                                      |
| Chunk size                   | 5 × 64,000 = 320,000 ledgers (bump/shrink by whole partitions)                                                              |
| Measured pace                | ~184 KB/ledger, ~64 min/100k on a home line — download-bound                                                                |

See also: `docs/runbooks/running-ingestion-components.md` (full flag reference,
minute-seam rules) and `docs/runbooks/seed-pool-registry.md` (registry seeding).
