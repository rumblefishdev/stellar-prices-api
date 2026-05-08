---
title: "Phoenix DeFi Hub canonical mainnet contract registry + swap-event shape"
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, phoenix, venue-attribution]
links:
  - "https://github.com/Phoenix-Protocol-Group/phoenix-contracts"
  - "https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/scripts/upgrade_mainnet.sh"
  - "https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool/src/contract.rs"
  - "https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/factory/src/contract.rs"
  - "https://api.stellar.expert/explorer/public/directory/"
history:
  - date: 2026-05-08
    status: developing
    who: claude
    note: >
      Extracted canonical addresses from upgrade_mainnet.sh on the
      phoenix-contracts main branch and verified all 11 pools via
      stellar.expert directory API. Confirmed Phoenix pool swap event
      shape from contracts/pool/src/contract.rs. Spot-check vs the top-5
      observed Symbol("swap") emitters: 2/5 are confirmed Phoenix pools
      (XLM/USDC, XLM/PHO).
  - date: 2026-05-08
    status: developing
    who: claude
    note: >
      Cross-check vs 0001 sample data revealed topic-kind error:
      Phoenix pools emit ScVal::String("swap"), not Symbol. Source uses
      &str tuple which compiles to String. 9 of 11 known Phoenix pools
      observed in the 4-day sample as String("swap") emitters (5,704
      events total). Section "Pool swap event format" updated with
      correction callout; rest of structural content unchanged.
---

# Phoenix mainnet registry

## Canonical addresses

Source of truth: [`scripts/upgrade_mainnet.sh`](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/scripts/upgrade_mainnet.sh)
(committed to `main`; lists addresses being upgraded by the team).
Cross-checked against the Stellar Expert directory API
(`https://api.stellar.expert/explorer/public/directory/<C-address>`) on
2026-05-08.

### Singletons

| Role | Contract ID | Stellar Expert label |
|------|-------------|----------------------|
| Factory | `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI` | *(not in directory — empty `{}`)* |
| Multihop (router) | `CCLZRD4E72T7JCZCN3P7KNPYNXFYKQCL64ECLX7WP5GNVYPYJGU2IO2G` | *(not in directory)* |
| Vesting | `CDEGWCGEMNFZT3UUQD7B4TTPDHXZLGEDB6WIP4PWNTXOR5EZD34HJ64O` | *(not relevant for trading)* |

> The factory and multihop are absent from `stellar.expert/directory`
> labels but their identity is anchored by the upgrade script committed
> to the team's repo. The 11 pools below all carry the `"Phoenix Pool"`
> label, which is the strongest cross-check available short of
> simulating a factory call.

### Pool contracts (XYK pools, 11 confirmed)

All entries below were verified to return
`{"name":"Phoenix Pool","domain":"phoenix-hub.io","tags":["defi"]}`
from `https://api.stellar.expert/explorer/public/directory/<C-address>`
on 2026-05-08.

| Pair (per upgrade script) | Pool contract ID | Stellar Expert label |
|---------------------------|------------------|----------------------|
| PHO / USDC  | `CD5XNKK3B6BEF2N7ULNHHGAMOKZ7P6456BFNIHRF4WNTEDKBRWAE7IAA` | Phoenix Pool |
| XLM / PHO   | `CBCZGGNOEUZG4CAAE7TGTQQHETZMKUT4OIPFHHPKEUX46U4KXBBZ3GLH` | Phoenix Pool |
| XLM / USDC  | `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX` | Phoenix Pool |
| XLM / EURC  | `CBISULYO5ZGS32WTNCBMEFCNKNSLFXCQ4Z3XHVDP4X4FLPSEALGSY3PS` | Phoenix Pool |
| USDC / VEUR | `CDQLKNH3725BUP4HPKQKMM7OO62FDVXVTO7RCYPID527MZHJG2F3QBJW` | Phoenix Pool |
| USDC / VCHF | `CBW5G5SO5SDYUGQVU7RMZ2KJ34POM3AMODOBIV2RQYG4KJDUUBVC3P2T` | Phoenix Pool |
| XLM / USDX  | `CDMXKSLG5GITGFYERUW2MRYOBUQCMRT2QE5Y4PU3QZ53EBFWUXAXUTBC` | Phoenix Pool |
| EURX / USDC | `CC6MJZN3HFOJKXN42ANTSCLRFOMHLFXHWPNAX64DQNUEBDMUYMPHASAV` | Phoenix Pool |
| XLM / EURX  | `CB5QUVK5GS3IU23TMFZQ3P5J24YBBZP5PHUQAEJ2SP5K55PFTJRUQG2L` | Phoenix Pool |
| XLM / GBPX  | `CCKOC2LJTPDBKDHTL3M5UO7HFZ2WFIHSOKCELMKQP3TLCIVUBKOQL4HB` | Phoenix Pool |
| GBPX / USDC | `CCUCE5H5CKW3S7JBESGCES6ZGDMWLNRY3HOFET3OH33MXZWKXNJTKSM3` | Phoenix Pool |

> Note: the upgrade script also lists 11 *stake* contracts (one per
> pool). Stake contracts emit bond/unbond events, not trade events,
> so they're out of scope for `prices_amm_trades` and not enumerated
> here. They're listed in `scripts/upgrade_mainnet.sh` if needed.

### Pool kinds

Phoenix has two pool implementations:

- `contracts/pool/src/contract.rs` — XYK (constant-product) pool.
  All 11 pools above appear to be this kind based on the upgrade
  script ordering and the WASM hash being shared (`phoenix_pool.wasm`
  in `make build`). Token-pair identifiers look like
  ordinary asset pairs (XLM/USDC, etc.).
- `contracts/pool_stable/src/contract.rs` — stable pool. The repo
  contains the code (`phoenix_pool_stable.wasm` is built but **not
  uploaded** by `upgrade_mainnet.sh` — only `phoenix_pool.wasm` is).
  No mainnet stable-pool address is currently known. **No
  attribution path for stable pools exists from this script alone.**
  If/when a stable pool is deployed, it will not appear in the
  upgrade script's `pools=()` array.

## Pool swap event format (XYK pool — `contracts/pool/src/contract.rs`)

> **Correction 2026-05-08** (verified by re-running `dump-swap-events`
> against the wider sample, then per-event ScVal-kind inspection). The
> source-code reading below referred to "Symbol" but the on-chain bytes
> use **`ScVal::String`** for both `topic_0` and `topic_1`. Cause: the
> Rust source uses `&str` tuple `("swap", "sender")` which `IntoVal`
> compiles to `String`, not `Symbol` (a `Symbol` would require
> `symbol_short!` / `Symbol::new(&env, ...)`). The 8-event grouping
> shape is correct; only the topic-kind label is wrong throughout this
> section. Replace `Symbol("swap")` with `String("swap")` and
> `Symbol(<field>)` with `String(<field>)` wherever it appears below.
> See `S-venue-attribution-mapping.md` §"Topic-kind correction".

The XYK pool emits **eight** separate events per `swap`, all with
`topic_0 = String("swap")` and a String field-name as `topic_1`.
This is *very different* from a single consolidated event.

From `contracts/pool/src/contract.rs:1172-1185`
([source](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool/src/contract.rs)):

```rust
env.events().publish(("swap", "sender"), sender);
env.events().publish(("swap", "sell_token"), sell_token);
env.events().publish(("swap", "offer_amount"), offer_amount);
env.events().publish(("swap", "actual received amount"), actual_received_amount);
env.events().publish(("swap", "buy_token"), buy_token);
env.events().publish(("swap", "return_amount"), compute_swap.return_amount);
env.events().publish(("swap", "spread_amount"), compute_swap.spread_amount);
env.events().publish(("swap", "referral_fee_amount"), compute_swap.referral_fee_amount);
```

Concretely:

```
topics = [Symbol("swap"), Symbol(<field>)]
data   = <field-typed value>
```

Where `<field>` is one of:

| `topic_1` | data type | meaning |
|-----------|-----------|---------|
| `Symbol("sender")` | `Address` | trader (user that called `swap`) |
| `Symbol("sell_token")` | `Address` | C-address of token being sold |
| `Symbol("offer_amount")` | `i128` | amount user offered |
| `Symbol("actual received amount")` | `i128` | amount actually received by pool (note: **field name has spaces**) |
| `Symbol("buy_token")` | `Address` | C-address of token being bought |
| `Symbol("return_amount")` | `i128` | amount sent to user |
| `Symbol("spread_amount")` | `i128` | spread |
| `Symbol("referral_fee_amount")` | `i128` | referral fee (typically 0 — referral path is `FIXM:`-disabled in current code, see line 1119) |

### Stable-pool variant

`contracts/pool_stable/src/contract.rs:1182-1189` emits **six** events
(no `actual received amount`, no `referral_fee_amount`):

```rust
env.events().publish(("swap", "sender"), sender);
env.events().publish(("swap", "sell_token"), sell_token);
env.events().publish(("swap", "offer_amount"), offer_amount);
env.events().publish(("swap", "buy_token"), buy_token);
env.events().publish(("swap", "return_amount"), return_amount);
env.events().publish(("swap", "spread_amount"), spread_amount);
```

### Implications for the indexer

1. **Phoenix pools are hidden inside the 44 `Symbol("swap")` emitters**
   from `R-swap-topic-shapes.md`. Confirmed: 2 of the top-5 emitters
   (4,128 events at XLM/USDC; 440 events at XLM/PHO) are Phoenix pools.
2. **Per-swap event count is 8 (XYK) or 6 (stable)**, so raw event
   counts massively over-state Phoenix swap volume vs. a venue that
   emits one event per swap. The wider-sample's **4,128 events at
   `CBHCRSVX...` ≈ 4128 / 8 ≈ 516 swaps** in ~4 days.
3. **Decoder must group by `(tx_hash, contract_id)`** and reduce the
   8 (or 6) events back into one logical trade row. Field names are
   the join key; `Symbol("sender")` plus the four token/amount fields
   are sufficient to populate
   `(token_in, token_out, amount_in, amount_out, trader)`.
4. **Field name `Symbol("actual received amount")` contains literal
   spaces** — preserve verbatim when matching, do not normalise.
5. **No inline pair_contract_id in the event** — `pair_contract_id`
   in `prices_amm_trades` should be set to the emitter contract
   (`contractId` from the `ContractEvent`), not derived from event
   data. (This matches Soroswap's pool-event behaviour per
   `R-swap-topic-shapes.md`.)
6. **`fee` is *not* directly emitted** as a separate event. It must be
   reconstructed: `commission_amount` is paid to `fee_recipient`
   inside `do_swap` (line 1110-1115) but is not published as an
   event. Only `spread_amount` (slippage) and the disabled
   `referral_fee_amount` are published. This is a meaningful
   schema gap vs. Aquarius (where `fee` is event field `data[2]`).
   For Phoenix, the indexer either (a) leaves
   `prices_amm_trades.fee` NULL, or (b) computes
   `fee = compute_swap_fee_bps × offer_amount / 10_000` from the
   pool config (one-shot lookup per pool). Recommend NULL for
   minimum-viable; document in §11 of amm-trades-schema.md.

## Factory event format (`contracts/factory/src/contract.rs`)

Pool creation emits one event:

```rust
// contracts/factory/src/contract.rs:177-178
env.events()
    .publish(("create", "liquidity_pool"), &lp_contract_address);
```

Concretely:

```
emitted_by: factory contract (CB4SVAWJ...)
topics    = [Symbol("create"), Symbol("liquidity_pool")]
data      = Address(lp_contract_address)
```

This is enumerable from contract events:

> Index every event from contract `CB4SVAWJA6TS...` with
> `topic_0 = Symbol("create")` and `topic_1 = Symbol("liquidity_pool")`;
> the event `data` is the new pool's contract ID.

This is the authoritative way to enumerate Phoenix pools (more
reliable than the upgrade script which lags behind production
deployments). **Recommend** the indexer scan the factory's history
once on bootstrap, then subscribe to factory events for new pools.

## Spot-check vs observed `Symbol("swap")` emitters

Top-5 from wider sample (`R-swap-topic-shapes.md` §"`Symbol("swap")`
revisited"):

| Events | Contract | Stellar Expert | Phoenix? |
|---:|---|---|---|
| 11,947 | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` | "Aquarius Router" (aqua.network) | **No** — Aquarius |
| 4,128 | `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX` | "Phoenix Pool" (phoenix-hub.io) | **Yes** — Phoenix XLM/USDC pool |
| 2,706 | `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` | *(not in directory)* | Not in registry |
| 2,480 | `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` | *(not in directory)* | Not in registry |
| 440 | `CBCZGGNOEUZG4CAAE7TGTQQHETZMKUT4OIPFHHPKEUX46U4KXBBZ3GLH` | "Phoenix Pool" (phoenix-hub.io) | **Yes** — Phoenix XLM/PHO pool |

**Headline:** 2 of the top-5 `Symbol("swap")` emitters are Phoenix XYK
pools. This **resolves possibility #1** from `R-swap-topic-shapes.md`
("Phoenix is hidden inside the 44 `Symbol("swap")` emitters") — Phoenix
*is* there, just under a 2-topic shape (`(swap, <field>)`) that the
original three-topic Aquarius decoder cannot handle.

The remaining 3 of top-5 are: 1 confirmed Aquarius router and 2
unlabelled. The unlabelled ones are out of scope for this note; see
parallel `R-aquarius-registry.md` and the synthesis note for follow-up.

### Why the 4-day sample showed only 2/11 Phoenix pools at high volume

In the wider sample, only 2 of the 11 Phoenix pools cracked the top-5
emitter list. Likely reasons:

- **Per-swap event multiplier (×8)** inflates emitter rank for any
  active Phoenix pool. The other 9 pools simply had < ~50 swaps in 4
  days, so they each generated < ~400 events (below the noise floor of
  the wider-sample top-5).
- **XLM/USDC and XLM/PHO** are the two pairs whose tokens are most
  liquid (XLM is the gas asset, USDC and PHO are the most-held
  Phoenix assets), explaining their dominant share of swap activity.

This is not contradictory with `R-swap-topic-shapes.md`'s observation
that 44 distinct contracts emit `Symbol("swap")` — it just means
Phoenix's contribution is concentrated in 2 pools and the long tail
is distributed across other pools and other venues (Aquarius router
+ Aquarius helper contracts + possibly more).

## Sources

- [Phoenix-Protocol-Group/phoenix-contracts](https://github.com/Phoenix-Protocol-Group/phoenix-contracts) — official monorepo
- [`scripts/upgrade_mainnet.sh`](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/scripts/upgrade_mainnet.sh) — canonical mainnet addresses
- [`contracts/pool/src/contract.rs`](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool/src/contract.rs) — XYK pool, swap event lines 1172-1185
- [`contracts/pool_stable/src/contract.rs`](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool_stable/src/contract.rs) — stable pool, swap event lines 1182-1189
- [`contracts/factory/src/contract.rs`](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/factory/src/contract.rs) — factory, pool-created event lines 177-178
- [`contracts/multihop/src/contract.rs`](https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/multihop/src/contract.rs) — multihop emits **no** consolidated swap event; delegates to underlying pools (so pool-level events are authoritative for trade rows)
- Stellar Expert directory API:
  `https://api.stellar.expert/explorer/public/directory/<C-address>`
  — used for label confirmation
- [Phoenix DeFi Hub Medium intro](https://medium.com/stellar-community/phoenix-building-the-first-defi-hub-on-stellar-cae669829ab5) — context (DEX launched 2024-05-07)

## Open follow-ups (out of scope for this note)

- Stellar Expert directory does **not** label the Phoenix factory or
  multihop. Worth submitting a label request to stellar.expert via
  their directory submission form so future indexers benefit.
- Stable-pool address is unknown. If/when one is deployed, it will not
  appear in `upgrade_mainnet.sh`'s `pools=()` array; the indexer
  should rely on the factory event scan (above) rather than the
  upgrade script.
- The two unlabelled high-volume `Symbol("swap")` emitters
  (`CCR2CH4GQVCZ...`, `CDMIM23WOUL5...`) are likely Aquarius — see
  `R-aquarius-registry.md`.
