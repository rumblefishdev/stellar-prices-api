---
id: "0216"
title: "Publish how old the price is, so a consumer can apply its own freshness policy instead of inheriting ours"
type: FEATURE
status: active
assignee: akot
related_adr: ["0292"]
related_tasks: ["0135", "0178", "0165", "0151", "0111", "0215"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, clickhouse, api, read-surface]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-20
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0135]]'s PR #228 review. 0135 decided `price_usd` is the
      latest *priced* close and is deliberately NOT age-bounded; the honest
      completion of that decision is to publish the age rather than blank the
      value. okarcz agreed the bound belongs only in the per-venue pipeline
      and that the age is its own task.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Activated. Decisions taken before code: the wire fields are the ones
      ADR 0292 (decision 5) already named — `as_of` and `price_status`
      (priced | carried | unpriced) — so this task delivers them instead of
      [[0147]]; "no price" is `toDateTime(0)` in the table and `""` on the
      wire (never a published 1970 — the trap 0147 recorded); `as_of` names
      `price_usd`'s candle, `price_xlm` / `market_cap_usd` get no field of
      their own and are documented as "no fresher than as_of"; ships inside
      the prepared 2026-09 rollout (DDL in step B, code in step G).
      Measured on prod 2026-09-22 08:13 UTC: of 4,342 assets with a price,
      0 are < 5 min old, 70 are 5-60 min, 1,306 are 1-6 h, 839 are 6-12 h,
      2,127 are 12-24 h.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Implemented on feat/0216. Three commits, 17 tracked files: the two
      columns run end to end through current_prices, mv_current_prices,
      current_price_usd, the three read queries, both DTOs and the published
      OpenAPI. Two traps each pinned by a test that runs in CI and each proven
      RED — the TO(...)-vs-SELECT order test (a contains check cannot see a
      same-type transposition, and that kind is SILENT on 26.3.10.60) and the
      epoch guard shared by all three query strings. 62 + 223 unit tests and
      the current_mv_it / views_it / price_it / list_it / endpoints_it
      #[ignore] suites all green against the local 26.3.10.60; one pre-existing
      endpoints_it failure (backfill_status, a fixture that rots with the
      clock) left alone. Still open: the prod before/after snapshot and the BE
      hand-off, both scripted in .planning/rollout-2026-09/ (uncommitted).
      Nothing deployed; no DDL ran on prod.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Code review (0 blockers, 7 warnings) addressed in one commit. One
      behavioural fix: `as_of` and `tip_at` are now bounded above at `now()`,
      so a future-dated candle can no longer publish a negative age — pinned
      by a new IT proven RED, with `price_usd` left exactly as it was. The
      other six were shipped prose or a fixture that claimed a path it did not
      take: two OpenAPI texts named fields their schema does not have, two
      `pub async fn`s had lost their doc comments to the SQL-builder
      extraction, `current.sql`'s own column inventory still said "eleven" and
      its DEPLOY ORDER block never recorded the 0286 `pf_trade_count`
      dependency, all three API fixtures wrote the table DEFAULTs by hand
      instead of taking them, and the portal's hand-written /price walkthrough
      still called `updated_at` the price's age. Five INFO items recorded and
      deliberately left. Bundle regenerated; still nothing deployed.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Second review round (an independent /code-review of the same branch),
      one commit. The tip is now bounded above ONCE, in `base_tip`'s WHERE,
      instead of inside the two `maxIf` predicates: a future-stamped candle is
      outside the whole tip, so `as_of` always names the candle `price_usd`
      came from and cannot name an older one. The oracle rate and its own
      timestamp became ONE tuple scalar under one WHERE, so the two can no
      longer be selected over different sets. `method` was missing from both
      portal /price samples and is now between `updated_at` and `as_of`, in
      wire order. The two "the age looks wrong" findings were answered with
      text, not code: the hourly enrichment cadence makes `carried` and an
      `as_of` about an hour behind `updated_at` the ORDINARY state of a traded
      asset, and `carried` also covers a newer trade on a pair with no USD
      conversion path — both now said in the published descriptions, the DTO
      docstrings, the sentinel table and the BE contract note. The sentinel
      proposal was declined. All suites green (62 + 223 unit, current_mv_it,
      price_it, list_it, openapi, 214 portal), the known pre-existing
      endpoints_it backfill_status failure aside; bundle regenerated; nothing
      deployed.
---

# Publish the price's age

## Summary

`price_usd` is the latest **priced** close, which for an asset whose recent
candles are un-enriched — or which simply stopped trading — can be materially
older than the row it sits in. `updated_at` is the MV's refresh time, so it
reads "a minute ago" no matter how old the price is. **No column carries the
age**, so a consumer cannot apply a freshness policy at all.

## Why this, and not a staleness bound

0135 considered bounding `price_usd` and rejected it for a measured reason:
on prod 2026-08-20, **1,091 of 4,444 assets (24.5%)** already publish a hard
zero. A bound tight enough to be a real freshness promise pushes more assets
into that sentinel — and a zero is strictly worse than an old-but-true price,
because a consumer cannot tell "worthless" from "we don't know".

One field cannot answer both *"what is the price"* and *"how fresh is it"*.
Publishing both lets each consumer pick its own threshold; a dashboard and a
liquidation engine legitimately want different ones.

## Design questions to settle first

- **What instant does it name for `price_xlm`?** That column is
  `price_usd / xlm_usd` — a quotient of **two independently dated** closes, so
  it is not a price "as of" any single instant and never was. okarcz's
  position, and it looks right: report the **older of the two**, otherwise
  `price_xlm` is handed an age it does not have.
- **`market_cap_usd`** multiplies `price_usd` by a supply figure carrying its
  own `asset_supply.fetched_at`. Same question, same answer shape.
- **Sentinel vs NULL** for "no price at all". `current_prices` columns are
  non-nullable by convention and use sentinels — [[0151]] owns that argument;
  a zero-epoch `DateTime` is the obvious analogue.

## Implementation notes

Cheap in itself: one row per asset, `ALTER TABLE … ADD COLUMN` is metadata
only, and the MV recomputes every row every minute, so there is **no
backfill**. Three traps, all from the PR #228 review:

- ⚠️ **Positional insert.** The `TO` clause carries an explicit column list
  precisely because an MV inserts positionally otherwise. A new column must go
  into **both** `TO(...)` and the SELECT, in the same order. Getting it wrong
  writes the price into the age column, with no error anywhere.
- ⚠️ **The clamp is coupled to the column type.** The SELECT clamps
  `change_*_pct` to ±999999 because `Decimal(10,4)` holds exactly that.
  Widening a clamp without widening its column overflows the INSERT and the
  refresh throws — and because this MV is **REPLACE, not APPEND**,
  `current_prices` then stops updating for *all* 4,444 assets, not just the
  offending one.
- ⚠️ Same REPLACE property: anything that *filters* rows here removes those
  assets from the table entirely rather than blanking a field. Emit sentinels,
  never filter.

Consider shipping alongside [[0178]], which already rewrites this MV
(DROP + re-CREATE) and already plans a provenance column following [[0165]]'s
`traded`/`peg`/`oracle` vocabulary — one exposure window instead of two, and
"where did this number come from" and "when was it true" are the same
conversation.

## Acceptance Criteria

- [x] `/price` and `/assets` expose the price's own timestamp, distinct from
      `updated_at`, and the published OpenAPI descriptions say which is which
      — `POST /prices/batch` too, and both `updated_at` texts now say in so
      many words that they are the refresh time and `as_of` is the price's own
- [x] `price_xlm`'s reported age is the older of its two inputs — or the
      opposite choice is recorded with its reasoning (decision 5 below: the
      opposite choice, with the older bound reaching the reader by
      construction)
- [x] `views.sql`'s sentinel table documents the new column for BE — both
      columns, including the epoch rule and the `''` not-yet-rewritten state
- [ ] Adding the column changes no existing value — verified by comparing a
      full `current_prices` snapshot before and after
      — **in repo: done.** Every pre-existing assertion in `current_mv_it` is
      green UNMODIFIED, including the whole 0072 column-set test. **On prod:
      open** — the before/after snapshot runs on rollout day, scripted in
      `.planning/rollout-2026-09/13-0216-snapshot-before.sh` and
      `62-0216-snapshot-diff.sh`
- [x] The `TO(...)` list and the SELECT projection are asserted to match in a
      test, so the positional-insert trap cannot recur silently

Not an acceptance box, but outstanding: the API contract hand-off to the docs
owner (0233/0163) is written as `.planning/CONTRACT-0216-api.md` (uncommitted)
and is Adam's to send. The task stays `active` until the rollout runs.

## Implementation Notes

Three commits on `feat/0216_publish-the-price-age-alongside-the-price`, cut
from `b359d58`. Nothing deployed, no DDL on prod.

**`feat(lore-0216): carry as_of and price_status through ClickHouse`** —
`schema/init.sql` (the two columns in `CREATE TABLE`, plus two idempotent
`ALTER TABLE … ADD COLUMN IF NOT EXISTS` statements with a comment block in
the `method` block's voice); `schema/current.sql` (a `usdc_as_of` scalar
copying `usdc_rate`'s WHERE verbatim, `maxIf(timestamp, close_usd > 0) AS
as_of` and `maxIf(timestamp, pf_trade_count > 0) AS tip_at` in `base_tip`,
both carried through both arms of `unfiltered`, the two guarded projections,
and the 13-name `TO(...)` list); `schema/views.sql` (two sentinel-table
entries and the `current_price_usd` forwarding); `src/lib.rs` (the order test,
statement count 41 → 43, the forwarding list 9 → 12 names);
`tests/current_mv_it.rs` (`status_of` / `as_of_of` helpers, an `insert_pair_pf`
helper, the unpriced / carried / oracle / dust-only assertions);
`tests/views_it.rs` (14 → 16 view columns).

**`feat(lore-0216): publish as_of and price_status on the price surfaces`** —
`queries_ch.rs` (the shared `AS_OF_SQL` epoch guard, three pure SQL builders,
the three `Row` structs, the guard unit test); `dto.rs` (both DTOs and
`from_row`); `assets/handlers.rs` and `batch/handlers.rs` (the two hand-built
literals); `openapi/descriptions.rs` (four new triples, four rewritten texts);
`tests/{price_it,list_it,endpoints_it}.rs` (wire assertions by VALUE on both
the real and the DEFAULT arm); `web/portal/public/openapi.json` (regenerated).

**`docs(lore-0216): …`** — this file and the guardrails inventory.

### RED proof (a) — transpose the two new names in the `TO(...)` list only

`as_of, price_status` → `price_status, as_of` in `current.sql`'s `TO(...)`,
nothing else touched.
`cargo test -p prices-clickhouse --lib current_sql_to_clause`:

```
running 1 test
test tests::current_sql_to_clause_and_select_project_the_same_columns_in_the_same_order ... FAILED

---- tests::current_sql_to_clause_and_select_project_the_same_columns_in_the_same_order stdout ----

thread 'tests::current_sql_to_clause_and_select_project_the_same_columns_in_the_same_order' (727685) panicked at packages/prices-clickhouse/src/lib.rs:766:9:
assertion `left == right` failed: the TO(...) list and the final SELECT disagree.
  TO:     ["asset_id", "price_usd", "price_xlm", "change_24h_pct", "change_7d_pct", "volume_24h_usd", "market_cap_usd", "vwap_24h", "sources", "updated_at", "method", "price_status", "as_of"]
  SELECT: ["asset_id", "price_usd", "price_xlm", "change_24h_pct", "change_7d_pct", "volume_24h_usd", "market_cap_usd", "vwap_24h", "sources", "updated_at", "method", "as_of", "price_status"]
A same-type transposition here (as_of/updated_at, price_status/method) is accepted by ClickHouse WITHOUT an error and publishes the refresh time as the price's age.

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 61 filtered out; finished in 0.00s
```

Restored byte-for-byte; re-run green (`1 passed`). Note this particular
transposition is `DateTime` ↔ `LowCardinality(String)` and would ALSO have
thrown `CANNOT_PARSE_DATETIME` at refresh time. The transposition the test
exists for — `as_of` ↔ `updated_at`, or `price_status` ↔ `method`, both
same-type — throws nothing at all and was measured silent on 26.3.10.60.

### RED proof (b) — drop the epoch guard from one of the three query strings

`current_price_sql`'s `{AS_OF_SQL}` replaced by a plain
`formatDateTime(c.as_of, …) AS as_of`.
`cargo test -p prices-api --lib every_current_price_query_guards_as_of`:

```
running 1 test
test assets::queries_ch::tests::every_current_price_query_guards_as_of_against_the_epoch ... FAILED

---- assets::queries_ch::tests::every_current_price_query_guards_as_of_against_the_epoch stdout ----

thread 'assets::queries_ch::tests::every_current_price_query_guards_as_of_against_the_epoch' (739125) panicked at packages/prices-api/src/assets/queries_ch.rs:2083:13:
current_price must project as_of through the shared epoch guard, got: SELECT toString(c.price_usd) AS price_usd, … c.method AS method, formatDateTime(c.as_of, '%Y-%m-%dT%H:%i:%SZ') AS as_of, c.price_status AS price_status FROM current_prices AS c FINAL INNER JOIN assets AS a FINAL ON a.asset_id = c.asset_id WHERE a.contract_address = ? LIMIT 1

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 222 filtered out; finished in 0.00s
```

Restored byte-for-byte; re-run green (`1 passed`).

### Verification run

`cargo fmt --all -- --check` clean · `cargo check --workspace` clean ·
`prices-clickhouse` unit 62 passed · `prices-api` unit 223 passed ·
`current_mv_it --ignored` 9 passed · `views_it --ignored` 16 passed ·
`price_it --ignored` 7 passed · `list_it --ignored` 10 passed ·
`endpoints_it --ignored` 9 passed / 1 failed
(`backfill_status_maps_both_streams` — pre-existing, see Issues) ·
`npm run openapi:verify-bundled` exit 0.

## Design Decisions

### From Plan

1. **Two columns, appended LAST, `as_of` then `price_status`** — in the table,
   the `TO(...)` list, the SELECT, all three `Row` structs, all three query
   strings, both DTOs, `current_price_usd` and the batch hand-copy. The one
   place "last" is not literal is `AssetListRow`, where `sort_key` (the cursor
   payload, not a published column) stays last and the two new fields go
   before it.
2. **`as_of` is `price_usd`'s own timestamp** — `maxIf(timestamp, close_usd >
   0)`, the SAME predicate `price_usd`'s `argMaxIf` uses, so the two can never
   name different candles. The oracle arm takes the rate reading's own time
   from a new scalar under a VERBATIM copy of `usdc_rate`'s WHERE.
3. **The 1970 trap.** `maxIf` over a window with no priced candle returns the
   DateTime default, not NULL. The MV forces `toDateTime(0)` whenever
   `price_usd <= 0` so the epoch is a decision rather than an accident, and
   one shared const maps exactly that value to `""` in all three queries.
   Pinned by a CI unit test, an MV IT and two wire ITs.
4. **`price_status` vocabulary**, keyed on values and never on `is_oracle`:
   `unpriced` when `price_usd <= 0`, `carried` when `as_of < tip_at`, else
   `priced`. `tip_at` is `maxIf(timestamp, pf_trade_count > 0)` — a dust-only
   minute forms no price, so it must not make a real price read stale.
5. **No separate age for `price_xlm` / `market_cap_usd`** — the recorded
   answer to this task's own "older of the two" question, and the opposite of
   the position in "Design questions" above. A second field would itself be a
   quotient of two dates, and would have to be recomputed for every derived
   column ever added. Instead the descriptions state the bound: both divide or
   multiply `price_usd` by an independently dated input, so both are no
   fresher than `as_of` and may be older. The reader gets the older bound by
   construction, with one field instead of three.
6. **The order-comparing positional test** replaces the `contains` check,
   which could not see order. It parses both sequences out of
   `split_statements(CURRENT_SQL)[1]` and asserts equality plus length 13.
7. **Descriptions** — four new integrator-voice triples and four rewritten
   texts; no task or ADR citation in anything published.
8. **`views.sql`** — both columns in the BE sentinel table, forwarded by
   `current_price_usd`, and the forwarding test's list extended from 9 names
   to 12 (it had never included `method`).
9. **Snapshot acceptance** — no existing assertion was modified or loosened.
10. **Guardrails inventory** — two new reader rows, and this task's `◐` status
    cells closed.
11. **No deploy, no DDL on prod, no `cdk`.**

### Emerged

12. **The three SQL strings became pure builder functions.** They were built
    inside `async fn`s taking a `&Client` and so were unreachable from a unit
    test — the guard could only have been checked by a test that needs a
    database, which is exactly the test CI does not run. `list_assets_sql`,
    `current_price_sql` and `current_prices_batch_sql` are plain
    `fn(…) -> String`, mirroring `usd_method_expr`, which this repo already
    unit-tests that way. Rejected: `include_str!` over the module's own source
    (no precedent here, and it breaks on any refactor).
13. **`insert_pair_pf` rather than a dust-only-specific helper.** The dust
    fixture needs a control — a price-forming newest minute that DOES read
    `carried` — or a status expression stuck on `priced` would pass it. One
    helper taking `pf_trade_count` explicitly builds both arms; an
    `insert_pair_dust` hard-coding 0 would have needed a twin.
14. **Parsing approach for the order test** (explicitly left to discretion):
    the `TO` list is the substring between `TO prices.current_prices` and the
    first `)`, asserted to be followed by `AS`; the projection is every
    `AS <ident>` at paren depth 0 between the last depth-0 `SELECT` and
    `FROM unfiltered AS u`, each asserted to be followed by `,` or the end.
    `split_statements` has already stripped every comment, which is what makes
    a depth scan tractable; the string literals in the projection
    (`'LowCardinality(String)'`, `'Map(String, Map(String, String))'`) carry
    balanced parens, so they do not disturb the depth count.
15. **The listing and batch fixtures now seed the new columns explicitly.**
    The plan expected the table DEFAULTs to cover it, but `AssetListRow` and
    `BatchPriceRow` are separate structs read by separate queries, and a
    fixture where every row carries the DEFAULT pair cannot tell a working
    projection from one that returns `''` for everything. One row per fixture
    carries a real pair dated 30 minutes behind `updated_at`; the rest carry
    the epoch/`''` pair, written explicitly to the same bytes the DEFAULT
    would produce.
16. **`descriptions.rs` placement is strictly alphabetical** — `price_status`
    before `price_usd`, matching the file's convention rather than the plan's
    parenthetical ("between `price_usd` and `sources`"). No test enforces
    order either way.
17. **The guardrails row for `current_price_usd` / `/price` / `/assets` was
    closed too**, not only the `current.sql` row the plan named. Its `◐` cell
    read "`price_status` + `as_of` — ADR 0292 §5", which is this task; leaving
    it open would have left the file asserting a gap that no longer exists.

## Issues Encountered

- **`init_sql_parses_into_statements` and `views_it`'s column count are
  outside the brief's test list.** Two more ALTERs make the `init.sql`
  statement count 41 → 43, and `current_price_usd` gaining two columns makes
  `views_sql_replaces_an_existing_v1_current_price_usd` count 16 rather than
  14. Both updated with their comments. `views_it`'s `#[ignore]` suite was
  added to the verification list; it was not in it.

- **`web/portal/public/openapi.json` is tracked and CI asserts it matches the
  extracted document** (`npm run openapi:verify-bundled`, ci.yml). The brief
  said "nothing else changes"; this is a required consequence of adding two
  DTO fields, not scope creep, and it ships in the same commit as the DTOs.
  `web/portal/src/api/generated.ts` is untracked and was left alone.

- **`endpoints_it::backfill_status_maps_both_streams` fails, and did before
  this task.** It asserts `sdex.status == "running"` against a fixture whose
  `last_push_at` is a hard-coded `2026-06-15 11:30:00`; today the handler
  correctly calls that `stalled`. Confirmed pre-existing by running the test
  against `b359d58`'s copy of the file, where it fails identically. Out of
  scope here and NOT fixed — the fixture rots with wall-clock time and wants a
  relative timestamp, which is somebody's call, not a side effect of this
  task.

- **The `5` in `current_sql_uses_no_unguarded_argmax_on_close_usd` did not
  move**, as predicted: `maxIf(timestamp, …)` matches neither counted prefix.
  `INTERVAL 2 HOUR` still appears exactly once, and the new scalar uses
  `max(timestamp)` — never `argMax(close_usd`, which that test panics on as a
  literal substring.

### Review round (2026-09-22, `260922-ftd-REVIEW.md` — 0 blockers, 7 warnings)

All seven fixed in one commit. Only WR-06 changed behaviour; the rest were
prose that shipped, a doc comment that migrated, or a fixture that claimed a
path it no longer took.

- **WR-01 — two published `as_of` texts named fields their schema does not
  have.** `PriceResponse.as_of` keeps the `price_xlm` clause (that DTO has
  `price_xlm`) and drops `market_cap_usd`, which it never published;
  `AssetListItem.as_of` and `.price_status` drop both and become one-line
  definitions plus "Same meaning as `PriceResponse.…`", the convention
  `AssetListItem.method` two entries away already used. Same correction in the
  two `dto.rs` docstrings. All four literals rewrapped in the file's `\`-
  continued style (IN-02 came along for free).
- **WR-02 — the SQL-builder extraction left two `pub async fn`s undocumented.**
  `list_assets`'s sort-policy doc moved back off `list_assets_sql`, and
  `current_prices_batch`'s "what it does" line back off
  `current_prices_batch_sql`. Each helper keeps only its own text.
- **WR-03 — `current.sql`'s column inventory still said "eleven".** Now
  thirteen, with an `as_of` and a `price_status` line in the same voice, two
  lines below the positional-insert warning whose whole subject is that list.
  `price_usd`'s entry now points at `as_of` for the age.
- **WR-04 — the new `pf_trade_count` dependency was nowhere in the DEPLOY
  ORDER header.** Added: this file is DROP + CREATE, so on a database without
  0286's `pf_*` columns the DROP succeeds and the CREATE fails with `Code: 47`,
  leaving `current_prices` with no writer; apply 0286's ALTERs first (`init.sql`
  does on a fresh apply; on prod that is step B, before step E).
- **WR-05 — `price_it.rs` asset 2 claimed the table-DEFAULT path while naming
  both columns explicitly.** Split into a second INSERT whose column list stops
  at `method`, so the row really takes `toDateTime(0)` / `''`. The same
  claim-vs-INSERT mismatch was in `list_it.rs` (assets 1/3/4) and
  `endpoints_it.rs` (asset 2) and was aligned the same way: every one of the
  three surfaces now has at least one row on the real DEFAULT path, and the
  by-value `""`/`""` wire assertions are unchanged.
- **WR-06 — `as_of` inherited `base_tip`'s lower-only window, so a future-dated
  candle published a negative age.** Both new aggregates are now
  `maxIf(timestamp, … AND timestamp <= now())`, matching the bound the oracle
  arm already carried. `argMaxIf`/`argMinIf` and the CTE's `WHERE` are
  untouched, so `price_usd` is exactly what it was (decision D-09) — which
  means that on such a defective row, and only there, `as_of` names an older
  candle than `price_usd`; the CTE comment now says so rather than claiming the
  two can never differ. Pinned by a new IT,
  `a_future_dated_candle_does_not_date_as_of_ahead_of_now`, which seeds a
  priced candle at `now() + 10 min` beside one at `now() - 5 min` and asserts
  `as_of` BY VALUE against the past one (a `<= now()` assertion would pass on
  any timestamp the MV happened to pick). Proven RED: with the bound removed it
  fails `left: "…10:56:00", right: "…10:41:00"`. The guard test's counts did
  not move — 5 guarded aggregates, one `INTERVAL 2 HOUR`, no `argMax(close_usd`.
- **WR-07 — the portal's hand-written `/price` walkthrough still called
  `updated_at` "when this price was last computed".** Corrected in
  `QuickStart.tsx` to "when this snapshot row was last refreshed — not the age
  of the price", and `as_of` / `price_status` added to both the `RESPONSE_FIELDS`
  table and `Endpoints.tsx`'s sample. `portal:typecheck`, `portal:lint` and the
  213 portal unit tests (which tie each rendered value to the string its Copy
  button writes) are green.
- **INFO-1 wording, taken with WR-01.** `priced` no longer overstates: it is
  "nothing newer is outstanding: `as_of` is the newest price-forming minute in
  the window, or no newer price-forming minute exists" — the no-price-forming-
  candle-at-all case reads `priced` too, and the text now admits it.

Left as they are, deliberately: `AS_OF_SQL`'s `pub(crate)` reach and its unstated
`current_prices AS c` alias assumption (IN-03); the `updated_at` formatter still
copy-pasted across the three builders (IN-04); the TZ-dependent
`"1970-01-01 00:00:00"` string in `current_mv_it` (IN-05 — CI and the local
26.3.10.60 are both UTC); the order test's paren-depth scan not tracking quotes
(IN-06 — the `len() == 13` assert is what turns a desync into a loud failure);
and `insert_pair_pf` leaving `pf_volume` on its DEFAULT beside
`pf_trade_count = 0` (IN-07 — `current.sql` reads neither column). None of them
changes what ships; each is a line in the review for whoever touches these next.

### Second review round (`/code-review`, 2026-09-22)

A second, independent review of the same branch. Three items changed code, two
were answered with text and the semantics kept, one was declined. One commit.

- **#3 applied — the tip is bounded ONCE.** `AND timestamp <= now()` moved out
  of the two `maxIf` predicates and into `base_tip`'s own `WHERE`, so the
  aggregates read `maxIf(timestamp, close_usd > 0)` /
  `maxIf(timestamp, pf_trade_count > 0)` again. The first round's bound made a
  future-stamped candle invisible to `as_of` but still visible to `price_usd`,
  which published a price the age field did not name; now every aggregate of
  the tip sees the same candle set, `as_of` always names `price_usd`'s candle,
  and `""` on the wire is exactly `price_usd = "0"`. A future-stamped candle is
  a data defect and is simply outside the tip — the oracle arm bounds
  `timestamp <= now()` the same way. The IT
  `a_future_dated_candle_does_not_date_as_of_ahead_of_now` now prices the
  future candle at 3.0 against the past one's 2.0 and asserts all three of
  `price_usd == 2.0`, `as_of == past` and `price_status == "priced"` — a bound
  back inside the `as_of` aggregate alone would publish 3.0 beside the older
  age and is caught by value. Proven RED again with the `WHERE` bound removed:
  `current_mv_it.rs:1276`, `left: "2026-09-22 11:44:00"`, `right:
  "2026-09-22 11:29:00"`. The guard test's counts did not move — 5 guarded
  aggregates, one `INTERVAL 2 HOUR`, no `argMax(close_usd`.
- **#4 applied — one USDC scalar, not two.** `usdc_rate` and `usdc_as_of` are
  now a single `usdc_reading` tuple,
  `(argMax(usd_rate, timestamp), max(timestamp))` under the one unchanged
  `WHERE`; `usdc_reading.1` / `.2` replace the two names in `usdc_tip`'s
  `price_usd` / `as_of` / `tip_at` and in the `> 0` guard. The "VERBATIM copy …
  must stay that way" comment is gone: with one subquery there is no second
  `WHERE` to keep in step. Verified on the local 26.3.10.60 that a tuple scalar
  over no matching row yields `(0, 1970-01-01 00:00:00)`, and the "no oracle
  reading" case now has an assertion —
  `the_oracle_allowlist_is_usdc_only_and_never_repegs_stellar_usdt` seeds the
  canonical USDC identity with no rate and asserts the arm emits NO row, so the
  `.1 > 0` guard is what keeps a 1970-dated zero tagged `oracle` off the wire.
- **#5 applied — `method` was missing from both portal `/price` samples.**
  Added to `QuickStart.tsx`'s `RESPONSE_FIELDS` and `Endpoints.tsx`'s sample
  between `updated_at` and `as_of` (wire order), value `"traded"`, glossed "How
  the price was obtained: traded, oracle, or empty when unavailable".
  `QuickStart.spec.tsx` needed no change — it derives its key list from
  `RESPONSE_FIELDS` rather than pinning one.
- **#1 and #2 answered by text; the semantics are unchanged and deliberate.**
  Both findings read a `carried` status and an `as_of` about an hour behind
  `updated_at` as a defect. It is not: `as_of` is the age of the USD PRICE, and
  USD values are computed by a pass that runs hourly, so that is what an
  actively traded asset looks like (the prod measurement in this task's history
  says the same in numbers — 0 of 4,342 assets under 5 minutes old). Shortening
  the enrichment cycle is a pipeline change and its own task, not a rename of
  this field. Nor can `carried` be split into "stale" and "unconvertible": the
  second case is a newer trade on a pair with no USD conversion path, and
  whether a quote HAS such a path is not data this snapshot holds — that is
  [[0147]]'s. So both cases are now stated, in the published `PriceResponse`
  `as_of` / `price_status` descriptions, the two `dto.rs` docstrings, the
  `views.sql` sentinel table and the BE contract note: the cadence makes
  `carried` ordinary rather than a fault, a freshness threshold tighter than it
  rejects the whole market, and `carried` may also mean no newer USD price is
  coming at all.
- **#6 declined — the view keeps its sentinels.** The proposal was to turn the
  epoch / `''` sentinels into NULLs at the `current_price_usd` boundary. That
  view's contract is non-nullable columns with documented sentinels (ADR 0292's
  first verdict), and its sentinel table already states that epoch means "no
  price" and must be read as absent, never as an age. Changing it would break
  every existing BE reader to restate something already documented.
