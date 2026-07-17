---
title: "Prod dry-run findings — measured baselines + two real bugs"
type: S
status: mature
spawned_from: "0097"
date: 2026-07-17
who: okarcz
---

# S — Prod dry-run findings (2026-07-17)

Synthesis of the first full-range run of `events-backfill` against `ch-prod-01`
over `[50457424, 63352611]`. Two shipped bugs, one bogus inherited baseline, and
four operational traps. Everything here is MEASURED on prod, not inferred.

## Decision Log

### 2026-07-17 — the "~824k Soroswap swaps" baseline is wrong; ground truth is 536,319

**Context.** 0096 recorded: *"BE's soroban_events (full history, activation→63.49M)
shows our 221 pools emit 824k swaps"*. 0097 was opened to fill those 824k. The
full-range dry-run produced **536,318** soroswap ticks — a 35% apparent shortfall
that triggered a long, and entirely wasted, hunt (registry coverage → resolution
gaps → classification blind spot). Every hypothesis was chasing a phantom.

**Measurement (prod, `[50457424, 63352611]`, by `JSONExtractString(topics_xdr, 2,
'value')`).** `sync 537,941` · **`swap 536,319`** · `deposit 1,303` ·
`withdraw 318` · `skim 23`. Total SoroswapPair events **1,075,904**;
swap+sync = 1,074,260. `unique_events == events` for every action → no unmerged
RMT duplicates inflating anything. `outside_registry = 0` → every SoroswapPair
event in range comes from a registered pool; registry coverage is **complete**
(soroswap = 221 pools, matching 0096's own count).

**Root cause of the bad number.** 824k matches nothing measurable. The likely
manufacture: `topics_xdr LIKE '%SoroswapPair%'` matches **all** pair events, not
just swaps — Soroswap emits a `sync` alongside nearly every swap, so a LIKE-based
count runs ~2× the true swap count.

**Decision.** Ground truth is **536,319 swaps**; we extract **536,318** (off by 1).
Do **not** re-seed `pool_registry` on the strength of the 824k claim — coverage is
already complete. **GOTCHA for any future Soroswap count: filter on topic[1]
(`JSONExtractString(topics_xdr, 2, 'value') = 'swap'`), never a `LIKE` on the
envelope.** The stray 1 swap is unexplained (a dispatch/decode error, 0.0002%) and
is not worth chasing.

### 2026-07-17 — Phoenix XYK swap groups are VARIABLE length; the `== 8` gate dropped 5,175 real swaps (LIVE bug)

**Context.** The dry-run logged **19,624** `amm dispatch error` warnings, all
Phoenix. Sampling two of them read as benign non-swap noise; the distribution did
not. Grouped as the code groups — by `(transaction_id, contract_id)`, per
`soroban.rs:337-371` — over `[50457424, 63352611]`:

| events | groups | what it is |
|---|---|---|
| 8 | 237,026 | full swap — **exactly our phoenix tick count** |
| **7** | **5,175** | **REAL swap, silently discarded** |
| 5 | 13,297 | liquidity (8,130 provide + 5,167 unbond) |
| 4 | 1,045 | withdraw-liquidity |
| 1 / 2 | 107 | misc |

**Root cause.** `dispatch.rs` gated on `n >= PHOENIX_XYK_EVENT_COUNT` (8), the
*fully-populated* shape, but Phoenix **omits optional fields**. The 5,175
seven-event groups carry `[sender, sell_token, offer_amount, buy_token,
return_amount, spread_amount, referral_fee_amount]` — every field the extractor
requires — missing only `actual received amount`, which `xyk.rs` reads and
**discards**. Only **four** fields are required (`sell_token`, `offer_amount`,
`buy_token`, `return_amount`); `sender` is optional (`TradeRow::trader` is an
`Option`). So ~**2.1%** of Phoenix swaps were dropped.

**This is a LIVE bug, not a 0097 bug.** `dispatch_phoenix` is shared with the
ledger-processor, so live Phoenix pricing has been ~2% short all along — a sibling
of the 0096 Soroswap defect, surfaced by 0097's instrumentation. **Live-era data
(post-63352611) stays wrong until the fixed ledger-processor is deployed**, which
nothing 0097 writes will fix. Fix landed on this branch (`f3c677b`) rather than a
separate task — flagged for a split decision since it ships live code in a
backfill PR.

**Decision.** Gate on **required-field presence**, never row count; the count is
only a floor (4). Liquidity groups stay rejected — they carry `token_a` /
`shares_amount` and none of the required fields. Groups >8 still consume only the
first swap (unobserved on prod: max group = 8).

**The instrumentation gap that hid it (fixed).** A dispatch error had **no
counter**: the `unresolved` fallback is deliberately scoped to Soroswap
pair-resolution misses (`soroban.rs:454`), so a failing Phoenix group produced no
tick, no `unresolved` row, and no metric — only a `warn!`. The summary read
`swaps dropped: 0` while 5,175 swaps died; it took hand-grepping 19,624 log lines
to find. `LedgerSoroban` now carries `dispatch_errors` (gated on the group holding
real swap events so liquidity noise can't inflate it), surfaced per source in the
summary. **Same blind-spot class as 0096: a loss channel with no counter.**

**Test that encoded the bug.** `xyk_extractor_rejects_fewer_than_8_rows` passed
only because its 5-row slice *also* lacked `return_amount`. Rewritten as
`xyk_extractor_rejects_group_missing_a_required_field`, asserting the reason
(`MissingField`), not the length.

### 2026-07-17 — operational findings from the first prod run

1. **CH auth**: the client sets `X-ClickHouse-User` / `X-ClickHouse-Key` as
   INDEPENDENT headers; `--clickhouse-user` had no default, so passing only
   `CLICKHOUSE_PASSWORD` sent a key with no user → rejected. Now defaults to
   `default`.
2. **`safe_log.rs` redaction ate every CH error code**: it scanned
   `err.to_string()` for a leading `Code: `, but the crate's `Display` prefixes
   `"bad response: "` → **every** server exception collapsed to
   `"detail suppressed"`, in the live Lambda and SDEX backfill too. Now matches on
   the variant. Cost ~4 rounds of blind guessing on what a `Code: 516` would have
   answered instantly.
3. **No `SETTINGS` clause in any read query**: the client sends reads as POST with
   an explicit `readonly=1` whenever the SQL exceeds its GET-length threshold —
   ours always do (they embed the registry contract-id list) — and ClickHouse
   rejects per-query setting changes under readonly with **code 164**. Bound reads
   by **chunking** (the memory limit is per-query), never by settings.
4. **The coverage probe was fatal AND unbounded**: one full-range scan of the wide
   `topics_xdr` column blew the 5.59 GiB quota (**code 241**) and its `?` aborted a
   COMPLETED 12.9M-ledger dry-run, discarding every tick total. Now chunked,
   non-fatal, and `print_summary` runs BEFORE it.
