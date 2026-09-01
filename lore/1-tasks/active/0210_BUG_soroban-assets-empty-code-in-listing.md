---
id: "0210"
title: "Soroban assets carry an empty asset_code in listing and detail — top-volume rows are unidentifiable to consumers"
type: BUG
status: active
related_adr: []
related_tasks: ["0120", "0139", "0242", "0252"]
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

- **Schema** — `prices.asset_symbol (contract_address, symbol, fetched_at)`,
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

## Acceptance Criteria

- [x] Soroban assets expose their on-chain symbol through the API — surfaced
      **as `asset_code` / `code`**, composed at read time, so §4.1's response
      shape is unchanged and the RFP's *Asset/Token Code* requirement is met.
      The `views.sql` interop contract is untouched (REST handlers do not read
      those views). *Criterion reworded from "in a field of their own" — see
      Implementation for why a new field was the wrong call.*
- [ ] `CBIJ…` shows its real symbol in list + detail — **verified locally**
      (resolves to `SolvBTC` against mainnet RPC); pending on prod
- [x] No new row is created in `prices.assets` for an asset that gains a symbol
      — structural, not incidental: nothing writes to `assets` at all. Pinned by
      `listing_composes_soroban_symbol_into_asset_code`, which asserts the
      soroban listing still returns exactly one row
- [x] The resolver rotates unresolvable contracts out instead of retrying them
      forever (empty-symbol sentinel on absent). Note the population turned out
      to need it less than expected — 52/52 resolve — but the sentinel is what
      makes an unresolvable *future* contract safe
- [ ] DDL applied and the worker deployed to prod; coverage query reaches zero
- [ ] 0120's conformance fixture updated after prod resolution
