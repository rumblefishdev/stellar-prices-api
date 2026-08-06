---
title: "The openapi-validator result — 13 errors to 5, and why 5 stays"
type: synthesis
status: mature
spawns: []
tags: [openapi, linting, ibm-openapi-validator, openapi-3-1]
links: []
history:
  - date: 2026-08-06
    status: mature
    who: akot
    note: "Split out of the 0124 README at archive time, per its Future Work plan."
---

## The `openapi-validator` result

`npx ibm-openapi-validator target/openapi.json --errors-only` — **13 errors,
now 5**. Kept here rather than in a spawned task: the fixable half was fixed,
and the rest are decisions with reasons, not open work.

### Fixed (8)

| Error | Count | Why it was real |
| ----- | ----- | --------------- |
| `ibm-integer-attributes` on 5 ledger fields | 5 | A Stellar ledger sequence is `uint32` in the protocol's `LedgerHeader`. The DTOs carry `u64` because ClickHouse returns `UInt64`, so the document promised a range 4 billion times wider than reality. `maximum: 4294967295` is a domain fact, not a limit we impose. |
| `ibm-integer-attributes` on the `limit` param | 1 | The 1..=200 bound was **already enforced** (`limit == 0 \|\| limit > MAX_LIMIT` → 400) and entirely invisible to clients — a caller sending `limit=500` got a 400 the document did not explain. Now `minimum: 1, maximum: 200`. [[0119]] owns extending this to the remaining params. |
| `ibm-operation-summary-length` on `/health` | 1 | utoipa publishes the whole rustdoc as `summary` — 223 characters of maintainer-facing prose where a one-line label belongs. Split into `summary` + `description`. |
| `ibm-integer-attributes` on `Candle.trade_count` | 1 | Reversed on review — see below. `maximum: 9007199254740991` (`2^53 - 1`). |

### Left, deliberately (5)

| Error | Count | Why not |
| ----- | ----- | ------- |
| `ibm-schema-type-format` — "invalid type" | 2 | `Option<T>` renders as `oneOf: [{type: null}, …]`. `"type": "null"` is valid OpenAPI 3.1 / JSON Schema 2020-12; the validator is applying 3.0's type list. |
| `$ref` must not sit beside other properties | 2 | Same construct: `{$ref, description}`. Legal in 3.1, illegal in 3.0. Removing it would mean dropping the field descriptions. |
| `ibm-path-segment-casing-convention` | 1 | `/api-docs-json` is not snake_case. Kept — see below. |

### The `trade_count` reversal

This was originally left as "no truthful maximum exists", reasoning that a trade
count has no protocol bound, `u64::MAX` exceeds JSON's safe-integer range, and
anything smaller would be invented. The first two halves of that are right and
the conclusion still didn't follow: **the ceiling is the safe-integer range
itself.**

`2^53 - 1` is the largest integer an IEEE 754 double represents exactly, and
JSON has no integer type — so above it a client's parser silently rounds
(`JSON.parse("9007199254740993")` yields `…992`). Publishing `maximum:
9007199254740991` therefore states a fact about the wire format, not a limit we
impose: values above it cannot be delivered correctly whatever the database
holds. That is the same *kind* of claim as the ledger bound, sourced one layer
down — the ledger ceiling is a protocol fact, this one is a transport fact.

Stellar's real volumes sit ~10 orders of magnitude below it, so it never binds
in practice and cannot make a future response contradict the document — which
was the actual worry behind the original decision. Same caveat as the ledger
fields, noted in review: it is a published ceiling, not a runtime clamp.

The four 3.1 entries are the same root cause and the concrete form of design
decision #4: they are not quality signals, they are a tool that predates 3.1.
If the project ever concludes it must satisfy a 3.0-era validator, that is a
dependency decision (downgrade utoipa) with a cost far beyond this task.

**Decided: stay at 5.** Reaching 0 is available and was measured, not assumed —
`ibm-openapi-validator -r <ruleset>` accepts a Spectral ruleset, and switching
off `ibm-schema-type-format`, `no-$ref-siblings` and
`ibm-path-segment-casing-convention` produces "passed the validator". It is not
taken, for two reasons.

The document is already right: `type: "null"` and `{$ref, description}` are
valid 3.1, so removing the findings would mean either disabling rules globally
(broader than the two narrow path entries in `.redocly.lint-ignore.yaml`) or
down-converting to 3.0 before linting — and the latter breaks decision #9 by
making the linted document stop being the served one.

Second, and the part that was not known when the six were first accounted for:
**errors are not where IBM's ruleset stops.** At warning level it also reports
`ibm-error-response-schemas` against `ErrorEnvelope`, demanding a `trace` string
and an `errors` array — IBM's error-container shape, not ours. No ruleset toggle
removes that honestly, and satisfying it means redesigning the error body on
every endpoint, breaking every client. So "adopt IBM's validator" is not a lint
cleanup; it is an API redesign plus a utoipa downgrade. That materially raises
the cost of a "yes, IBM's specifically" answer to the open question for Oskar,
and should be said out loud when asking it.

### The path question — decided: keep `/api-docs-json`

The AC allowed "or the agreed public path", and this PR is the last cheap moment
to rename, so it was weighed rather than defaulted:

- It already exists in the axum router and predates this task.
- `milestone-1-evidence.md` documents it as the path the router defines.
- Hyphenated path segments are ordinary REST; snake_case is IBM house style, not
  a spec requirement. `/openapi.json`, the industry convention, fails the same
  rule.

Renaming buys one lint error against churn in a submitted document and a broken
path for anyone already reading it.
