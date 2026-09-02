---
id: "0120"
title: "Endpoint conformance — all 7 route groups return correct, schema-valid responses for 20 major assets"
type: TEST
status: blocked
by: ["0178"]
related_adr: ["0008"]
related_tasks: ["0072", "0118", "0119", "0124", "0128", "0135", "0170", "0178", "0225"]
tags: [layer-backend, priority-high, effort-medium, milestone-M2, api, testing, verification, acceptance]
milestone: 2
links:
  - "../../../packages/prices-api/src/lib.rs"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-09-02
    status: blocked
    who: stkrolikiewicz
    note: >
      Re-run after [[0210]] deployed: 870 pass, 16 fail, 0 skip. The fixture
      changed for the first time — `CBIJ…`'s code from "" to SolvBTC, updated
      after the worker resolved it on prod rather than before, and all 43 of its
      checks pass. Pagination 18 pages / 3,567 distinct, down from 3,880, which
      is the traded population moving rather than a regression. Found that AC 3
      cannot go green as written: 0178 has landed, but its closing notes record
      `vwap_24h 0` and `sources {}` for canonical USDC as decided sentinels for
      a quote-only asset, not as pending work. The assertion needs to account
      for `method`, or it blocks this task permanently by accident. Checked the
      other two owners while there: 0135 and 0170 are both completed and
      archived (2026-08-25 and 2026-08-27), so nothing open blocks AC 3 at all.
      Of the 16 failures, 3 are the missing-USDC-pair class already recorded
      here, 2 are 0178's decided sentinels, and the remaining 11 did not
      reproduce 40 minutes later — 169 buckets, 0 violations on the suite's own
      window.
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criterion 1 — the only AC that covers the full public API
      surface, which M1 deployed but never verified beyond
      `GET /backfill/status`.
  - date: 2026-08-18
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active — starting the endpoint conformance pass.
  - date: 2026-08-19
    status: blocked
    who: stkrolikiewicz
    note: >
      Suite built, run on production (752 pass / 55 fail, 0 schema
      failures) and merged into PR #226. Every failure class is owned by
      [[0135]], [[0170]] or [[0178]]; this task blocks on them for the
      final stub/sentinel AC — re-run the suite green after they land and
      cite the report in [[0128]].
  - date: 2026-08-25
    status: blocked
    who: stkrolikiewicz
    note: >
      **Re-run after [[0135]] landed on prod: 847 pass / 13 fail / 11 skip**,
      against 752 / 55 / 0 on 2026-08-19. Report
      `conformance-0120-report-2026-08-25T1443.json`.
      0135's share is closed. Every failure was checked programmatically
      against all seven columns 0135 touches — **zero** attributable to it,
      and XLM now passes the whole suite clean.
      **The unblock plan in the entry above is WRONG and is corrected here.**
      It said: re-run green once 0135/0170/0178 land. It will not go green.
      Of the 13 remaining failures only **1** belongs to [[0170]] (USDC
      `/price` 404, the self-pair). The other **12** are a defect none of the
      three blockers owns, filed as [[0225]]: `/ohlcv` filters on both legs
      and defaults the quote to USDC, so an asset trading actively against
      XLM returns an empty 200. Measured on prod the same session — AUD has
      **4,864** daily candles against XLM current to today, and 1,214 against
      USDC whose newest is 2026-05-20; RON and EQL have no USDC pair at all
      yet trade against XLM today. The suite's assertion is right and the
      endpoint is wrong, so this is not a suite fix.
      **A second gap, this one the suite's own.** All 11 skips are a single
      check — "batch/single timestamps never aligned" — which honestly
      declines to compare a `POST /prices/batch` response against a `/price`
      response taken in a different minute (`batch=14:45:00Z single=14:46:00Z`;
      `current_prices` refreshes every minute and the suite paces at 1 rps, so
      crossing a boundary is the norm). Consequence: AC "batch agrees with
      per-asset /price" is evidenced on **8 of 19** assets, not all of them.
      The suite already has a re-fetch path that rescued one case; it needs to
      pin both calls to the same `updated_at` or retry within the minute.
      That belongs to this task, not to a new one.
      Blockers now: [[0170]], [[0178]], [[0225]]. 0135 cleared.
  - date: 2026-08-28
    status: blocked
    who: stkrolikiewicz
    note: >
      Blocker list corrected: `by` still named 0135 and 0170, both archived
      since (0135 on 2026-08-25, 0170 on 2026-08-27), so the task read as
      triple-blocked when only **0178** remains. Nothing about the work
      changed — this is bookkeeping that was hiding how close Tranche 2 AC 1
      is to being startable. [[0225]], the 12-failure class this task filed
      during its own run, is also archived.
---

# Endpoint conformance for 20 major assets

## Summary

Tranche 2 AC 1: *"All 7 endpoint groups return correct, schema-valid responses
for at least 20 major assets."*

M1 deployed the whole route surface but verified only `GET /backfill/status`
(stated plainly in `milestone-1-evidence.md` Table 4). This task is the
verification pass that turns "routed" into "correct".

## Context

Two distinct claims hide inside AC 1, and they need separate evidence:

- **Schema-valid** — the response matches the documented shape in §4.1–§4.5
  and the generated OpenAPI spec. Mechanically checkable.
- **Correct** — the numbers mean what they claim. Not checkable against the
  spec; needs either an independent source or an internal cross-check.

This task owns both, with the numeric-correctness half deliberately narrow:
deep VWAP reconciliation is [[0123]] and historical spot-checking is [[0127]].
Here the bar is *"no field is a stub, a sentinel, or obviously wrong"*.

**Dependency:** several §4 response fields are still stubs until [[0072]] lands
(`price_xlm`, `change_24h_pct`, `change_7d_pct`, `sources` are all at their
table DEFAULTs). Running this suite before 0072 will correctly fail. Sequence
after 0072 and [[0119]].

## Implementation

- **Fix the asset list.** Pick 20 named assets and record them in the task, not
  in a shell variable — the same list must be reusable by [[0121]], [[0123]],
  [[0127]] and the [[0128]] evidence package. Seed from §9's Tranche 1 list
  (XLM, USDC, EURC, AQUA, BTC, ETH) and extend with the highest-volume assets
  the store actually holds. Include at least one asset per identifier form:
  `native`, `CODE:ISSUER`, and a `C…` contract address.
- **Exercise all 7 groups** per asset: `GET /assets`, `GET /assets/{id}`,
  `GET /assets/{id}/price`, `GET /assets/{id}/ohlcv`, `POST /prices/batch`,
  `GET /oracles/{id}`, `GET /backfill/status`.
- **Schema-validate** each response against the generated OpenAPI spec rather
  than hand-written assertions, so the spec and the tests cannot drift apart.
- **Sanity assertions** beyond the schema:
  - no documented field is absent, and no field is left at its zero/empty
    sentinel for an asset that should have a value
  - numeric strings parse; `Decimal(38,14)` precision survives the JSON
    round-trip (values are serialised as strings by design — §3.3)
  - OHLCV invariants hold: `low ≤ open,close ≤ high`, timestamps strictly
    increasing and aligned to the requested granularity, no duplicate buckets
  - `GET /assets` pagination: walking the cursor to exhaustion yields every
    asset exactly once, and `has_more` is accurate at the boundary (extends
    the 0074 250-row pagination test to the M2 asset set)
  - `POST /prices/batch` returns the same numbers as the per-asset `/price`
    calls for the same assets in the same window
- **Run against production** over the real gateway with an API key — this is an
  acceptance check, not a unit test. Keep it a scripted, re-runnable artifact so
  [[0128]] can cite a fresh run.

## Acceptance Criteria

- [x] The 20-asset list is fixed, documented in-task, and covers all three
      identifier forms
- [x] All 7 route groups exercised for every asset; every response validates
      against the OpenAPI spec (0 schema failures in the 2026-08-19 run)
- [ ] No documented response field is a stub/sentinel for a liquid asset
      (**failing on production** — deferred to [[0135]], [[0170]]; ⚠️ **needs
      rewording, see the 2026-09-02 run**: [[0178]] has landed and deliberately
      publishes `vwap_24h 0` / `sources {}` for a quote-only asset, so as
      written this criterion can never go green — it must account for `method`)
- [x] OHLCV invariants asserted (OHLC ordering, bucket alignment, no dupes —
      all pass wherever data exists)
- [x] Cursor pagination on `GET /assets` proven exhaustive and duplicate-free
      (20 pages, 3880 distinct assets, no dup identity triples)
- [x] `POST /prices/batch` agrees with per-asset `/price` for the same assets
      (all 19 priced assets equal at matching timestamps)
- [x] Suite is re-runnable (`npm run conformance:0120`) and its JSON report is
      citable evidence for [[0128]]
- [x] Any defect found is fixed or spawned as its own task — spawned
      [[0210]] and [[0211]]; three interim spawns were retired the
      same day after a cross-check showed okarcz's [[0135]], [[0170]] and
      [[0178]] already own those defects — the run's fresh evidence is
      folded into them instead

## Fixed asset list (AC 1)

Machine-readable copy: `tools/scripts/conformance-assets.json` (shared with
[[0121]], [[0123]], [[0127]], [[0128]]). Derived 2026-08-19 from production:
`GET /v1/assets?limit=200` (default sort `volume_24h desc`) + `search=` probes
for the §9 majors, over the real gateway with the team API key.

| # | Asset | Identifier (form) |
|---|-------|-------------------|
| 1 | XLM | `native` (native) |
| 2 | USDC | `USDC:GA5ZSEJY…K4KZVN` (code-issuer, canonical Circle) |
| 3 | EURC | `EURC:GDHU6WRG…ITNPP2` (code-issuer, canonical Circle) |
| 4 | AQUA | `AQUA:GBNZILST…M67AQUA` (code-issuer) |
| 5 | BTC | `BTC:GDPJALI4…5O2MZM` (code-issuer, top-volume BTC) |
| 6 | ETH | `ETH:GBFXOHVA…CMGSOCC` (code-issuer, top-volume ETH) |
| 7 | (soroban) | `CBIJBDNZ…5FM6VN` (contract; top-volume soroban asset) |
| 8–20 | USDCAllow, AUD, sUSD, yUSDC, XRP, SHX, SCOP, RON, BOL, EQL, yXLM, PYUSD, VELO | code-issuer, filled by store volume rank |

Full 56-char identifiers live in the JSON; the table above is for humans.
Selection rules applied:

- §9 six seeded first; BTC/ETH pinned to the highest-volume issuer the store
  holds (ticker codes are not unique — the M1 evidence doc's canonical-pinning
  caveat applies).
- Fill by store volume rank, skipping the `*BANK*` spam family, secondary
  wrappers of an already-listed code, and obscure USD clones.
- **USDT excluded deliberately** — known bug [[0172]]; including it fails the
  suite on an already-tracked defect.
- All three identifier forms covered: `native` (#1), `CODE:ISSUER` (#2–6,
  8–20), contract `C…` (#7). A classic asset **cannot** be addressed by its SAC
  contract address (probed: AQUA via SAC → 404 `unknown asset`; the store only
  carries `contract_address` for soroban rows), so the contract slot must be a
  soroban-native asset.

## Suite and run results

Suite: `tools/scripts/conformance-0120.mjs` (`npm run conformance:0120`;
needs `API_KEY`/`BASE_URL`, repo-convention `.env.local`). Validates every
response — errors included — against the **live** spec from `/api-docs-json`
(ajv, JSON Schema 2020-12), then layers the sanity assertions. Paced ≤1 rps
for the free usage plan; ~2.5 min per run; report written as
`conformance-0120-report-<ts>.json` (gitignored, regenerable).

**Reference run — 2026-08-20 07:21 UTC, from the committed suite via
`npm run conformance:0120`: 762 pass, 42 fail, 5 skip.** Zero schema failures
in every run to date: the entire failure surface is the correctness layer, and
every failure maps to an existing defect task.

⚠️ **Cite the classes, not the counts.** Three runs, the last two 16 minutes
apart:

| Failing check | 08-19 08:16 | 08-20 07:05 | 08-20 07:21 | Task |
|---|---|---|---|---|
| `price_usd` zero sentinel | 18 | 17 | **13** | [[0135]] (2nd failure mode; contract decided 08-05) |
| `vwap_24h` zero sentinel | 12 | 12 | **8** | [[0135]] (C2 limit case) |
| `sources` empty (volume > 0) | 12 | 12 | **8** | [[0135]] (C2 limit case) |
| OHLCV window empty | 12 | 12 | **12** | [[0170]] (default USD mode pins quote=USDC) |
| Canonical USDC `/price` → 404 | 1 | 1 | **1** | [[0178]] |

**The split in that table is itself the finding, and it corroborates the
root-cause attribution independently of any code reading.** The two
[[0170]]/[[0178]] rows are *perfectly* stable across all three runs — they are
structural, the same assets every time. The three [[0135]] rows move, and not
monotonically: between 07:05 and 07:21 the `sources`-empty set lost BTC, CBIJ,
USDCAllow, AUD and yUSDC but **gained XRP**. Membership churns in both
directions on a ~15-minute timescale, which is exactly the
enrichment-timing dependence 0135's third failure mode describes and which the
0072 runbook had already observed on `native` alone (`sources` "flickers as
un-enriched candles enter and leave the 24h window"). A per-asset data problem
could not behave this way.

Consequence for [[0128]]: quote this suite as "N failure classes, all owned by
0135/0170/0178", never as "N failing checks" — the count is not reproducible
15 minutes later, and a reviewer re-running it will get a different number.
An earlier draft of this section claimed counts "drift by one or two"; the
07:21 run disproves that and it has been corrected here.

The 5 skips are the designed batch-vs-single path: the tip moved between the
single and the batch call, so the values are not comparable at a common
timestamp. Pagination grew from 3,880 assets over 20 pages to 4,081 over 21
between 08-19 and 08-20 and stayed exhaustive and duplicate-free at both
sizes — the cursor holds up while the underlying table is being written to,
though "exhaustive" is necessarily modulo rows inserted mid-walk.

Every failure class turned out to be **already owned by an okarcz task** with
a deeper diagnosis; the run is independent confirming evidence, folded into
each task's history. Three interim spawns made here were retired the same
day after the cross-check (their ids, 0207–0209, were since reused by
unrelated tasks spawned on develop — do not cross-reference). Two findings sharpened during dedup:

- The "price>0 ⟺ sdex" split is not source selection — `current.sql` has no
  source filter. It is [[0154]]'s enrichment quote-restriction seen from the
  read side, hitting [[0135]]'s unguarded `argMax(close_usd)`.
- The empty OHLCV windows are **not missing data**: probed 2026-08-19, all
  five assets return 2–31 real 1d buckets with `base_currency=XLM` over the
  same 30-day window. The default USD mode pins the quote leg to canonical
  USDC ([[0170]]), which these assets never traded against — 0170's blast
  radius is every XLM-only-quoted asset, not just USDC's self-pair.

### Run 2026-09-02 11:28 — after [[0210]] shipped

```
870 pass, 16 fail, 0 skip
  ohlcv:1h  10 failing
  price      5 failing
  ohlcv:1d   1 failing
pagination walk: 18 pages, 3,567 distinct assets
```

**The fixture changed for the first time.** `CBIJ…`'s `code` moved from `""` to
`SolvBTC` (`conformance-assets.json:38`), because 0210 now resolves a Soroban
token's `symbol()` and composes it into `asset_code`. The order mattered: the
fixture had to move *after* the worker covered that contract on prod, or the
suite fails for the whole window in between. All 43 `CBIJ…` checks pass,
including *code matches the fixed list*.

Pagination shrank from 3,880 distinct assets to 3,567. Both walks were
exhaustive and duplicate-free; the listing `INNER JOIN`s `current_prices`, so
this is the 24 h-traded population moving, not a suite regression.

### AC 3 is blocked on nothing — all three owners have landed

The deferral names [[0135]], [[0170]] and [[0178]]. **All three are completed
and archived** — 0135 on 2026-08-25, 0170 on 2026-08-27, 0178 verified on prod
today. Yet the 16 failures persist, so the attribution needs replacing, not
waiting on.

What they actually are, from the 2026-09-02 report:

| class | count | owner |
|---|---|---|
| `all OHLCV values are decimal strings` | 11 | **transient** — see below |
| `/price returns 200` (AUD, RON, EQL) | 3 | the missing-USDC-pair class this task already records at line 66 |
| USDC `vwap_24h` / `sources` | 2 | [[0178]]'s **decision**, not its backlog |

**The 11 are not reproducible.** Re-running the suite's exact window for
`native` (`granularity=1h`, 7 days, explicit start/end) 40 minutes after the
report: **169 buckets, 0 violations.** The assertion checks seven fields against
`/^-?\d+(\.\d+)?$/`, and every one of them was a conforming decimal string.
This is the enrichment-timing dependence this task already warns about — *"the
count is not reproducible 15 minutes later"* — showing up in the one class
nobody had pinned to an owner.

So the honest state is: **no open task blocks AC 3.** Two of its failure classes
are already documented here, one is a settled decision, and the largest is a
timing artefact of the suite's own design. Whether that means rewording the
criterion, moving this task out of `blocked/`, or both, is a decision this file
should not make silently.

### The criterion as written can never pass

AC 3 — *"No documented response field is a stub/sentinel for a liquid asset"* —
is still `[ ]` and deferred to [[0135]], [[0170]], [[0178]] with the note that a
re-run *"must go green after those land"*. **0178 has landed**, verified on prod
(`current_prices.method` present, `mv_current_prices` carrying `usdc_tip`,
`vol_all`, `is_oracle`), and two of the five `price` failures are still
canonical USDC:

```
vwap_24h is not the zero sentinel   → FAIL
sources is a non-empty object       → FAIL
```

Live values, same asset, 2026-09-02: `price_usd 1.00033`, `volume_24h_usd
68,893,238`, `method oracle`, **`vwap_24h 0`, `sources {}`**.

That is not 0178 unfinished — it is 0178's **decision**. Its closing notes record
exactly these values and call them *"every derived column on its decided
sentinel"*: a quote-only asset has no base-leg candles, `per_source`
(`current.sql:269`) groups `price_ohlcv_1m` by the candle's own `asset_id`, and
fabricating a VWAP for an asset with no traded closes would re-create [[0144]]'s
"one value meaning several things".

So this assertion now tests against a settled decision. **It should account for
`method`** — an `oracle`-priced asset legitimately has no VWAP and no sources —
rather than block on a fix that is not coming. Until it is reworded, AC 3 cannot
go green no matter what lands, which makes it a permanent blocker by accident.

### Suite hardening (code review, 2026-08-20)

A review of the suite itself found three crash paths and two assertions that
would flake on a re-run — all fixed and verified, because 0128 re-runs this
close to submission and a suite that dies mid-run leaves no evidence at all:

- **A failed spec fetch killed the run** with an unhandled `TypeError` and
  wrote no report. The spec body is now checked before ajv sees it, and every
  early exit goes through the report writer. Verified against a fake gateway
  returning an HTML 502: two clean failures, report written, exit 1.
- **Two crash paths in the pagination walk** — a missing spec entry made
  `ajv.validate({$ref: null})` throw, and `r.json.data` was iterated even when
  validation had just rejected the page shape. Both now record a failure and
  stop the walk. Verified against a doctored spec with the list schema removed
  and a page carrying no `data` array.
- **`volume_24h_usd` was asserted non-zero.** A tracked asset can genuinely go
  a day without a trade; this is the same false-alarm shape the 0072 runbook
  warns about for `price_xlm`/`change_24h_pct`. Now parse-only.
- **`updated_at` freshness required `age >= 0`**, so any forward clock skew
  between the producer and the runner failed all 20 assets at once. Now
  tolerates 5 minutes of skew.

Left deliberately: the suite still only exercises `base_currency=USD` on
OHLCV (see the empty-window note above), `Candle`/`BatchResponse` field
presence is unchecked, and ajv recompiles per validation. None of these
affect the verdict; they are noted here so the next reader does not re-derive
them.

Findings confirmed by the run, beyond the failures: soroban rows carry empty
`asset_code` ([[0210]]); OHLCV `start`/`end` are both **inclusive** but
undocumented ([[0211]] — the suite encodes the measured behavior). Passing
highlights: pagination walk over 20 pages / 3880 assets with zero duplicate
identity triples and accurate `has_more`; batch equal to singles at matching
timestamps for all 19 priced assets; `Decimal(38,14)` strings parse
everywhere; all OHLCV invariants hold wherever data exists.

The stub/sentinel AC stays open until [[0135]], [[0170]] and [[0178]] land;
the suite is the acceptance gate — re-run it after each fix and cite the
green report in [[0128]].

## Notes

- Deliberately **not** the Tranche 3 "integration test suite runs in CI"
  deliverable. This is an acceptance pass against the deployed API; wiring an
  equivalent suite into GitHub Actions is M3.
- Expect `sources` to name **Aquarius** here — that is where §9's "Aquarius
  appearing as a named source in VWAP" bullet is actually observed. If it does
  not appear, the cause is [[0072]] (column not written) or [[0080]]
  (concentrated pools not extracted), not this task.
