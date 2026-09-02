---
id: "0139"
title: "current_price_usd returns duplicate rows — assets is keyed on natural identity, not asset_id"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0072", "0061", "0067", "0144", "0150"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "milestone-M2"]
milestone: 2
links: []
history:
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Re-measured on prod while deploying [[0210]]: 3,300 asset_ids now serve
      6,606 identities (was 3,275 ids on 2026-08-03), and one id carries three
      identities rather than two. 10 of those ids touch soroban contracts,
      covering 20 identities — the measurement that made 0210 key its symbol
      table on contract_address instead of asset_id, so that work sits beside
      this defect rather than adding to it.
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Found during [[0072]] step 5 on ch-prod-01. `current_price_usd` returned
      **4,442 rows for 4,068 `current_prices` rows** — 374 duplicates. Cause:
      `prices.assets` is `ReplacingMergeTree(updated_at) ORDER BY (asset_code,
      issuer_address, contract_address)`, so `FINAL` dedups on natural identity
  - date: 2026-08-06
    status: backlog
    who: okarcz
    note: >
      **The "deeper question" is answered, and it is option 1 — genuine
      `asset_id` collisions between unrelated assets, not superseded
      identities.** BE's samples (4194 = STW/ARBRIDGE, plus 4628, 4195, 4287)
      are all different issuers; the code path matches (in-memory `next_id`
      counter in `canonical.rs:169-225`, no shared sequence, ≥3 components
      assigning independently). Blast radius is **every table keyed on
      `asset_id`**, not this view. Also: first consumer sizing — 5.5% of every
      pool BE displays a TVL for is tainted. See
      `0144/notes/S-be-0199-response-received.md`.
      and **not** on `asset_id`; **3,275 asset_ids are mapped to two or more
      natural identities**, and the view's `INNER JOIN … ON a.asset_id =
      c.asset_id` multiplies them out. Believed **pre-existing** (the v1
      six-column view carried the same join) — 0072 only made it measurable.
      BE reads this view in-cluster (0199 contract) and has just been pointed at
      its new columns, so they are consuming the duplicates too.
---

# `current_price_usd` fans out on duplicate `asset_id`

## Summary

`prices.current_price_usd` joins `current_prices` to `assets` on `asset_id`:

```sql
FROM prices.current_prices AS c FINAL
INNER JOIN prices.assets  AS a FINAL ON a.asset_id = c.asset_id
```

`prices.assets` (`init.sql:48-66`) is:

```sql
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address)
```

`FINAL` collapses on **natural identity**, so a given `asset_id` survives on as
many rows as it has distinct `(asset_code, issuer_address, contract_address)`
tuples. Joining on `asset_id` therefore fans out.

## Measured on ch-prod-01, 2026-08-03

```
current_prices FINAL rows                       4,068
current_price_usd rows                          4,442   (+374 duplicates)
asset_ids with >1 row in assets FINAL           3,275
```

## Re-measured on prod, 2026-09-02 — still live, and slightly worse

Taken while deploying [[0210]], a month after the numbers above:

```
distinct natural identities                     207,754
distinct asset_ids                              204,448
excess identities (identities − ids)              3,306
asset_ids serving >1 identity                     3,300
identities living under those ids                 6,606
worst single asset_id                                 3   identities
```

The 3,275 figure of 2026-08-03 is now **3,300 affected ids**. It grows with the
registry, which is what "the intern assigns ids that do not track identity"
predicts — this is not drift in the measurement, it is the defect accreting.

**One id carries three identities**, not merely two. Any fix that assumes pairs
will leave that case behind.

### The soroban slice, and why [[0210]] did not inherit it

Of the 3,300, **10 ids touch a Soroban contract**, covering 20 identities. That
measurement is why 0210's `prices.asset_symbol` is keyed on `contract_address`
rather than `asset_id`: an `asset_id`-keyed symbol would have been
unattributable for 19% of the 52-contract population, and would have added to
this defect rather than sitting beside it.

So the symbol table is unaffected — but the `assets` side of every join it
participates in still carries the ambiguity. 0210 dissolved its own exposure,
not this one.

### A measurement hazard worth knowing before re-counting

`prices.assets` is rewritten in full every hour ([[0140]]), so a plain `count()`
reads 1×, 2× or 3× depending on where the merge cycle is — 623,154 / 207,741 /
415,495 were observed on three consecutive hours. **Every number in this section
is `uniqExact`.** A raw count taken mid-cycle led, during 0210's deploy, to a
false conclusion that the registry had doubled.

## The deeper question this exposes

374 duplicate rows is the symptom. **3,275 asset_ids mapped to more than one
natural identity is the disease** — `asset_id` is supposed to be the surrogate
key for a natural identity, and at that scale it is not unique. Before patching
the view, establish which is true:

1. **ID assignment genuinely collides** — two different assets were handed the
   same `asset_id`. Then every table keyed on `asset_id` is suspect, not just
   this view, and the blast radius is far wider than a read surface.
2. **Historical rows with superseded natural identities persist** — e.g. an
   asset whose `contract_address` was filled in later (the §12.4 SAC collapse,
   [[0061]]) creates a *new* natural-identity row while the old one remains,
   both carrying the same `asset_id`. Then `assets` is behaving as designed and
   only the view's join is wrong.

Option 2 is the more likely reading given the §12.4 write-time collapse and
`sac_address` being a later addition — but it must be **measured, not assumed**.
The discriminator: for a sample of duplicated `asset_id`s, inspect the differing
tuples and their `updated_at`. Superseded identities will look like the same
asset gaining a `contract_address`/`sac_address`; genuine collisions will look
like unrelated assets.

### ⚠️ ANSWERED 2026-08-06 — it is **option 1**, the worse one

BE ran the discriminator for us (`0144/notes/S-be-0199-response-received.md`).
Every sampled pair is **unrelated assets with different issuers**, not one asset
gaining a contract address:

| `asset_id` | identity A | identity B |
|---|---|---|
| 4194 | `STW` (`GA2LHOPXZF…`) | `ARBRIDGE` (`GBACKRJVX7…`) |
| 4628 | `GESARA` | `GL1` |
| 4195 | `SPACEWALK` | `GIFT` |
| 4287 | `INSILVERMINE` | `NUTT` |

4194's two identities carry **862 series rows each, the same last bucket, and
prices identical to 14 decimals** — one asset's candles served under both names.

**The code path matches.** `asset_id` is "an app-assigned UInt32 surrogate"
(`init.sql:45`) handed out by an **in-memory counter**: `AssetRegistry::
from_existing` seeds `next_id = max(existing id) + 1` and `get_or_assign`
increments it locally (`canonical.rs:169-225`). There is no shared sequence and
no uniqueness constraint. At least three components construct their own registry
— the live ledger processor, asset-discovery, and events-backfill — so two
running concurrently load the same watermark and hand the same number to two
*different* newly-discovered assets. Nothing errors, because `assets` sorts on
natural identity and those are legitimately two distinct rows.

That explains the long-tail concentration: collisions only occur on assets being
discovered for the *first* time while two writers run. Established assets are
already in everyone's snapshot.

**Consequences, and they widen this task:**

- The blast radius is **every table keyed on `asset_id`**, not this view.
  `price_ohlcv_*` is `ORDER BY (asset_id, quote_asset_id, source, timestamp)` —
  two assets' candles are interleaved in one key space and cannot be separated
  after the fact by the id alone.
- Patching the join fixes the *symptom*. The fix must also make id assignment
  authoritative (a shared sequence, or derive the id from the identity by hash)
  and decide what to do with already-collided ids — renumber, or accept and
  disambiguate at read time.
- Option 2 is **not disproven**; four samples show option 1 exists. Both may be
  present. Sample more widely before choosing a repair strategy.

## Measured consumer impact (BE, 2026-08-06)

First real sizing, from the only external consumer:

```
identities in assets FINAL      204,381
asset_ids in assets FINAL       201,146     <- more identities than ids
duplicated asset_ids              3,279     (6,564 rows; our 08-05 count: 3,278)
BE pools touching a tainted identity   3,128
BE pools tainted AND priced            1,286   = 5.5% of every pool BE shows a TVL for
```

All long tail, no flagship (STW/Farsight, SGB/STW, NUTT/yXLM, …). BE are
deliberately **not** working around it — "we can't tell a tainted row from a
clean one without your authority data, and replicating that judgement would fork
the single source of truth" — so this fix is the only path for them.

## Does the fan-out inflate `volume_base`? (BE's question, answered from the SQL)

Read from `views.sql`; **not yet measured on prod**. Three answers:

1. **`price_usd_series` / `_1h` — no.** The `GROUP BY` includes the identity
   columns (`views.sql:190, 228`), so duplicates land in **different groups**.
   Each candle appears once per group; `sum(volume_base)` is real traded volume
   and the weighted average is arithmetically correct. The defect is **pure
   misattribution** — `ARBRIDGE` publishes `STW`'s real price from `STW`'s real
   weights.
2. **Cross-identity aggregation — yes, double-counted.** The volume is real
   once but appears under two identities. Any market-wide total sums it twice.
3. **`current_price_usd` — yes, genuinely inflated.** `views.sql:289-308` joins
   `assets FINAL ON asset_id` with **no `GROUP BY` at all**; a duplicated id
   emits two complete rows including `volume_24h_usd` (`views.sql:303`).

### ⚠️ Open — check `usd_reference` before closing this task

`usd_reference` / `_1h` (`views.sql:155-166, 201-212`) join `assets` **twice**
(base *and* quote), `GROUP BY p.timestamp` **only**, and filter on the *joined*
identity. If the XLM-native or the USDC `asset_id` is among the 3,279, **foreign
candles are admitted as XLM/USDC** — contaminating the reference series
consumers use to distinguish "no reference existed" from "prices-api bug", and
feeding the pivot tier's `xlm_usd`. Uniform duplication leaves the weighted
value unchanged, so it would be **invisible in the number**.

Low prior — collisions cluster on newly-discovered assets and XLM is `asset_id
9` — but it is one query and must not be assumed.

## Implementation (once the above is settled)

If option 2 — pick one row per `asset_id` deterministically, e.g. `argMax` over
`updated_at` in a subquery before the join, or key the join on natural identity
rather than `asset_id`. Prefer the latter if `current_prices` can carry it: it
removes the surrogate-key dependency instead of papering over it.

If option 1 — this becomes an ingestion-side task and the view fix is only a
stopgap. Spawn accordingly.

- Audit the **other** read surfaces in `views.sql` for the same join
  (`price_usd_series`, `identity_by_contract`, …) — if they join on `asset_id`
  against `assets`, they fan out identically and this is not a one-view bug.
- Add a test that fails on fan-out: seed two `assets` rows sharing an `asset_id`
  with different natural identities, assert the view returns one row per
  `current_prices` row.
- Tell BE once a direction is chosen — they read this view in-cluster and were
  pointed at it on 2026-08-03.

## Acceptance Criteria

- [ ] Determined whether the 3,275 duplicated `asset_id`s are ID collisions or
      superseded natural-identity rows, with the measurement recorded.
- [ ] `current_price_usd` returns exactly one row per `current_prices` row.
- [ ] Every other view in `views.sql` audited for the same join defect.
- [ ] **O2 — the XLM-native and USDC `asset_id`s checked against the 3,279
      duplicates** (one query; see the open section above). If either collides,
      `usd_reference` / `_1h` admit foreign candles as XLM/USDC and contaminate
      the pivot tier's `xlm_usd` — invisibly, because uniform duplication leaves
      the weighted value unchanged. Low prior, but it must not be assumed.
      Raised by [[0144]] while answering BE's `volume_base` question.
- [ ] A test fails if the fan-out reappears.
- [ ] BE informed of the resolution.
