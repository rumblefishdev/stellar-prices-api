---
id: "0210"
title: "Soroban assets carry an empty asset_code in listing and detail — top-volume rows are unidentifiable to consumers"
type: BUG
status: active
related_adr: []
related_tasks: ["0120", "0139", "0242", "0252", "0256", "0257", "0258", "0259"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M2, api, metadata, defect]
milestone: 2
links: []
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: "Spawned from the 0120 conformance run."
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      Promoted, and the implementation section rewritten: the original plan
      (resolve `symbol()` into `assets.asset_code`) would have caused a
      production defect, not fixed one. `asset_code` is part of the sort key
      of a `ReplacingMergeTree`, so amending it on an existing row writes a
      SECOND row rather than replacing — the same `asset_id` on two natural
      identities, which is exactly the live fan-out [[0139]] measured on
      3,275 ids. `GET /assets` would then return the token twice, once
      nameless and once named. Independently, `views.sql:117-121` forces
      `asset_code = ''` for contract-kind rows *deliberately and in
      anticipation of this task* ("the view does not depend on the writer
      keeping it blank"), so the symbol would not surface there anyway.
      Re-scoped onto `asset_metadata`. Effort revised from small to medium.
  - date: 2026-08-28
    status: active
    who: stkrolikiewicz
    note: >
      Measured the population, and corrected the record. **This branch already
      held the right answer and never merged**, so `develop`'s copy of the task
      still showed the original `assets.asset_code` plan — and a fresh pass on
      2026-08-28 re-derived the same re-scope from scratch and briefly committed
      the WRONG destination to `develop` (ca62fea) before finding this branch.
      Nothing here changes: the 2026-08-20 analysis stands, verified again
      against `init.sql` (`assets` really is `ORDER BY (asset_code,
      issuer_address, contract_address)`) and `views.sql:117-121` (contract rows
      really are normalised to `asset_code = ''`).
      New this pass — the measured population, which nobody had counted:
      **52** soroban rows in `prices.assets`, all 52 with an empty code, all
      active; **20** ever produced a candle; **4** are visible through
      `GET /v1/assets` (the listing INNER JOINs `current_prices`, so an
      API-side count answers "how many consumers see this broken today", not
      "how many rows need fixing"). Registry total 207,493. So the backfill is
      ~52 RPC calls, not a paginated job — which is why the worker can copy
      supply-worker's *shape* without needing its scale machinery.
  - date: 2026-09-01
    status: active
    who: stkrolikiewicz
    note: >
      Implemented. Destination changed **again**, and this time away from what
      this file recommended: not `asset_metadata` but a new single-writer
      `prices.asset_symbol` keyed on `contract_address`. `asset_metadata`'s
      writer replaces the whole row, so a symbol writer would clobber
      `home_domain` — the task-0067 hazard, planned in rather than live — and
      `asset_id` is unattributable for 10 of the 52 rows (0139). The symbol
      surfaces AS `asset_code`, composed at read time, because the RFP names
      Asset/Token Code as required metadata and §4.1 has no other field;
      `views.sql` governs the BE view surface, which the REST handlers never
      read. Resolution triggers on absence, not staleness, which removed the
      stalest-first ordering, the time budget and the env config the plan had
      copied from supply-worker. Measured: **all 52 prod contracts resolve**
      over RPC in 6.6 s, 0 sentinels — the earlier "~35 of 52 will not resolve"
      came from BE's table coverage (17/52) and measured their table, not the
      chain. 98 gated tests pass (7 new), Lambda builds for arm64. Search and
      sort deliberately stay on the stored column: five registry contracts
      already self-declare USDC, USDT (×2), BTC and XRP without being SACs of
      those assets, all zero-volume and so not API-visible; identity
      verification spun out as [[0252]]. Not yet on prod.
  - date: 2026-09-02
    status: active
    who: stkrolikiewicz
    note: >
      Deployed to production and verified end to end. DDL applied by hand on the
      shared box (BE told first), grants checked, the 52-contract sweep re-run
      after the sanitiser change (52/52, 0 rejections), asset-discovery deployed
      and run three times to full coverage, then prices-api. `GET
      /v1/assets/CBIJ…` returns `code: "SolvBTC"`; all five API-visible Soroban
      assets are named where every one read `""` before. Coverage query 0,
      0 sentinels, 0 non-zero attempts — the retry machinery never fired, which
      is the expected outcome for 52 live tokens. 0120's fixture updated and its
      assertion green. Review of PR #275 produced five code fixes before this:
      a JSON-RPC error at HTTP 200 was sentinelling every contract in a run,
      archived contract state was treated as permanent, `is_control` missed
      bidi/format characters, `http_client` silently dropped its timeout, and
      a rejected symbol was sentinelled with no log at all. Four findings
      spawned as [[0256]]-[[0259]]. Not archived: PR #275 is still open, so the
      implementation is not on develop.
---

# Soroban assets have empty asset_code

## Summary

Every soroban-native row in `GET /v1/assets` (including `CBIJ…`, ranked #6 by
24h volume at ~$29.5k) has `asset_code: ""` in both the listing and
`GET /v1/assets/{id}` (`code: ""`). Consumers see a top-10 asset with no
name — only a 56-char contract address.

## Context

Found by [[0120]]. Soroban token metadata (symbol/name) lives on the token
contract; the ingest pipeline evidently never resolves it into
`prices.assets`.

## Population (measured on prod, 2026-08-28)

| | count |
|---|---|
| soroban rows in `prices.assets` | **52** |
| …with an empty `asset_code` | **52** (all of them) |
| …that ever produced a candle | 20 |
| …visible in `GET /v1/assets` today | 4 |
| registry total, for scale | 207,493 |

The three numbers differ for a reason worth knowing before anyone re-counts:
the listing does `FROM current_prices INNER JOIN assets`, so it shows only
assets that traded in the last 24 h. A count taken off the API answers "how
many consumers can see broken today", not "how many rows need fixing".

So the backfill is **~52 RPC calls**, not a paginated job with a time budget.

## Where the symbol must NOT go

`prices.assets.asset_code`. Two independent reasons, both load-bearing:

1. **It is in the sort key.** `ORDER BY (asset_code, issuer_address,
   contract_address)` on a `ReplacingMergeTree(updated_at)` — dedup happens
   *within* a sort key, never across it. Writing `('SHX','','C…')` where
   `('','','C…')` exists leaves BOTH rows, one `asset_id` on two identities.
   Every `ON a.asset_id = c.asset_id` join then fans out: `GET /assets`
   returns the token twice, once nameless and once named. That is [[0139]],
   already live on 3,275 ids — this would deliberately add to it.
2. **The read surface forces it blank anyway.** `views.sql:117-121` normalises
   contract-kind rows to `asset_code = ''` at read time and says why: *"so the
   (contract ⇒ asset_code='') interop contract holds even if
   discovery/metadata ever populates a symbol into a Soroban token's
   asset_code — the view does not depend on the writer keeping it blank."*
   Someone closed this door on purpose.

## Implementation

**Built on 2026-09-01.** Destination is a new single-writer table
**`prices.asset_symbol`**, keyed on `contract_address`.

The earlier plan in this section — a `symbol` column on `prices.asset_metadata`,
surfaced as its own API field — was superseded during implementation. Both halves
were wrong, each for its own reason:

- **`asset_metadata` would have re-created the hazard 0067 fixed.**
  `write_asset_metadata` (`writer.rs:295-310`) writes the **whole row**, and the
  table is `ReplacingMergeTree(updated_at) ORDER BY (asset_id)` where the newest
  row wins wholesale. A symbol writer would blank `home_domain` and vice versa —
  precisely the two-writer clobber that moved `home_domain` out of `assets` in
  the first place, and the reason `asset_supply` has its own table
  (`init.sql:173`: *"single-writer table so supply … and price … never fight"*).
  `home_domain` has no production writer **today**, so this was a trap being
  planned in rather than a live bug.
- **`asset_id` is the wrong key.** Measured: **10 of the 52** Soroban rows share
  an `asset_id` with another row ([[0139]]), so an `asset_id`-keyed symbol is
  unattributable for 19% of the population. The symbol belongs to the *contract*;
  `contract_address` is a Soroban token's natural key, and keying on it dissolves
  the 0139 exposure instead of inheriting it.
- **The symbol surfaces AS `asset_code`, composed at read time.** The RFP names
  *Asset/Token Code (string)* as required asset metadata, and §4.1's response
  shape has no other field for it — so a new field would have left the stated
  requirement unmet while adding a shape change. This is safe because the REST
  handlers read tables directly and never touch `views.sql`'s views: that file's
  `contract ⇒ asset_code = ''` normalisation governs the **BE view surface**, a
  different consumer, and is untouched.

⚠️ **Published-value change.** Soroban `asset_code` / `code` goes from `""` to a
symbol. A consumer detecting Soroban assets by an empty code breaks; it must use
`asset_type`. Called out in the PR as a `BREAKING CHANGE` trailer, the way 0118
did.

### What was built

- **Schema** — `prices.asset_symbol (contract_address, symbol, attempts,
  fetched_at)`,
  `ReplacingMergeTree(fetched_at) ORDER BY (contract_address)`. Statement-count
  guard in `prices-clickhouse/src/lib.rs` bumped 31 → 33 (this table plus
  [[0178]]'s `current_prices.method` ALTER, which landed on develop in between).
  Documented as §3.10a of the database-schema overview.
- **Resolver** — `packages/asset-discovery/src/symbols.rs`. The envelope builder
  and JSON-RPC structs are a **local copy** of oracle-worker's, not an extraction:
  moving them into `prices-ingest-core` would have added an optional `reqwest` to
  a crate seven packages depend on, and risked a regression in working oracle
  code, to avoid duplicating ~30 lines of stable boilerplate. Wrong trade.
- **Trigger is ABSENCE, not staleness.** A contract with any `asset_symbol` row is
  never re-fetched. `symbol()` is fixed at deploy for every real token
  implementation, so a freshness threshold would buy re-verification nobody asked
  for and pay permanent RPC load and a config surface for it. This removed the
  stalest-first ordering, the time budget and the env-var config the plan carried
  from `supply-worker` — that worker's queue is the whole 207k registry and never
  empties; this one is 52 rows and then nothing. Steady state is **zero work**.
- **The three-way outcome policy is kept**, because it is what stops starvation:
  resolved → write; *absent* (simulated fine, no usable string) → write an
  empty-symbol **sentinel** so the contract leaves the queue permanently;
  *transient* (HTTP/timeout/non-2xx) → write nothing, retry next run. The split is
  load-bearing — an RPC outage must not sentinel all 52 and name them `""`
  forever. `error_for_status()` is what routes a 429/5xx to the transient arm.
- **Validation** — `sanitize_symbol`: trim, non-empty, ≤32 characters (counted in
  chars, not bytes), no control characters. The cap matters for the
  `ScVal::String` arm, which XDR leaves unbounded; `ScVal::Symbol` is already
  bounded at 32. A rejected symbol is treated as absent, so it sentinels rather
  than retrying forever.
- **API** — `if(a.asset_code != '', a.asset_code, sym.symbol) AS asset_code` plus
  a `LEFT JOIN … FINAL` in the listing and detail queries. Row structs, DTOs,
  handlers and the OpenAPI shape are unchanged.
- **Bounded, and ahead of the ledger scan.** `MAX_CONTRACTS_PER_RUN` (25) ×
  `RPC_TIMEOUT_SECS` (5) = 125 s worst case, inside the Lambda's 5 min, before
  the unbounded S3 scan. Per [[0218]], both stages run and both report — a symbol
  failure is logged and returned in the JSON, never behind the scan's `?`.

### Measured on 2026-09-01

**All 52 prod contracts resolve** against mainnet RPC — 0 sentinels, 0 transient
failures, 6.6 s for the whole set. The earlier "~35 of 52 will not resolve" figure
came from BE's `soroban_contract_metadata` coverage (17/52) and measured **their
table**, not the chain. `CBIJ…` returns `SolvBTC`, matching BE's independent
reading, which is why that value doubles as an oracle for the prod assertion.

### Deliberately out of scope: search and sort

`?search=` is `startsWith(a.asset_code, ?)` (`queries_ch.rs:164`) and `sort=code`
orders on `a.asset_code` (`:86`) — both left on the **stored** column. A Soroban
token is therefore displayed by its symbol but is not findable or orderable by it.

That inconsistency is the cheaper half of a trade. `symbol()` is a string the
contract itself controls, so making it searchable is exactly what would let a
hostile token surface under a well-known code. Not hypothetical: **five contracts
already in our registry self-declare `USDC`, `USDT` (twice), `BTC` and `XRP`
without being SACs of those assets.** All five are currently zero-volume, so none
is API-visible (the listing `INNER JOIN`s `current_prices`) — this task does not
publish them. Pinned by a test so a later change to the boundary is a decision
rather than an accident. Verification of asset identity is [[0252]].

### Two alternatives measured on 2026-08-28, both declined

**BE's `default.soroban_contract_metadata`, on the same box.** It carries
`contract_id`, `name`, `symbol`, `decimals` over 3,922 rows, and an in-cluster
join is an established pattern in the other direction (ADR 0007 kept the shared
box partly because a dedicated one "breaks BE's in-cluster `price_usd_series`
JOIN"). Coverage of our 52 is **17** — but the right 17: `CBIJ…` resolves to
`SolvBTC`, plus `XAUM` and `xSolvBTC`, i.e. three of the four assets a consumer
can see today, and it yields `decimals` too, which `symbol()` alone does not.
**Declined as the mechanism** because 17/52 is not coverage, and an acceptance
criterion should not depend on another team's table before they have agreed to
it being a dependency. Worth raising with BE separately — as a supplement that
would cut RPC calls, not as the path. *Post-implementation note: direct RPC
covers 52/52, so the supplement would buy `decimals`, not coverage.*

**The SAC subset.** Measured: **11** of the 52 are SACs of classic assets already
in the registry, resolvable with no RPC at all. But none of the 11 is visible
through the API, so it closes no part of this task's acceptance criteria — and the
split it exposes turned out to be an identity defect rather than a naming one, now
[[0242]]. Not sequenced first, contrary to what this section said before the
measurement, and now moot for coverage: RPC resolves all 52 anyway.

- 0120 pins `code: ""` for `CBIJ…` in `conformance-assets.json:36-41`, asserted
  with `===` at `conformance-0120.mjs:308-313`. Update it to `SolvBTC` **after**
  the worker has resolved it on prod, or the suite fails in the window between
  fixture change and resolution.

⚠️ [[0139]] is unfixed. Keying on `contract_address` rather than `asset_id` means
this table does not inherit the ambiguity — but the `assets` side of the join
still carries it, so do not treat the join as sound.

### Deploy runbook

**Order is load-bearing.** Both `list_assets` and `asset_detail` join
`asset_symbol` unconditionally, and nothing applies schema automatically — no
CI step, no CDK step; `grep -r 'prices-clickhouse-init\|init.sql' .github/
Makefile infra/src` returns nothing. Prod DDL is hand-applied. Deploying
`prices-api` before the table exists fails `GET /v1/assets` and
`GET /v1/assets/{id}` with `UNKNOWN_TABLE` — a 500 on the two highest-traffic
routes. Note the PR gate is *merge*, which is not the deploy gate.

1. **Tell BE.** Shared box (ADR 0007). Pure `CREATE TABLE`, no restart, nothing
   they read is touched — but nobody changes schema there silently.
2. **Apply the DDL**, then confirm the table is there before anything ships:
   ```sql
   CREATE TABLE IF NOT EXISTS prices.asset_symbol (
       contract_address  String,
       symbol            String    DEFAULT '',
       attempts          UInt8     DEFAULT 0,
       fetched_at        DateTime  DEFAULT now()
   ) ENGINE = ReplacingMergeTree(fetched_at) ORDER BY (contract_address);
   ```
   ⚠️ If an earlier version of this DDL was already applied, `IF NOT EXISTS`
   will **not** add the new column — run
   `ALTER TABLE prices.asset_symbol ADD COLUMN attempts UInt8 DEFAULT 0` instead.
3. **Check the API reader's grant covers the new table.** If the role is granted
   per-table rather than `ON prices.*`, the table existing is not enough and the
   symptom is identical to it missing. Verify with `SHOW GRANTS FOR <reader>`
   before deploying, not after.
4. **Deploy `asset-discovery`** and let one run complete. Watch for
   `sentinelling for good` and `returned nothing usable` — with all 52 expected
   to resolve, either line means something changed on-chain or the character
   class is too strict.
5. **Deploy `prices-api`.** Only now is the read side safe.
6. **Confirm coverage reaches zero** and stays there:
   ```sql
   SELECT count() FROM prices.assets
   WHERE contract_address != ''
     AND contract_address NOT IN (
         SELECT contract_address FROM prices.asset_symbol
         WHERE symbol != '' OR attempts >= 3
     )
   ```
7. **Then** update 0120's fixture (`conformance-assets.json:36-41`) from
   `code: ""` to `SolvBTC`. Doing it earlier fails the suite in the window
   between fixture change and prod resolution.

**Re-run the 52-contract sweep before step 4.** The symbol sanitiser moved from
"reject control characters" to a positive character class after review, and the
sweep that measured 52/52 predates that change. A legitimate symbol carrying a
character outside the class would now be rejected — recoverable, since it takes
three runs to sentinel and the rejection is logged with the raw string, but
better caught before deploy.

### Deployed to production, 2026-09-02

Ran in the runbook's order, and the order mattered less than expected only
because every check passed.

| Step | Result |
|---|---|
| DDL on the shared box | table created, four columns; BE told first |
| Reader/writer grants | `prices_reader` / `prices_writer` are `ON prices.*` — nothing to add |
| 52-contract sweep, re-run after the sanitiser change | **52/52**, 0 rejections, 9.5 s |
| `asset-discovery` deploy + 3 runs | 52/52 covered, 09:54 → 10:19 |
| `prices-api` deploy | `code: "SolvBTC"` live in detail and listing |
| 0120 fixture | updated, suite green on that assertion |

**Nothing exercised the retry machinery.** 0 sentinels, 0 non-zero `attempts` —
the `attempts` counter added after review never fired on this population. That
is the expected outcome for 52 live tokens; it exists for the contract that
fails later, not for these.

**The stricter character class rejected nothing** — but `PEPE TOKEN` contains a
space, and would have been rejected had the positive class not included one.
That was a judgement call made while writing it; the sweep is what turned it
from a guess into a fact.

**Structural criterion re-verified on prod after the fact:** `prices.assets`
still holds 52 contracts and 52 identities, unchanged by the deploy.

## Issues Encountered

- **The registry looked like it had doubled.** `count()` on `prices.assets` read
  415,494 against the 207,493 recorded on 2026-08-28, and the Soroban subset 104
  against 52 — both exactly 2×, which reads as duplication, not growth. It was
  neither: `uniqExact` returned 207,754 and 52. The worker re-inserts the whole
  registry every hour into a `ReplacingMergeTree`, so a raw count lands wherever
  the merge cycle happens to be — 1×, 2× or 3× depending on the minute. Cost a
  detour mid-deploy; now [[0257]].
- **The first `cdk diff` proposed creating the entire stack.** Every resource
  showed `[+]`. The cause was the wrong AWS profile: the credentials could not
  assume the CDK lookup/deploy roles, so CDK diffed against an empty template
  instead of the deployed one. Deploying on that diff would have attempted to
  recreate nine live Lambdas. The warning lines above the table say so, and are
  easy to scroll past.
- **Deploying one Lambda deploys nine.** CDK synthesises the whole app, so every
  artifact must exist, and the stack is the deploy unit. Established that the
  Rust build is byte-reproducible here — so pinning the others to their deployed
  commit *would* isolate a single function — but chose not to: seven of the nine
  differed only by `init.sql` embedded via `include_str!`, and the eighth
  (`oracle-worker`, task 0231) was five days stale against `develop` while the
  API already had the matching shared-crate changes. Deploying resolved a
  version split rather than creating one.
- **`prices-clickhouse-init`'s doc comment omits `--features lambda`.** The crate
  has `default = []`, so following that comment builds the non-Lambda `main`,
  which prints a message and exits. It would deploy cleanly and do nothing.

## Design Decisions

### From Plan

1. **Destination is a single-writer table keyed on `contract_address`.** Not
   `assets.asset_code` (sort-key column — a write adds a row rather than
   replacing), not `asset_metadata` (whole-row writer, would clobber
   `home_domain`), not `asset_id`-keyed (ambiguous for 10 of the 52).
2. **The symbol surfaces as `asset_code`, composed at read time**, because the
   RFP names *Asset/Token Code* as required and §4.1 has no other field.
3. **Three-way outcome policy** — resolved / absent / transient — so an RPC
   outage cannot name the population `""`.

### Emerged

4. **Trigger is absence of a row, not staleness.** Removed the stalest-first
   ordering, time budget and env config the plan had copied from
   `supply-worker`. That worker's queue is the whole registry and never empties;
   this one is 52 rows and then nothing.
5. **`attempts` counter, added after review.** The simulation's `error` field
   mixes a contract with no `symbol()` against a failed ledger read, and telling
   them apart means matching host error text — a protocol-version detail.
   Counting observes the difference instead. Also closed the queue-starvation
   finding, since a persistently failing contract now leaves the head.
6. **Positive character class instead of `!is_control()`.** `char::is_control`
   is Unicode category Cc only, so `U+202E`, `U+200B` and `U+FEFF` passed. The
   existing test used `U+001B`, which *is* Cc, and so gave false confidence.
7. **`http_client` panics rather than falling back.** `Client::default()` has no
   timeout, which would silently unbound the stage's whole time budget.
8. **Search and sort deliberately left on the stored column.** Confirmed on prod
   by the sweep: contracts self-declare `USDC`, `USDT` (×2), `BTC`, `XRP`, plus
   bare `USD` and `EUR`. Making those searchable is exactly the exposure
   [[0252]] is about.
9. **`classify` extracted as a pure function.** The bug review found was
   unreachable from a test because classification only ran behind live HTTP.

## Future Work

Spawned rather than left as prose:

- [[0256]] — the ledger scan half of this same worker has never run on prod
- [[0257]] — the hourly full-registry rewrite
- [[0258]] — `api_reader` / `ingestion_writer` hold `DROP` and `SYSTEM` on `*.*`
- [[0259]] — prod schema is hand-applied with no drift check
- [[0252]] — asset identity verification, spawned earlier from the same work

## Acceptance Criteria

- [x] Soroban assets expose their on-chain symbol through the API — surfaced
      **as `asset_code` / `code`**, composed at read time, so §4.1's response
      shape is unchanged and the RFP's *Asset/Token Code* requirement is met.
      The `views.sql` interop contract is untouched (REST handlers do not read
      those views). *Criterion reworded from "in a field of their own" — see
      Implementation for why a new field was the wrong call.*
- [x] `CBIJ…` shows its real symbol in list + detail — **verified on prod
      2026-09-02**: `GET /v1/assets/CBIJ…` returns `code: "SolvBTC"`, and all
      five API-visible Soroban assets are named (SolvBTC, XAUM, HITZ, BnUSD,
      xSolvBTC) where every one read `""` before
- [x] No new row is created in `prices.assets` for an asset that gains a symbol
      — structural, not incidental: nothing writes to `assets` at all. Pinned by
      `listing_composes_soroban_symbol_into_asset_code`, which asserts the
      soroban listing still returns exactly one row
- [x] The resolver rotates unresolvable contracts out instead of retrying them
      forever (empty-symbol sentinel on absent). Note the population turned out
      to need it less than expected — 52/52 resolve — but the sentinel is what
      makes an unresolvable *future* contract safe
- [x] DDL applied and the worker deployed to prod; coverage query reaches zero
      — 52/52 covered in three runs over 25 minutes, 0 sentinels, 0 non-zero
      `attempts`, all 52 symbols identical to the pre-deploy sweep
- [x] 0120's conformance fixture updated after prod resolution — `code` moved
      from `""` to `SolvBTC` and the suite's *code matches the fixed list* now
      passes. The 16 remaining failures are the classes 0120 already owns
      (0135/0170/0178); 0 schema failures, none in the `detail` group
