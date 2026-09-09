---
title: "BE 0199 reply — short version, handed over for sending"
type: generation
status: mature
spawned_from: notes/G-be-0199-reply.md
spawns: []
tags: [be-facing, contract, prices-api, handed-over]
links: []
history:
  - date: 2026-08-05
    status: mature
    who: okarcz
    note: >
      Condensed from [[G-be-0199-reply]] at the operator's request — plain
      language, no internal jargon, short enough to read in two minutes.
      **This is the version handed over to be sent**; the long draft stays as
      our record of the reasoning behind each answer. Handed over 2026-08-05;
      the operator sends it themselves, so this task's "reply sent" criterion
      closes when they confirm.
---

# BE 0199 reply — short version

> **Status: handed to the operator 2026-08-05 for sending.** Verbatim text
> below. Long-form reasoning: [`G-be-0199-reply.md`](G-be-0199-reply.md).
> Measurements: [`G-phase0-prod-queries.md`](G-phase0-prod-queries.md).

---

Hi — we've been through your three findings. All three are real: two are bugs on
our side, one is a fair request. Everything below is measured on prod.

## 1. XLM price_usd = 0 — our bug

The query that picks the newest USD price doesn't skip rows that aren't priced
yet. USD prices are filled by a separate job running hourly; XLM trades every
minute, so its newest candle is almost always unpriced. That's why you see 0
nearly always, not occasionally.

Fix: we'll publish the newest _priced_ close. The value can then be up to ~50
min old (~25 avg) but it will always be a real number. If stale-but-real is
worse for you than nothing, say so now — you're the only consumer.

The same bug hits `sources` and `vwap_24h`: a venue whose newest candle isn't
priced yet vanishes from both, so `vwap_24h` is currently averaged over only
some venues with nothing saying so. It drops the _busiest_ venues first (sdex
was missing from XLM). Same fix, same release.

On 0039: it's finished and archived, and the updater it described was never
built — it became the `mv_current_prices` view you see ticking. Native XLM
pricing is owned by our task 0135.

## 2. Materialising price_usd_series — yes

Our schema pre-authorised this; your measurement is the trigger.

Two corrections. We assumed half your scan was our duplicate-identity bug. We
measured it: **+4.7%, not 2×**. Fixing it won't speed you up and we won't
pretend otherwise. Your 4.6 s is the group-by and weighted average, not the
join — our identical join costs 344 ms, so the materialised table does attack
the right thing.

But those duplicate identities are a _correctness_ problem for you: **548,439
daily rows are published under identities that never traded them**, mostly in
the long tail. If you key on natural identity, check yours. We'll fix it before
materialising.

## 3. The dust print — our bug

Your 12:00 bucket reads volume 42,037.752 — your 42,038, so we're on the same
rows. Your 13:00 bucket that read 1.3085 now reads 0.16931. That's the real
problem: **a closed historical bucket silently changed value.**

On your two options:

- "wait until everything is enriched" can't terminate — some pairs can never be
  priced (see §4).
- "weight in the unenriched rows" measures **0.000023 against a true ~0.170**:
  an unpriced row enters as a zero at full weight. Please don't.

We'll ship a coverage gate and expose the coverage share so you can set your own
bar. Note ~17% of buckets sit at exactly 50% coverage _permanently_, because
path payments book one trade against two quotes and we can only price one — so
the gate measures coverage against **priceable** volume, not total.

Good news: every wrongly-zeroed rollup row we found is in the bucket currently
being formed, and repairs itself once the bucket closes. This costs you the live
edge, not your history.

## 4. What you didn't ask about — please read this one

**About two-thirds of our daily candles have no USD price at all, and never
have.** Stable for 24 months.

We only price a candle when its _quote_ asset is USDC, USDT or XLM, or has an
oracle. Everything else stays empty — including yXLM-quoted candles, which are
never priced even though we price yXLM itself fine (114,330 candles in 7 days).
Same for XRP.

**None of the fixes above change this.** If your pools trade mainly against
other quotes, you'll get sparse series from us regardless. Please check your
asset list against this before sizing your work. We're adding a second pivot
step — pricing anything quoted in an asset we already price — which is the
biggest improvement available to you, and your report is what surfaced it.

## Meanwhile

Your `price_usd_series_1h` workaround for #1 walks straight into #3. Until the
gate ships, don't trust a single hour's close — use a multi-hour median, or
check that neighbouring hours agree.

Order of work: pre-roll scripts → XLM fix → rollup fix → enrichment cost →
pivot step → coverage gate → identity fix → materialised table. The first two
are days away; the table is last because we won't bake the identity bug into it.

Thanks for measuring — you found things our own tests didn't.
