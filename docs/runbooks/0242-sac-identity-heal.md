# Runbook — healing SACs held as contract identities (task 0242)

A Stellar Asset Contract (SAC) is a facet of its classic asset, never an asset of
its own. Production holds 36 SACs as `Contract` identities in `prices.assets`,
each with its own `asset_id`. This runbook removes them without a SQL rekey:
0286 phase 3 rebuilds the history under the classic ids, and a small cleanup
deletes what is left.

**Applies to:** `prices.assets`, `prices.asset_symbol` and the seven
`prices.price_ohlcv_*` tiers on ch-prod-01; the Compute, EventBridge and
Observability stacks.

**Read first:** [`0286-reingest-history.md`](0286-reingest-history.md) (phase 3,
its stages and its orchestrator) and lore task 0242 (decisions D1–D9, PC3, PC6,
PC9).

---

## 0. What and why

Measured 2026-10-05 (task 0242 §"Re-measured on prod, 2026-10-05"):

| Group | Contract rows | What their candles are                                                                                      |
| ----- | ------------- | ----------------------------------------------------------------------------------------------------------- |
| A     | 13            | Split pairs: the classic row exists. 145 of 151 SAC-keyed `1m` rows are byte-identical to a classic row     |
| B     | 23            | SACs whose classic was never in `assets`. Their candles are the **only copy** of those trades               |
| —     | 6 latent      | Pool-leg SACs with no identity yet (SUSHI, HYPE, STELLA, TESTTTT, TEST77, TEST12). They mint on first trade |

Group A's 13: POINTS, USDM1, SODA, VCHF, USDP, VEUR, WHLAQUA, USD, USDT0, XCR,
ESP, EUR, FrogST.

The heal is the 0286 phase-3 re-ingest, not a SQL rekey (D3). A rekey would need
four kinds of row operation (delete the duplicates, move group B, merge three
mixed coarse buckets, re-enrich), plus a base/quote flip for Z/Q and XLD/SODA,
whose order changes when `Contract` becomes `Credit`. Phase 3 drops and rebuilds
every month anyway; with the classic identities in `assets` it writes them under
the classic ids by construction.

The code of this task (one PR) stops new mints: a SAC leg resolves to its classic
only on a sha256-verified proof, and a SAC BE flags `is_sac` that nothing proves
is skipped and counted, never minted (D1, D2). The code protects the future; this
runbook heals the past.

---

## 1. Order of events

| #   | Step                                                                                                                                                                                                                    | When                                                                                          | Section |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | ------- |
| 1   | Seed the 29 classic identities. Works with the `events-backfill` already on ch-prod-01 (D4)                                                                                                                             | After phase-3 stage B (202402–202404), before stage C                                         | §3      |
| 2   | Re-plan phase 3 to 202609, run stage C with `--to-month 202609` (D4; the plan ended at 202608)                                                                                                                          | After the seed                                                                                | §4a–§4d |
| 3   | Deploy the PR: Compute (live ledger processor + API), then EventBridge (probe). Not Observability (D8). Swap `events-backfill` on ch-prod-01 only between phase-3 stages, never while a month is at its `amm` step (D7) | After stage D (`finish`) and before §5 (D9: the API alias needs the classic history in place) | §4e     |
| 4   | Residual cleanup (D3)                                                                                                                                                                                                   | After stage D (`finish`) AND after the live slice is deployed with at least one cold start    | §5      |
| 5   | Deploy Observability with the `SacContractIdentities` alarm actions on (D6, D8)                                                                                                                                         | After §5's post-checks are green                                                              | §6      |

Step 4 waits for the live deploy because until then a warm container that predates
the seed, or a SAC the seed does not cover, can still mint a contract identity
and write candles under it.

---

## 2. Operator identity and tooling

- **CH shell:** `dev_shared` over mTLS, with the `chq` function, set up exactly
  as in [`0139-asset-id-migration.md` → Who runs it, and where](0139-asset-id-migration.md#who-runs-it-and-where).
  `echo "SELECT currentUser()" | chq` must print `dev_shared`.
- **Orchestrator shell:** `fishuser-hero`, the checkout `reingest_0286.py` runs
  from, as in 0286 §3.
- **AWS shell:** production credentials, a checkout of the merged PR.
- **API:** `API=https://prices-api.sorobanscan.rumblefish.dev/v1` and an operator
  `PRICES_API_KEY`.

```bash
mkdir -p ~/heal-0242
Q=~/Projects/stellar-prices-api/.planning/quick/261005-htu-0242-sac-minted-as-a-second-identity-res
XCR=XCR:GBLJBHWVORDFI4J7CLBDRPECMYT3XO5S6GERXGC74VXOJMZPLI6ZU3S7
XCR_SAC=CDJQXBQO5ICVQUPHZHW7SHOM56K2UNNPPAIXUUSA3XACEI6Q4JQLXNVI
XLM_SAC=CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA
```

`$Q` is in Adam's checkout, under the gitignored `.planning/`. The seed and its
checks stay there, outside the repo.

Every `chq` step stops at the first non-zero exit. Read the message before going on.

---

## 3. Seed (after stage B, before stage C)

`$Q/seed-0242.sql` inserts 29 classic identities: the 23 of group B plus the 6
latent SACs. Every `sac_address` in it was re-derived from code + issuer by
sha256 (`$Q/measurements/sac_check.py`: ok 29, bad 0), and none of the 29 was in
`assets` on 2026-10-05. Adam runs it.

**Why `sac_address` must be filled.** The ingest registry would resolve a seeded
classic's SAC even with the column empty, because `from_existing` derives it.
But `identity_by_contract`, the API alias (D5), the `events-backfill` preload
filter and the probe all read the stored column (0242 research, pitfall 8).

**Pre-check.**

- `reingest_0286.py status` shows 202404 done and no month in progress.
- ```bash
  chq < $Q/seed-0242-check.sql | tee ~/heal-0242/seed-check-before.tsv
  ```
  Query 1 prints `N N`, query 2 prints `N`. Record query 3 (`count() - uniqExact(asset_id)`)
  as `COLLISIONS_BEFORE`.
  - `N = 0` when the seed comes before the §4e deploy, the order of §1.
  - `N > 0` when the 0242 build went live first: the live processor proved some of
    the 29 in-band and interned their classics itself, with `sac_address`. Those
    rows belong to live; the run below inserts only the `29 - N` others and keeps
    only those for the seed rollback.
  - The two numbers of query 1 differ: one of the 29 is held without
    `sac_address`. Stop and bring it to Adam.

**Run.** Stage the 29 rows, keep the ones `assets` does not hold yet, then
insert exactly those:

```bash
echo "CREATE TABLE prices.bak_0242_seed_stage AS prices.assets" | chq
sed 's/^INSERT INTO prices\.assets /INSERT INTO prices.bak_0242_seed_stage /' $Q/seed-0242.sql | chq
chq <<'SQL'
CREATE TABLE prices.bak_0242_seed ENGINE = MergeTree ORDER BY asset_id AS
SELECT asset_id, asset_code, issuer_address, sac_address FROM prices.bak_0242_seed_stage
WHERE (asset_code, issuer_address) NOT IN (SELECT asset_code, issuer_address FROM prices.assets FINAL);
INSERT INTO prices.assets (asset_code, asset_type, issuer_address, contract_address, sac_address, is_active)
SELECT asset_code, asset_type, issuer_address, contract_address, sac_address, is_active
FROM prices.bak_0242_seed_stage
WHERE (asset_code, issuer_address) IN (SELECT asset_code, issuer_address FROM prices.bak_0242_seed);
SELECT count() FROM prices.bak_0242_seed_stage;
SELECT count() FROM prices.bak_0242_seed
SQL
```

The two counts print `29` and `29 - N`. `bak_0242_seed` holds the rows this seed
added and nothing live wrote; the seed rollback and §7 read it.

**Post-check.**

```bash
chq < $Q/seed-0242-check.sql | tee ~/heal-0242/seed-check-after.tsv
```

Query 1 prints `29 29` (29 rows, all with `sac_address`), whatever `N` was.
Query 2 prints `29` (each seeded SAC is a `credit` row of
`identity_by_contract`). Query 3 equals `COLLISIONS_BEFORE` (AC4).

**When it takes effect.** Phase 3 reads `assets` at the start of every
`events-backfill` run, so stage C sees the seed at once. The live processor sees
it at its next cold start (`resumed cursor from ClickHouse` in
`/aws/lambda/prices-production-ledger-processor` after the seed time). Until
then a warm container can still mint one of the 29.

**Seed rollback.** Only while no candle names a seeded id, i.e. before stage C
writes any:

```bash
for T in 1m 15m 1h 4h 1d 1w 1M; do
  echo "SELECT '$T', count() FROM prices.price_ohlcv_$T WHERE asset_id IN (SELECT asset_id FROM prices.bak_0242_seed) OR quote_asset_id IN (SELECT asset_id FROM prices.bak_0242_seed)" | chq
done                                                           # every line 0
echo "ALTER TABLE prices.assets DELETE WHERE (asset_code, issuer_address) IN (SELECT asset_code, issuer_address FROM prices.bak_0242_seed) SETTINGS mutations_sync = 2" | chq
echo "SELECT mutation_id, is_done, latest_fail_reason FROM system.mutations WHERE database = 'prices' AND table = 'assets' AND NOT is_done" | chq   # no row
chq < $Q/seed-0242-check.sql   # query 1 prints the `N N` of the pre-check again (`0 0` in the order of §1)
```

A mutation is fine on a ~211k-row table. `mutations_sync = 2` makes the
`ALTER` return only once the mutation is done on every replica, and fail if it
fails (`dev_shared` is not `readonly`, so SETTINGS are allowed). Do not re-seed
or start stage C before query 1 reads the pre-check's numbers again. Once stage C
has written candles under the seeded ids there is no seed rollback: those ids
are the right identities.

---

## 4. Stage C and the deploy

### 4a. Extend the plan to 202609

The phase-3 plan ends at 202608 (`plan` took the last complete month on the day
it ran). Stage C must reach 202609: it holds the only copies of the USDT0 and
SODA trades and part of 0139's hole (D4).

Re-planning is safe: `cmd_plan` rewrites only the state's month map
(`st.d["plan"]`), and the progress of every month lives under a separate key
(`st.d["months"]`), which `plan` does not touch (`tools/scripts/reingest_0286.py`
`cmd_plan`).

**Pre-check** (orchestrator shell, between stages, no `run` holding the lock):

```bash
cp -p ~/reingest-0286/state.json ~/reingest-0286/state.json.pre0242
python3 -c "import json,os; p=json.load(open(os.path.expanduser('~/reingest-0286/state.json')))['plan']; print(min(p), max(p), len(p))"
```

It prints `<FIRST> 202608 <N>`. Pass `<FIRST>` back explicitly, so the re-plan
cannot start from a different first month.

**Run.**

```bash
python3 tools/scripts/reingest_0286.py plan --from-month <FIRST> --to-month 202609
```

**Post-check.** The last line reads `<N+1> months, <FIRST>..202609, archive tip …`.
`reingest_0286.py status` still shows every finished month as done.

**Rollback.** `cp -p ~/reingest-0286/state.json.pre0242 ~/reingest-0286/state.json`.

### 4b. Run stage C

**Pre-check: the seed is in** (§3 post-check: query 1 prints `29 29`). It is
mandatory, not just ordered first. The `events-backfill` on ch-prod-01 until the
§4e swap resolves a SAC only when its classic is in `assets`, so stage C without
the seed rebuilds group B's history, BLTA/BLTB/BLTC/PPRIME/LumenJoule (U3, §8)
included, under the SAC ids again: the second identities this task removes.

The flags are stage B's (task 0286, "Stage B command"), plus `--ack-0285` (202607
on) and `--ack-0139-binaries` (0286 §10a). Use the restart wrapper of 0286 §3:

```bash
python3 tools/scripts/reingest_0286.py run --to-month 202609 <stage B's flags> --ack-0285 --ack-0139-binaries --yes
```

**Which `events-backfill` is on the host.** A one-ledger dry run tells (CH host,
as `default`):

```bash
read -rs CH_PW
CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill --start 51500460 --end 51500460 \
  --clickhouse-url http://localhost:8123 --dry-run | grep 'unproven sac swaps:'
```

A 0242 binary prints `unproven sac swaps:` followed by a count; a binary built
before 0242 prints nothing. Both are fine for stage C once the seed is in (PC6).
A month run on a pre-0242 binary carries the note `printed no unproven sac swaps:
line`. Once a month of the state has printed the line, a later month without it
is a STOP: the host binary went back to a pre-0242 build (review of 0242).

**Post-check: U3's candles are back under the classic ids.** Once 202609 is done
(CH shell):

```bash
chq <<'SQL' | tee ~/heal-0242/u3-post-stage-c.tsv
WITH ['CCUYL75XNUTAHFE3RZAN7NEFQWZ7OIRJZNN4KUXXPA34RAHVUGLTBGT5',   -- BLTA
      'CAANCS7TKC7IFWLRACKL4T4HMNMDUWSWNOL5EV244TVQCU3O7BM7N4VD',   -- BLTB
      'CB2J5FHCJJBQE5UPQ5FPZAWIEQD5KKP4GWLZZ3QT5EFVIT3FW5CI2OGK',   -- BLTC
      'CBVWPBYEDJ7GYIUHL2HITMEEWM75WAMFINIQCR4ZAFZ62ISDFBVERQCX',   -- LumenJoule
      'CBI2NXIZQ33L3K5RMQW53OGV52HDHZU2AUISCUFXDYTDR345VKPHAQEP'    -- PPRIME
     ] AS sacs,
     (SELECT groupArray(asset_id) FROM prices.assets FINAL
      WHERE contract_address = '' AND has(sacs, sac_address)) AS classic,
     (SELECT groupArray(asset_id) FROM prices.assets FINAL
      WHERE has(sacs, contract_address)) AS contract
SELECT toYYYYMM(timestamp) AS month, length(classic) AS classics,
       countIf(has(classic, asset_id) OR has(classic, quote_asset_id)) AS classic_rows,
       sumIf(trade_count, has(classic, asset_id) OR has(classic, quote_asset_id)) AS classic_trades,
       sumIf(pf_trade_count, has(classic, asset_id) OR has(classic, quote_asset_id)) AS classic_pf_trades,
       countIf(has(contract, asset_id) OR has(contract, quote_asset_id)) AS sac_id_rows
FROM prices.price_ohlcv_1h FINAL
WHERE source = 'soroswap' AND timestamp >= '2026-03-01 00:00:00' AND timestamp < '2026-10-01 00:00:00'
  AND (has(classic, asset_id) OR has(classic, quote_asset_id)
       OR has(contract, asset_id) OR has(contract, quote_asset_id))
GROUP BY month ORDER BY month
FORMAT TSVWithNames
SQL
```

`classics` = 5 and `sac_id_rows` = 0 on every line. `classic_trades` equals BE's
swap count on the four registry pools of these SACs (two BLT pools,
LumenJoule/USDC, PPRIME/USDC; measured 2026-10-05, `$Q/U3-root-cause.md`):

| month               | 202603 | 202604 | 202605 | 202606 | 202607 | 202608 | 202609 | total  |
| ------------------- | ------ | ------ | ------ | ------ | ------ | ------ | ------ | ------ |
| `classic_trades`    | 24     | 7,037  | 3,301  | 23,422 | 10,189 | 6,958  | 24,649 | 75,580 |
| `classic_pf_trades` | 24     | 0      | 0      | 6      | 0      | 0      | 0      | 30     |

The query counts candle rows, not assets, so a BLTB/BLTA trade, which names two
of the five, counts once. `classic_pf_trades` is LumenJoule's 24 and PPRIME's 6: every BLT fill is
dust (about 0.0001 units each), so BLTA, BLTB and BLTC come back with volume and
`trade_count` but `pf_trade_count = 0` and no price (ADR 0287 §1). That is the
rule working, not a gap. Any `sac_id_rows` above 0, a month short of the table,
or `classics` below 5: stop before §4e and bring the output to Adam.

### 4c. A STOP on `unproven sac swaps:`

A 0242 binary prints, after `negative apply order:`,

```
unproven sac swaps:        0
```

Anything but `0` means swaps were skipped because a leg is a contract BE flags
`is_sac` and no proof resolved it (D2). The orchestrator STOPs on it
(`amm_summary`), after either pass, and the STOP names the pass. It reads the
line after the `--dry-run` pass first, so a STOP there comes after `drop_1m` and
`sdex` and before anything AMM is written: the month's `1m` holds SDEX-only
candles, its coarse tiers are untouched, and the month stays at its `amm` step.
A STOP on the write pass is step 4's last case.

1. **Find the contracts.** In `~/reingest-0286/<M>/events-backfill.log`:
   - the WARN `swaps skipped on SACs with no proof (task 0242 D2); the month is NOT complete`,
     whose `sample` lists up to 20 contracts;
   - the start-of-run INFO `preloaded SAC proofs (task 0242)` (`sacs_verified`,
     `sacs_rejected`) and, when `sacs_rejected > 0`, the WARN
     `SAC proofs that did not verify`.
2. **Check each contract** (CH shell; `C1,C2` = the contracts, quoted):

   ```bash
   chq <<'SQL'
   SELECT contract_id, groupUniqArray(is_sac) AS is_sac_versions FROM default.soroban_contracts
   WHERE contract_id IN ('C1', 'C2') GROUP BY contract_id;
   SELECT sc.addr AS contract, x.signature, x.ledger_sequence,
          JSONExtractString(x.topics_xdr, JSONLength(x.topics_xdr), 'value') AS sep11
   FROM (SELECT contract_id, signature, ledger_sequence, topics_xdr FROM default.soroban_events
         WHERE contract_id IN (SELECT id FROM default.soroban_contracts WHERE contract_id IN ('C1', 'C2'))
           AND signature IN ('transfer', 'mint', 'burn', 'clawback')
         LIMIT 3 BY contract_id) AS x
   INNER JOIN (SELECT id, any(contract_id) AS addr FROM default.soroban_contracts
               WHERE contract_id IN ('C1', 'C2') GROUP BY id) AS sc ON sc.id = x.contract_id
   SQL
   ```

   Then derive the SAC of each `sep11` (`CODE:ISSUER`, or `native`) with
   `$Q/measurements/sac_check.py` and compare it with the contract.

3. **Fix, then `run` again.**
   - **The SEP-11 derives to the contract:** the SAC is real and its proof did not
     reach the run. Seed its classic as §3 seeded the 29: one row
     `(asset_code, 'classic', issuer, '', sac_address = the contract, 1)`, with
     the §3 checks narrowed to it. Then `run` with the same flags. It resumes at
     the `amm` step, and its `--dry-run` pass is the proof: the binary derives
     the SAC itself, so a wrong seed STOPs again with the same count and writes
     nothing. Then remove the wrong row (§3's seed rollback, narrowed to it)
     before anything else: the API alias would answer the contract as it.
   - **BE has corrected `is_sac` to false:** the next run treats it as a genuine
     token only once no version of its row says `true`. The code reads BE's
     `soroban_contracts` (a ReplacingMergeTree) without `FINAL`, on purpose, so an
     unmerged old version keeps the contract a candidate and its swaps skipped.
     `run` again only when `is_sac_versions` above is `[false]`. While it holds
     both values, BE's parts have not merged: wait for them (never `OPTIMIZE`
     BE's table), or take step 4.
4. **`rollback` the month instead** (`reingest_0286.py rollback <M>`, which
   restores `1m` and the four fine tiers from the month's snapshot and forgets
   the month, so the next `run` starts it at `gates`):
   - when no SAC event exists for the contract, or its SEP-11 does not derive to
     it. No data fix can prove it. Stop phase 3 there and bring it to Adam;
   - when the fix cannot land the same day: the month serves SDEX-only `1m`
     candles while it waits;
   - when the non-zero line came from the **write** pass (the log shows that
     attempt's dry-run summary reading `0`, then the write's non-zero): the
     month's AMM candles are in without those swaps.

### 4d. `--amm wait`

`amm-done` records only `--fallbacks`. It does not read `unproven sac swaps:`
(nor `swaps dropped (unresolved):`). On this path the operator is the gate:

1. Run the host command the orchestrator prints with `--dry-run` first, and read
   `unproven sac swaps:`. Non-zero: do not run the write, do not `amm-done`;
   follow §4c.
2. Run the write. Read the line again. Non-zero: do not `amm-done`; `rollback`
   the month (§4c step 4).
3. Only on `0` (or no such line, a pre-0242 binary): `amm-done <M> --fallbacks <negative apply order>`.

### 4e. Deploy the PR

**When: after stage D (`finish`), before §5 (D9).** The API alias answers a SAC
address as its classic. Until stage C has rebuilt a SAC's months under the
classic id, that classic holds only part of the history, while the `Contract`
row still holds the rest. Deployed earlier, `GET /v1/assets/{C…}/ohlcv` would
lose the pre-heal AMM series. Group B is the visible case: its classics were
seeded on 2026-10-06 and fill month by month as stage C runs. §5 still needs the
live slice deployed with one cold start, so the deploy goes between the two.

**Pre-check before the live deploy: `prices_writer` can read BE's SAC set.** The
live Lambda loads `default.soroban_contracts` at cold start as `prices_writer`.
When it cannot, it does not fail Init (PC3, reversed in review): it ingests with
no SAC candidates, so an unproven SAC mints a `Contract` identity, and every run
publishes `SacCandidatesUnavailable`. Deploy with the read working.

```bash
chq <<'SQL'
SELECT user_name, role_name, access_type, database, table FROM system.grants
WHERE (user_name = 'prices_writer'
       OR role_name IN (SELECT granted_role_name FROM system.role_grants WHERE user_name = 'prices_writer'))
  AND access_type IN ('SELECT', 'ALL')
  AND (database = 'default' OR database IS NULL)
  AND (table = 'soroban_contracts' OR table IS NULL)
SQL
```

At least one row (on 2026-10-05: `prices_writer SELECT default soroban_contracts`).
Then the read itself, as that identity (fishuser-hero, the orchestrator's writer
bundle):

```bash
curl -sS --fail-with-body --cert ~/prices-mtls/prices_writer.crt --key ~/prices-mtls/prices_writer.key \
  --cacert ~/prices-mtls/ca.crt https://ch.sorobanscan.rumblefish.dev/ \
  --data-binary "SELECT count() FROM (SELECT DISTINCT contract_id FROM default.soroban_contracts WHERE is_sac)"
```

About 4,042 (M13b). An error or `0`: **do not deploy Compute**; fix the grant
first. The deploy would not stop ingest, but it would run without candidates.

**Record what runs now** (AWS shell), for the rollback:

```bash
for f in ledger-processor api-handler rollup-freshness-probe; do
  aws lambda get-function-configuration --function-name "prices-production-$f" \
    --query '[FunctionName,CodeSha256,LastModified]' --output text
done | tee ~/heal-0242/deployed-before.tsv
```

**Deploy** (AWS shell, `cd infra`):

| Order | Target                               | Ships                                                                                    |
| ----- | ------------------------------------ | ---------------------------------------------------------------------------------------- |
| 1     | `make deploy-production-compute`     | the live ledger processor (proof pre-pass, skip, `SacUnprovenSkipped`) and the API alias |
| 2     | `make deploy-production-eventbridge` | `rollup-freshness-probe` with `SacContractIdentities`                                    |

**Observability waits for §6 (D8).** Deployed now, `sac-contract-identities`
would sit in ALARM, red on the dashboard's Row-0 alarm strip, through stage C,
stage D and §5. That strip is the SCF evidence of task 0294 (all alarms OK).
Its two alarms come in §6 instead. Every Observability deploy from a commit
that holds this PR creates them, so tell the team: no
`make deploy-production-observability` from `develop` until §6. A deploy that
cannot wait shows the red tile; its description says it is expected.

Until §6 no alarm watches `SacUnprovenSkipped` or `SacCandidatesUnavailable`.
Read both by hand after the cold start and at least daily until §6:

```bash
for M in SacUnprovenSkipped SacCandidatesUnavailable; do
  echo "== $M"
  aws cloudwatch get-metric-statistics --namespace Prices/Ingest --metric-name "$M" \
    --dimensions Name=Environment,Value=production \
    --start-time "$(date -u -d '-1 day' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)" \
    --period 3600 --statistics Sum
done
```

No datapoints is healthy: both are published only when something is wrong.
Any datapoint: §9.

**Post-checks.**

- The live Lambda cold-started on the new build: `/aws/lambda/prices-production-ledger-processor`
  shows `loaded the is_sac contract set` with `sac_contracts` ≈ 4,042, then
  `resumed cursor from ClickHouse`. No Init error, and no WARN
  `is_sac contract set unreadable`.
- The probe: `aws lambda invoke --function-name prices-production-rollup-freshness-probe ~/heal-0242/probe.json`,
  then `jq '.asset_id_uniqueness | {sac_contract_rows, sac_contract_scanned, be_sac_contracts}' ~/heal-0242/probe.json`.
  `sac_contract_rows` = 36 (the baseline until §5), `be_sac_contracts` ≈ 4,042.
  If the file holds an `errorMessage`, read the metric instead:
  `aws cloudwatch get-metric-statistics --namespace Prices/Rollup --metric-name SacContractIdentities --dimensions Name=Environment,Value=production --start-time "$(date -u -d '-30 min' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)" --period 900 --statistics Maximum`.
- `curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XLM_SAC" | jq '{asset, code}'` → `"native"`, `"XLM"`.

**`events-backfill` on ch-prod-01.** Build it from the merge commit as in 0286
§10a, keep the current one as `~/events-backfill.pre0242` (never overwrite it),
and swap only between phase-3 stages, never while a month is at its `amm` step
(D7). Confirm with the one-ledger dry run of §4b.

**Rollback.** Redeploy the previous commit with the same two `make` targets (the
CodeSha256 in `deployed-before.tsv` is what must come back). On the host,
`cp -p ~/events-backfill.pre0242 ~/events-backfill`. Nothing in the PR writes a
schema change.

---

## 5. Residual cleanup (after stage D and the live cold start)

What phase 3 does not rewrite stays under the SAC ids: 202404 (stage B ran before
the seed, so Z/Q was re-minted under its SAC ids, U2), any month after 202609,
and the 1w/1M rows `finish` rolled from those. Then 72 metadata rows: 36 in
`assets`, 36 in `asset_symbol`.

**Precondition.** `reingest_0286.py finish` done (stage D); §4e post-checks green
(the live slice has cold-started).

### 5a. Capture the SAC ids

The probe's predicate, with each contract's classic beside it:

```bash
chq <<'SQL'
CREATE TABLE prices.bak_0242_sac_ids
ENGINE = MergeTree ORDER BY contract_address AS
SELECT c.contract_address AS contract_address,
       c.asset_id         AS asset_id,
       cl.asset_id        AS classic_id,
       cl.asset_code      AS classic_code,
       cl.issuer_address  AS classic_issuer,
       now()              AS captured_at
FROM (SELECT contract_address, asset_id FROM prices.assets FINAL
      WHERE contract_address != ''
        AND (contract_address IN (SELECT sac_address FROM prices.assets WHERE sac_address != '')
             OR contract_address IN (SELECT contract_id FROM default.soroban_contracts WHERE is_sac))) AS c
LEFT JOIN (SELECT sac_address, asset_id, asset_code, issuer_address FROM prices.assets FINAL
           WHERE sac_address != '') AS cl ON cl.sac_address = c.contract_address;
SELECT count(), countIf(classic_id = 0) FROM prices.bak_0242_sac_ids;
SELECT count() AS identities, uniqExact(asset_id) AS ids, identities - ids AS collisions FROM prices.assets FINAL
SQL
echo "SELECT * FROM prices.bak_0242_sac_ids ORDER BY classic_code FORMAT TSVWithNames" | chq > ~/heal-0242/sac_ids.tsv
```

**Check.** 36 rows, 0 without a classic (the seed gave group B theirs). Group A
is the 13 codes of §0. More than 36: the probe has seen new mints. They are in
the capture and go through the same gates; name them on the task. A row with
`classic_id = 0` has no classic to re-key to: every candle under it is an only
copy (5b). Record `identities`, `ids` and `collisions` as the AC4 "before".

### 5b. Count the residual, and gate it

**The window after the last phase-3 month.** Measured 2026-10-05: 0 rows since
2026-10-01 on any tier keyed on the 36 SAC ids. The window closes only once the
seed is in AND live has cold-started since, or the 0242 build is deployed; until
then live can still write under a SAC id. Re-count it now, for every month after
202609 (`1w`/`1M` roll from `1d`, so `1d` decides):

```bash
for T in 1m 15m 1h 4h 1d; do
  chq <<SQL
SELECT '$T' AS tier, toYYYYMM(timestamp) AS month, count() AS sac_rows, sum(trade_count) AS trades
FROM prices.price_ohlcv_$T FINAL
WHERE timestamp >= '2026-10-01 00:00:00'
  AND (asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids)
       OR quote_asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids))
GROUP BY month ORDER BY month
SQL
done | tee ~/heal-0242/window.tsv
```

- Empty → go on.
- Any row → it is one of the residual rows below, and the PC9 gate decides it
  like any other. A month after 202609 can only be re-run once it is complete
  (`plan` refuses the current month).

**Every residual row is an only copy until proven otherwise.** A residual row
exists only where phase 3 did not rebuild its bucket under the classic id after
the seed, so it may hold trades that no classic row holds. A trade count cannot
tell: a classic row with more trades can hold different ones (for example a SAC
hour written before the live cold start beside a classic hour written after it),
and by §5 the `1m`/`15m` retention has removed the finer rows that would show it.

The only proof that releases a row is a **byte-level duplicate**: a classic row
at the same pair after re-keying (both orientations, because Z/Q and XLD/SODA
flip), the same source and bucket, the same `trade_count` and the same volume on
each leg. That is the double count of group A (145 of 151 SAC-keyed `1m` rows on
2026-10-05). Every other row is an _only copy_, whatever the classic row holds.
Per tier and partition:

```bash
for T in 1m 15m 1h 4h 1d 1w 1M; do
  chq <<SQL
WITH (SELECT groupArray((asset_id, classic_id)) FROM prices.bak_0242_sac_ids) AS sc,
     arrayMap(t -> t.1, sc) AS s, arrayMap(t -> t.2, sc) AS c
SELECT '$T' AS tier, x.p AS partition, count() AS residual_rows,
       countIf(y.hit) AS byte_identical, residual_rows - byte_identical AS only_copy,
       sumIf(x.trade_count, NOT y.hit) AS only_copy_trades,
       groupUniqArrayIf(x.sac, NOT y.hit) AS only_copy_sacs
FROM (SELECT _partition_id AS p, if(has(s, asset_id), asset_id, quote_asset_id) AS sac,
             transform(asset_id, s, c, asset_id) AS a, transform(quote_asset_id, s, c, quote_asset_id) AS q,
             least(a, q) AS lo, greatest(a, q) AS hi, source, timestamp, trade_count,
             if(a = lo, volume_base, volume_quote) AS vol_lo, if(a = lo, volume_quote, volume_base) AS vol_hi
      FROM prices.price_ohlcv_$T FINAL
      WHERE has(s, asset_id) OR has(s, quote_asset_id)) AS x
ANY LEFT JOIN (SELECT least(asset_id, quote_asset_id) AS lo, greatest(asset_id, quote_asset_id) AS hi,
                      source, timestamp, trade_count,
                      if(asset_id = lo, volume_base, volume_quote) AS vol_lo,
                      if(asset_id = lo, volume_quote, volume_base) AS vol_hi, true AS hit
               FROM prices.price_ohlcv_$T FINAL
               WHERE has(c, asset_id) OR has(c, quote_asset_id)) AS y
  USING (lo, hi, source, timestamp, trade_count, vol_lo, vol_hi)
GROUP BY partition ORDER BY partition
SQL
done | tee ~/heal-0242/residual.tsv
```

`s` and `c` come from one read of pairs, so `s[i]` and `c[i]` are always the same
capture row. Two separate `groupArray` reads need not return the same order once
`bak_0242_sac_ids` has more than one part, and would re-key a SAC to the wrong
classic.

`vol_lo`/`vol_hi` are each leg's volume, whichever side it is stored on, so a
flipped pair compares leg by leg. A row with `classic_id = 0` re-keys to `0`,
matches nothing and counts as an only copy. On a scratch fixture a SAC hour of 1
trade beside a classic hour of 2 different trades reads `only_copy = 1`.

`quote_asset_id` is not a key prefix, so this scans each tier: run it off-peak.

**The PC9 gate. It STOPs §5 on any partition with `only_copy > 0`**, in any tier,
`1w` and `1M` included. A coarse bucket or a month phase 3 did not re-ingest after
the seed is no exception: only the byte-level match above releases a row. An
only-copy row is never deleted silently. For each such partition Adam picks one
of the two, and it is recorded on task 0242 before 5c:

- **Re-run the month through phase 3.** In a fresh state dir, as the 0139 second
  pass does (0286 §10d), so a finished month can run again:

  ```bash
  mkdir -p ~/reingest-0286-0242 && cp -r ~/reingest-0286/ledger-cache ~/reingest-0286-0242/
  python3 tools/scripts/reingest_0286.py plan --state-dir ~/reingest-0286-0242 --from-month <M> --to-month <M>
  python3 tools/scripts/reingest_0286.py run --state-dir ~/reingest-0286-0242 --from-month <M> --to-month <M> <stage C's flags> --yes
  ```

  That rewrites `1m` and the four fine tiers. The month's `1w`/`1M` rows are only
  rebuilt by `finish`, which TRUNCATEs both tables whole (hours, then the
  whole-history re-enrichment). Until it runs again they still read as only
  copies: run `finish` again, or record them as below. Then repeat 5b; the gate
  passes only when every partition reads `only_copy = 0` or carries a recorded
  loss.

- **Record the loss** on task 0242, with Adam's ack: tier, partition,
  `only_copy_trades`, `only_copy_sacs`. For `1w`/`1M` rows of a re-run month, say
  that the classic's `1w`/`1M` lack those trades until the next `finish`.

Either way the rows are in the 5c copy before 5d deletes them. **Expected here:**
202404's Z/Q rows, in every tier stage B wrote and in the `1w`/`1M` rows `finish`
rolled from them (U2).

### 5c. Rollback copies, before any delete

Every residual candle row and the 72 metadata rows. The column lists come from
`system.columns` without MATERIALIZED, ALIAS and EPHEMERAL columns:
`assets.asset_id` is MATERIALIZED (re-derived on re-insert), and the candles'
`base_*`/`quote_*` identity columns are EPHEMERAL (their ids are stored).

**Pre-check.** `echo "SHOW TABLES FROM prices LIKE 'bak_0242_%'" | chq` lists only
`bak_0242_sac_ids`, `bak_0242_seed` and `bak_0242_seed_stage`. `CREATE TABLE` below has no `IF NOT EXISTS`,
so a second run fails instead of overwriting the first copy.

```bash
chq <<'SQL' > ~/heal-0242/bak.sql
SELECT concat(
  'CREATE TABLE prices.bak_0242_', table, ' AS prices.', table, ';\n',
  'INSERT INTO prices.bak_0242_', table, ' (', cols, ') SELECT ', cols, ' FROM prices.', table, ' FINAL WHERE ',
  if(table IN ('assets', 'asset_symbol'),
     'contract_address IN (SELECT contract_address FROM prices.bak_0242_sac_ids)',
     'asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids) OR quote_asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids)'),
  ';')
FROM (SELECT table, arrayStringConcat(groupArray(name), ', ') AS cols
      FROM system.columns
      WHERE database = 'prices'
        AND table IN ('assets', 'asset_symbol', 'price_ohlcv_1m', 'price_ohlcv_15m', 'price_ohlcv_1h',
                      'price_ohlcv_4h', 'price_ohlcv_1d', 'price_ohlcv_1w', 'price_ohlcv_1M')
        AND default_kind NOT IN ('MATERIALIZED', 'ALIAS', 'EPHEMERAL')
      GROUP BY table)
ORDER BY table
FORMAT TSVRaw
SQL
cat ~/heal-0242/bak.sql && chq < ~/heal-0242/bak.sql
```

This creates `prices.bak_0242_assets`, `prices.bak_0242_asset_symbol` and
`prices.bak_0242_price_ohlcv_<tier>` for the seven tiers.

**Post-check.**

```bash
for t in assets asset_symbol price_ohlcv_1m price_ohlcv_15m price_ohlcv_1h price_ohlcv_4h \
         price_ohlcv_1d price_ohlcv_1w price_ohlcv_1M; do
  echo "SELECT '$t', count() FROM prices.bak_0242_$t" | chq
done | tee ~/heal-0242/bak-counts.tsv
```

`assets` and `asset_symbol` equal the 5a count (36). Each tier equals the sum of
its `residual_rows` in `residual.tsv`.

### 5d. Delete the residual candles, fine tiers first

One lightweight DELETE per tier and partition, over the partitions 5c copied.
Fine before coarse, so a re-roll of a coarse tier between two statements reads an
already cleaned finer tier.

```bash
for T in 1m 15m 1h 4h 1d 1w 1M; do
  for P in $(echo "SELECT DISTINCT _partition_id FROM prices.bak_0242_price_ohlcv_$T ORDER BY 1 FORMAT TSVRaw" | chq); do
    echo "DELETE FROM prices.price_ohlcv_$T IN PARTITION ID '$P' WHERE asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids) OR quote_asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids);"
  done
done | tee ~/heal-0242/delete-candles.sql
cat ~/heal-0242/delete-candles.sql && chq < ~/heal-0242/delete-candles.sql
```

**Post-check (M4).** Every line `0`:

```bash
for T in 1m 15m 1h 4h 1d 1w 1M; do
  echo "SELECT '$T', count() FROM prices.price_ohlcv_$T FINAL WHERE asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids) OR quote_asset_id IN (SELECT asset_id FROM prices.bak_0242_sac_ids)" | chq
done
```

### 5e. Delete the 36 `assets` rows

**Pre-check.** `echo "SELECT count() FROM prices.assets FINAL WHERE contract_address IN (SELECT contract_address FROM prices.bak_0242_sac_ids)" | chq`
equals the 5a count.

```bash
echo "DELETE FROM prices.assets WHERE contract_address IN (SELECT contract_address FROM prices.bak_0242_sac_ids)" | chq
```

**Post-check.** The same count reads `0`.

### 5f. Delete the 36 `asset_symbol` rows

After `assets`, because the symbol queue is driven by the `assets` contract rows
(`asset-discovery` `symbols.rs`): deleted first, they would be fetched again.

**Pre-check.** `echo "SELECT count() FROM prices.asset_symbol FINAL WHERE contract_address IN (SELECT contract_address FROM prices.bak_0242_sac_ids)" | chq`
equals the 5a count.

```bash
echo "DELETE FROM prices.asset_symbol WHERE contract_address IN (SELECT contract_address FROM prices.bak_0242_sac_ids)" | chq
```

**Post-check.** The same count reads `0`.

### 5g. Post-checks

```bash
chq <<'SQL'
SELECT count() FROM prices.assets FINAL
WHERE contract_address != ''
  AND (contract_address IN (SELECT sac_address FROM prices.assets WHERE sac_address != '')
       OR contract_address IN (SELECT contract_id FROM default.soroban_contracts WHERE is_sac));
SELECT count() FROM prices.assets FINAL WHERE contract_address != '';
SELECT uniqExact(contract), countIf(asset_kind = 'contract'), max(n)
FROM (SELECT contract, any(asset_kind) AS asset_kind, count() AS n FROM prices.identity_by_contract
      WHERE contract IN (SELECT contract_address FROM prices.bak_0242_sac_ids) GROUP BY contract);
SELECT count() AS identities, uniqExact(asset_id) AS ids, identities - ids AS collisions FROM prices.assets FINAL
SQL
```

| Check                               | Expected                                                                       |
| ----------------------------------- | ------------------------------------------------------------------------------ |
| SAC contract rows (M1)              | `0`                                                                            |
| Contract rows left                  | the genuine tokens only: 23 on 2026-10-05, plus any minted since               |
| `identity_by_contract`              | the 5a count, `0` contract rows, `max(n)` = `1`: one classic row per SAC       |
| identities / ids / collisions (AC4) | identities and ids each down by exactly the 5a count; collisions equal to 5a's |
| SAC-keyed candles (M4)              | `0` in every tier (5d post-check)                                              |

The probe and the alarm metric:

```bash
aws lambda invoke --function-name prices-production-rollup-freshness-probe ~/heal-0242/probe.json
jq '.asset_id_uniqueness | {sac_contract_rows, collisions: (.identities - .ids)}' ~/heal-0242/probe.json
```

`sac_contract_rows` = `0`, and `SacContractIdentities` reads `0` in CloudWatch
(§4e's `get-metric-statistics`). `AssetIdCollisions` unchanged (AC4).

The API (D5, AC2, AC3):

```bash
curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XCR_SAC" | jq '{asset, asset_kind, contract}'
curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XCR_SAC/price" | jq -S '{asset, price_usd, volume_24h_usd}'
curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XCR/price" | jq -S '{asset, price_usd, volume_24h_usd}'
W="granularity=1d&start=2026-09-01T00:00:00Z&end=2026-10-01T00:00:00Z"
diff <(curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XCR_SAC/ohlcv?$W" | jq -S .data) \
     <(curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XCR/ohlcv?$W" | jq -S .data) && echo SAME
curl -sS -H "x-api-key: $PRICES_API_KEY" "$API/assets/$XLM_SAC" | jq '{asset, asset_kind, code}'
```

- XCR by its SAC: `asset` = `XCR:GBLJ…`, `asset_kind` = `credit`, `contract` = `""`.
- Both `/price` calls print the same object: one asset, one volume. `SAME`.
- The XLM SAC answers 200 as XLM: `asset` = `native`, `code` = `XLM`. A 404
  means prod's XLM row lost its `sac_address`: read it first
  (`SELECT sac_address FROM prices.assets FINAL WHERE asset_code = 'XLM' AND issuer_address = ''`).
  On 2026-10-05 it held `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA`,
  and no classic row lacked one.

### 5h. Rollback

Re-insert from the copies with the same column lists, metadata first:

```bash
chq <<'SQL' > ~/heal-0242/restore.sql
SELECT concat('INSERT INTO prices.', table, ' (', cols, ') SELECT ', cols, ' FROM prices.bak_0242_', table, ';')
FROM (SELECT table, arrayStringConcat(groupArray(name), ', ') AS cols
      FROM system.columns
      WHERE database = 'prices'
        AND table IN ('assets', 'asset_symbol', 'price_ohlcv_1m', 'price_ohlcv_15m', 'price_ohlcv_1h',
                      'price_ohlcv_4h', 'price_ohlcv_1d', 'price_ohlcv_1w', 'price_ohlcv_1M')
        AND default_kind NOT IN ('MATERIALIZED', 'ALIAS', 'EPHEMERAL')
      GROUP BY table)
ORDER BY table IN ('assets', 'asset_symbol') DESC, table
FORMAT TSVRaw
SQL
cat ~/heal-0242/restore.sql && chq < ~/heal-0242/restore.sql
```

Check: §5a's capture predicate counts the 5a count again, and each tier's
SAC-keyed count equals `bak-counts.tsv`. `asset_id` re-derives to the same value
from the same identity. Keep the `bak_0242_*` tables until 0242 is closed; then
`DROP TABLE prices.bak_0242_<name> SYNC`, one by one.

---

## 6. Deploy Observability with the alarm on (D6, D8)

Observability was held back in §4e (D8), so this deploy creates the three 0242
alarms: `prices-production-ledger-processor-sac-unproven`,
`prices-production-ledger-processor-sac-candidates-unavailable` and
`prices-production-sac-contract-identities`, the last with its actions on.

**Pre-check.** §5g is green and `SacContractIdentities` reads `0` for at least
two periods (30 min):

```bash
aws cloudwatch get-metric-statistics --namespace Prices/Rollup --metric-name SacContractIdentities \
  --dimensions Name=Environment,Value=production \
  --start-time "$(date -u -d '-45 min' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)" \
  --period 900 --statistics Maximum
```

Every `Maximum` is `0`. Created with its actions on while the metric read 36,
the alarm would fire at once.

**Run.** Set `opsAlarms.sacContractIdentitiesActionsEnabled` to `true` in
`infra/envs/production.json` through a PR; once merged:

```bash
cd infra && make deploy-production-observability
```

**Post-check.**

```bash
aws cloudwatch describe-alarms \
  --alarm-names prices-production-sac-contract-identities prices-production-ledger-processor-sac-unproven \
    prices-production-ledger-processor-sac-candidates-unavailable \
  --query 'MetricAlarms[].[AlarmName,ActionsEnabled,StateValue]' --output text
```

All three print `True`, then `OK` (`INSUFFICIENT_DATA` until the first
evaluation). The `INSUFFICIENT_DATA` → `OK` transitions send OK notifications:
expected.

**Rollback.** The same key back to `false` and the same deploy turns the
actions off. Redeploying Observability from a commit without this PR removes
all three alarms.

---

## 7. Acceptance evidence

| AC                                                       | Evidence                                                                                                                                                                                                                                                   |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AC1 a SAC no longer mints a second identity, both orders | `packages/prices-ledger-processor/tests/sac_identity.rs` (T2 SAC first, T3 both orders, T4 the SAC event after the trade), through the real `process_ledger`; `events-backfill` `a_swap_on_an_unproven_sac_is_skipped_and_counted_and_a_proven_one_prices` |
| AC2 the existing pairs are resolved                      | §5g: 0 SAC contract rows, one `identity_by_contract` row per SAC, 0 SAC-keyed candles. Group A by phase 3 plus 5d; group B by the seed plus phase 3                                                                                                        |
| AC3 one asset, one volume (XCR)                          | §5g: XCR by SAC and by `CODE:ISSUER` print the same price, volume and candles. For BLTA/BLTB/BLTC/PPRIME/LumenJoule, §4b's U3 post-check: their volume under the classic ids; BLTA/BLTB/BLTC carry no price by design (dust)                               |
| AC4 no new duplicate `asset_id`                          | §3 query 3 and §5a/§5g: `count() - uniqExact(asset_id)` unchanged at every step; `AssetIdCollisions` unchanged                                                                                                                                             |

Paste the outputs of §3, §5a, §5b, §5g and §6 into task 0242, then tell BE once
the heal lands: they read `current_price_usd` and `identity_by_contract`, and
each SAC now resolves to one classic row.

---

## 8. Open items

- **U3, settled 2026-10-05** (`$Q/U3-root-cause.md`). The 75,580 Soroswap swaps
  on BLTA, BLTB, BLTC, PPRIME and LumenJoule had candles until 2026-10-02. The
  task-0139 re-key left them in `price_ohlcv_*__pre0139`: each of the five SAC
  contract identities shared its old id with an unrelated classic asset, so all
  five were `colliding`, and `fill` copies only `mapped`/`sentinel` ids. It is a
  sub-case of 0139's hole (colliding ids have no candles 2024-02..2026-09), not
  an ingest or dust-rule bug, and needs no code. Stage C after the seed rebuilds
  them from BE's events under the classic ids; §4b's post-check proves it, and
  BLTA/BLTB/BLTC come back with volume and no price (dust). Without the seed
  stage C would rebuild them under the SAC ids, which is why §4b requires it.
- **U5.** USDT0's 17:26 SAC trade came after its classic's 17:00-hour SDEX
  candle: consistent with the warm-container race of 0139, not traced to a
  container.
- **Known residual risk.** The live processor reads BE's `is_sac` set once per
  cold start. A SAC deployed after it, traded with no SAC event in the same
  transaction, is not a candidate yet and still mints a `Contract` identity; so
  does a SAC BE does not flag, and so does every unproven SAC of a container
  whose cold start could not read the set (`SacCandidatesUnavailable`). The
  in-band proof missed 0 of 338,601 sampled legs, so this should stay rare.
  `SacContractIdentities` counts it, §9.

---

## 9. When an alarm fires

**`prices-production-ledger-processor-sac-unproven`** (live, actions on). The
live processor skipped trades on a contract BE flags `is_sac` that no SAC event
in the same transaction proved. Those trades have no candle. The WARN
`skipped trades on is_sac contracts with no SAC proof (task 0242)` in
`/aws/lambda/prices-production-ledger-processor` names the contracts. Check each
as in §4c step 2. A real SAC: seed its classic as §3 did; the next cold start
resolves it. The skipped minutes stay without those trades until a re-ingest of
their month.

**`prices-production-ledger-processor-sac-candidates-unavailable`** (live,
after §6). A container cold-started without BE's `is_sac` set. The WARN
`is_sac contract set unreadable` carries the error: usually the grant of §4e's
pre-check, a renamed table or a BE migration. Ingest is running. Fix the read,
then force a cold start (any configuration change of
`prices-production-ledger-processor`, or wait for the container to recycle).
Any SAC minted meanwhile shows in `SacContractIdentities`: treat it as below.

**`prices-production-sac-contract-identities`** (after §6). A contract row of
`assets` is a SAC again. The alarm's description holds the query that names it.
Treat it as a one-row §5: capture, gate its candles, copy, delete. Find which
path minted it first (a pre-0242 binary somewhere, or a SAC BE does not flag).
