---
title: 'Phoenix XYK pool contract interface (XLM/USDC reference) — contrast surface for stable pool'
type: research
status: seed
spawned_from: ../README.md
spawns: []
tags: [phoenix, xyk, contract-interface, reference, stable-pool-contrast]
links:
  - 'https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool/src/contract.rs'
  - 'https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool_stable/src/contract.rs'
history:
  - date: 2026-05-15
    status: seed
    who: oski
    note: >
      Captured the live XYK XLM/USDC pool interface as a reference
      baseline. Task 0032 targets the stable-pool variant; this note
      gives the contrast surface so the stable-pool interface diff is
      easy to spot when the first mainnet stable pool is found.
---

# Phoenix XYK pool contract interface (reference)

## Headline

Recording the **XYK** (constant-product) pool contract interface as the
known reference shape. Task 0032 is about the **stable-pool** variant
which is not yet observed on mainnet. When a stable pool is found, its
interface should be diffed against this baseline to confirm the
6-event swap grouping (vs XYK's 8 events) is the only material delta.

## Source

- **Pool**: Phoenix XLM/USDC (XYK)
- **Contract ID**: `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX`
- **WASM SHA-256**: `167ab414a226427de34c19947ef9c5cf38c6c0ed91ecf9392f7cef3278ff506c`
- **WASM size**: 36810 bytes
- **Contract meta description**: `Phoenix Protocol XYK Liquidity Pool`
- **Toolchain**: Rust 1.85.1
- **Stellar SDK**: 22.0.7#211569aa49c8d896877dfca1f2eb4fe9071121c8
- **Captured**: 2026-05-15 via `stellar contract fetch` against mainnet
  (`https://mainnet.sorobanrpc.com`), then `sha256sum` on the binary
  and `stellar contract info meta --wasm` for the description.

## Interface — user-facing entrypoints

```rust
fn provide_liquidity(
    sender: Address,
    desired_a: Option<i128>,
    min_a: Option<i128>,
    desired_b: Option<i128>,
    min_b: Option<i128>,
    custom_slippage_bps: Option<i64>,
    deadline: Option<u64>,
    auto_stake: bool,
)

fn swap(
    sender: Address,
    offer_asset: Address,
    offer_amount: i128,
    ask_asset_min_amount: Option<i128>,
    max_spread_bps: Option<i64>,
    deadline: Option<u64>,
    max_allowed_fee_bps: Option<i64>,
) -> i128

fn withdraw_liquidity(
    sender: Address,
    share_amount: i128,
    min_a: i128,
    min_b: i128,
    deadline: Option<u64>,
    auto_unstake: Option<AutoUnstakeInfo>,
) -> (i128, i128)
```

## Interface — admin / lifecycle

```rust
fn update_config(
    new_admin: Option<Address>,
    total_fee_bps: Option<i64>,
    fee_recipient: Option<Address>,
    max_allowed_slippage_bps: Option<i64>,
    max_allowed_spread_bps: Option<i64>,
    max_referral_bps: Option<i64>,
)
fn upgrade(new_wasm_hash: BytesN<32>)
fn migrate_admin_key() -> Result<(), ContractError>
fn propose_admin(new_admin: Address, time_limit: Option<u64>) -> Result<Address, ContractError>
fn revoke_admin_change() -> Result<(), ContractError>
fn accept_admin() -> Result<Address, ContractError>
fn query_admin() -> Result<Address, ContractError>
fn add_new_key_to_storage() -> Result<(), ContractError>

fn __constructor(
    stake_wasm_hash: BytesN<32>,
    token_wasm_hash: BytesN<32>,
    lp_init_info: LiquidityPoolInitInfo,
    factory_addr: Address,
    share_token_name: String,
    share_token_symbol: String,
    default_slippage_bps: i64,
    max_allowed_fee_bps: i64,
)
```

## Interface — queries (read-only)

```rust
fn query_config() -> Config
fn query_share_token_address() -> Address
fn query_stake_contract_address() -> Address
fn query_pool_info() -> PoolResponse
fn query_pool_info_for_factory() -> LiquidityPoolInfo
fn simulate_swap(offer_asset: Address, offer_amount: i128) -> SimulateSwapResponse
fn simulate_reverse_swap(ask_asset: Address, ask_amount: i128) -> SimulateReverseSwapResponse
fn query_share(amount: i128) -> (Asset, Asset)
fn query_total_issued_lp() -> i128
fn query_version() -> String
```

## Why this matters for task 0032

The task aims to capture the first mainnet **stable-pool** observation
and confirm its 6-event swap grouping. Three things to compare against
this XYK baseline once a stable pool is found:

1. **`swap` signature parity** — stable-pool's `swap(...)` is expected
   to carry the same args (sender, offer_asset, offer_amount,
   ask_asset_min_amount, max_spread_bps, deadline, max_allowed_fee_bps)
   so a single decoder can pivot on event count rather than function
   signature. Any divergence is a red flag for the 0018 consumer spec.

2. **Event emission delta** — XYK emits 8 events per swap (per task
   0018 §3); stable-pool source claims 6 (no `actual received amount`,
   no `referral_fee_amount`). The first mainnet observation must
   verify the remaining 6 events appear in the **same order** as the
   matching XYK events.

3. **WASM hash discovery** — both XYK and stable-pool contracts are
   deployed through the Phoenix factory
   (`CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI`).
   Knowing the XYK WASM hash from this XLM/USDC pool gives us a known
   negative — any factory-deployed pool whose WASM hash differs is a
   stable-pool candidate.

## Followups

- ~~Pull the actual XYK WASM hash for `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX`~~
  **DONE 2026-05-15** — hash recorded above.
- ~~Scan the Phoenix factory for all pools and group by WASM hash.~~
  **DONE 2026-05-15** — see
  [evidence/phoenix_pool_inventory_2026-05-15.txt](evidence/phoenix_pool_inventory_2026-05-15.txt).
  11 pools total. 10 share the XYK hash above; 1 pool
  (`CD5XNKK3...IAA`, PHO/USDC) carries a different WASM
  `13b158655e40396957537bf1c528c6542b315930c1c9e0df640f57293c8af2ca`
  but is **also an XYK build** (same interface, same meta string, same
  protocol version `"2.0.0"`, only 237 bytes larger). Token-economic
  basis confirms this: a PHO/USDC pair would never use a stable curve.
  See [S-no-stable-pool-deployed.md](S-no-stable-pool-deployed.md)
  for the synthesis.
- When a stable pool eventually shows up in the factory, create
  `R-phoenix-stable-pool-interface.md` alongside this note and diff
  the two interfaces in an `S-` synthesis note.
