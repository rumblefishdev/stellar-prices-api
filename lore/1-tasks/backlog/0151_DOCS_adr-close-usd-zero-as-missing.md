---
id: "0151"
title: "ADR: close_usd's zero-as-missing sentinel is what makes the whole 0144 bug class expressible"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0144", "0135", "0146", "0147", "0148", "0149", "0138", "0154", "0111"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "schema", "adr", "data-correctness"]
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/prices-clickhouse/schema/views.sql"
  - "../active/0144_BUG_be-0199-usd-read-surface-defects/notes/I-usd-rate-table.md"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 9). Too invasive to retrofit
      during the 0144 fix chain, but it should be written down before the next
      surface is built on the same footing.
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Raised to phase 5 / priority-high by 0144's phase-0 revision — [[0147]]'s
      coverage gate and [[0154]] both need this ADR's definition of
      "priceable", so it moved from last to a prerequisite of two tasks.
  - date: 2026-08-06
    status: backlog
    who: okarcz
    note: >
      **Scope widened.** A third target shape emerged in session — normalise the
      USD *rate* out of `close_usd` into a per-`(quote_asset_id, timestamp)`
      table, making `close_usd` a derived cache. Written up in
      [[I-usd-rate-table]] (under 0144's notes). It changes this ADR's question
      from "how do we mark a missing value" to "what is the source of truth for
      a USD price", and it reaches [[0154]] and [[0111]] as well.
  - date: 2026-08-06
    status: backlog
    who: okarcz
    note: >
      **Rate-table question DECIDED same day** — adopt narrowly as [[0154]]'s
      implementation (keyed on natural identity, `close_usd` untouched), reject
      the schema-wide refactor for now. Three of its four selling points did not
      survive BE's response: NULL is actively bad for the consumer, the bugs it
      dissolves are one-liners already queued, and [[0139]] turned out to be
      genuine `asset_id` collisions, which a rate table keyed on
      `quote_asset_id` would have inherited. Revisit trigger recorded: if 0139's
      repair needs a fact-table migration anyway, doing both at once gets
      cheaper. This ADR now has one real decision in it already.
---

# ADR — `close_usd` zero-as-missing

## Summary

`close_usd` is `Decimal(38,14) DEFAULT 0` on a **non-nullable** column
(`init.sql:114`). So three distinct facts share one value:

1. **not yet priced** — enrichment has not reached this row (transient)
2. **will never be priced** — exotic quote, no oracle; a permanent floor by
   design (`ch_enrich.rs:31-32`)
3. **genuinely worth nothing**

Every defect in [[0144]] is a different aggregate meeting that value:

| Aggregate | What it does with the sentinel | Task |
|---|---|---|
| `argMax(close_usd, ts)` | returns it, discarding priced rows | [[0146]], [[0135]] |
| `sum(close_usd * volume)` | weights it as a real zero | [[0147]] |
| `WHERE close_usd > 0` | fixes the arithmetic by silently changing the population | [[0147]], [[0135]] |

And `views.sql`'s header already promises consumers **value-or-absent**
semantics classified against `usd_reference` — which is a contract the storage
does not implement. "Partially enriched" is a third state that today
masquerades as a good value.

## What the ADR must decide

- Whether `Nullable(Decimal)`, a **companion status column**, or a **normalised
  rate table** is the target shape. Nullable makes the class unrepresentable but
  costs on a hot column; a status column is additive but leaves the sentinel in
  place for anything that ignores it; the rate table is the largest change and
  the largest win — see below.
- ⚠️ **Storage-NULL must not become published-NULL.** BE, 2026-08-06: *"a NULL
  renders as a dash and removes the pool from every USD view we have."* They
  chose stale-but-real over absent explicitly (they already carry prices forward
  48h by design). So whatever shape wins **in storage**, the published surface
  must keep emitting a number where it emits one today —
  Nullable-plus-fallback, never Nullable-and-propagate. This is a
  consumer-observable contract term, not an internal choice.
  → `0144/notes/S-be-0199-response-received.md`
- Whether state 2 (permanently unpriceable) deserves its own marker distinct
  from state 1 — [[0147]]'s coverage gate needs to distinguish them, and today
  it can only infer the difference from volume share.
- The migration story: `close_usd` is written by enrichment, the sweep, six
  rollup MVs and four pre-roll scripts. Any change touches all of them.
- Whether this is worth doing at all, or whether [[0146]] + [[0147]] leave the
  residual risk low enough to accept the sentinel permanently. **"Accept it,
  with these guardrails" is a legitimate outcome** — the point is that it be
  decided rather than inherited.

## The third option — normalise the rate

> ### ✅ DECIDED 2026-08-06 — adopt it **narrowly inside [[0154]]**; reject the schema-wide refactor for now
>
> The rate table is built as [[0154]]'s implementation (shape and constraints
> are in that task), keyed on **natural identity** rather than `asset_id`.
> `close_usd` is **not** touched: it stays non-nullable, `DEFAULT 0`, written in
> place, with no consumer-visible change.
>
> **Why not the full refactor.** Three of its four selling points did not
> survive BE's 2026-08-06 response
> (`0144/notes/S-be-0199-response-received.md`):
>
> 1. **The headline benefit lost its consumer.** "Absence becomes representable
>    ⇒ `NULL` ⇒ aggregates skip it" was the pitch. BE: *"a NULL renders as a dash
>    and removes the pool from every USD view we have."* The internal benefit
>    survives; the value-or-absent framing does not.
> 2. **The bugs it dissolves are the cheap ones.** It would obviate [[0145]],
>    [[0146]] and half of [[0147]] — but 0145 is a mechanical one-line guard with
>    a deadline, [[0135]] is two lines, and 0146 fixes a live edge BE now
>    structurally avoids (they stop one bucket short of the forming bucket). A
>    schema migration is not the way to avoid three one-liners already queued.
> 3. **[[0139]] grew underneath it.** Confirmed 2026-08-06 as *genuine `asset_id`
>    collisions between unrelated assets*. A rate table keyed on
>    `(quote_asset_id, timestamp)` would be new authoritative infrastructure on a
>    **non-unique key**. It does not add the problem — enrichment already
>    resolves rates by `quote_asset_id` — but stacking two structural changes
>    where one rests on the other's broken assumption is not a migration anyone
>    can reason about. Hence: key on natural identity, and do not generalise yet.
>
> **What survived, and it is enough:** it makes [[0154]] — the top of the queue,
> with the consumer's numbers behind it — a self-join on a small table instead of
> another pass over a fact table [[0111]] already re-scans every batch.
>
> Scoping to 0154 also shrinks both gating unknowns: the projection cost applies
> to one backfill rather than a general write path, and the time-resolution rule
> must be settled only at the granularities 0154 prices, not all six.
>
> **Revisit if** [[0139]]'s repair turns out to need a fact-table migration
> anyway — two assets are interleaved in one key space, so it might. Doing both
> in one pass would then be cheaper than doing them separately. Re-ask once 0139
> has a repair strategy, **not before**.

**Full write-up of the idea: [[I-usd-rate-table]]**
(`0144/notes/I-usd-rate-table.md`).

`close_usd` is not a stored fact, it is a cached product. All three enrichment
tiers compute `close × <USD rate of the QUOTE asset at that time>`
(`ch_enrich.rs:9-11, 22-30`), and that rate is a function of
`(quote_asset_id, timestamp)` **only** — never of the candle being priced. We
look it up, multiply it into hundreds of millions of rows, and discard it.

Storing the rate instead — a few thousand rows per bucket rather than one price
per candle — and demoting `close_usd` to a derived cache:

- makes absence representable (no rate row ⇒ `NULL` ⇒ CH aggregates skip it), so
  the class dissolves rather than being guarded — [[0145]], [[0146]] and half of
  [[0147]] become unnecessary rather than fixed;
- turns [[0154]]'s second pivot hop into a self-join on a small table;
- removes [[0111]]'s cause (enrichment stops asking "which candles lack a
  price?", which is only answerable by reading every candle);
- fixes finding 1 outright rather than mitigating it — a candle is priced when
  it is written, not up to ~50 min later;
- resolves [[0149]] and mostly evaporates [[0148]] (a derivable value is not
  lost).

**Two unknowns gate it, and neither is measured:** `quote_asset_id` is the
*second* sort-key column on `price_ohlcv_1m` (`init.sql:122`), so "every candle
quoted in X" has no clean index path — probably a projection, cost unknown; and
the rate's time-resolution across the six granularities is undecided (a `_1d`
candle needs a *daily* rate — average? close? vwap?), which if got wrong invents
a subtler restatement of finding 3.

Migration is additive: build the rate table, backfill, verify it reproduces
today's `close_usd`, and only then decide the column's fate.

## Ordering

**No longer last.** Raised to phase 5 on 2026-08-05 because [[0147]] and
[[0154]] both need this ADR's definition of "priceable". The 0144 chain still
supplies the evidence on what the guardrails cost — but the guardrail-vs-
normalise question cannot wait behind it, since [[0154]] would be built on the
answer.

## Acceptance Criteria

- [ ] ADR filed in `lore/2-adrs/` with a decision, not just an analysis.
- [x] The rate-table option ([[I-usd-rate-table]]) decided 2026-08-06 — adopted
      **narrowly inside [[0154]]**, schema-wide refactor rejected for now, with
      the revisit trigger recorded. The two gating unknowns move to 0154 at
      reduced scope.
- [ ] The ADR carries the **rejected** option and its reasoning, not just the
      chosen one — a rejected option with a revisit trigger is the point.
- [ ] `close_usd`'s published null-ness is stated as a **contract term**, with
      BE's dash-renders-as-missing constraint cited.
- [ ] Cross-linked from [[0144]] and from `init.sql`'s column comment.
- [ ] The `views.sql` value-or-absent contract either implemented or the
      header corrected to match reality.
- [ ] If "accept the sentinel" is the outcome, the guardrails that make it
      acceptable are enumerated and each has a test.
