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
  - date: 2026-08-04
    status: backlog
    who: okarcz
    note: >
      All findings reproduced locally on CH **26.3.10.60** (the prod pin) —
      scripts in `repro/`, results inline below. **All three are bugs**, and the
      root cause of finding 3's second half is NOT the two-writer version race
      first hypothesised: it is an unguarded `argMax(close_usd, t.timestamp)` in
      **all six rollup MVs**, the same defect pattern as finding 1. The version
      collision is real but is the reason the [[0114]] sweep's repair does not
      stick, not the reason the value is zero. Also measured: removing the
      `close_usd > 0` filter (BE's option B, taken literally) is **worse** than
      keeping it.
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

## Verdict

All five mechanisms below were **reproduced locally on CH 26.3.10.60**, the prod
pin, using the shipped SELECTs verbatim. Scripts and run instructions:
[`repro/`](repro/README.md).

| # | BE finding | Bug? | Root cause |
|---|---|---|---|
| 1 | native XLM `price_usd` = 0 | **Yes** | `argMax(close_usd, …)` with no `> 0` guard in `mv_current_prices` |
| 2 | `price_usd_series*` read cost | **No** — a valid request | but ~2× of the measured scan **is** a bug: [[0139]]'s `asset_id` fan-out |
| 3i | dust print becomes the bucket price | **Yes** | `WHERE close_usd > 0` makes the weighting population depend on enrichment timing |
| 3ii | priced bucket reverts to unpriced | **Yes** | `argMax(close_usd, …)` with no `> 0` guard in **all six rollup MVs** |
| 3ii-b | [[0114]]'s repair does not stick | **Yes** | MV re-append at `sum(version)` overtakes the sweep's `version + 1` |

**One defect pattern explains 1, 3ii and (indirectly) 3i:** `close_usd`'s
"missing" value is `0`, a number that is valid input to every aggregate we run
over it. `argMax` happily returns it, `sum(x*w)` happily weights it, and
`WHERE close_usd > 0` — the one place we do guard — fixes the arithmetic by
silently changing the population instead.

> ⚠️ **Correction to this task's first revision.** Finding 3ii was filed as a
> suspected two-writer version race. That was wrong as a *root cause*: TEST A
> reproduces BE's exact observation with a single writer and no version
> interaction at all. The version collision is real and independently confirmed
> (TEST E) — it is why the sweep's repair does not survive — but it is
> downstream. Fixing only the version arithmetic would leave the zero-propagation
> intact.

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

**Reproduced (TEST C).** Four `_1m` candles for native XLM, the two newest
unpriced — 13:59 because enrichment has not run yet, 14:00 because its quote is
exotic and so will *never* be enriched:

```
asset_id   price_usd_asis   price_usd_if_guarded
       9                0                  0.421
```

One `argMaxIf` recovers the real price. The un-enriched tip is the whole story.

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

**Reproduced (TEST D).** One `_1d` candle on an `asset_id` shared by two
identities, joined exactly as the view does:

```
joined_rows   distinct_candles
          2                  1

asset_code   issuer_address   bucket                 close_usd   volume_base
DUPA         …AAA1            2026-08-03 00:00:00         1.05          5000
DUPB         …AAA2            2026-08-03 00:00:00         1.05          5000
```

Both halves confirmed: **2× read amplification** (BE's "twice") *and* `DUPB`
publishing a price series for a candle it never traded. The prod magnitude
depends on how many of 0139's 3,275 duplicate `asset_id`s carry candles — that
part still needs the query below.

Materializing before fixing this bakes the fan-out into a physical table.

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

**(i) Partial enrichment (13:29) — the filter.** Enrichment runs `rate(1 hour)`,
so a live bucket is routinely part-priced. Whichever rows happen to be enriched
become 100% of the weight. A dust print is the pathological case because it
carries real volume ≈ 0 but a wildly off unit price — which is [[0116]]
(single-trade candles carrying nonsense unit prices, measured up to $29.6M
`close_usd`). 0116 makes the input junk; this filter is what lets one junk row
*be* the answer.

**Reproduced (TEST B).** BE's exact scenario — a 42,038-unit row not yet
enriched beside a 0.764-unit dust print that is, with a fully-enriched 12:00
control:

```
asset_code   bucket                 close_usd          <- the shipped view
yXLM         2026-08-04 12:00:00    0.17002069076055
yXLM         2026-08-04 13:00:00    1.30850000000000   <- 7.7x, BE's number
```

**(ii) An enriched value going back to zero (14:13) — the rollup chain.**
Partial enrichment does not explain a bucket that had a priced row and then had
none; that is data moving **backwards**. The cause is the *same* unguarded
`argMax` as finding 1, one layer down. Every rollup tier carries `close_usd`
forward with

```sql
argMax(close_usd, t.timestamp)            AS close_usd
```

(`rollups.sql:90, 111, 132, 153, 174, 195` — **all six MVs**). `argMax` takes
the value from the *latest* sub-bucket. When the latest sub-bucket is not yet
enriched, the coarse row inherits **0** — discarding the priced sub-buckets
underneath it. A partly-enriched hour does not roll up as partly priced; it
rolls up as **unpriced**.

**Reproduced (TEST A).** One hour of `_15m` sub-buckets, the two earlier ones
priced and the two later ones not, through `mv_ohlcv_15m_to_1h`'s SELECT
verbatim:

```
timestamp             close   volume_base   close_usd_asis   close_usd_if_guarded
2026-08-04 13:00:00   0.172         42000                0                  0.171
```

No second writer, no version interaction, no race — a single pass over the data
returns zero. This is the mechanism behind BE's 14:13 reading.

**(ii-b) And the [[0114]] sweep cannot durably repair it.** The coarse sweep
exists precisely to re-derive `close_usd` on coarse rows, and it wins by
`version + 1` (`repair.rs:20-22`). But `mv_ohlcv_15m_to_1h` re-appends the same
hour **every 15 minutes for 8 hours** (`REFRESH EVERY 15 MINUTE`, window
`now() - INTERVAL 8 HOUR`) at `version = sum(version)` over its `_15m` rows —
and every enrichment event under those sub-rows adds 1 to that sum. Two such
events and the MV overtakes the sweep.

**Reproduced (TEST E):**

```
after the sweep repairs it        close_usd 0.171   version 401
after the next MV refresh         close_usd 0       version 402
what the view publishes now       (empty)
```

The bucket vanishes from the view — BE's observation exactly. So the repair path
we already built is defeated inside the re-aggregation window. This *is* a
distinct defect from the filter and probably deserves its own task, but note the
ordering: **fix the `argMax` and this mostly stops mattering**, because the MV
would re-append a correct value rather than a zero. Fix only the version
arithmetic and the zero-propagation survives untouched.

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
- **⚠️ Their option B, taken literally, is worse than the status quo.** TEST B
  measured it: weighting over all rows with the filter removed returns
  **0.000023** against a true ~0.170, because an unpriced row enters as a zero
  numerator against a full-weight denominator. Their wording ("weight over the
  unenriched rows too **once they land**") suggests they mean *defer until
  enriched*, which is the coverage gate — but the literal reading is a trap and
  the reply should say so.
- **A `status` column beats silent absence.** The header already promises
  value-or-absent semantics classified against `usd_reference`; "partially
  enriched" is a third state that today masquerades as a good value. Consider
  exposing coverage (e.g. `priced_volume_share`) so a consumer can set its own
  bar rather than inheriting ours.

---

## Verification queries (prod, ch-prod-01 — operator-run)

> The **mechanisms** are settled — see `repro/`. These queries no longer decide
> *whether* the bugs are real; they size the **blast radius** on prod, which is
> what the fix priority and the BE reply should be calibrated against. Read-only.

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

**D. Size the zero-propagation across the rollup tiers (finding 3ii):**

```sql
-- how many recent coarse rows carry close_usd = 0 while their own `close` is
-- non-zero — i.e. rows the argMax zeroed rather than rows with no market
SELECT
    'price_ohlcv_1h' AS tbl,
    countIf(close_usd = 0 AND close > 0) AS zeroed,
    count()                              AS total,
    round(countIf(close_usd = 0 AND close > 0) / count(), 4) AS share
FROM prices.price_ohlcv_1h FINAL
WHERE timestamp >= now() - INTERVAL 48 HOUR
UNION ALL
SELECT 'price_ohlcv_1d', countIf(close_usd = 0 AND close > 0), count(),
       round(countIf(close_usd = 0 AND close > 0) / count(), 4)
FROM prices.price_ohlcv_1d FINAL
WHERE timestamp >= now() - INTERVAL 30 DAY;
```

This is the number that decides how much of the historical `close_usd` estate is
affected, and whether a backfill/re-sweep is needed after the MV fix.

---

## Implementation sketch

Ordered by dependency, not by importance. Steps 1 and 2 are the same one-line
defect in two places and should ship together.

1. **Guard the `argMax` in the rollup chain** — `argMaxIf(close_usd,
   t.timestamp, close_usd > 0)` in all six MVs in `rollups.sql`. Carries the
   latest *priced* close instead of a zero. Note this deliberately decouples
   `close_usd` from `close` (they may come from different sub-buckets); that is
   the right trade — an approximately-right USD close beats a fabricated zero —
   but **write it down in the header**, because the two columns silently ceasing
   to be same-row is exactly the kind of thing that bites a future reader.
   Delivery is blocked on [[0142]]: `rollups.sql` is `IF NOT EXISTS` with no
   `DROP`, so editing the file and re-applying **changes nothing** on prod.
2. **Guard the `argMax` in `mv_current_prices`** (finding 1) — same fix, and the
   decision belongs to [[0135]]. Cheap; blocked only on the contract call.
3. **Finding 3i — the coverage gate.** Replace the bare `close_usd > 0` filter in
   `price_usd_series` / `price_usd_series_1h` with a volume-share gate, and/or
   expose `priced_volume_share` so consumers set their own bar. Pick the
   threshold from query C's real distribution, not from taste. Coordinate with
   [[0118]] and [[0131]] so we ship one definition of "priced enough", not three.
   Still needed after step 1 — the gate covers the case where the *base* table's
   own rows are unpriced, which no rollup fix can reach.
4. **Sweep/MV version ownership** (3ii-b) — after step 1 this is much less
   urgent, but two writers with incompatible version arithmetic on one column is
   a latent trap worth closing. Own it in a follow-up.
5. **Finding 2 — materialize**, under the population rule settled in step 3 and
   after 2a ([[0139]]) is fixed. Refresh mode per §2c; delivery per [[0142]].

Steps 1–5 are independently shippable. Splitting them into separate tasks is
expected — this task is the triage and the BE-facing contract, not the
implementation.

## Acceptance Criteria

- [x] Mechanisms reproduced on CH 26.3.10.60 (`repro/`) — all three findings are
      bugs; root cause of 3ii corrected from the version race to the unguarded
      `argMax`.
- [ ] Verification queries A–D run on prod and their results recorded here, to
      size the blast radius (how many assets, how much of the coarse estate).
- [ ] BE has a written answer covering: 0039's actual status and the real owner
      of native XLM pricing; **why "wait until every row is enriched" cannot
      terminate**; **why removing the filter outright is worse than keeping it**;
      and what we will ship instead.
- [ ] No coarse row carries `close_usd = 0` while its own `close > 0` and a
      priced sub-bucket exists underneath it — regression test on the prod pin.
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
- **The `argMax` fix has a historical tail.** It corrects the coarse tables from
  the moment it lands; it does not retroactively repair rows already zeroed and
  aged out of the MVs' re-aggregation windows. Query D sizes that estate. The
  [[0114]] sweep is the existing repair path and should pick them up once it can
  no longer be clobbered (step 4) — confirm rather than assume.
- **`zero-as-missing` is the design choice underneath all of this.** `close_usd`
  is `Decimal(38,14) DEFAULT 0` on a non-nullable column (`init.sql:114`), so
  "not yet priced", "will never be priced" and "genuinely worth nothing" are one
  value. Every bug in this task is a different aggregate meeting that value. A
  `Nullable(Decimal)` or a companion status column would make the whole class
  unrepresentable — expensive and invasive now, but worth an ADR before the next
  surface is built on the same footing. Note `views.sql` already promises
  consumers value-or-absent semantics, which is the contract the storage does
  not actually implement.
