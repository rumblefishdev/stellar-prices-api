---
id: "0252"
title: "The API publishes no asset-identity signal — asset codes are not unique, 415 issuers publish BTC, and the only thing separating a fake from the real one is attacker-influenceable volume"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0210", "0120", "0139", "0040", "0119", "0118", "0178"]
tags: [layer-backend, layer-api, priority-high, effort-large, milestone-M2, api, security, metadata]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-ingest-core/src/writer.rs"
history:
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Sharpened after [[0210]] shipped. The self-declared Soroban names are no
      longer inferred from BE's table — all 52 contracts were resolved directly
      over RPC, confirming the five recorded here and finding three more (bare
      `USD`, `EUR`, and `USDP`). Those symbols are now *published* as
      `asset_code`, and `GET /v1/assets/{contract}` already returns
      `code: "USDC"` for one of them, so the list is inventory rather than
      forecast. Added the mechanism and real cost of manufacturing volume, and
      corrected this file's earlier claim that volume is simply
      attacker-influenceable: `volume_quote_usd` is priced off the quote leg, so
      the attack is capital- and throughput-bound, not free. Also recorded that
      [[0040]] has enforced `CODE:ISSUER` since 2026-07-01 — addressing was
      never the gap, discovery is.
  - date: 2026-09-01
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0210]] after a meeting raised asset/contract impersonation
      as an attack vector. Measured on prod the same day: the surface is live,
      not theoretical. Kept out of 0210 deliberately — the problem predates it,
      affects 207k classic rows rather than 52 Soroban ones, and coupling them
      would block a small fix behind a large one.
---

# The API carries no way to tell a real asset from one impersonating it

## Summary

On Stellar an asset's identity is `(code, issuer)` for classic assets and the
contract address for Soroban ones. **The code alone is not unique and never
was** — anyone can issue an asset coded `USDC`, and any contract can return
`symbol() = "USDC"`. Our API exposes the code prominently, matches `?search=`
on it, and orders by a metric an attacker can influence, while publishing no
signal at all about whether an asset is what it claims to be.

We are a **price source**. An integrator that resolves a name to the wrong
asset prices a portfolio or a collateral position against a different
instrument.

## Measured on prod, 2026-09-01

Distinct issuers per classic asset code:

| code | issuers |
|---|---|
| NFT | **2,176** |
| XRP | 683 |
| sPUNK | 479 |
| BTC | 415 |
| GOLD | 352 |
| ETH | 317 |
| XLM | **305** |
| VELO | 260 |

And on the Soroban side, five contracts already in `prices.assets`
self-declare `USDC`, `USDT` (twice), `BTC` and `XRP` **without being SACs of
those assets** — verified against `assets.sac_address`, none matches. All five
are currently zero-volume, so the listing's `INNER JOIN current_prices` keeps
them invisible; they surface the moment they trade.

### Confirmed by resolving all 52 contracts, 2026-09-02

[[0210]] shipped, so the self-declared names are no longer an inference from
another team's table — every Soroban contract in the registry was asked
directly, over `simulateTransaction`. All 52 answered. Among them:

| symbol | contracts |
|---|---|
| `USDT` | **two** — `CBBUOYO3…` and `CCU77UVQ…` |
| `USDC` | `CDPV3H7C…` |
| `BTC` | `CBOZ6HCY…` |
| `XRP` | `CB7OOP3V…` |
| `USD` | `CBS7NEHF…` |
| `EUR` | `CARCEFDR…` |
| `USDP` | `CCOB35AE…` |

The five this task recorded are confirmed exactly. **Three more were not in
that count** — the bare currency codes `USD` and `EUR`, and `USDP`. Plus a
prefixed family that is not impersonation but sits adjacent to the originals
under a prefix search: `yUSDT`, `nBTC`, `nETH`, `BnUSD`, `USDM1`.

⚠️ Since 0210 these symbols are **published** as `asset_code` / `code`. They
are still absent from the listing, which `INNER JOIN`s `current_prices` and so
requires 24 h volume — but `GET /v1/assets/{contract}` reads `FROM assets` with
only `LEFT JOIN`s, so it already returns `code: "USDC"` for `CDPV3H7C…` today.
The list is a real inventory of what a consumer can be handed, not a forecast.

This is also the concrete argument for the boundary 0210 kept: `?search=` and
`sort=code` read the **stored** `assets.asset_code`, which is `''` for contract
rows, so none of the above is findable by name. Moving search onto the resolved
symbol before this task lands would make `?search=USD` return seven unverified
contracts alongside the real thing.

## Why today's de facto defence is not one

For `USDC` the canonical Circle issuer carries **$86,173,279** of 24 h volume
and its eight impostors carry `$0.18`, `$0.01`, `$0.01`, `$0`, `$0`. The
default `sort=volume_24h desc` therefore separates them cleanly — **today**.

But volume is a quantity an attacker can manufacture. The defence is
**empirical, not structural** — it holds until somebody finds it worth
defeating.

**How, and what it actually costs.** Asset codes on Stellar are unpermissioned,
so issuing one coded `BTC` is free. Two accounts under one operator then trade
it back and forth: each trade is real on-chain and our ledger path records it,
and a completed round trip leaves the position unchanged minus fees. Since
[[0178]] a trade's USD value credits **both** legs, so the fake side is credited
the full amount every time.

The cost is not the volume figure. **Capital is one round trip, not the total** —
$86 M of 24 h volume needs roughly $1,000 recycled 86,000 times, not $86 M. Base
fees are a fraction of a cent per operation, and 86,000 trades inside a 24 h
window is about one a second, well inside Stellar's throughput.

*An earlier version of this section said volume was simply "attacker-influenceable",
which overstates it.* There is a real floor: `volume_quote_usd` is the value of
the **quote** leg at its own USD price, so the attacker cannot inflate the figure
by declaring a silly price for their own token — they must move genuine value
through XLM or USDC. The attack is capital- and throughput-bound, not free. The
accurate claim is that it is **manufacturable well below the value of the
confusion it buys**, and that the bar scales with the *target's* liquidity: out-
ranking Circle's USDC is conspicuous, out-ranking the canonical `BTC` issuer on
Stellar is not.

**And the ranking column is unfiltered by design.** `current.sql:40` documents
`volume_24h_usd` as *"trailing-24h USD volume, ALL sources (a total, never
filtered)"*. §5.5's `min_volume_usd` threshold and the inter-source outlier
filter from [[0118]] apply to `vwap_24h`, **not** to this column — and the
listing sorts on this one. The metric that orders search results has no
anti-manipulation filter, deliberately, because it is meant to be a faithful
total.

Nothing in the pipeline detects self-trading: no counterparty analysis, no
unique-account heuristic. And the visibility floor is a single trade, since the
listing only requires a `current_prices` row.

[[0120]] already worked around this by hand, curating its 20-asset conformance
list to skip "the `*BANK*` spam family, secondary wrappers of an already-listed
code, and obscure USD clones", and pinning BTC/ETH to the highest-volume issuer
with a canonical-pinning caveat. That curation is the evidence that no
automatic signal exists.

## Addressing is already strict — discovery is the gap

Worth stating plainly, because it changes what this task is *for*.

[[0040]] built `AssetIdentifier::parse` (`identity.rs:46`), live since
**2026-07-01**, and it knows exactly three forms:

```
"native"                      → Native
"CODE:ISSUER"   (colon)       → Classic { code, issuer }   ← issuer parsed as a PublicKey
"C…"            (C-strkey)    → Contract
```

**None of them is a bare code.** `USDC` alone does not parse; it is a `400`
before any query runs. So a consumer cannot address a classic asset without
naming its issuer, and cannot land on the wrong `BTC` by omission. Someone had
already concluded that a code does not identify an asset.

[[0119]] later hardened everything around it — granularity, cursor, batch body,
ranges — and recorded that identifier parsing *"is already solid"*. It did not
need changing.

**That closes addressing and leaves discovery wide open.** The strictness moves
the risk one step earlier rather than removing it: the consumer must obtain an
issuer from somewhere, and the only somewhere we offer is this API's listing and
`?search=`, ranked by an unfiltered, manufacturable volume column. A consumer who
picks the wrong issuer from our own ranking then asks a perfectly well-formed
question about the wrong asset, and every validation in the path passes.

So this task is not "make the API stricter". The API is strict. It is **give the
caller the evidence the strictness assumes they already have** — which is why
the deliverable is a verification signal on the response, not another parser.

One consequence for scope: `?search=` is a *filter*, and ordering is independent
of it (`sort` defaults to `Volume24h`, `order` to `Desc` — `handlers.rs:241-242`).
Searching `BTC` therefore returns all 415 issuers ordered by the very metric
this task argues is not evidence. Whatever signal lands must reach the ordering,
not just the response body, or the default path is unchanged.

## What we already have and do not use

`prices.asset_metadata.home_domain` exists for exactly this — SEP-1's
bidirectional proof, where the issuer declares a `home_domain` and that
domain's `stellar.toml` lists the asset back, so an impersonator would need to
control both the account and the domain. **It has no production writer**
(`write_asset_metadata`, `writer.rs:295-310`, is called from one test), so the
column is served, always empty, on every response.

## Scope

Three parts, separable:

1. **Populate `home_domain` and verify it.** Read the issuer's `home_domain`
   from Horizon, fetch `https://<domain>/.well-known/stellar.toml`, and confirm
   the `[[CURRENCIES]]` block lists this exact `(code, issuer)`. Store the
   verdict, not just the domain — an unverified domain claim is worth little on
   its own.
2. **Expose the verdict.** A field on `AssetListItem` / `AssetDetail` — likely
   an enum (`verified` / `unverified` / `unknown`) rather than a bare boolean,
   because "we have not checked" and "we checked and it failed" are different
   facts and must not collapse.
3. **Decide what ranking and search should do with it.** Options, in rough
   order of cost: leave ordering alone and let consumers filter; add a
   `?verified=` filter; break volume ties by verification; stop ranking search
   results on volume alone. This part needs a decision, not just an
   implementation — it changes what every existing consumer sees.

⚠️ **Only after (1)–(3) should `?search=` be extended to Soroban symbols.**
[[0210]] deliberately left `startsWith(a.asset_code, ?)` and `sort=code` on the
**stored** column, so a Soroban token is displayed by its self-declared symbol
but is not findable or orderable by it. Extending search without a verification
signal is precisely what would let a hostile token surface under a well-known
code. There is a test pinning that boundary
(`list_it.rs::search_and_sort_still_read_the_raw_column`); changing it should be
a decision made here.

## Notes

- Soroban tokens have no `home_domain` and no SEP-1 equivalent. SEP-41 offers
  no identity claim either — `symbol()` is self-declared. Options worth
  measuring: a SAC's derivable link to a verified classic asset ([[0242]] is
  the same derivation from the other side), issuer/deployer reputation, or an
  explicit allowlist for the majors. This part may honestly end at "we cannot
  verify Soroban tokens; label them as such".
- ⚠️ [[0139]] is unfixed, so any `asset_id`-keyed verification verdict inherits
  ambiguous ids. Prefer keying on natural identity, as [[0210]] did.
- Worth checking what Horizon, StellarExpert and Lobstr do here before
  designing — this is a solved-ish problem in the ecosystem and copying a
  convention beats inventing one.

## Acceptance Criteria

- [ ] `home_domain` has a production writer and is populated for the assets the
      API actually serves — not just present as a column
- [ ] The API publishes a verification verdict that distinguishes "verified",
      "failed verification" and "not checked"
- [ ] Verification is bidirectional (SEP-1): an issuer claiming a domain is not
      enough; the domain's `stellar.toml` must list the asset back
- [ ] For a code with many issuers — `BTC` (415) is the sharpest classic case —
      a consumer can tell the canonical one from the rest **without** relying on
      volume ordering
- [ ] A decision is recorded on whether search and ranking use the signal, with
      its reasoning, whichever way it goes
- [ ] Soroban tokens are either verifiable by some stated mechanism, or
      explicitly labelled unverifiable — not silently indistinguishable
