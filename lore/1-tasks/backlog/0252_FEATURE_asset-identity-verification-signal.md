---
id: "0252"
title: "The API publishes no asset-identity signal — asset codes are not unique, 415 issuers publish BTC, and the only thing separating a fake from the real one is attacker-influenceable volume"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0210", "0120", "0139"]
tags: [layer-backend, layer-api, priority-high, effort-large, milestone-M2, api, security, metadata]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-ingest-core/src/writer.rs"
history:
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

## Why today's de facto defence is not one

For `USDC` the canonical Circle issuer carries **$86,173,279** of 24 h volume
and its eight impostors carry `$0.18`, `$0.01`, `$0.01`, `$0`, `$0`. The
default `sort=volume_24h desc` therefore separates them cleanly — **today**.

But volume is a quantity an attacker can manufacture: wash trading against
one's own asset costs fees, and that is the same economics [[0118]] addressed
for dust venues in the §5.5 VWAP. The defence is **empirical, not structural**
— it holds until somebody finds it worth defeating.

[[0120]] already worked around this by hand, curating its 20-asset conformance
list to skip "the `*BANK*` spam family, secondary wrappers of an already-listed
code, and obscure USD clones", and pinning BTC/ETH to the highest-volume issuer
with a canonical-pinning caveat. That curation is the evidence that no
automatic signal exists.

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
