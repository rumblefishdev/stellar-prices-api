---
id: "0252"
title: "The API publishes no asset-identity signal — asset codes are not unique, 415 issuers publish BTC, and the only thing ordering them is a volume figure that overclaims what it measures"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0210", "0120", "0139", "0040", "0119", "0118", "0178"]
tags: [layer-backend, layer-api, priority-high, effort-large, milestone-M3, api, security, metadata]
milestone: 3
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-ingest-core/src/writer.rs"
history:
  - date: 2026-09-02
    status: active
    who: stkrolikiewicz
    note: >
      Moved to **milestone 3**. Checked against Tranche 2's six acceptance
      criteria — endpoint conformance, load test, cache TTL, verifiable VWAP and
      two on history depth — and this task closes none of them. Tranche 3 is
      "Production Launch & Validation" and lists a security review, which is
      where it belongs. The M2 tag was set on the assumption that a
      serious-sounding task belongs to the current milestone; checking says
      otherwise. Priority within M3 stays high, and part 1 (populating
      `home_domain`) is worth doing earlier on its own merits, since the column
      is in the API contract and has always shipped empty. Also recorded the
      response shape — `verified_domain`, `verification`, `issuers_with_this_code`
      on both list and detail — and measured that the registry and visible issuer
      counts diverge sharply (NFT: 2,176 vs 1), which makes the choice of number
      a design decision rather than a detail.
  - date: 2026-09-02
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Scoping and the two design decisions are already recorded from
      today's measurement work — SEP-1 coverage, the `blackrock.co.com` finding
      that turns the deliverable into a published domain rather than a boolean,
      and scope item 3's ranking decision. What remains is implementation:
      populate and verify `home_domain` (item 1), expose the domain and the
      how-it-ended enum (item 2), and surface a contested code (item 3).
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Scope item 3 decided. Ordering stays on `volume_24h desc`, no `?verified=`
      filter, and ambiguity is surfaced instead. The filter is rejected on the
      SEP-1 measurement — `?verified=true` would include `blackrock.co.com` and
      exclude a legitimate issuer whose toml was briefly unreachable, which is a
      stronger claim on weaker evidence than publishing nothing. Volume stays
      because it overclaims nothing: it answers "most traded", which is true and
      already documented as an unfiltered total. The real defect is that a flat
      ranked list hides the other 414, so the fix is visibility: the verified
      domain per row, plus a count of how many issuers share the searched code.
      Measured to size it — 341 of 2,884 visible codes have more than one
      issuer, covering 989 of 3,530 visible pairs, so this is a quarter of the
      surface rather than an edge case. Also settled that [[0210]]'s search
      boundary should stay permanently for Soroban: no SEP-1 equivalent means no
      domain to publish beside a self-declared symbol.
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Measured the SEP-1 half. `prices.asset_metadata` holds **zero** rows on
      prod — empty, not sparse — while `home_domain` ships as `""` on every
      response. Scoping to what consumers can see makes this tractable: 1,948
      distinct issuers behind API-visible assets against 59,332 in the registry.
      Coverage is good (48/50 of the top issuers by volume declare a domain,
      43/50 of a random visible sample), far better than the 17/52 that sank
      BE's table as a mechanism in 0210. But running the full bidirectional
      check on the eight highest-volume BTC issuers found that
      **`blackrock.co.com` passes** — a lookalike domain serving a well-formed
      stellar.toml that lists the asset back. SEP-1 proves domain control, not
      legitimacy. So the deliverable changes: publish the verified *domain*, not
      a boolean, because a boolean would launder that into an endorsement this
      API cannot make.
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Sharpened after [[0210]] shipped. The self-declared Soroban names are no
      longer inferred from BE's table — all 52 contracts were resolved directly
      over RPC, confirming the five recorded here exactly. An earlier version of that
      note claimed three more — `USD`, `EUR`, `USDP` — which was wrong: all
      three are SACs faithfully naming their classic asset, and belong to
      [[0242]]. Those symbols are now *published* as
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

| symbol | contracts | SAC? |
|---|---|---|
| `USDT` | **two** — `CBBUOYO3…` and `CCU77UVQ…` | no |
| `USDC` | `CDPV3H7C…` | no |
| `BTC` | `CBOZ6HCY…` | no |
| `XRP` | `CB7OOP3V…` | no |

**The five this task recorded are confirmed exactly, and there are no others.**

⚠️ *A first pass at this section claimed three more — the bare codes `USD` and
`EUR`, and `USDP`. That was wrong, and the correction is the useful part:
checking each against `assets.sac_address` shows all three are **SACs** of
classic assets that genuinely carry those codes, so they name their underlying
asset faithfully. They belong to [[0242]], not here.* The two populations are
disjoint and a symbol alone does not tell them apart — the SAC check does.

Also present, and not impersonation: a prefixed family that merely sits adjacent
to the originals under a prefix search — `yUSDT`, `nBTC`, `nETH`, `BnUSD`,
`USDM1`.

⚠️ Since 0210 these symbols are **published** as `asset_code` / `code`. They
are still absent from the listing, which `INNER JOIN`s `current_prices` and so
requires 24 h volume — but `GET /v1/assets/{contract}` reads `FROM assets` with
only `LEFT JOIN`s, so it already returns `code: "USDC"` for `CDPV3H7C…` today.
The list is a real inventory of what a consumer can be handed, not a forecast.

This is also the concrete argument for the boundary 0210 kept: `?search=` and
`sort=code` read the **stored** `assets.asset_code`, which is `''` for contract
rows, so none of the above is findable by name. Moving search onto the resolved
symbol before this task lands would surface these five alongside the real
issuers — and, because a prefix match does not know a SAC from an impostor,
would mix them with the legitimate wrappers and the `y…`/`n…` family under one
undifferentiated `?search=USD`.

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

## Measured 2026-09-02: SEP-1 works, and does not do what this task assumed

### `asset_metadata` is empty, not sparse

```
prices.asset_metadata          0 rows
```

Zero. The table exists with `asset_id`, `home_domain`, `updated_at` and has never
held a row. Confirmed in code: the only caller of `write_asset_metadata` outside
its own definition is a test (`asset-discovery/tests/enrichment_survives_it.rs:48`).
`home_domain` is nonetheless in the response contract and ships as `""` on every
asset in both listing and detail.

### The population is bounded if scoped to what consumers see

| | |
|---|---|
| distinct issuers, whole registry | 59,332 |
| distinct issuers, assets visible through the API | **1,948** |
| visible `(code, issuer)` pairs | 3,530 |

Verifying the registry is a 59k-account job. Verifying **what a consumer can
actually be shown** is 1,948 Horizon lookups — the same shape as [[0210]]'s
52-contract sweep, thirty times larger, still one Lambda run. Fewer `stellar.toml`
fetches than that, since one domain serves many issuers.

### Coverage is good

Sampled against Horizon:

| sample | declares `home_domain` |
|---|---|
| top 50 issuers by 24 h volume | **48 / 50** |
| random 50 from the API-visible set | **43 / 50** |

Far better than the 17/52 that made BE's `soroban_contract_metadata` unusable as
a mechanism in 0210. SEP-1 has enough coverage here to be worth building on.

### ⚠️ But bidirectional SEP-1 does not answer this task's question

Run end to end on the eight highest-volume `BTC` issuers — Horizon `home_domain`,
then `https://<domain>/.well-known/stellar.toml`, then a `[[CURRENCIES]]` entry
matching `code="BTC"` **and** that exact issuer:

| issuer | domain | verdict |
|---|---|---|
| `GDPJALI4AZ…` | ultracapital.xyz | verified |
| `GBVOL67TMU…` | stellarport.io | verified |
| `GAUTUYY2TH…` | dead.apay.io | verified |
| `GD6PQQAIG5…` | **blackrock.co.com** | **verified** |
| `GCNSGHUCG5…` | interstellar.exchange | toml unreachable |
| `GARRC2RFPP…` | jfkrise.com | toml unreachable |
| `GAYEYN65Z4…` | djtstellar.com | toml unreachable |
| `GCQVEST7KI…` | — | no domain |

`blackrock.co.com` **passes completely**. It serves a well-formed toml:

```toml
[[CURRENCIES]]
code="BTC"
issuer="GD6PQQAIG5FSIBKGM5FH7RUUSKP5V4VT2VWDX5OHDEEZFPPUYAU2RKUR"
status="live"
name="Bitcoin"
```

Note the domain: `blackrock.co.com`, not BlackRock's. Someone registered a
lookalike, published a toml, and the protocol confirms them.

**SEP-1 proves domain control, not legitimacy** — which is exactly what it is
specified to prove. It is a real, checkable fact and it does useful work here:
three of eight could not even serve a toml. But it cannot answer *"is this the
BTC I mean"*, and no bidirectional check can, because the question is about a
name's reputation rather than about cryptography.

### What that changes about the deliverable

**Publish the verified domain, not a boolean.** A `verified: true` on
`GD6PQQAIG5…` would launder `blackrock.co.com` into an endorsement this API
cannot make. A `verified_domain: "blackrock.co.com"` states the checkable fact
and leaves the judgement where it can actually be made — with the consumer, who
can see `.co.com`. This is the reason browsers show a URL rather than only a
padlock, and the same reasoning applies.

So scope item 2 below (*"an enum verified / unverified / unknown"*) is wrong as
written and is superseded: the enum still has a place for **how** the check
ended — verified, domain unreachable, asset not listed back, no domain claimed,
not checked — but the domain string is the field that carries the meaning.

## Scope

Three parts, separable:

1. **Populate `home_domain` and verify it.** Read the issuer's `home_domain`
   from Horizon, fetch `https://<domain>/.well-known/stellar.toml`, and confirm
   the `[[CURRENCIES]]` block lists this exact `(code, issuer)`. Store the
   verdict, not just the domain — an unverified domain claim is worth little on
   its own.
2. **Expose the verdict.** ⚠️ **Reshaped by the 2026-09-02 measurement above** —
   the primary field is the **verified domain string**, not a boolean or a bare
   enum, because a lookalike domain passes SEP-1 and a boolean would endorse it.
   An enum still carries *how the check ended* (verified / domain unreachable /
   asset not listed back / no domain claimed / not checked), since "we have not
   looked" and "we looked and it failed" must not collapse — but it sits beside
   the domain rather than replacing it.
3. **Ranking and search — decided 2026-09-02, see below.** Ordering stays on
   volume; ambiguity becomes visible instead. No `?verified=` filter.

⚠️ **Only after (1)–(3) should `?search=` be extended to Soroban symbols.**
[[0210]] deliberately left `startsWith(a.asset_code, ?)` and `sort=code` on the
**stored** column, so a Soroban token is displayed by its self-declared symbol
but is not findable or orderable by it. Extending search without a verification
signal is precisely what would let a hostile token surface under a well-known
code. There is a test pinning that boundary
(`list_it.rs::search_and_sort_still_read_the_raw_column`); changing it should be
a decision made here.

## Decision: ranking stays on volume, ambiguity becomes visible

Taken 2026-09-02, after measuring SEP-1 and the shape of the ambiguity. Recorded
here with the reasoning because the alternatives are all plausible and the one
that looks most protective is the one the measurement rules out.

### The measurement that decides it

Among API-visible classic assets:

```
distinct codes                         2,884
codes with more than one issuer          341   (12%)
visible (code, issuer) pairs under them  989   (28% of 3,530)
```

Ambiguity is not an edge case. More than a quarter of what a consumer can see
sits under a code that at least one other visible issuer also uses.

### What we cannot do

**There is no canonical `BTC` on Stellar.** Any ordering we choose over 415 `BTC`
issuers implies an answer to "which one is real", and we have no basis for that
answer. So the question is not "order better" — it is "stop implying an answer
we cannot justify".

That eliminates two of the four options outright:

- **`?verified=` filter — rejected.** It is the boolean the SEP-1 measurement
  above rules out. `?verified=true` would *include* `blackrock.co.com`, which
  passes the check in full, and *exclude* a legitimate issuer whose toml happened
  to be unreachable that hour. It would be worse than nothing: a filter named
  "verified" that quietly endorses a lookalike is a stronger claim than an
  unfiltered list, made on weaker evidence.
- **Break volume ties by verification — rejected.** `volume_24h_usd` is a
  high-precision decimal; exact ties essentially never occur, so this changes
  nothing while sounding like it does.

### What we do instead

**Keep `sort=volume_24h desc` as the default.** Not because volume is good
evidence of identity — this task exists because it is not — but because it is
*honestly labelled*. It answers "most traded", which is true, measurable, and
already documented as an unfiltered total (`current.sql:40`). Replacing it with
an identity-derived order would swap a metric that overclaims nothing for one
that overclaims a great deal.

**Make the ambiguity visible in the response.** The real defect is not that the
order is wrong; it is that a flat ranked list *hides* the existence of the other
414. A consumer reading `data[0]` today has no way to know the name they searched
is contested. So:

- each row carries the **verified domain** (from item 2), and
- a search or listing response says **how many issuers share this code**.

That turns "here is BTC" into "here is one of 415 things called BTC, issued by
whoever controls ultracapital.xyz" — which is the true statement, and is
actionable by a consumer in a way a rank never is.

**Leave the default listing alone.** Ranked by volume across all assets, it is a
"most traded" view and volume is exactly the right metric for it. Nothing here
changes it.

### Consequence for [[0210]]'s boundary

`?search=` stays on the **stored** `assets.asset_code`, so Soroban symbols remain
unfindable — and, unlike the classic side, that should not change even after this
task lands. Soroban tokens have no SEP-1 equivalent and no `home_domain`, so
there is no verified domain to publish beside them; extending search to their
self-declared symbols would surface the five impersonating contracts with nothing
to qualify them. The boundary is pinned by
`list_it.rs::search_and_sort_still_read_the_raw_column`.

### What this decision is not

It does not make the API safe to resolve names against. It makes the API stop
implying that it is. A consumer who needs *the* USDC still has to know Circle's
issuer, and this task's job is to give them enough on the response to check that
they got it — not to guess it for them.

## Proposed table shape — 2026-09-02, awaiting agreement

Drafted at the end of the session, **not yet agreed**. Nothing is implemented.

```sql
CREATE TABLE IF NOT EXISTS prices.asset_verification (
    asset_code      String,
    issuer_address  String,
    home_domain     String                  DEFAULT '',
    verdict         LowCardinality(String)  DEFAULT '',
    attempts        UInt8                   DEFAULT 0,
    checked_at      DateTime                DEFAULT now()
)
ENGINE = ReplacingMergeTree(checked_at)
ORDER BY (asset_code, issuer_address);
```

**Keyed on the pair, not the issuer.** Two facts with different scopes:
`home_domain` belongs to the *account* (1,953 Horizon lookups), the SEP-1
back-reference to the *pair* (3,522). A pair key lets the API join on what
`assets` already has, at 1.8× duplication of the domain string — measured, and
negligible. The worker dedupes Horizon calls by issuer in memory. Not `asset_id`,
for [[0139]]'s reason: 3,300 ids are ambiguous.

**`verdict` carries how the check ended**, because today's measurement showed
these are different facts and a boolean loses them:

| value | meaning |
|---|---|
| `verified` | domain declared, toml reachable, `[[CURRENCIES]]` lists this pair |
| `not_listed` | domain and toml fine, but this asset is not in it |
| `unreachable` | domain declared, toml could not be fetched or parsed |
| `no_domain` | the account declares no `home_domain` |
| `''` | not checked yet |

Three of the eight BTC issuers sampled were `unreachable`, which is a different
statement from `not_listed` and should not collapse into one "unverified".

### ⚠️ The one place this must NOT copy [[0210]]

0210 triggers on **absence** of a row and does nothing in steady state, because
`symbol()` is fixed at contract deploy. **That reasoning does not transfer.**
`home_domain` is mutable: an issuer can change it, a domain can lapse, a toml can
be rewritten. `blackrock.co.com` could be gone tomorrow, and a legitimate issuer
that was `unreachable` this hour may verify next week.

So the queue needs: no row **or** `checked_at` older than a threshold **or** a
failure under the attempt cap. That is the config surface 0210 deliberately
removed — justified here because the underlying fact genuinely changes.

Sizing: 1,953 issuers ÷ 50 per hourly run ≈ 1.6 days for a full cycle, so a
7-day threshold leaves ample margin.

### What the API publishes

`home_domain` **only when `verdict = 'verified'`**, with `verdict` always present.
An unverified claim must not be rendered the way a verified one is — which is the
whole point of the `blackrock.co.com` finding above.

### Response shape — 2026-09-02, also awaiting agreement

Three fields, on **both** `AssetListItem` and `AssetDetail`:

```json
"verified_domain": "ultracapital.xyz",
"verification":    "verified",
"issuers_with_this_code": 25
```

**`verified_domain` is populated only when `verification = 'verified'`.** On any
other verdict it stays empty and the enum says why. An unverified claim rendered
the way a verified one is would be the `blackrock.co.com` mistake in a different
place.

**The enum is not optional decoration.** An empty `verified_domain` on its own
collapses four different facts: no domain declared, toml unreachable, toml fine
but asset not listed, and **not checked yet** — which is what every row will say
for the first days after the writer ships. Without the enum, "we have not looked"
is indistinguishable from "we looked and it failed".

⚠️ **Soroban tokens need a fifth value.** They have no `home_domain` and no SEP-1
equivalent, so `verified_domain` is permanently empty for them. Something like
`not_applicable`, so they do not read as classic assets we simply have not got to
yet.

### ⚠️ `issuers_with_this_code` — which number, measured 2026-09-02

The registry count and the API-visible count are wildly different, and they mean
different things:

| code | issuers in registry | visible through the API |
|---|---|---|
| NFT | 2,176 | **1** |
| XRP | 683 | 60 |
| BTC | 416 | 25 |
| ETH | 317 | 13 |
| USDC | 228 | 20 |

**Leaning to the visible count**, because it is consistent with the same response:
a consumer can page the list and confirm it. The registry count is a number they
cannot act on from here.

But note what that costs: `NFT` would publish **1**, which is true of this API and
misleading about Stellar, where 2,176 accounts have issued something called NFT.
Whichever is chosen, the field name has to carry it — `no_of_issuers` reads as
"how many exist" and would be wrong under either reading. Hence
`issuers_with_this_code`, documented as counting what this API can return.

### Where the exposure actually is

Both fields matter more on **detail** than on the listing. The listing
`INNER JOIN`s `current_prices`, so a zero-volume impersonator never appears.
`asset_detail` reads `FROM assets` with only `LEFT JOIN`s and has no such floor —
`GET /v1/assets/CDPV3H7C…` returns `code: "USDC"` today for a contract that is not
a SAC of USDC. Shipping these fields to the listing alone would leave the one
surface where the problem is live untouched.

### Open questions for the next session

1. **Table name** — `asset_verification` (names the act) or `asset_domain`
   (names the data)? Leaning to the first, since `verdict` is the payload and the
   domain is supporting evidence.
2. **Staleness threshold as a constant or an env var?** Leaning to a 7-day
   constant, revisited if it proves too slow.
3. **Scope to API-visible issuers only?** 1,953 visible against 59,332 in the
   registry. Leaning to visible-only: the rest cannot be handed to anyone.
4. **`issuers_with_this_code` — visible or registry count?** See the measurement
   above. Leaning to visible, with the field named and documented so the choice
   is legible.

Also note: `prices-clickhouse/src/lib.rs`'s statement-count guard will need
bumping again (33 → 34) when this lands, as it did for [[0210]].

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
- [ ] The API publishes the **verified domain**, not only a verdict — measured
      2026-09-02: `blackrock.co.com` passes bidirectional SEP-1 in full, so a
      boolean would endorse a lookalike
- [ ] Verification is bidirectional (SEP-1): an issuer claiming a domain is not
      enough; the domain's `stellar.toml` must list the asset back
- [ ] For a code with many issuers — `BTC` (415) is the sharpest classic case —
      a consumer can tell the canonical one from the rest **without** relying on
      volume ordering
- [x] A decision is recorded on whether search and ranking use the signal, with
      its reasoning — **decided 2026-09-02**: ordering stays on volume, no
      `?verified=` filter, and ambiguity is surfaced instead (see the Decision
      section). The implementation of that decision remains open below
- [ ] A search or listing response makes a contested code visible — 341 of 2,884
      visible codes have more than one issuer, and today a flat ranked list hides
      that entirely
- [ ] Soroban tokens are either verifiable by some stated mechanism, or
      explicitly labelled unverifiable — not silently indistinguishable
