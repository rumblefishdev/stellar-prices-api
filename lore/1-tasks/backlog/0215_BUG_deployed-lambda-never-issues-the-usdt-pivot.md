---
id: "0215"
title: "The deployed enrichment Lambda emits one pivot statement where the source emits two — the USDT pivot is never sent, and the artifact does not match any commit"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0209", "0212", "0111", "0172", "0182", "0141", "0213"]
tags: ["priority-high", "effort-small", "enrichment", "clickhouse", "deploy", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Split from 0209, whose 2026-08-20 root cause this falsifies. Measured from
      system.query_log while re-measuring 0111. Cheap to fix and NOT blocked by
      0111 — which is why it is split rather than folded in.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Two root-cause hypotheses raised and FALSIFIED the same day — both are
      recorded in the task body so they are not re-run. Neither `strings` on the
      deployed bootstrap nor the Lambda's `LastModified` discriminated anything:
      `USDT_ISSUER` is present in a PRE-0172 binary too (USDT was a peg member
      then), and the artifact was deployed 2026-08-20, a week after the merge.
      What did discriminate was reading the SQL the binary EMITS out of
      system.query_log — the peg's `IN (3)` vs `IN (3, 111)`, and the resolver's
      `result_rows`. Feature flags were checked and are not involved: nothing on
      the pivot path is `#[cfg]`-gated beyond `#[cfg(test)]`. Next step is a
      LOCAL reproduction, not more prod archaeology.
---

# The deployed Lambda emits one pivot where the source emits two

## Summary

`prices.price_ohlcv_1m` has no USDT-priced rows because the deployed enrichment
Lambda **never sends the statement that would write them** — not once on the
schedule, with no error and no `QueryStart`. It is not the backlog [[0209]]
blamed, and it is not gated on [[0111]].

⚠️ The mechanism is NOT yet established. Four measured facts contradict the
source (see Root cause); the leading explanation is an artifact built from an
uncommitted tree, but that is **inference, not measurement**. Reproduce locally
before acting.

## Evidence (prod `system.query_log`, measured 2026-08-21)

`pivot_sql` bakes its reference id into the SQL text as a literal
(`CAST({ref_id} AS UInt32) AS ref_asset_id`), so the log reads the deployed
binary's behaviour directly rather than the source's intent:

| pivot ref | runs | first seen | last seen |
|---|---|---|---|
| XLM (`id 4`) | 7,352 | 2026-08-07 09:20:15 | **2026-08-21 08:27:19** |
| USDT (`id 111`) | 6,493 | 2026-08-18 11:40:08 | **2026-08-18 14:30:04** |

USDT's entire lifetime is a 2 h 50 m window on 2026-08-18 — the run of [[0182]]
from execution host C, using a **locally built** binary. The hourly schedule has
never issued one.

⚠️ The run counts match `%ref_asset_id%` across **all** tiers, so they include
the repair tool's coarse-table work. The `first_seen`/`last_seen` boundaries
carry the finding; the totals do not.

Corroborated a third way by `written_rows`: over the 7 days to 2026-08-21 the
XLM pivot wrote 660-720K rows/day on `price_ohlcv_1m` while the USDT pivot wrote
nothing, because it was never invoked. [[0209]]'s `pivot_written = 0` therefore
describes THIS task, not a throughput limit.

Corroborated independently by the run ratio: through 2026-08-19 the log shows
`peg_insert : pivot_insert` at exactly **1:1** (70:70, 67:67, 66:66), where the
source issues one peg and **two** pivots per step.

## Root cause — the deployed artifact is not built from this source

⚠️ **Two hypotheses were measured and FALSIFIED. Do not re-run them.**

1. ⛔ **"The binary predates [[0172]]"** — falsified. `strings` on the deployed
   bootstrap finds `USDT_ISSUER` (that proves nothing: pre-0172 USDT was a *peg
   member*, so the constant was already compiled in), and the peg statement's own
   text settles it — `system.query_log` shows `quote_asset_id IN (3, 111)`
   running to 2026-08-13 10:19:39 and `IN (3)` from 2026-08-14 08:21:59 to now.
   `stable_ids()` is post-0172. `LastModified` is 2026-08-20T12:12:39, a week
   after the merge, and also proves nothing on its own ([[0141]]).
2. ⛔ **"`refs.usdt` is `None`"** — falsified. `resolve_reference_ids()` returns
   **`result_rows = 3`** on every scheduled invocation (72/day, last 2026-08-21
   09:27:38). All three reference assets come back.

### What the evidence forces

| # | measured | source implies |
|---|---|---|
| 1 | USDT pivot never sent — no `QueryStart`, no exception, 2 days | should be sent every step |
| 2 | peg emits `IN (3)` | binary is post-0172 |
| 3 | resolver returns 3 rows | `pivot_ids() == [xlm, usdt]` |
| 4 | `resolve → has_any() → run_peg_pivot_tier(&refs)`, both pivots in one `Vec`, no break | two statements per step |

Facts 2-4 make one statement per step impossible for this source. **So the
deployed artifact is not built from it.** `stable_ids()` and `pivot_ids()`
changed in the SAME commit (`6807025`), so no tagged commit produces the observed
half-state — post-0172 peg, pre-0172 pivot set. A binary built from an
**uncommitted working tree** does.

That is [[0141]] in a form neither of its existing checks catches: not a stale
artifact but a *work-in-progress* one. `LastModified` looked current and
`strings` found the constant. Record this — the discriminator that worked was
reading the **emitted SQL** out of `system.query_log`, never the artifact.

## Implementation

- ✅ **The source is EXONERATED — done 2026-08-21.** `reference_ids_helpers`
  asserts `full.pivot_ids() == vec![5, 7]` and `peg_sql_never_pegs_usdt` asserts
  `IN (3)`; both green (39 unit tests pass). Feature flags are not involved —
  nothing on the pivot path is `#[cfg]`-gated beyond `#[cfg(test)]`. So the code
  emits two pivots and the deployed artifact emits one. **Do not go looking for a
  source bug.**
- ⚠️ **Close the coverage gap that let this hide.** The tests cover
  `pivot_ids()` and `pivot_sql()` *separately*; NOTHING asserts that
  `enrich_peg_pivot_step` issues **two** statements. That assertion — count the
  statements one step sends, against a local CH — is what would have caught this,
  and it is the reason a green suite coexisted with a dark quote leg for 8 days.
- Rebuild from a verified-clean tree, redeploy, and confirm by reading the
  **emitted SQL** — not `strings`, not `LastModified`, not deploy exit status
  ([[oracle-writers-span-two-stacks]]). Both artifact-level checks looked healthy
  while the artifact was wrong.
- Rebuild, then verify **on the emitted SQL**, not on `strings` or
  `LastModified` — both looked healthy while the artifact was wrong. Redeploy and
  confirm by **measurement**, never by deploy exit status
  ([[oracle-writers-span-two-stacks]]).
- Add a guard so a silently-absent reference is refused rather than reported
  healthy. 0204's gap 4 already carries `resolved_legs` in its metric row for
  exactly this reason; the enrichment pass has no equivalent.
- ⚠️ The enrichment Lambda ships from `eventbridge-stack`, which is where
  `CleanupRule` lives. Check `describe-rule` **before and after** the deploy
  ([[cleanup-rule-shreds-backfill-output]], [[0200]]).

## Acceptance Criteria

- [x] The source is shown correct — `pivot_ids() == [xlm, usdt]` and the peg
      excludes USDT, both green (2026-08-21). The defect is the artifact.
- [ ] A test asserts `enrich_peg_pivot_step` issues TWO pivot statements, so a
      silently-narrowed pivot set fails the suite instead of the quote leg.
- [ ] After the fix, `system.query_log` shows `CAST(111 AS UInt32) AS
      ref_asset_id` running on the hourly schedule, outside any hand-run window.
- [ ] `peg_insert : pivot_insert` reaches 1:2 on `price_ohlcv_1m`.
- [ ] USDT-quoted `_1m` rows are measurably written — `written_rows > 0` on the
      USDT pivot, recorded before/after.
- [ ] `CleanupRule` verified `DISABLED` before and after the deploy.
- [ ] A missing reference asset fails loudly instead of silently narrowing
      `pivot_ids()`.

## Out of scope

- The 1.56M peg-valued `_1m` rows already on prod — that is [[0212]].
- The full-table scan and the 556.78M XLM-quoted backlog — that is [[0111]].
  ⚠️ Fixing this task adds a **third** statement per batch to a pass that
  already reads 739.68M rows each time, so it makes 0111 worse. Sequence
  accordingly.
