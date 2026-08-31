---
id: "0168"
title: "Publish the real peg rate in price_usd_series instead of a hardcoded $1"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0165", "0167", "0154", "0151", "0150", "0172", "0196", "0173", "0170", "0246"]
tags:
  ["priority-medium", "effort-small", "clickhouse", "read-surface", "be-interop", "data-correctness", "milestone-M2"]
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-07
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0165]]. 0165 ships a peg-fill arm at a flat `$1` because it
      is the only value available without new infrastructure. That is a ~0.1%
      systematic error on every row — small USDC depegs (0.999/1.001) are routine
      across chains, not a crisis event — and it contradicts our own candles,
      which the oracle tier already prices depeg-aware. Needs [[0167]]'s
      `prices.usd_rate`. Scoped as its own task rather than a caveat in 0165's
      view header, so the header can point at an ID instead of a regret.
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      SCOPE REDUCED by 0172 and put on hold for one identity. The peg set is now
      USDC ONLY — USDT was removed because it genuinely depegged in June 2022 and
      trades at ~$0.13. Do not restore USDT here. See the hold note below.
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      HOLD LIFTED. 0196 deleted the mis-attributed usd_rate rows (44,318) and
      oracle_prices rows (46,423) on prod, verified 0 with no regrowth. usd_rate
      now holds canonical USDC only, measured at par, so "swap the constant 1
      for the measured rate" is safe as designed. The restriction survives as a
      standing condition on ADDING peg members, now guarded by two pinning
      tests rather than by prose.
  - date: 2026-08-31
    status: active
    who: okarcz
    note: >
      Activated straight after [[0178]] closed, because 0178 CREATED an
      inconsistency this task resolves: the tip surface now publishes canonical
      USDC at the MEASURED rate tagged method='oracle' (prod, $1.00005447313041),
      while price_usd_series still publishes the hardcoded 1 tagged 'peg'. Our
      two surfaces disagree about the same asset. 0178 also built the read
      pattern this needs - prices.usd_rate, ASOF at-or-before, method='oracle',
      refused past a staleness window, and allowlisted to USDC BY NAME because
      Reflector prices the USDT TICKER at par while the Stellar issuer's token
      is worth ~$0.13 (widening stays gated on [[0173]]). Transplant that shape;
      do not re-derive it. Note 0178 emitted 'oracle' first, ahead of the
      vocabulary reservation in views.sql:179.
  - date: 2026-08-31
    status: active
    who: okarcz
    note: >
      IMPLEMENTED AND GREEN LOCALLY, NOT YET DEPLOYED. Both views now carry the
      measured rate through a peg_rate column on arm B and fall back to $1 only
      where no observation exists in the bucket; 'oracle' vs 'peg' tells the two
      apart. 12 tests green on the prod pin 26.3.10.60 (10 views_it incl. 2 new
      + 26 ohlcv_it + 28 lib guards), no existing test needed a behaviour change
      - 0165 wrote them against fallback semantics exactly so this would hold.
      Resolution is 0167's rule (last observation in the bucket, never an
      average) written as an argMax per bucket rather than an ASOF, which makes
      the staleness window the bucket itself and keeps the right side tiny.
      ⚠️ Found an UNPLANNED disagreement with 0170's /ohlcv peg series, which
      ASOFs at the bucket START with no staleness bound - spawned [[0246]].
      Remaining: the prod internal-consistency check, which needs the deploy.
---

# ✅ HOLD LIFTED 2026-08-13 — this task is unblocked

The hold below was specific to one identity and that identity is gone from both
sources. [[0196]] deleted **44,318 mis-attributed `usd_rate` rows** (and 46,423
from `oracle_prices`) on 2026-08-13, verified at 0 with no regrowth after both
writers were fixed and deployed. `usd_rate` now contains **canonical USDC only**,
measured at 1.000086–1.000639 over 2026-03 → 2026-08 — genuinely par.

So the design "swap the hardcoded `1` for the measured rate" is now safe as
written: arm B is USDC-only, `usd_rate` is USDC-only, and the two agree.

⚠️ **The hold becomes a standing condition, not a lifted restriction.** Adding
any identity to the peg set is a claim that the oracle feed prices *that issuer*,
not merely an asset whose code matches. Two tests now fail if the sets change
without a deliberate edit — `peg_identities_is_exactly_canonical_usdc`
(`oracle-worker`) and `reflector_resolves_exactly_xlm_and_usdc_and_nothing_else`
(`prices-ingest-core`). The general mapping question stays [[0173]].

The original hold note is kept below unedited, because it is the reasoning that
made the trap visible and it should not have to be rediscovered.

---

# ⚠️ HOLD — do not close 0172 with this task (added 2026-08-12)

This task's design is "swap the hardcoded `1` for the measured rate in
`prices.usd_rate`". For **USDC that is still exactly right** and this task should
proceed on that basis.

For **USDT it is actively harmful**, and would look like a fix:

- `prices.usd_rate` stores **Reflector's ticker feed** for "USDT" — Tether's own
  token, genuinely at par (`0.999232` in 2026-08) — keyed under the Stellar
  issuer `GCQTGZQQ…TG6V`, whose IOU is worth **~$0.13** ([[0172]]).
- Sourcing from `usd_rate` would therefore publish ~$1.00 for that identity
  again, with `method = 'oracle'` — which reads as *more* authoritative than the
  `peg` placeholder it replaced. Same 7.4× error, better disguise.

[[0172]] already removed USDT from the peg arm, so there is nothing here to
"restore". The mis-attributed rows are [[0196]]; the general symbol→issuer
mapping problem is [[0173]].

**Before implementing: confirm the peg set is still USDC-only, and do not add an
identity to it because `usd_rate` happens to have rows for it.**

# Publish the real peg rate, not `$1`

## Summary

[[0165]] gives `price_usd_series` a peg-fill arm so USDC becomes publishable at
all. That arm emits a **constant `$1`**. This task swaps the constant for the
measured rate from [[0167]]'s `prices.usd_rate`, falling back to `$1` only where
no observation exists.

## Context

USDC does not sit at exactly `$1`. It trades at 0.999–1.001 as a matter of
routine, so a hardcoded `1` is not a rare-depeg approximation — it is a **~0.1%
systematic error on every published row, permanently**.

Worse, it is inconsistent with our own data. The enrichment oracle tier is
depeg-aware and *"wins where it applies"* (`ch_enrich.rs:20`), setting a
`TF/USDC` candle's `close_usd` from the Reflector USDC rate. So in the oracle
window our candles already price USDC at 0.9993 while this view would publish
`USDC = 1.0000` for the same bucket. Two of our own surfaces disagreeing is a
defect a consumer will eventually file back at us.

The rate is **already collected** — `oracle_worker` polls it
(`oracle-worker/src/lib.rs:33`) — it is simply never published.

## Why this is a separate task and not part of 0165

- 0165 unblocks **1,433 pools that have no price at all** (67.8% of every
  never-priced pool BE holds). Flat `$1` is 0.1% wrong; absent is 100% wrong.
  Holding that behind new infrastructure is the wrong trade.
- The rate table ([[0167]]) is real work with its own verification gate.
- **The view's shape does not change**, so this is a genuine refinement rather
  than rework — see below.

## Implementation

The entire change is one expression. 0165's peg arm becomes a `LEFT JOIN` onto
`prices.usd_rate` (resolved by `ASOF` at the bucket's end per [[0167]]'s rule),
carrying `peg_rate = coalesce(r.usd_rate, 1)`; arm A carries `peg_rate = 0`.

```sql
if(max(is_peg) = 1 AND sum(w) = 0,
   CAST(max(peg_rate) AS Decimal(38, 14)),        -- was: CAST(1 AS Decimal(38, 14))
   CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14)))
```

`max()` picks arm B's value because every arm-A row carries `0`.

Same key, same column, same `Nullable(Decimal(38,14))` type, same purely-additive
property. No schema migration, no consumer change, BE integrates once.

## Three things 0165 must do so this stays a one-line change

Fold these into 0165 **before it merges**, or this task turns into a rewrite:

1. **Write the regression test against fallback semantics, not against `$1`.**
   Assert *no rate row → `$1`* and *rate row present → that rate*, with the
   second case simply unseeded for now. A test asserting "peg asset → exactly
   1.0" has to be rewritten here.
2. **Carry the provenance discriminator from day one.** Once both paths exist, a
   consumer cannot tell `1.0000` (a real oracle reading) from `1.0000` (the
   fallback). That is the `close_usd = 0` mistake — one value meaning several
   things — and it is far cheaper to ship the column than to retrofit it. Use
   [[0167]]'s `method`/`hops` vocabulary, do not invent a second.
3. **Point the view header comment at this task ID**, not at a prose caveat.

## Acceptance Criteria

- [x] `price_usd_series` and `price_usd_series_1h` publish the measured peg rate
      where an observation exists within the staleness window.
      `peg_fill_publishes_the_measured_rate_and_falls_back_only_without_one`.
- [x] `$1` remains the fallback where no observation exists (deep history,
      pre-**2026-03-11**), and is **distinguishable** from a measured `1.0000`.
      The distinguishability half is its own test —
      `a_measured_rate_at_exactly_par_is_labelled_oracle_not_peg` — because it
      is the one property no value-based check can cover: both rows read `1`.
- [x] No `NULL` introduced — BE: *"a NULL renders as a dash and removes the pool
      from every USD view we have."* `close_usd` is still non-Nullable
      `Decimal(38, 14)`, same column order, same arity; the unmatched-join path
      yields the Decimal DEFAULT `0` (prod runs `join_use_nulls = 0`), which the
      `> 0` test routes to the `$1` fallback. Pinned by the
      `countIf(close_usd <= 0) = 0` assertion carried in three tests.
- [ ] The published value agrees with what the oracle tier baked into candles in
      the same bucket — the internal-consistency check that motivates this task.
      **Needs the deploy** — see the runbook at the end of this file. Cannot be
      checked locally: it compares against enriched prod candles.
- [x] Applies to every peg asset, not a USDC special case. ⚠️ **The AC's "(USDT
      too)" is STALE** — [[0172]] removed USDT from the peg set in 2026-08-12
      and [[0196]] made that a standing condition. The MECHANISM is general: the
      rate join follows whatever identity the peg predicate admits, keyed on the
      full natural identity, so a member added later is priced without touching
      this code. The SET is deliberately one member. Widening it stays gated on
      [[0173]], and two tests fail if it moves without a deliberate edit.
- [x] Visible in [[0150]] if that materialises the view — **vacuous today**:
      0150 has not shipped and nothing materialises `price_usd_series`
      (`grep price_usd_series schema/` hits only `init.sql`'s comments and
      `views.sql`). Recorded rather than ticked silently, so 0150 knows to
      re-check rather than assume this was verified against something.

## ⚠️ Known adjacent gap this task does NOT close — the enrichment peg tier

**Measured on prod 2026-08-10**, from live `oracle_prices` readings:

| | rate | off par |
|---|---|---|
| USDC | 1.00066784838102 | **+0.067%** |
| USDT | 0.99930223861292 | **−0.070%** |

Three properties, each of which strengthens the case for this task:

1. **The ~0.1% figure is real**, ~0.07% per asset. Not hypothetical, not a
   depeg event — this is an ordinary Sunday afternoon.
2. **The two deviate in OPPOSITE directions**, so the spread *between* them is
   **~0.137%**. A flat `$1` is therefore not a small uniform offset that mostly
   cancels; anything comparing a USDC-denominated value against a
   USDT-denominated one carries the whole 0.14%.
3. **It is a persistent bias, not noise.** Five consecutive 5-minute readings
   held the same sign and magnitude to four decimal places. Jitter around par
   would average out across many candles; a stable offset does not — it is
   present on *every* row, always in the same direction. That is a stronger
   argument than a depeg would be: a depeg is rare and visible, this is
   permanent and invisible.

**The gap:** this task fixes the *view's* peg fallback. The enrichment **peg
tier** bakes the same flat `$1` into `close_usd` itself —

> a USDC- or USDT-quoted candle gets `close_usd = close × $1`, exact and
> oracle-free, back to SDEX genesis  (`ch_enrich.rs`)

— so every USDC-quoted candle's `close_usd` is ~0.067% **low** and every
USDT-quoted one ~0.070% **high**, wherever the oracle tier did not win. That is
all deep history before the oracle window (**2026-03-11**, measured) plus anything outside the
staleness bound. **Shipping this task leaves that untouched**, and a reader
comparing the view against the candles will find them disagreeing by that margin.

**Why it is not folded in here.** Pointing the peg tier at [[0167]]'s
`prices.usd_rate` is the obvious fix and becomes possible once that table
exists — but it is a *write-path* change to the enrichment hot loop, which
[[0111]] is already the open performance task for, and correcting history means
re-enrichment rather than a view swap. Different risk class, different task.

⚠️ **Do not queue it ahead of [[0172]] on magnitude alone.** 0.07% on `close_usd`
is plausibly acceptable for TVL; 0172 is USDT candles reading ~0.14 against USDC,
a ~7× error on 102 live pools. Fix the order-of-magnitude problem first.

## Out of scope

- Building the rate table or populating it — [[0167]].
- **Correcting `close_usd` itself** (the enrichment peg tier above). Noted
  deliberately rather than filed, 2026-08-10 — file it when 0111 makes the
  enrichment write path safe to touch, or when someone needs better than 0.07%.
- `current_price_usd` / `current_prices`, which is suspected to carry the same
  base-only assumption as 0165 but is a refreshable-MV rebuild and must not ride
  along on a view swap.

## Notes

- Deep history stays flat `$1` permanently — there is no oracle reading before
  **2026-03-11** (measured on prod 2026-08-10; earlier task text said ~2025-09,
  which was never verified and is wrong).
  That is a data-availability fact, not a gap to close, and the same shape as the
  pre-Soroban tail having no USD reference at all.

  ## ⚠️ "But the candles know the real rate" — they do not. MEASURED on prod 2026-08-31.

  Raised as a challenge to the fallback and worth settling permanently, because
  it is the obvious objection and it is wrong for a non-obvious reason.

  **Half 1 — there is no USDC candle to read.** Canonical USDC as a BASE:

  ```
  SELECT count() FROM prices.price_ohlcv_1d FINAL WHERE asset_id = <canonical USDC>
  -> 0
  ```

  It is our top-preference quote, so canonicalisation makes it the quote on every
  pair it appears in. That is the whole reason [[0165]] needed a peg arm.

  **Half 2 — the quote-side candles cannot price it either, because the
  derivation is CIRCULAR.** Getting USDC's dollar price out of an XLM/USDC candle
  needs XLM's dollar price, and before the oracle window that is *defined* as
  XLM's price in USDC (`ch_enrich.rs`, `pivot_sql`):

  ```sql
  SELECT timestamp, sum(close * volume_base) / sum(volume_base) AS usd
  FROM price_ohlcv_1m WHERE asset_id = <XLM> AND quote_asset_id = <USDC>
  ```

  So `USDC = $1` is baked into the definition of the reference asset. Measured,
  over every USDC-quoted candle before 2026-03-11:

  | implied rate (`close_usd / close`) | candles |
  |---|---|
  | **1.00000000** | **654,291** |

  **One distinct value.** Not clustered near par — a single value, because the
  enrichment peg tier multiplied every one of them by a literal `$1`. Deriving a
  rate from these returns the assumption that produced them, and would arrive
  labelled `'traded'`/`'oracle'` instead of `'peg'` — the same number wearing a
  badge that says "measured". Strictly worse than the honest fallback.

  **Where the objection IS right, and what would actually fix it.** Real USDC did
  deviate — ~$0.88 over the SVB weekend in March 2023 — and our deep history
  shows `$1.0000` for those days. That is real missing information, and it is
  unrecoverable *from inside our data*: during a depeg, "XLM rose 13%" and "USDC
  fell 12%" are the same candle. Breaking the circle needs an anchor OUTSIDE the
  USDC-denominated system — an external historical price feed, or an oracle
  backfill (Reflector's does not reach back). File that if deep history in real
  dollars is ever wanted; the candles cannot supply it.


---

# Implementation Notes — 2026-08-31

## What changed

`packages/prices-clickhouse/schema/views.sql` only. Both `price_usd_series` and
`price_usd_series_1h`, kept byte-identical apart from the grain, as their own
header demands.

Arm B gained a `peg_rate` column; arm A carries `CAST(0 AS Decimal(38, 14))` so
the `UNION ALL` types line up. The outer expression became:

```sql
if(max(is_peg) = 1 AND sum(w) = 0,
   if(max(peg_rate) > 0, max(peg_rate), CAST(1 AS Decimal(38, 14))),
   CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14))) AS close_usd,
CAST(if(max(is_peg) = 1 AND sum(w) = 0,
        if(max(peg_rate) > 0, 'oracle', 'peg'),
        'traded') AS LowCardinality(String)) AS method
```

`max()` picks arm B's rate because every arm-A row carries `0` — as the task
predicted. The `sum(w) = 0` guard is untouched, so a peg member that also trades
as a base still keeps its market value.

**The shape of the view did not change**: same seven columns, same order, same
`Nullable`-free `Decimal(38, 14)`, same one row per (identity, bucket). Verified
against `system.columns` and by a duplicate-key count.

## The resolution rule, and why it is an argMax and not an ASOF

[[0167]] states the rule for this consumer by name: *the newest observation at or
before the **bucket's end**, never an average*. Written literally that is an
`ASOF LEFT JOIN` per candle row. It is written instead as a tiny aggregate joined
by `(identity, bucket)`:

```sql
SELECT …identity…, toStartOfInterval(timestamp, INTERVAL 1 DAY) AS bucket,
       argMax(usd_rate, timestamp) AS usd_rate
FROM prices.usd_rate FINAL WHERE method = 'oracle'
GROUP BY …
```

Three things follow, and they are the reason for the choice:

1. **It is the same value.** The newest observation `<=` bucket end that is not
   older than the bucket start IS the bucket's last observation.
2. **The staleness window becomes the bucket itself** — for free, with no second
   predicate. An unbounded `ASOF` would forward-fill a dead oracle's last reading
   across years of buckets, still labelled `'oracle'`.
3. **It is cheap.** ~87k observations collapse to one row per bucket *before* the
   join, instead of ASOF-ing every arm-B candle row.

`argMax` is a LAST, not a mean — averaging is what 0167 forbids, and the reason
is composition: the daily close must equal the last hourly close of the same day.
It does, and there is a test asserting it against the two views rather than
against a constant.

`toStartOfInterval` is the SAME function the rollup MVs use to build these
buckets (`rollups.sql`), so the flooring agrees under any server timezone rather
than only under UTC.

## Tests

12 green on the prod pin (ClickHouse 26.3.10.60), plus the guards:

| suite | result |
|---|---|
| `views_it` (`--ignored`) | 10 passed, **2 new** |
| `prices-api` `ohlcv_it` (`--ignored`) | 26 passed |
| `prices-clickhouse` lib guards | 28 passed |
| `oracle-worker` + `prices-ingest-core` (peg-set guards) | 65 passed |

**No existing test needed a behaviour change.** That is [[0165]]'s doing: it
wrote its peg tests against *fallback semantics* rather than against the literal
`1`, and pre-committed to a `method` column, exactly so this task would be a
one-expression edit. Both of its three "things 0165 must do" that were in its
control held up. The only edits to existing tests were **doc comments** saying
0168 has now shipped and that the fixture deliberately seeds no `usd_rate` rows.

New tests:

- `peg_fill_publishes_the_measured_rate_and_falls_back_only_without_one` —
  five properties: the measured rate is published and tagged `oracle`; the
  bucket's LAST observation wins; no observation → `$1`/`peg` with **no
  forward-fill into the next bucket**; a `method = 'pivot'` row planted later in
  the same day is ignored; and the daily close equals the last hourly close.
  Asserted on `toString(close_usd)` — the exact decimal, not a float epsilon,
  because a `toFloat64` round-trip would hide loss at the 14th place.
- `a_measured_rate_at_exactly_par_is_labelled_oracle_not_peg` — the
  distinguishability AC. Two buckets that both read `1`, told apart only by
  `method`. No value-based check can cover this, which is the point.

## Design Decisions

### From Plan

1. **`peg_rate` on arm B, `0` on arm A, `max()` in the outer SELECT.** Exactly
   the shape the task specified. Arm A's `0` is what makes `max()` a selector
   rather than a comparison.
2. **`$1` stays the fallback, and stays labelled.** `method = 'peg'` now means
   specifically "no measured rate for this identity in this bucket".
3. **`method = 'oracle'` only.** A [[0154]] `'pivot'` row cannot stand in for a
   measurement; the same choice `current.sql`'s tip surface makes.

### Emerged

4. **The staleness window is ONE BUCKET WIDTH, i.e. the observation must fall
   inside the bucket.** The task said "within the staleness window" without
   fixing one. Enrichment's `FORWARD_FILL_WINDOW_S` is 300 s at a 1m candle's
   own timestamp, which does not transfer to a day-end resolution. A bucket-width
   window is self-describing, needs no constant, scales with the grain, and makes
   the argMax form above exact rather than approximate.
   **Accepted cost:** the NEWEST bucket reads `'peg'` until the first poll lands
   inside it — up to ~5 minutes at the top of each hour (`_1h`) or each UTC day.
   It is a ~0.07{'iss': 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'}tep on a partial bucket and it is visible in `method`.
5. **`ifNull(r.usd_rate, …)` is belt-and-braces; the real discriminator is
   `> 0`.** Prod runs `join_use_nulls = 0`, so an unmatched `LEFT JOIN` yields
   the column DEFAULT (`0` for a Decimal), never `NULL` — a `coalesce()`/`IS NULL`
   form here would be **dead code on prod**, which is the trap that has now bitten
   this codebase on three surfaces. The `> 0` test reads the same under both
   settings. **No `SETTINGS` clause was added**: `prices_reader` is read-only and
   refuses one outright (code 164), which is how /ohlcv 500'd on 2026-08-27.
6. **Keyed on the full natural identity, not hardcoded to USDC.** `current.sql`
   allowlists USDC *by name* because it synthesises a row from nothing. Here the
   peg predicate already fences the set, and the rate join simply follows
   whatever identity passes it — so there is no second copy of the issuer literal
   to drift, and a member added under [[0173]] is priced without touching this
   code. The fence is on the SET, and it is guarded by
   `peg_identities_is_exactly_canonical_usdc` and
   `reflector_resolves_exactly_xlm_and_usdc_and_nothing_else`. The predicate now
   carries a comment saying so.
7. **The AC "applies to every peg asset (USDT too)" was written before
   [[0172]].** Read as "not a USDC special case" — satisfied by 6 — not as an
   instruction to re-add USDT, which the standing condition forbids.

## Issues Encountered

- **[[0170]]'s `/ohlcv` peg series reaches a DIFFERENT value for the same
  bucket.** Not known when this task was written — 0170 shipped after it.
  `ohlcv_peg_series` (`queries_ch.rs`) ASOFs at the bucket's **START**
  (`r.rts <= b.bkt`) with **no staleness bound at all**. So its daily "close" is
  the *previous* day's last reading, and after an oracle outage it forward-fills
  the last known rate indefinitely, still labelled `'oracle'`.
  This view follows the rule `init.sql` states for this consumer by name, which
  is also the only one under which a daily close equals the last hourly close.
  The two therefore disagree by the intraday drift in normal operation and by the
  whole gap after an outage — which is the same "two of our own surfaces
  disagree" defect class that motivated this task, in a new place.
  **NOT fixed here**: it changes a shipped endpoint's published values.
  Spawned [[0246]]. Documented at the join site in `views.sql` so it cannot be
  rediscovered from scratch.
- **`prices.usd_rate` needs no new grant.** The production read grant is
  `GRANT SELECT ON prices.*` — database-wide — so the reader picks the new table
  up automatically. Checked before writing the join, because a missing grant on a
  view's new dependency fails only on prod.

## Future Work

- [[0246]] — reconcile `/ohlcv`'s peg series with this view's resolution rule.

## Remaining before this task can close

The prod internal-consistency AC. It compares against enriched prod candles and
cannot be checked locally. Runbook below.

---

# 📕 DEPLOY RUNBOOK — two `CREATE OR REPLACE VIEW`s

## What this is

Two view bodies, replaced in place. **No table, no data, no MV.** Since [[0134]]
every view in `views.sql` is `CREATE OR REPLACE`, which is atomic — there is no
window where the view does not exist, and nothing to lose if it fails. This is
the lightest deploy class in this repo and deliberately does NOT need 0178's
DROP + recreate ceremony.

Run as the **DDL-capable user** — `prices_reader` and `prices_writer` hold no DDL
grants; on ch-prod-01 schema DDL is an operator action as the container's
`default` user over the loopback native port. CH password from AWS Secrets
Manager, never typed. SQL goes through the operator's `CHQ <<'SQL'` heredoc.

## Step 0 — capture the rollback, on the Hetzner CH host

```sql
SHOW CREATE VIEW prices.price_usd_series;
SHOW CREATE VIEW prices.price_usd_series_1h;
```

Keep both verbatim. **They are the rollback** — paste them back and the old
behaviour returns. Nothing else is needed.

Baseline, to compare after:

```sql
SELECT method, count() AS rows, min(bucket) AS first_bucket, max(bucket) AS last_bucket
FROM prices.price_usd_series
WHERE asset_code = 'USDC' AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
GROUP BY method ORDER BY method;
```

Expect **before**: one row, `peg`, spanning the whole history.

## Step 1 — apply

Paste the two `CREATE OR REPLACE VIEW` statements from the merged
`packages/prices-clickhouse/schema/views.sql` (daily first, then `_1h`), as one
block. Comments travel with them; that is intended — the fences live in the view.

## Step 2 — the split appears

Re-run the baseline query.

Expect **after**: two rows.

| method | expected span |
|---|---|
| `peg` | everything before **2026-03-11**, plus any bucket the oracle sat out |
| `oracle` | from 2026-03-11 to the tip |

Then the values:

```sql
SELECT method, min(close_usd) AS lo, max(close_usd) AS hi, count() AS rows
FROM prices.price_usd_series
WHERE asset_code = 'USDC' AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
GROUP BY method;
```

`oracle` must sit inside **0.999 – 1.001** (measured 1.000086 – 1.000639 over
2026-03 → 2026-08). Anything outside that band means the join matched the wrong
identity — **stop and roll back**. `peg` must be exactly `1`.

## Step 3 — the open AC: agreement with the candles

This is the internal-consistency check that motivates the task. The enrichment
oracle tier bakes the rate into `close_usd` itself, so a USDC-quoted candle's
implied rate is `close_usd / close`:

```sql
SELECT
    p.timestamp                                              AS bucket,
    round(avg(toFloat64(p.close_usd) / toFloat64(p.close)), 8) AS implied_from_candles,
    round(any(toFloat64(s.close_usd)), 8)                    AS published_by_view,
    any(s.method)                                            AS view_method,
    count()                                                  AS candles
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
LEFT JOIN prices.price_usd_series AS s
       ON s.asset_code = 'USDC'
      AND s.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
      AND s.bucket = p.timestamp
WHERE q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND q.contract_address = ''
  AND p.close > 0 AND p.close_usd > 0
  AND p.timestamp >= now() - INTERVAL 14 DAY
GROUP BY bucket
ORDER BY bucket;
```

**PASS:** `implied_from_candles` and `published_by_view` agree to ~1e-4 on
`view_method = 'oracle'` rows — both are the same Reflector feed near the day's
end. Before this change `published_by_view` was a flat `1.00000000` against an
`implied_from_candles` of ~0.9993–1.0007; that gap closing IS the deliverable.

🔴 **Read this before calling a disagreement a failure.** Rows where
`implied_from_candles` is **exactly 1.00000000** are candles the enrichment
**peg tier** priced, not the oracle tier — it multiplies by a literal `$1`. Those
are the *known adjacent gap* this task deliberately does not close (see the
section above), not evidence the deploy went wrong. They are expected to
dominate deep history and to be rare in the last 14 days.

## Step 4 — read it as the reader

The views gained a dependency (`prices.usd_rate`). The prod read grant is
`GRANT SELECT ON prices.*`, so this should pass — confirm rather than assume,
because a missing grant on a new dependency fails ONLY on prod:

```sql
-- as prices_reader
SELECT bucket, close_usd, method
FROM prices.price_usd_series
WHERE asset_code = 'USDC' AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
ORDER BY bucket DESC LIMIT 5;
```

⚠️ No `SETTINGS` clause is in either view, and none may be added: `prices_reader`
is read-only and refuses one before a row runs (code 164) — the 2026-08-27
`/ohlcv` 500.

## Step 5 — cost

Arm B gained a join. The right side is ~87k rows collapsed to one per bucket, so
it should be noise against the existing FINAL scan — but that scan was **never
measured at prod scale** (`views.sql` says so), so measure the delta rather than
trusting the estimate:

```sql
SELECT count() FROM prices.price_usd_series
WHERE bucket >= now() - INTERVAL 30 DAY;
```

Compare the elapsed time against the same query on the captured old definition.
A material regression is not a correctness problem — push the peg set onto the
primary key as `views.sql` describes, or materialise per [[0150]].

## Rollback

Paste back the two `SHOW CREATE VIEW` outputs from Step 0. Atomic, instant, no
data involved.
