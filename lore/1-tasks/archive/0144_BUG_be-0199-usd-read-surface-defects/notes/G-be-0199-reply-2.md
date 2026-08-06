---
title: "BE 0199 reply 2 — the volume_base answer, handed over for sending"
type: generation
status: mature
spawned_from: notes/S-be-0199-response-received.md
spawns: []
tags: [be-facing, contract, prices-api, handed-over]
links: []
history:
  - date: 2026-08-06
    status: mature
    who: okarcz
    note: >
      Closes the loop opened by BE's 2026-08-06 response. Answers the one
      question we owed back (does the identity fan-out inflate `volume_base`),
      corrects our own ambiguous wording on the fan-out direction, confirms the
      re-rank their pool-coverage numbers forced, and asks O3. Same short,
      plain-language shape as [[G-be-0199-reply-short]]. **Handed over
      2026-08-06; the operator sends it themselves.**

      The `volume_base` answer is read from `views.sql`, not measured on prod —
      stated as such in the text, because the three surfaces differ and the
      distinction is the whole answer.
---

# BE 0199 reply 2 — the `volume_base` question

> **Status: handed to the operator 2026-08-06 for sending.** Verbatim text
> below. Reasoning and the SQL it rests on:
> [`S-be-0199-response-received.md`](S-be-0199-response-received.md).

---

Thanks — that closes almost everything. One answer owed, one correction of our
own, and one question back.

## Your question: does the fan-out inflate `volume_base`?

Three different answers depending on which surface you're on. This is read off
the view definitions rather than measured, but the mechanism is unambiguous in
each case.

**The views you actually read (`price_usd_series`, `price_usd_series_1h`): no.**
The group-by includes the identity columns, so the duplicated rows land in
*different* groups. Inside either group each candle appears exactly once, the
`volume_base` you're weighting on is the real traded volume, and your weighted
average is arithmetically correct.

So the fan-out is pure **misattribution, not inflation** — the wrong identity
publishes the right price, computed from the right weights. Your
weighted-average reasoning is safe as it stands. Your own evidence agrees:
identical to 14 decimals is exactly what two groups holding the same
un-duplicated candle set produce.

**If you sum across identities yourself: yes, you double-count.** The volume is
real once but appears under two names. Relevant the moment you compute anything
market-wide.

**`prices.current_price_usd`: yes, genuinely inflated.** That view joins on
`asset_id` with no group-by at all, so a duplicated id emits two complete rows —
`volume_24h_usd` included. If you touch that column, don't sum it over the view
until the identity fix lands.

## A correction on our side

Our line about "548,439 daily rows published under identities that never traded
them" was ambiguous, and reading it as identity→many ids cost you a check. The
direction is the other way: **one `asset_id` maps to two or more natural
identities**. Sorry — our wording, your time.

## Confirming the re-rank

Your pool numbers changed our order. With 44.4% of your 52,369 pools priceable
today, the enrichment cost work and the second pivot step move to the top of the
queue behind the two quick fixes — the pivot step is what moves that percentage,
and nothing else we had planned does. The materialised table moves back, on your
own advice.

We won't quote you a headroom figure yet. Of the pools priceable *ever* but not
in the last 48h, we don't know how many still trade, and that single unknown is
the difference between the pivot step taking you toward 96% and toward ~50%.
Which brings us to:

## Two asks

1. **Do you aggregate `volume_base` or `volume_24h_usd` across identities
   anywhere?** If yes, the second and third answers above are live for you today,
   not theoretical, and we'd want to flag it rather than leave it in a reply.
2. **If you have the per-pool list behind your coverage numbers, we'd take it.**
   You clearly measured it; it would save us re-deriving which of the
   currently-unpriceable pools are still active, and that's the number gating our
   estimate above.
