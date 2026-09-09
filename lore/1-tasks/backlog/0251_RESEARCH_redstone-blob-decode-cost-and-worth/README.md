---
id: "0251"
title: "How much work is it to unpack the RedStone blob, and is a second oracle feed worth having at all?"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0173", "0128", "0172", "0246", "0248"]
tags: [layer-ingest, priority-low, effort-small, milestone-M2, scf, oracle, decision]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/soroban.rs"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-09-01
    status: backlog
    who: okarcz
    note: >
      Raised from a question about why RedStone is not used. The premise turned
      out to be half wrong — RedStone IS ingested, we simply never decode its
      payload, so every row carries price_usd = 0 under a reserved asset_id
      sentinel. Filed as an M2 DECISION rather than a feature: the open question
      is not how to build it but whether it earns its place, and nobody has
      opened a real payload to find out how big the job is. Not urgent — no
      oracle sets a published price, so nothing is broken while this is open.
---

# RedStone: what is in the blob, and is decoding it worth it?

## Summary

Two questions, in order. The second only matters if the first comes back cheap.

1. **What does it actually take** to decode RedStone's `updated_feeds` payload
   into per-asset USD prices we could store?
2. **Is a second oracle feed worth having**, given that no oracle sets a
   published price on this service?

The deliverable is a **recorded decision with a work estimate behind it** — not
an implementation. A well-argued *no* closes this task completely.

## Context — what exists today

RedStone is already wired into the ledger path. `decode_redstone`
(`prices-ingest-core/src/soroban.rs`) recognises `REDSTONE` events and writes one
`oracle_prices` row per event. But:

- the payload is a **base64 XDR `bytes` blob** (an `updated_feeds` map) and the
  full decode was deliberately deferred, so `price_usd` is written as **`0`**;
- RedStone does not emit a per-asset symbol at the event level, so there is no
  tradeable asset to resolve — the row carries the reserved
  `ORACLE_FEED_NO_ASSET_ID = 0` sentinel instead;
- the emitting contract is deliberately **not** interned into the
  `AssetRegistry`. An oracle feed is not an asset, and interning it would leak
  the feed's contract address into the contract-keyed read surfaces
  (`identity_by_contract`, `current_price_usd`) where a consumer resolving a
  pool leg could match an oracle as if it were a token.

So the raw data has been accumulating in a decodable form the whole time. That
was the point of capturing it: measure the byte footprint, keep the payload,
decide later.

⚠️ **Nothing is broken while this stays open.** Our prices come from observed
on-chain trades. Reflector is used only to convert quote-asset volumes to USD
and to price the USDC peg; RedStone sets nothing. This is an enrichment
question, not a defect.

## Question 1 — the cost

Answer it from a **real payload**, not from documentation. The rows are already
on prod:

```sql
SELECT timestamp, length(raw_data) AS bytes, substring(raw_data, 1, 120) AS head
FROM prices.oracle_prices
WHERE oracle_name = 'redstone'
ORDER BY timestamp DESC
LIMIT 5;
```

What to establish, in this order — stop early if the answer is bad:

- **Does it decode at all** with the `stellar-xdr` version we already pin? An
  `ScVal` map of feed-id → price is a morning's work; anything needing a
  RedStone-specific codec or an off-chain payload is a different task entirely.
- **What identifies each feed** — a ticker string, a numeric feed id, a hash?
  This is the crux; see Question 2's gate.
- **How many feeds per event, and which assets** do they cover? A feed set that
  is XLM-and-USDC-only adds nothing Reflector does not already give us.
- **What cadence and what price scale** (decimals)? Reflector is 14-decimal
  SEP-40; a different scale is a conversion, not a problem, but it must be
  recorded.
- **How many rows and bytes** have accumulated, so the storage question is
  answered with a number.

## Question 2 — the worth

**The case for.** Reflector is currently **uncorroborated**. It is the single
source for the USDC peg rate ([[0246]], `price_usd_series`) and for USD volume
conversion, and we have no independent way to notice if it drifts or stalls.
A second feed is a cross-check, and cross-checks are how the 2026-08 oracle
defects were eventually caught.

**The case against.** It changes no published number, so the benefit is
entirely operational. And it inherits the mapping problem:

🔴 **This is a ticker→issuer claim, and that class of claim has already cost us
7.4×.** RedStone will identify a feed by its own name for an asset; filing that
against a specific Stellar issuer is an assertion, exactly like Reflector's
"USDT" reading being filed against a Stellar IOU trading at ~$0.13 ([[0172]]).
**Route any mapping through [[0173]] rather than pattern-matching a code**, and
do not generalise a loader to "whatever feeds appear".

**A cheaper alternative to weigh explicitly:** if the goal is noticing that
Reflector has stalled or drifted, an alarm on `usd_rate` freshness plus a band
check may buy most of the benefit for a fraction of the work. Say why that is or
is not enough before recommending the decode.

## Acceptance Criteria

- [ ] A real prod payload is decoded (or shown not to decode), with the attempt
      and its result recorded — including the `stellar-xdr` version used
- [ ] The payload's structure is documented: what identifies a feed, how many
      feeds, which assets, the price scale, the cadence
- [ ] A work estimate for the full decode path, in the same shape as the
      existing Reflector path (decoder → identity resolution → write)
- [ ] The ticker→issuer question is answered explicitly, with [[0173]]'s
      reasoning applied — not deferred a second time
- [ ] A recorded decision: build, defer, or drop, with the reason. A **no** is a
      complete outcome and should say what would reverse it
- [ ] Accumulated `redstone` row count and byte footprint stated as measured
      numbers
- [ ] **§4.4 of `prices-api-general-overview.md` reflects reality** — the example
      response shows what the endpoint actually returns, with a sentence saying
      RedStone events are ingested for reference and carry no decoded price.
      §3.4's `oracle_name` vocabulary comment corrected in the same edit. This
      lands whatever the decision is; it is only bundled here so §4.4 is written
      once (see the section above)

## Out of scope

- **Implementing the decode.** If the answer is "build it", that is a FEATURE
  task spawned from here, not this one.
- **Any change to what sets a published price.** Whatever this concludes, prices
  stay derived from observed trades; a second oracle would be reference data on
  the same footing as Reflector.

## 📝 The §4.4 doc defect — FOLDED INTO THIS TASK (decided 2026-09-01)

`docs/prices-api-general-overview.md` §4.4 shows a `GET /oracles/{id}` example
response containing a **`redstone` entry with a price of `1.0001`**. That
response is unreachable today: RedStone rows carry `asset_id = 0` while the
endpoint queries `WHERE asset_id = ?` with a registry id, and registry ids start
at 1. So every real response has exactly one entry, `reflector`.

§3.4's column comment has a milder version — it lists
`'reflector', 'chainlink', 'redstone', 'band'` as the `oracle_name` vocabulary,
where only two are ever written and one only as a zero-priced placeholder.

This is a consumer-facing contract doc sitting in the SCF evidence chain
([[0128]]), and it is the same family as [[0248]] and 0237.

**Decided 2026-09-01: fix it as part of this task, not before it.** It was
offered as a standalone correction and deliberately declined — the two are about
the same thing, and splitting them means editing §4.4 twice if the decision is
"build". The doc fix is therefore an acceptance criterion below rather than a
separate change.

⚠️ **Consequence to hold, since it is now load-bearing:** while this task sits in
the backlog, the published spec advertises a response element that cannot arrive.
If an external consumer integrates against §4.4 in the meantime, that is where
the confusion comes from — and if this task is dropped without being worked, the
doc correction must be lifted back out rather than dropped with it.
