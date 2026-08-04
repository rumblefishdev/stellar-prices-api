---
id: "0144"
title: "BE 0199 report: close_usd read surfaces publish a wrong answer while enrichment is in flight, and price_usd_series won't scale"
type: BUG
status: backlog
related_adr: []
related_tasks:
  ["0135", "0139", "0116", "0114", "0061", "0072", "0118", "0131", "0138", "0142", "0143"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "be-interop", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../packages/enrichment-worker/src/repair.rs"
history:
  - date: 2026-08-04
    status: backlog
    who: okarcz
    note: >
      Filed from a BE-team report (their task 0199, LP analytics wiring),
      measured on prod 2026-08-04. Three findings against the surfaces we handed
      them in [[0061]]/[[0072]]: (1) `current_price_usd.price_usd = 0` for
      native XLM itself, (2) a request to promote `price_usd_series*` to the
      materialized table our own views.sql header pre-authorized, (3)
      `price_usd_series*` returning a dust print as the whole bucket price
      because the `close_usd > 0` filter changes the weighting population
      mid-enrichment. Findings 1 and 3 share one root cause; finding 2 is
      independent but ordered behind 3.
---

# BE 0199 report — three defects in the USD read surfaces

## Summary

BE wired their LP analytics (their task 0199) to `prices.*` and measured three
problems on prod on 2026-08-04. Two are correctness bugs in what we publish, one
is a performance request our own schema header already pre-authorized.

The common thread behind findings 1 and 3 is structural: **`close_usd` is baked
by a separate, lagging enrichment pass, and every read surface treats
"not yet enriched" and "no USD price exists" as the same value — zero.** Both
surfaces then filter or aggregate on that zero, so a partially-enriched window
produces a confidently wrong number rather than an absent one.

This is our first external consumer measuring these views, so treat their
numbers as the ground truth about what we actually ship.

---

## Finding 1 — `current_price_usd.price_usd` is 0 for native XLM

**BE reported:** the updater ticks (3,316 assets, fresh `updated_at`) but XLM
carries the unavailable-sentinel, so any spot-based consumer with an XLM leg
reads nothing. They have switched to `price_usd_series_1h`'s last close as a
workaround, and ask whether native pricing is "in 0039's scope soon".

### Mechanism

`mv_current_prices` (`packages/prices-clickhouse/schema/current.sql`) computes
the headline price in its `unfiltered` CTE:

```sql
argMax(close_usd, timestamp)      AS price_usd
```

over `price_ohlcv_1m` for the trailing 24h, **with no `close_usd > 0` guard** —
unlike every neighbouring CTE (`per_asset`, `ref_7d`, `open_24h`), all of which
filter. So the newest 1m candle wins outright, and if that candle is not priced
the asset publishes 0.

XLM is the worst possible case for that, for two compounding reasons:

1. It is the most-traded asset, so it almost always has a candle in the most
   recent minute — i.e. a candle newer than the last enrichment pass. Enrichment
   runs `rate(1 hour)` in prod (`infra/envs/production.json`), so the tip is
   un-enriched for most of every hour.
2. XLM has the widest set of counter-assets, so its newest candle is often an
   exotic-quote pair (quote ∉ {USDC, USDT, XLM}, no oracle) which enrichment
   documents as the **permanent** deep-history floor — it will never be priced
   at all (`ch_enrich.rs`, `count_remaining_at_volume_zero` docs).

For XLM, then, this is not intermittent — it is close to chronic.

### This is [[0135]], now measured on the one asset that matters most

0135 already carries this exact failure mode as its second scope item (21 of
3,022 assets publishing `price_usd = 0` while `vwap_24h` and `sources` carry a
real price). [[0072]]'s rollout note and the `current.sql` comment block record
XLM as one of the affected assets on 2026-08-03. [[0138]] fixed the *derived*
symptom (`change_24h_pct` fabricating −100) by guarding the numerator, and
deliberately left `price_usd` itself on the 0 sentinel because that decision
belongs to 0135.

**The fix is one line** — `argMaxIf(close_usd, timestamp, close_usd > 0)` — but
it changes the published contract (the headline price becomes "latest *priced*
close" rather than "latest close"), which is precisely the decision 0135 exists
to make. Decide it there; this task's job is to carry BE's measurement into it
and raise its urgency.

### Answer BE is owed on the 0039 question

**[[0039]] is completed and archived, and the Current Price Updater it named was
eliminated, not shipped.** Open Q#1 resolved on 2026-06-25: 5 of 6
`current_prices` columns are SQL-derivable, so the `rate(1m)` Lambda became the
refreshable MV `prices.mv_current_prices`. The thing BE observes "ticking" is
that MV. So "is native pricing in 0039's scope" has no yes/no answer as posed —
the owner is 0135 against `current.sql`, and their XLM measurement is the
strongest argument yet for doing it.

Their `price_usd_series_1h` workaround is sound and should stay in place until
0135 lands — but see finding 3, which affects that surface too.

---

## Finding 2 — materialize `price_usd_series*` (their §6 request)

**BE measured:** bucket-range pushdown works (1.89M of 19.6M rows for a 90-day
window), but identity cannot push down because the key columns are computed, so
a 104-week chart window scans every asset's daily candles twice — **70.7M read
rows / 4.6 s / 2.1 GiB per uncached request**. They ask for an identity-keyed
materialized table `ORDER BY (asset_kind, asset_code, issuer_address,
contract_address, bucket)`.

**The pre-authorization is real and we should honour it.** Both our schema
header and the design note say so in as many words:

- `views.sql:197-198` — "promote to a materialized table only if measured read
  latency demands it (design note §6)".
- `R-historical-usd-close-design.md` §6.3 — "no new physical table required;
  promote to a materialized `price_usd_1d` only if read latency demands it."

BE has now supplied the measurement that trips the trigger. Three things must be
settled before it is built, though:

### 2a. Check whether half that scan is [[0139]], not physics

Their "**scans every asset's daily candles twice**" is suspicious. Both
`price_usd_series` and `price_usd_series_1h` join

```sql
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
```

which is the *identical* join shape 0139 filed against `current_price_usd`:
`prices.assets` is `ReplacingMergeTree(updated_at) ORDER BY (asset_code,
issuer_address, contract_address)`, so `FINAL` dedups on **natural identity, not
`asset_id`** — and 0139 measured **3,275 `asset_id`s mapped to two or more
natural identities** on prod. A ~2× row multiplication is exactly what that
produces.

If confirmed, the consequence here is worse than 0139's duplicate rows. In
`price_usd_series` the fan-out feeds a `GROUP BY` on identity, so **one
`asset_id`'s candles are attributed to every natural identity sharing that id** —
a second identity would publish a price series it never traded. The volume
weighting itself is invariant to uniform duplication, so the *numbers* stay
right for the real identity; the *rows* do not.

Materializing before checking this bakes the fan-out into a physical table.
Check it first (query below).

### 2b. It must be ordered behind finding 3

A materialized table built from today's `close_usd > 0` population inherits
finding 3's defect and makes it durable — a dust-print bucket becomes a stored
fact instead of a transient view artifact. Settle the population rule first,
then materialize under it.

### 2c. The refresh mode is the dangerous part, not the DDL

This lands in the blast radius of everything we learned this month:

- [[0095]]/[[0090]] — a refreshable MV with a `TO` table refreshes as an **atomic
  REPLACE** over its window; that is what wiped the coarse tables. `APPEND` +
  `sum(version)` was the fix.
- But `APPEND` is wrong here in the obvious form: a bucket's `close_usd` legitimately
  *changes* as enrichment lands, so a naive append leaves both versions and lets
  RMT version arithmetic decide — which is exactly the collision in finding 3.
- [[0142]] — `rollups.sql`-style `CREATE MATERIALIZED VIEW IF NOT EXISTS` edits
  **silently no-op** on a provisioned target. Whatever 0142 settles on is the
  delivery mechanism.
- [[0143]] — no `DEPENDS ON` anywhere in the cascade; a new tier reading a rollup
  inherits that race.

A plain scheduled rebuild of a bounded recent window (rather than an MV) may be
the cheaper, safer answer. Decide explicitly; do not default.

---

## Finding 3 — a dust print can become the whole bucket price

**BE measured on yXLM (`GARDNV3Q…`), 2026-08-04:** at 13:29 the 13:00 hour's
only enriched row was a **0.764-unit dust print at 1.3085 USD**, so
`price_usd_series_1h` returned 1.3085 against ~0.170 in every neighbouring hour
— **7.7×**. By 14:13 every 13:00 row read `close_usd = 0` and the bucket had
vanished from the view entirely.

They are explicit, and correct, that **the weighting maths is sound** — the same
kind of dust print sits in the 12:00 bucket beside 42,038 units of real volume
and moves the weighted close by nothing. It is the `close_usd > 0` filter
changing the *population*.

```sql
-- views.sql, price_usd_series and price_usd_series_1h
CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base))
     / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38,14)) AS close_usd
FROM prices.price_ohlcv_1d AS p FINAL   -- (_1h in the hourly variant)
...
WHERE p.close_usd > 0
```

The filter was written for a good reason — an un-enriched row would drag the
weighted mean toward zero — but it silently makes the denominator
`sum(volume_base)` **over the enriched subset only**, which is a different and
arbitrary population from one minute to the next.

### There are two distinct mechanisms here, and BE saw both

**(i) Partial enrichment (13:29).** Enrichment runs `rate(1 hour)`, so a live
bucket is routinely part-priced. Whichever rows happen to be enriched become
100% of the weight. A dust print is the pathological case because it carries
real volume ≈ 0 but a wildly off unit price — which is [[0116]] (single-trade
candles carrying nonsense unit prices, measured up to $29.6M `close_usd`).
0116 makes the input junk; this filter is what lets one junk row *be* the answer.

**(ii) An enriched value going back to zero (14:13).** Partial enrichment does
not explain a bucket that had a priced row and then had none — that is data
moving **backwards**. Suspected cause: `price_ohlcv_1h` has **two writers on
`close_usd` with incompatible version arithmetic**.

- The rollup MV `mv_ohlcv_15m_to_1h` appends the bucket with
  `argMax(close_usd, …)` from `_15m` and `version = sum(version)`
  (`rollups.sql:98-114`).
- The [[0114]] coarse sweep re-enriches the same coarse rows in place and wins by
  **`version + 1`** (`repair.rs:20-22`).

For a *live, still-accumulating* bucket, `_15m` keeps gaining rows every refresh,
so the MV's `sum(version)` keeps climbing and can overtake — or tie — the
sweep's `version + 1`, re-appending `close_usd = 0` on top of the swept value.
`rollups.sql:29` states outright that the RMT tie-break "is not contractual".
**Unverified — this is the hypothesis to test first**, and if it holds it is a
separate defect from the filter and probably deserves its own task.

### BE's two proposed options — one is unimplementable as stated

They ask: exclude a bucket until all its rows are enriched, **or** weight over
the unenriched rows too once they land.

- **"Until all rows are enriched" cannot terminate.** Enrichment documents a
  **permanent** exotic-quote floor: candles whose quote is not USDC/USDT/XLM and
  which have no oracle keep `close_usd = 0` forever, by design
  (`ch_enrich.rs:31-32`). Any bucket containing one such row would be suppressed
  in perpetuity. This must be told to BE plainly — it is the kind of gate that
  looks fine in a test and strands real assets on prod.
- **A coverage gate is the implementable version of their intent:** publish the
  bucket only when the enriched rows account for ≥ X% of the bucket's
  `volume_base` (or `volume_quote`). That prices a bucket as soon as its real
  volume is priced, ignores a permanently-unpriceable dust tail, and — being a
  weight-share test rather than a row-count test — is immune to the dust-print
  case by construction. [[0131]] already proposes exactly this shape as a
  pre-roll gate; the same predicate belongs in the read surface.
- **Worth pairing with [[0118]]** (`min_volume_usd` inclusion threshold), which
  drops dust rows before they can be weighted at all. Coverage gate and dust
  threshold are complementary, not alternatives.
- **A `status` column beats silent absence.** The header already promises
  value-or-absent semantics classified against `usd_reference`; "partially
  enriched" is a third state that today masquerades as a good value. Consider
  exposing coverage (e.g. `priced_volume_share`) so a consumer can set its own
  bar rather than inheriting ours.

---

## Verification queries (prod, ch-prod-01 — operator-run)

Run before designing the fix. Read-only.

**A. Confirm the XLM tip is un-enriched and identify its quote (finding 1):**

```sql
SELECT p.timestamp, p.source, q.asset_code AS quote, p.close, p.close_usd, p.volume_base
FROM prices.price_ohlcv_1m AS p FINAL
LEFT JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
WHERE p.asset_id = (
        SELECT asset_id FROM prices.assets FINAL
        WHERE asset_code = 'XLM' AND issuer_address = '' AND contract_address = '' LIMIT 1)
  AND p.timestamp >= now() - INTERVAL 2 HOUR
ORDER BY p.timestamp DESC
LIMIT 20;
```

Expect: the newest rows carry `close_usd = 0`; older rows carry a real value.
Note which quote the newest row uses — if it is an exotic quote, that row will
*never* be enriched.

**B. Confirm/deny the [[0139]] fan-out in `price_usd_series` (finding 2a):**

```sql
-- how many candle rows the join multiplies out
SELECT
    count()                                        AS joined_rows,
    countDistinct(p.asset_id, p.timestamp, p.source, p.quote_asset_id) AS distinct_candles
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.timestamp >= now() - INTERVAL 104 WEEK;
```

A ratio near 2.0 confirms BE's "twice" is our fan-out, not their query.

**C. Reproduce the population shift on a live bucket (finding 3i):**

```sql
SELECT
    p.timestamp AS bucket,
    count()                                        AS rows_total,
    countIf(p.close_usd > 0)                       AS rows_priced,
    sum(p.volume_base)                             AS vol_total,
    sumIf(p.volume_base, p.close_usd > 0)          AS vol_priced,
    sumIf(p.volume_base, p.close_usd > 0) / nullIf(sum(p.volume_base), 0) AS priced_volume_share
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE a.asset_code = 'yXLM' AND a.issuer_address = 'GARDNV3Q…'   -- full G-address
  AND p.timestamp >= now() - INTERVAL 12 HOUR
GROUP BY bucket ORDER BY bucket DESC;
```

`priced_volume_share` is the proposed gate's input — check what it looks like
across a normal day before picking X.

**D. Test the version-clobber hypothesis (finding 3ii):**

```sql
-- per-row versions in a recent live bucket, WITHOUT final — shows both writers
SELECT timestamp, source, quote_asset_id, close_usd, version, _part
FROM prices.price_ohlcv_1h
WHERE asset_id = (SELECT asset_id FROM prices.assets FINAL
                  WHERE asset_code = 'yXLM' AND issuer_address = 'GARDNV3Q…' LIMIT 1)
  AND timestamp >= now() - INTERVAL 4 HOUR
ORDER BY timestamp DESC, version DESC;
```

Looking for: a row with `close_usd > 0` at version V sitting under a row with
`close_usd = 0` at version ≥ V. That is the clobber, and it makes 3ii a separate
bug from the filter.

---

## Implementation sketch

Ordered by dependency, not by importance.

1. **Run the verification queries.** They decide whether 3ii and 2a are real, and
   they change the scope of everything below.
2. **Finding 3ii, if confirmed** — one owner for `close_usd` per coarse row.
   Either the rollup MV must not overwrite a swept value (carry the swept version
   forward / exclude already-priced rows from re-append) or the sweep must win
   unconditionally. Spawn as its own BUG; it is a data-regression, not a view bug.
3. **Finding 3i — the coverage gate.** Replace the bare `close_usd > 0` filter in
   `price_usd_series` / `price_usd_series_1h` with a volume-share gate, and/or
   expose `priced_volume_share` so consumers can set their own bar. Pick X from
   query C's real distribution, not from taste. Coordinate with [[0118]] and
   [[0131]] so we ship one definition of "priced enough", not three.
4. **Finding 1 — decide [[0135]].** Guard the `argMax` in `mv_current_prices`.
   Cheap; blocked only on the contract decision.
5. **Finding 2 — materialize**, under the population rule settled in step 3 and
   after 2a is resolved. Refresh mode per §2c; delivery per [[0142]].

Each of 2–5 is independently shippable. Splitting them into separate tasks once
the verification lands is expected — this task is the triage and the BE-facing
contract, not the implementation.

## Acceptance Criteria

- [ ] Verification queries A–D run on prod and their results recorded in this
      task; 2a and 3ii confirmed or ruled out.
- [ ] BE has a written answer covering: 0039's actual status and the real owner
      of native XLM pricing; **why "wait until every row is enriched" cannot
      terminate**; and what we will ship instead.
- [ ] `price_usd_series` / `price_usd_series_1h` cannot return a bucket whose
      published price rests on a negligible share of the bucket's volume — with
      a regression test on CH **26.3.10.60** that reproduces BE's yXLM case.
- [ ] A bucket that is fully unpriceable is absent; a bucket that is *pending*
      enrichment is distinguishable from one that is *priced* — not conflated.
- [ ] No enriched `close_usd` can be overwritten by a later zero (or the
      mechanism is documented as impossible, with evidence).
- [ ] If materialized: identity-keyed as BE requested, refresh mode justified
      against [[0095]], and the [[0142]] no-op trap accounted for so the DDL
      actually lands on prod.
- [ ] BE re-measures the 104-week window and confirms the seek.

## Notes

- **Do not merge this into [[0135]].** 0135 owns one column on one surface;
  findings 2 and 3 are a different surface with a different consumer contract.
  Finding 1 should be *resolved* in 0135, referencing BE's measurement.
- BE's `price_usd_series_1h` workaround for finding 1 routes them straight into
  finding 3. Tell them: until the coverage gate ships, a single-hour close can be
  a dust print — prefer a multi-hour median, or check that neighbouring hours
  agree before trusting a spot value.
- The dust-print exposure is not confined to these views. [[0116]] documents the
  same junk candles reaching every OHLCV granularity, so `/ohlcv` has it too;
  the difference is that the view's filter turns a junk *row* into the *whole
  bucket's* answer.
- Their measured numbers are worth keeping: 19.6M daily candle rows total, 1.89M
  for a 90-day window, 70.7M read rows / 4.6 s / 2.1 GiB for 104 weeks, 3,316
  assets ticking in `current_prices`. Useful baselines for whatever we ship.
