---
title: "BE's response to our 0199 reply — what it confirms, what it changes, what it asks"
type: synthesis
status: mature
spawned_from: notes/G-be-0199-reply-short.md
spawns: []
tags: [be-facing, contract, prices-api, closes-phase-0]
links:
  - "../../../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-06
    status: mature
    who: okarcz
    note: >
      BE replied to the short reply sent 2026-08-05. **This closes 0144's last
      acceptance criterion** (reply sent + acknowledged). Every contract
      decision is confirmed; they supplied pool-level coverage numbers that
      re-rank two tasks, sized [[0139]]'s consumer impact for the first time,
      and asked one question we owe them an answer to.
---

# BE's response — analysis

## Verdict: the plan holds. Four adjustments, one open question.

Nothing in the response contradicts a measurement or reverses a decision. It
**confirms** every contract call, and it moves priority in the direction phase 0
already pointed — harder.

---

## Confirmed, no action needed

| Our position | BE's answer |
|---|---|
| Decision 1 — publish latest *priced* close, ~50 min stale | ✅ "clear yes". They already carry prices forward **up to 48h by design**, so ~50 min is noise |
| Decision 2 — guard `per_source`, keep the venue in `sources`/`vwap_24h` | ✅ accepted implicitly with §1 |
| Option A ("wait for full enrichment") cannot terminate | ✅ accepted, withdrawn |
| Option B (weight in the zeros) measures 0.000023 vs ~0.170 | ✅ accepted, withdrawn — "both dust-print fixes we proposed were bad" |
| The ~2× read claim was ours and was wrong (+4.7%) | ✅ they corrected their own notes; cost is the `GROUP BY` |
| 0039 is not the owner; [[0135]] is | ✅ corrected on their side |
| Expose `priced_volume_share` so they set their own bar | ✅ explicitly requested — [[0147]] criterion confirmed by the consumer |

**⚠️ New hard constraint from §1, and it cuts against [[0151]]:** *"a NULL renders
as a dash and removes the pool from every USD view we have."* So whatever
[[0151]] decides about `Nullable(Decimal)` at the **storage** layer, the
**published** surface must not start emitting NULL where it emits a number
today. Nullable-plus-fallback, not Nullable-and-propagate. Record this in the
ADR; it is a consumer-observable contract, not an internal choice.

---

## Changes to the plan

### 1. [[0154]] confirmed as the biggest win — with the consumer's own numbers

BE measured all **52,369 classic pools** for both-legs-priceable:

| What their code reads | Pools | % |
|---|---|---|
| `price_usd_series_1h`, 48h (detail + list) | 23,228 | **44.4%** |
| `price_usd_series` daily, 90d (chart) | 35,673 | 68.1% |
| `price_usd_series` daily, ever | 50,258 | 96.0% |

And they pre-emptively falsified the obvious alternative explanation. Worst-leg
staleness per pool: **≤2d 44.7%, ≤7d 46.3%, ≤30d 57.1%, never priced 4.0%** —
loosening 2d → 7d buys **+1.6pp**. Their conclusion, which matches ours: *"the
gap isn't recency, it's the quote-asset restriction."*

They rank the pivot step **above the materialised table** they originally asked
for. This is independent confirmation of phase 0's finding-2 re-ranking, from
the party who filed the request.

> ⚠️ **Do not read 44.4% → 96% as [[0154]]'s headroom.** The spread between "48h"
> and "ever" is mostly pools whose legs *stopped* being priced — plausibly dead
> long-tail pools that stopped trading at all, in which case pricing them helps
> nobody. **Open question O1** below.

### 2. [[0111]] is now gated in front of the consumer's #1 ask

Decision 4 already made 0111 a blocker of 0154. BE has now put a number on what
sits behind it: **more than half their pools have no headline TVL**, and 0154 is
the only item that moves it. 0111 is no longer "not acutely urgent" in any
framing.

### 3. [[0150]] drops — the requester deprioritised it themselves

*"more than the materialised table, which only makes a query we can already
cache faster."* Ordering was already last (phase 10); the **priority tag drops
to low** and the justification is rewritten. Still worth doing, no longer worth
doing soon.

### 4. [[0146]]'s consumer-visible urgency drops again

BE shipped their own §3 mitigation: **both read paths now stop one bucket short
of the in-progress bucket**, costing up to one bucket of freshness against their
48h budget. Since phase 0 established the `argMax` defect reaches *only* the
in-flight bucket, **the only consumer now structurally avoids the rows 0146
fixes.**

0146 still ships — we should not publish a wrong number because the one consumer
learned to dodge it, and `/ohlcv` plus any future consumer still reads it — but
it is no longer defensible as urgent. Keep it after [[0145]] and [[0135]];
do not let it outrank [[0111]]/[[0154]].

---

## [[0139]] — BE confirmed the mechanism independently, and sized it

They reverse-engineered the cause from outside with no access to our schema:

> *"`prices.assets FINAL` returns 204,381 identities from 201,146 asset_ids.
> More identities than ids means a single asset_id survives FINAL under several
> identities — which points at the table's sorting key not being asset_id
> alone."*

Correct, and exactly `init.sql:65` — `ORDER BY (asset_code, issuer_address,
contract_address)`. Their **3,279** duplicated asset_ids (6,564 rows) matches our
3,278 from 08-05 to within a day's drift.

Worked example, matching TEST D precisely — `asset_id 4194` is both `STW`
(`GA2LHOPXZF…`) and `ARBRIDGE` (`GBACKRJVX7…`): **862 series rows each, same
last bucket, prices identical to 14 decimals.** Same shape for GESARA/GL1
(4628), SPACEWALK/GIFT (4195), INSILVERMINE/NUTT (4287).

**Consumer impact, new and previously unmeasured:** 3,128 pools touch a tainted
identity; **1,286 are tainted *and* priced — 5.5% of every pool BE displays a
TVL for.** All long tail, no flagship. → recorded in [[0139]].

They are deliberately **not** working around it: *"we can't tell a tainted row
from a clean one without your authority data, and replicating that judgement
would fork the single source of truth."* That is the right call and it means
0139's fix is the only path.

> **One framing note on our side:** BE opened this section with *"the mechanism
> may not be where you're looking"*, having read our line "548,439 daily rows
> are published under identities that never traded them" as possibly meaning
> identity→many ids. Our direction was right all along (0144 §2a says
> "asset_ids mapped to two or more natural identities"), but the reply's wording
> was ambiguous enough to cost them a check. Worth a sentence when we answer.

---

## The question BE asked, and the answer from the SQL

> *"Do the tainted identities also inflate `volume_base`? Our weighted-average
> reasoning assumes the weights are the real traded volume."*

**Read from `views.sql`; not yet measured on prod. Three different answers for
three surfaces:**

**1. `price_usd_series` / `price_usd_series_1h` — the views BE reads: NO.**
The `GROUP BY` includes the identity columns (`views.sql:190, 228`), so the two
duplicate rows land in **different groups**. Within either group each candle
appears exactly once, `sum(volume_base)` is the real traded volume, and the
weighted average is arithmetically correct. **The error is pure misattribution,
not inflation** — `ARBRIDGE` publishes `STW`'s real price computed from `STW`'s
real weights. Their weighted-average reasoning is safe. (Their own evidence
agrees: identical to 14 decimals is what you get when both groups hold the same
un-duplicated candle set.)

**2. Any cross-identity aggregation BE does themselves: YES, they double-count.**
The volume is real *once* but appears under two identities. Summing across
identities counts it twice. Directly relevant if they compute a market-wide
total.

**3. `prices.current_price_usd`: YES, genuinely inflated.** `views.sql:289-308`
joins `assets FINAL ON asset_id` with **no `GROUP BY` at all** — this is 0139's
original filing. A duplicated asset_id emits two complete rows, `volume_24h_usd`
(`views.sql:303`) included. Anyone summing that column over the view
double-counts. Warn them if they touch it.

### O2 — a risk to us that this question surfaced

`usd_reference` / `usd_reference_1h` (`views.sql:155-166, 201-212`) join
`assets` **twice** (base *and* quote) and `GROUP BY p.timestamp` **only**, then
filter on the *joined* identity:

```sql
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC' AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
```

If the XLM-native asset_id or that USDC asset_id were duplicated, **foreign
candles would be admitted as XLM/USDC** — contaminating the very reference
series consumers use to tell "no reference existed" from "prices-api bug", and
feeding the pivot tier's `xlm_usd`. Uniform duplication leaves the weighted
*value* unchanged, so this would be invisible in the number.

Low prior — the collision mechanism (concurrent in-memory counters on
newly-discovered assets) makes early well-known ids unlikely to collide, and XLM
is `asset_id 9`. **But it is one query and it must not be assumed.** → O2.

---

## Open questions

| # | Question | Owner | Why it matters |
|---|---|---|---|
| **O1** | Of the pools priceable "ever" but not in 48h (≈51.6pp), how many still trade? | [[0154]] | Decides whether 0154 moves BE from 44.4% toward 96% or toward ~50%. Do not quote headroom until answered. |
| **O2** | Are the XLM-native or USDC `asset_id`s among the 3,279 duplicates? | [[0139]] | If yes, `usd_reference` admits foreign candles as XLM/USDC — silently. One query. |
| **O3** | Does BE aggregate `volume_base`/`volume_24h_usd` across identities anywhere? | BE | Determines whether answer 2/3 above is theoretical or live for them. Ask in the response. |

---

## What we owe BE

A short response, not another long one:

1. **`volume_base` answer** — no inflation in the views they read (group-by
   separates the duplicates); the weights are real. But flag the two places
   where it *does* bite: their own cross-identity sums, and
   `current_price_usd`'s volume column.
2. **A one-line correction of our own wording** on the fan-out direction, since
   ours cost them a check.
3. **Confirm the re-rank:** [[0111]] → [[0154]] is now the top of the queue
   behind the two quick fixes, and [[0150]] moves back on their own advice.
4. **Ask O3.**
5. Optionally ask for their per-pool list behind O1 — they clearly have it, and
   it would save us the measurement.
