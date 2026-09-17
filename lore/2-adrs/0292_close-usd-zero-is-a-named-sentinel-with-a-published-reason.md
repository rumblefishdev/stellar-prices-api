---
id: "0292"
title: "`close_usd = 0` stays the storage sentinel for 'no USD value' — its four meanings are named, the reason is computed at read time and published beside the number, and no NULL replaces a number a consumer reads today"
status: proposed
deciders: [akot]
related_tasks: ["0151", "0144", "0146", "0147", "0154", "0167", "0171", "0198", "0286", "0139"]
related_adrs: ["0287", "0011"]
tags: [clickhouse, schema, contract, read-surface, enrichment, data-correctness]
links:
  - "../1-tasks/active/0151_DOCS_adr-close-usd-zero-as-missing/README.md"
  - "../../docs/database-schema/close-usd-zero-guardrails.md"
  - "../../packages/prices-clickhouse/schema/init.sql"
  - "../../packages/prices-clickhouse/schema/views.sql"
  - "../../packages/prices-clickhouse/schema/current.sql"
  - "../../packages/prices-clickhouse/src/rollup_sql.rs"
  - "../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../packages/prices-api/src/assets/queries_ch.rs"
history:
  - date: "2026-09-17"
    status: proposed
    who: akot
    note: >
      Written from task 0151 after a code audit of every site that reads or
      writes the zero (three passes, on the 0286 code) and four research passes
      on how exchanges, data vendors, oracles and API guidelines encode a
      missing price. Six decisions, each checked against outside practice; the
      rate-table question was already decided 2026-08-06 and is recorded, not
      re-opened.
---

# ADR 0292: `close_usd = 0` is a named sentinel with a published reason

**Related:**

- [Task 0151: ADR — close_usd zero-as-missing](../1-tasks/active/0151_DOCS_adr-close-usd-zero-as-missing/README.md)
- [ADR 0287: candle prices come from price-forming fills](0287_candle-prices-come-from-price-forming-fills-and-a-windowed-close.md)
- [Guardrail inventory (living document)](../../docs/database-schema/close-usd-zero-guardrails.md)

---

## Context

`close_usd` is `Decimal(38, 14) DEFAULT 0` on a non-nullable column of all seven
candle tables. It is not a stored fact: every enrichment tier computes it as
`close × <USD rate of the quote asset at that time>` and re-inserts the row at
`version + 1`. Until that happens — or when it never can — the column reads 0.

So one value states four different things:

| # | Meaning | Lifetime | Told apart today by |
|---|---|---|---|
| 1 | **Pending** — enrichment has not reached the row | minutes to hours | row age only |
| 2 | **Unpriceable** — the quote asset has no oracle and no reference market | until one appears | inference ("a pass made no progress", the frontier's `Exhausted`) |
| 3 | **Genuinely worth zero** | permanent | nothing — the code assumes it does not happen |
| 4 | **No price in this bucket** — no price-forming fill, so `close = 0` and `close_usd` can only be `rate × 0` (ADR 0287) | permanent, and correct | `pf_trade_count = 0` |

Every defect in task 0144's chain was a different aggregate meeting that value as
if it were a number: `argMax(close_usd, ts)` returned it and discarded the priced
rows under it (0146, 0135); a weighted mean either weighted it as a real zero or,
filtered with `WHERE close_usd > 0`, silently changed its own population so that
one 0.764-unit print became 100 % of an hour's weight, 7.7× off (0147); a
zero-weight group published `Decimal128::MIN` or raised (0171, 0198). Meaning 4
arrived with 0286 and produced two more within a day of being audited (0286
decisions 14 and 15).

Two constraints are not ours to change:

- **The consumer.** BE, 2026-08-06: *"a NULL renders as a dash and removes the
  pool from every USD view we have."* They chose stale-but-real over absent.
- **The identifiers.** `asset_id` has genuine collisions (task 0139), so any new
  authoritative structure keyed on it inherits them.

## Decision

1. **The sentinel stays in storage.** `close_usd` remains non-nullable
   `DEFAULT 0`, written in place. It is not made `Nullable`, and the USD rate is
   not normalised into a schema-wide rate table with `close_usd` as a derived
   cache (that option was adopted narrowly for [[0154]]/[[0167]] on 2026-08-06
   and rejected as a refactor; see Alternatives). How absence is *published* is a
   separate question, decided in §5.

2. **The "no price" marker is `pf_trade_count = 0`; `close = 0` is its
   consequence.** A reader that needs to know whether a bucket has a price asks
   the count, not the price. One exception, until 0286 phase 3 re-ingests the
   history: a legacy row carries `pf_trade_count = trade_count` from the column
   DEFAULT while its `close` underflowed to 0, so every price gate keeps its
   second term (`close >= 1e-12`, `rollup_sql::PRICE_FORMING_CHILD`). A bucket
   with no price keeps its volume and trade count — excluded from price, counted
   in volume.

3. **The reason is computed at read time, not stored.** A published row is
   classified from columns it already has:

   | Status | Rule |
   |---|---|
   | `no_price` | `pf_trade_count = 0` |
   | `priced` | `close >= 1e-12 AND close_usd >= 1e-12` |
   | `unpriceable` | not priced, and the quote asset has no conversion path (identity of the quote asset — dictionary data, not per row) |
   | `pending` | not priced, and a conversion path exists |

   A value carried forward is published with the timestamp of the candle it came
   from (`as_of`). A per-row `usd_status` column is **not** added now; it is
   recorded as an option with a window (see Alternatives).

4. **One definition of "priced", and the coverage it feeds.** A candle is priced
   iff both legs clear the precision floor — `close >= 1e-12 AND
   close_usd >= 1e-12` — the line the rollups (`RATE_BEARING_CHILD`) and `/ohlcv`
   (`convertible`) already draw. For [[0147]]'s gate:
   `priced_volume_share = Σ pf_volume(priced) / Σ pf_volume(eligible)`, where
   *eligible* is a candle whose quote asset has a conversion path. Unpriceable
   legs and dust-only candles are outside the denominator: neither can ever be
   converted, so inside it they would be a permanent "missing" share. The gate is
   the default, the share is always on the wire, and an absolute floor sits
   beside the ratio so that 100 % of a 0.764-unit bucket still fails. The
   threshold X is measured on our own fully-enriched history after the 0286
   rollout — no outside methodology has a number to copy.

5. **What a consumer sees, per surface — written down, not unified.**

   | Surface | No USD price for the bucket |
   |---|---|
   | `price_usd_series*`, `usd_reference*` | the row is absent (NULL after the consumer's LEFT JOIN) |
   | `/ohlcv` | bucket present, price fields `null`, volume and counts present, `pf_trade_count` says why |
   | `current_prices`, `current_price_usd`, `/price`, `/assets` | the last priced close within 24 h is carried; only with none does a literal `0` appear, with `method = ''` |

   The literal `0` stays — replacing it with `null` in place breaks the consumer's
   rule and is a breaking change by any API-versioning standard. It gains two
   additive fields: `price_status` (`priced` | `carried` | `unpriced`) and `as_of`
   (the carried candle's own timestamp; today `updated_at` is the MV refresh time
   and no column carries the price's age). `0 → null` is recorded as a future
   **versioned** change. Storage-level absence never becomes a published NULL
   where a number is published today.

6. **The ADR holds the invariants; the inventory lives beside the code.** This
   record states what must stay true and how it is confirmed (below). The
   site-by-site list of every reader and writer of the zero, its guard and the
   test that pins it is a living document,
   [`close-usd-zero-guardrails.md`](../../docs/database-schema/close-usd-zero-guardrails.md),
   changed in the same PR as the code it describes. Invariants are asserted on
   stored data by a scheduled query in `rollup-freshness-probe` with an alarm —
   **not** by ClickHouse `CHECK` constraints.

## Confirmation

Invariants, and the mechanism that confirms each:

| Invariant | Confirmed by |
|---|---|
| No aggregate over `close_usd`, or over `close` as a price, runs without a guard that excludes the sentinel | the guardrail inventory; unit tests on every generated rollup/pre-roll rendering (`rollup_sql.rs`); `no_preroll_script_uses_an_unguarded_argmax_on_close_usd` |
| A rate is never derived from a value under the precision floor | `rollup_pf_it` (both legs), `ohlcv_it` (`…at_the_decimal_floor`) |
| The shipped MV bodies are the generator's | text-equality pins in `prices-clickhouse/src/lib.rs`; the 0142 drift detector on deployed definitions |
| A row no statement can change is not re-selected forever | `a_dust_only_minute_is_priced_once_and_is_never_reselected`; `the_external_reset_never_reopens_a_candle_that_has_no_price`; the zero-product case — test added by task 0151 |
| `pf_trade_count = 0 ⇒ close = 0` and `close_usd > 0 ⇒ close > 0` on stored rows | scheduled assertion in `rollup-freshness-probe` (added by task 0151) |
| Surfaces agree on the same row, or disagree only as §5 says | cross-surface test added by task 0151; extended by [[0147]] |

## Consequences

### Positive

- The bug class has a definition: a reader of `close_usd` is wrong unless it can
  say which of the four meanings it excludes and how.
- BE can tell *not yet* from *never* from *no price* without us storing it, and
  can see how old a carried price is.
- [[0147]] and [[0154]] get one definition of "priced" instead of three.
- No migration of seven tables, six MVs, five re-inserting statements and the
  pre-rolls — a month after 0286 touched all of them.

### Negative

- The sentinel has no system support. Every new reader, view or aggregate is a
  fresh chance to repeat the bug, and only the inventory and its tests stand in
  the way. With `Nullable`, `argMax` and `avg` would be right with no guard.
- `unpriceable` is a claim as of now: a reference market can appear, and rows
  classified by identity change status retroactively.
- A consumer that ignores `price_status` still reads `0` as a price. The fields
  document the hazard; they do not remove it.
- Meaning 3 stays unrepresentable. Accepted: no asset we index is worth exactly
  zero at 14 decimal places while trading.

### Follow-ups, not part of this ADR's delivery

- `price_status`, `as_of` and `priced_volume_share` on the wire — [[0147]] (or a
  task spawned from it). This ADR decides them; it does not ship them.
- Frontier oscillation on floor months (`Exhausted → Pending` every recheck,
  with a misleading "gained work" warning) — accepted as noise until measured.

## Alternatives considered

### `Nullable(Decimal)` — REJECTED

Makes the class unrepresentable: `argMax` skips NULL in both arguments, `-OrNull`
returns NULL on empty input, and Kimball's guidance is that "null-valued
measurements behave gracefully in fact tables". Dune stores `amount_usd` as NULL
at DEX-wide scale. Rejected because (a) the headline benefit has no consumer —
BE removes a pool on NULL, so every surface would need a fallback anyway; (b)
ClickHouse's own documentation: "Using `Nullable` almost always negatively
affects performance", with a separate NULL-mask file per column, on the hottest
column of a fact table read by six MVs; (c) the bugs it dissolves were one-line
guards, all shipped; (d) a partial move breaks MVs loudly — a Nullable
expression into a non-nullable column.
**Revisit if** a second consumer appears that wants absence, or a guard regression
reaches production despite the inventory.

### Schema-wide USD rate table, `close_usd` as a derived cache — REJECTED (2026-08-06), recorded

The strongest case for it is ClickHouse's denormalisation warning: one rate change
rewrites many rows, which is what task 0268's 9.94 M-candle campaign was. Against:
storing the converted amount at write time is what Kimball (a pair of columns per
financial fact, converted in ETL), dbt practice, Dune and the Uniswap subgraphs all
do; a query-time JOIN builds the right table in memory on top of `FINAL`; our
pivot rate is itself derived from candles, so the table would be a second cache,
not a source of truth; and it would be keyed on `asset_id`, which collides
([[0139]]). Adopted narrowly for [[0154]] via [[0167]], keyed on natural identity.
**Revisit if** [[0139]]'s repair needs a fact-table migration anyway.

### Per-row `usd_status Enum8` in storage — DEFERRED, with a window

Two of the four research passes recommended it: it is Darwen's *reason* column,
it costs no NULL mask, guards become `usd_status = 'priced'`, and meaning 3
becomes representable. Every system we found that tells a consumer *why* a value
is missing has such a channel — Pyth's `PriceStatus` enum beside `prev_price` /
`prev_timestamp` is the verified example; the research notes list LSEG's
Ok/Suspect state, FIX tag 276 and CoinGecko's `is_stale` as well. Not now, because the Rust writer is name-routed and a
statement that omits the column silently takes its DEFAULT — the `pf_trade_count`
trap, freshly paid for; because a status needs an aggregation rule in six MVs;
and because `unpriceable` is not permanent.
**Decide before 0286 phase 3 starts.** That run rewrites every row of history —
the only cheap moment to add a column the writers must all carry. Input: whether
read-time classification proved reliable in the first week after the 0286 rollout.

### ClickHouse `CHECK` constraints — REJECTED

They catch every writer, including ad-hoc `INSERT … SELECT`. But a violated CHECK
fails the whole insert: one bad row stalls a refreshable MV's tier, turning a
data defect into a freshness incident — the failure 0137's alarm exists for.
Whether the refreshable-APPEND path evaluates them is undocumented. A scheduled
assertion detects the same thing with no write-path risk.

### Publish `null` instead of `0` on the current-price surfaces, in place — REJECTED

No surveyed API publishes `0` for an unknown price, and Google AIP-149 names the
ambiguity ("0 is distinct from no rating"). But changing how an existing field is
populated is breaking under AIP-180 ("must not change visible behavior or
semantics"), GitHub's and Stripe's rules, and it is exactly what BE asked us not
to do. Additive fields now; `null` only behind a version.

### One missing-value encoding API-wide — REJECTED

No guideline we read demands it; Zalando and Azure require only that `null` and
absent mean the same, which holds. Omitting an empty interval (Coinbase: "No data
is published for intervals where there are no ticks") and returning it with null
prices beside real volume and count (Kaiko) are both mainstream; our series views
match the first and `/ohlcv` the second. The research notes record further
examples on each side, and one vendor documenting a different rule per asset
class, which we did not re-verify.

## References

Read 2026-09-17; every quote and claim attributed to a source in this list was
checked against the page. Anything attributed to "the research notes" was not.

- ClickHouse — [Nullable](https://clickhouse.com/docs/sql-reference/data-types/nullable), [argMax](https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/argmax)
- Kimball — [nulls in fact tables](https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/fact-table-null/), [multiple currencies](https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/multiple-currencies/)
- Kaiko — [OHLCV: null prices, zero volume](https://docs.kaiko.com/rest-api/cefi-spot-market-data/trade-aggregations/trade-count-ohlcv-and-vwap.md); Coinbase — [candles omit empty intervals](https://docs.cdp.coinbase.com/exchange/reference/exchangerestapi_getproductcandles)
- Uniswap v3 subgraph — [`pricing.ts`](https://raw.githubusercontent.com/Uniswap/v3-subgraph/main/src/common/pricing.ts) (`findEthPerToken` returns 0 below `MINIMUM_NATIVE_LOCKED`), [`swap.ts`](https://raw.githubusercontent.com/Uniswap/v3-subgraph/main/src/v3/mappings/swap.ts) (`volumeUSD` vs `untrackedVolumeUSD`, `txCount` always) — the precedent for "excluded from price, counted in volume"
- Stellar Horizon — [`trade_aggregation.go`](https://raw.githubusercontent.com/stellar/stellar-horizon/master/internal/db2/history/trade_aggregation.go): buckets are rebuilt only from trades under a rounding-slippage filter (pool trades; NULL counts as 0), and a filtered trade loses its volume too
- Pyth — [`PriceStatus`, `prev_price`, `prev_timestamp`](https://github.com/pyth-network/pyth-sdk-rs/blob/main/pyth-sdk-solana/src/state.rs); Chainlink — [`updatedAt`](https://docs.chain.link/data-feeds/api-reference); Dune — [forward-fill caps, $10k threshold](https://docs.dune.com/data-catalog/curated/prices/overview)
- Google — [AIP-149 field presence](https://google.aip.dev/149), [AIP-180 backwards compatibility](https://google.aip.dev/180)
- MADR — [template, "Confirmation"](https://raw.githubusercontent.com/adr/madr/main/template/adr-template.md)
- Full research notes and the site-by-site audit: task 0151's `notes/`.
