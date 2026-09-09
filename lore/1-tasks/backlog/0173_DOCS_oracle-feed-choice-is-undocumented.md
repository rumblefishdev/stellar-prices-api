---
id: "0173"
title: "The oracle feed every USD price depends on was never justified — no ADR, no rejection rationale, and a plausible alternative was never evaluated"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0039", "0048", "0167", "0168", "0154", "0172", "0165"]
tags:
  ["priority-medium", "effort-small", "oracle", "soroban", "adr-input", "external-dependency", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../archive/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md"
history:
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Raised while explaining the oracle path during 0167. The question "why
      this contract?" has no answer in the repo. Filed now rather than later
      because 0167 is about to snapshot these readings into a FOREVER-RETAINED
      table - the moment to sanity-check the source is before it becomes
      permanent history, not after.
---

# The oracle feed choice is undocumented

## The gap

Every USD price this system publishes ultimately derives from one Soroban
contract:

```rust
// packages/oracle-worker/src/lib.rs
pub const REFLECTOR_CEX_DEX: &str = "CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN";
```

**Nothing in the repo says why.** There is no ADR covering the oracle choice —
the nine that exist cover backfill strategy, PK design, the runtime framework
and the CH host, but not the single external dependency the entire USD estate
rests on. [[0048]] records that the feed *exists*; [[0039]] records that we
*call* it. Neither records a decision.

⚠️ **Reflector is not an official Stellar oracle.** SDF does not operate one.
**SEP-40 is a standard** (the interface a price oracle should expose); Reflector
is a third-party network implementing it. "SEP-40 compliant" is easy to misread
as endorsement, and the code comment says only *"Reflector … oracle (SEP-40),
Stellar mainnet"*, which does not disambiguate.

## What was actually known at the time

[[0048]] sampled real Soroban events and found **four** live oracle feeds:

| Contract | Feed | Keyed by |
|---|---|---|
| `CALI2BYU…LE6M` | Reflector — **Stellar on-chain assets** | token **contract address** |
| `CBKGPWGK…CJZC` | Reflector — FX symbols | symbol (EUR, GBP, XAU…) |
| **`CAFJZQWS…4DLN`** | Reflector — global crypto symbols | symbol (BTC, USDC, USDT…) ← **chosen** |
| `CA526Y2N…XUSG` | RedStone | bytes-encoded XDR |

So alternatives were *observed* and never *evaluated*. The choice looks
reasonable — we needed USD prices for XLM/USDC/USDT by ticker, and this feed has
them — but "reasonable and unwritten" is how a decision becomes folklore.

## The alternative worth actually assessing

`CALI2BYU…LE6M` keys prices by **Stellar token contract address**, not by global
ticker. That is a materially different question being answered:

- **Our feed** says what *"USDC the ticker"* trades at across external CEXs and
  DEXs — a real-world dollar value, independent of Stellar.
- **That feed** would say what *a specific Stellar asset* is worth.

The distinction is not academic. Our symbol → issuer mapping
(`reflector_key_to_identity`) asserts that Reflector's `USDC` means USDC at the
canonical Circle issuer. That is almost certainly right, but [[0165]] measured
**56 other issuers using the code `USDC`**, and the oracle can say nothing about
any of them. The mapping is an assumption with no test and no comment marking it
as one.

It may also bear on [[0172]] (USDT/USDC closing at ~0.14 on SDEX): a
Stellar-native oracle would either corroborate the Stellar market or contradict
it, and **either answer is informative** about whether those candles are wrong or
merely thin.

⚠️ **This task is not a proposal to switch.** The current feed is plausibly the
*right* choice for a USD reference — you generally want the real-world dollar
value, not a thin local market's opinion of it. The deliverable is a decision on
the record, not a migration.

## Why now

[[0167]] snapshots these readings into `prices.usd_rate`, which is
**forever-retained by design**. Sanity-checking the source belongs before it
becomes permanent history. It is also cheap right now: the table is empty and
nothing consumes it yet.

## Scope

- ADR recording: which feed, why, which alternatives were rejected and on what
  grounds, and what would trigger revisiting.
- **A stated position on the symbol → issuer mapping** — it is currently an
  unmarked assumption in `reflector_key_to_identity`.
- **An operational policy for the config that already exists.** `REFLECTOR_CONTRACT`
  and `SOROBAN_RPC_URL` are both overridable (`main.rs`), so we have the
  mechanism to switch feed or endpoint and **no documented answer to when we
  should**. Include: what happens if Reflector redeploys the contract, if the
  public RPC endpoint rate-limits, or if the feed goes stale — noting the worker
  is deliberately non-critical, so a dead oracle degrades silently by design.
- Whether a second feed should be polled as a cross-check. ⚠️ Note this is
  **not free**: `oracle_prices` is keyed `(asset_id, oracle_name, timestamp)`
  and would hold both, but [[0167]]'s `usd_rate` has **no oracle column** — two
  feeds disagreeing at one timestamp would need a documented precedence rule
  before a second feed is enabled, or the snapshot's winner becomes arbitrary.

## Acceptance Criteria

- [ ] ADR authored covering feed choice, rejected alternatives, and the revisit
      trigger; linked from `oracle-worker`'s contract const so the next reader
      finds it from the code.
- [ ] `CALI2BYU…LE6M` (Stellar on-chain assets) explicitly evaluated — adopted,
      rejected with reasons, or spawned as its own task. Not left unmentioned.
- [ ] The symbol → issuer mapping stated as a deliberate assumption, in the code
      and in the ADR.
- [ ] Operational policy recorded for contract/endpoint override, staleness, and
      Reflector redeployment.
- [ ] A position on second-feed cross-checking, including the `usd_rate`
      precedence consequence if the answer is yes.

## Out of scope

- Changing the feed. If the ADR concludes we should, that is its own task with
  its own backfill question — `oracle_prices` history is feed-specific.
- Oracle *availability* alerting. Real, but a monitoring concern; this task is
  about the decision record.

## Notes

- The contract has been a bare constant since `ffc07e4`
  (*"feat(lore-0039): oracle worker — Reflector (SEP-40) via Soroban RPC"*).
- Reflector reports **14 decimals** as a protocol constant, not from on-chain
  metadata ([[0048]] §2.3) — so `REFLECTOR_DECIMALS` is another undocumented
  external assumption, and it happens to line up exactly with our
  `Decimal(38,14)`. Worth stating explicitly in the same ADR: if a future feed
  reports different precision, that silent alignment breaks.
