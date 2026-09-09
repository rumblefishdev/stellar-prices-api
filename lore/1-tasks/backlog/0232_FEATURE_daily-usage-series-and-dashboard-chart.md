---
id: "0232"
title: "Daily requests chart — expose the per-day series /usage already reads, and draw it"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0188", "0193", "0194"]
tags: [layer-backend, layer-frontend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, dashboard]
milestone: 3
links:
  - "../archive/0193_FEATURE_portal-presentable-ui-pass.md"
history:
  - date: "2026-08-25"
    status: backlog
    who: claude
    note: >
      Spawned from [[0193]]. The dashboard frame (`778:2499`) has a
      "Daily requests — April 2026" bar chart with a per-bar tooltip; the data
      it needs is read by the backend already and thrown away before it leaves
      the gateway. Adam chose to ship the rest of the frame first and do this
      as its own slice.
  - date: "2026-08-27"
    status: backlog
    who: akot
    note: >
      Renumbered 0222 → 0226. The id had been taken on `develop` by the
      no-invocations alarm bug (PR #250) before this branch merged it in;
      found by [[0193]]'s review round. The two code comments that cited it
      (`app.tsx`, `app.spec.tsx`) re-pointed in the same change.
  - date: "2026-08-28"
    status: backlog
    who: akot
    note: >
      Renumbered 0226 -> 0232. The id collided a second time: `develop` had
      meanwhile taken 0226 for the oracle-worker registry-load bug, found when
      `develop` was merged into [[0193]]'s branch ahead of PR #249. The two
      code comments that cite it (`app.tsx`, `app.spec.tsx`) re-pointed in the
      same change.
---

# Daily requests chart

## Summary

The dashboard's Monthly Usage card should show a bar per day of the current
period, with the day's request count on hover — the frame's
`Daily requests — April 2026`.

## Context

**The data is already fetched.** `Gateway::usage_of`
(`packages/prices-api/src/portal/keys/gateway.rs`) pages through `GetUsage` and
builds `days: Vec<(i64, i64)>` — one `[used, remaining]` pair per day — and then
`summarize_days` collapses the series to two numbers before returning it. No
extra control-plane call is needed to serve the chart; the series is discarded
a few lines after it is read.

What is missing is the contract: `KeyUsage` carries `used`/`remaining`/`limit`,
`UsageResponse` (`portal/usage/mod.rs`) serialises those, and `PortalUsage` in
`web/portal/src/api/portal.ts` mirrors them. None of the three has a per-day
field.

## Implementation

- **Gateway:** keep the daily series on `KeyUsage` beside the summary. Each
  entry needs the DATE as well as the count — `GetUsage` returns the pairs
  positionally against the queried range, so the date has to be reconstructed
  from `start_date` plus the index. Mind the case `summarize_days` already
  handles: AWS's own period can roll partway through the range.
- **Route:** add `days: [{ date: "YYYY-MM-DD", used: u64 }]` to
  `UsageResponse`. Only `used` — `remaining` is a running balance, not a
  per-day allowance (task 0157's close), and publishing it per day invites
  exactly the misreading 0188 avoided.
- **Cache:** the series rides in `CachedAnswer` with the rest of the answer;
  no second TTL.
- **Frontend:** a bar per day inside the Monthly Usage card, the day's count on
  hover and on focus (keyboard reach is not optional — the tooltip is the only
  place the number exists), and the empty state kept honest: a key AWS has no
  row for yet renders no chart, exactly as it renders no bar today.
- Drawn with SVG or CSS rather than a charting dependency, unless somebody
  argues otherwise: one series of ~31 bars against a fixed axis is less code
  than the wrapper around a library would be, and the bundle is served to every
  visitor of the landing page too.

## Acceptance Criteria

- [ ] `/usage` carries a dated per-day series, and no additional `GetUsage`
      call is made to produce it
- [ ] The days a key existed for but was unused are present as zeroes; the days
      before it existed are absent, not zero
- [ ] The chart renders the current period, labelled with its month
- [ ] Each bar's count is reachable by keyboard as well as by pointer
- [ ] A key with nothing recorded yet renders the card exactly as it does
      today — no chart, no invented zeroes ([[0188]]'s rule)
- [ ] Rust and portal test suites cover the new field and the empty case
