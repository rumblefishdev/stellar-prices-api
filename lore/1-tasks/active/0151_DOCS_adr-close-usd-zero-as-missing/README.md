---
id: "0151"
title: "ADR: close_usd's zero-as-missing sentinel is what makes the whole 0144 bug class expressible"
type: DOCS
status: active
assignee: akot
related_adr: ["0287", "0292"]
related_tasks: ["0144", "0135", "0146", "0147", "0148", "0149", "0138", "0154", "0111", "0286", "0167"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "schema", "adr", "data-correctness"]
links:
  - "../../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../../packages/prices-clickhouse/schema/views.sql"
  - "../../archive/0144_BUG_be-0199-usd-read-surface-defects/notes/I-usd-rate-table.md"
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
  - date: 2026-08-07
    status: backlog
    who: okarcz
    note: >
      **Table ownership moved out of [[0154]] to [[0167]].** Shape and
      constraints are unchanged — this is an extraction, not a re-decision. The
      trigger was [[0165]]: publishing a real peg rate needs the table, 0154 is
      hard blocked behind [[0111]], and the peg rates live in `oracle_prices`
      under a 13-month expiry. Checked against the rejection below and it does
      not re-open it (`close_usd` untouched, nothing NULL published, natural
      identity key retained) — full check in 0167 §Relationship to 0151. Of the
      two gating unknowns this decision shrank, 0167 hits **time-resolution**
      only (settled there, on peg assets where the choice is numerically cheap
      and the vocabulary carries to 0154) and does **not** hit projection cost.
  - date: "2026-09-17"
    status: active
    who: akot
    note: >
      Activated; taken by akot. Written against [[0286]] / ADR 0287 as the
      baseline, which widens the question: a bucket with no price-forming
      fill now stores `close = 0`, so `close_usd = 0` gains a FOURTH meaning
      (no price to convert - permanent and correct) and `close = 0` is itself
      a sentinel. 0286 also adds guardrails this ADR must enumerate: the
      enrichment candidate predicate `close_usd = 0 AND close > 0`, the rate
      predicate `close_usd > 0 AND close > 0` in all six MVs, and `/ohlcv`'s
      price-absent contract beside `price_usd_series`, which has no pf gate.
      [[0146]] in the table below is closed as superseded by 0286.
  - date: "2026-09-17"
    status: active
    who: akot
    note: >
      ADR 0292 proposed, with the guardrail inventory it points to
      (`docs/database-schema/close-usd-zero-guardrails.md`). Converted to a
      directory: `notes/R-zero-sentinel-code-audit.md` (every reader and writer
      of the zero on the 0286 code; two defects it found were fixed in 0286 as
      its decisions 14 and 15) and `notes/R-outside-practice.md` (how
      exchanges, vendors, oracles and API guidelines encode a missing price —
      the six decisions checked one by one). Decided: the sentinel stays; the
      "no price" marker is `pf_trade_count = 0`; pending / unpriceable /
      no_price are computed at read time and published with `as_of`; one
      definition of "priced" for 0147 and 0154; `0` stays on the current-price
      surfaces with additive `price_status`; the inventory is a living
      document and invariants are asserted by the freshness probe, not by
      CHECK constraints. Still open here: the zero-reading / zero-product loop
      in `oracle_sql`, the cross-surface test under the floor, the probe
      assertion, the `views.sql` / `init.sql` comment fixes. The wire fields
      are decided here and shipped by 0147.
  - date: "2026-09-18"
    status: active
    who: akot
    note: >
      **Everything still open above is closed, and every guardrail is now
      pinned by a test SEEN to fail without it.** Six local commits: the
      `oracle_sql` no-op guard (`479fd80`), the probe's zero-invariant
      assertion (`2e04d9b`), its CloudWatch ladder (`07e5dbf`), the comment
      fixes with the inventory and ADR Confirmation (`394fa4b`), three new
      behavioural ITs closing the audit's A4 gaps (`16e4857`), and the docs
      (runbook, inventory, this README). Nine RED proofs recorded: each guard
      term was removed, the named test seen to fail, and the source restored
      byte-identical before any commit. Suites green on ClickHouse 26.3.10.60:
      `ch_enrich_it` 58/58, `rollup_freshness_it` 22/22 single-threaded,
      `prices-clickhouse` ITs, workspace units 172/62/59, infra typecheck +
      lint. The four `◐` rows the inventory assigned to 0151 are `✅` and its
      "Open gaps, by owner" table names 0151 nowhere. New runbook
      `docs/runbooks/0151-zero-invariant-probe-rollout.md` carries the deploy
      order — probe and alarms go out only AFTER [[0286]]'s schema step, since
      the query reads `pf_trade_count` and a failed read fails the WHOLE probe
      invocation, not just its own check. **Not deployed: that is an operator
      step.** The cross-surface test under the floor stays with [[0147]], where
      the fix is — a test of agreement cannot land before the thing it asserts.
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
>
> ### 📌 Amendment 2026-08-07 — the table moved to [[0167]]; the decision did not change
>
> "Adopt it narrowly inside 0154" is still the decision; only the **task that
> builds it** moved. 0154's blocker is [[0111]], and that blocker is about the
> *second pivot tier* adding a join over a 490–545M-row candidate set — it says
> nothing about the table, nor about a small aggregation of the narrow
> `oracle_prices` into rate rows. Keeping the two coupled would have cost the
> depeg-aware peg history, which expires from `oracle_prices` on a 13-month
> retention (`cleanup-worker/src/lib.rs:24`) starting ~2026-10.
>
> **Nothing above is re-opened:** `close_usd` stays non-nullable and written in
> place, no `NULL` reaches a consumer, the key stays natural identity, and the
> [[0139]] revisit trigger is untouched (0167 needs no fact-table migration).
> 0154 keeps the tier and now consumes a table validated before it arrives.
>
> Of the two gating unknowns this scoping shrank, 0167 pulls **one** forward —
> time-resolution — and settles it as *observations + `ASOF` at-or-before +
> staleness bound*, matching `ch_enrich.rs:434-440`. Projection cost stays with
> 0154, untouched.

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

- [x] ADR filed in `lore/2-adrs/` with a decision, not just an analysis —
      `0292_close-usd-zero-is-a-named-sentinel-with-a-published-reason.md`,
      `## Decision` (six numbered verdicts, accepted by Adam 2026-09-17).
- [x] The rate-table option ([[I-usd-rate-table]]) decided 2026-08-06 — adopted
      **narrowly inside [[0154]]**, schema-wide refactor rejected for now, with
      the revisit trigger recorded. The two gating unknowns move to 0154 at
      reduced scope.
- [x] The ADR carries the **rejected** option and its reasoning, not just the
      chosen one — `## Alternatives considered` carries six: `Nullable(Decimal)`,
      the schema-wide rate table (with its 2026-08-06 revisit trigger), a per-row
      `usd_status` (DEFERRED, with a window), ClickHouse `CHECK` constraints,
      publishing `null` in place on the current-price surfaces, and one
      missing-value encoding API-wide.
- [x] `close_usd`'s published null-ness is stated as a **contract term**, with
      BE's dash-renders-as-missing constraint cited — ADR 0292 §5 and the
      Context bullet quoting BE, 2026-08-06: *"a NULL renders as a dash and
      removes the pool from every USD view we have."*
- [x] Cross-linked from [[0144]] (`related_adr: ["0292"]` and its 2026-09-17
      cross-link history note) and from `init.sql`'s column comment
      (`⚠️ That 0 is a SENTINEL with four meanings, kept on purpose (ADR 0292)`).
- [x] The `views.sql` value-or-absent contract either implemented or the
      header corrected to match reality — **header corrected**: the
      value-or-absent promise is now scoped to `price_usd_series*` /
      `usd_reference*`, and `current_price_usd`'s header says plainly that it
      publishes a sentinel `0` rather than omitting the row.
- [x] If "accept the sentinel" is the outcome, the guardrails that make it
      acceptable are enumerated and each has a test — the enumeration is
      `docs/database-schema/close-usd-zero-guardrails.md`, and nothing it
      assigns to 0151 is still ◐ or ⛔. The guards closed in this pass, each
      seen RED against its removed term before being kept:
      `the_oracle_statement_never_writes_a_row_it_cannot_change`,
      `a_usd_close_that_rounds_to_zero_is_written_once_and_never_rewritten`,
      `an_oracle_reading_of_zero_writes_nothing` (commit `479fd80`);
      `the_zero_invariant_scan_counts_only_rows_that_break_an_invariant`
      (`2e04d9b`);
      `a_dust_only_minute_quoted_in_a_pegged_asset_is_priced_once_by_the_peg_tier`,
      `the_pivot_ignores_a_reference_minute_that_formed_no_price`,
      `the_pivot_reset_never_re_opens_a_day_whose_only_reference_is_dust`
      (`16e4857`).
