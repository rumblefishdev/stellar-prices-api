---
name: stellar-prices-api
description: Use when you need the USD or XLM price of a Stellar asset — XLM, USDC, any classic CODE:ISSUER asset or Soroban token — current prices, 24h change and volume, OHLCV candles / price history, oracle readings, or many prices in one call. Covers the Rumble Fish Stellar Prices API (prices across SDEX, Soroswap, Aquarius, Phoenix and SushiSwap, with a cross-venue 24h VWAP), its free API key, endpoints, curl recipes and error handling.
license: MIT
---

# Stellar Prices API

REST API with USD and XLM prices for the assets traded on SDEX and the supported
Soroban AMMs: classic assets, XLM and Soroban tokens. Rumble Fish runs it for
the Stellar ecosystem.

**Free: 100,000 requests/month per key. A key takes about a minute to get.**

## When to use this skill

Horizon and RPC are the source of raw chain data; this API aggregates prices on
top of it. Horizon's `/trade_aggregations` gives candles for one asset pair at a
time, built from classic (SDEX) trades, so swaps on Soroban AMMs are not in it.
RPC returns contract events for a limited window: about 7 days with the default
retention. Use this API when the task needs:

- **A USD price** for any asset, ready to use, without routing its pairs to a
  USD stablecoin yourself.
- **One view across venues**: SDEX and the Soroban AMMs (Soroswap, Aquarius,
  Phoenix, SushiSwap), with a 24h VWAP that filters out low-volume venues and
  outliers, plus a per-venue breakdown.
- **Price history**: OHLCV candles from 1 minute to 1 month, in USD or against
  XLM.
- **Many assets at once**: up to 100 prices in one request.
- **An oracle cross-check**: the latest Reflector reading, to compare with the
  traded price.

This API only reads prices.

## Related skills

The official Stellar skills cover what this API does not:

- Balances, transactions, contract events, ledgers →
  [Stellar data: RPC + Horizon](https://skills.stellar.org/skills/data/SKILL.md)
- Issuing assets, trustlines, the SAC bridge →
  [Stellar assets](https://skills.stellar.org/skills/assets/SKILL.md)
- A dApp or frontend that shows these prices (keep the key on its backend) →
  [Frontend & wallets](https://skills.stellar.org/skills/dapp/SKILL.md)

## Step 1: get the API key (do this first)

Every `/v1` endpoint needs an API key. Before the first call, look for it in
the `STELLAR_PRICES_API_KEY` environment variable, then in a
`STELLAR_PRICES_API_KEY=` line of the project's `.env`. This check reads that
one line without executing `.env` and without printing the key:

```bash
KEY=${STELLAR_PRICES_API_KEY:-$(sed -n 's/^\(export \)*STELLAR_PRICES_API_KEY=//p' .env 2>/dev/null | head -1 | tr -d "\"'\r")}
[ -n "$KEY" ] && echo "key set" || echo "key missing"
```

**If the key is missing, stop and ask the user for it.** Do not call the API
without a key, and do not guess or invent prices. Tell the user how to get one:

1. Open https://sorobanscan.rumblefish.dev/api/?utm_source=stellar-skill
2. Click **Sign in with Discord**. The account must be a member of the
   [Stellar Developers Discord](https://discord.gg/stellardev), must have passed
   the server's membership screening, and must be older than 5 minutes.
3. The first sign-in issues the key, free. The dashboard shows it: click
   **Copy key**. If the dashboard says no key was found, click
   **Generate API Key**.

Then have the user export it, e.g. `export STELLAR_PRICES_API_KEY=...`, or put
it in the project's `.env`.

> **Never suggest Regenerate as a fix.** Each Discord account has one active
> key. **Regenerate** on the dashboard deactivates it within about 30 s and
> issues **no new key until the next quota period** (the 1st of the month,
> 00:00 UTC). It is allowed once per period and cannot be undone, so the user
> can be left without any key for weeks. It does not fix a 403, a 429 or any
> other error. Suggest it only when the key has leaked, and tell the user about
> the wait before they press it. After a regenerate, the dashboard shows the
> date a new key can be issued; from then on it offers **Get my API key**. That
> card talks about a key "suspended" for exceeding the quota: after a
> Regenerate this is expected and is not a penalty.

Handle the key like a secret:

- Never print it, log it, or commit it.
- Never put it in client-side or browser code. Call the API from a backend and
  read the key from the environment there.

## Basics

|                      |                                                                             |
| -------------------- | --------------------------------------------------------------------------- |
| Base URL             | `https://prices-api.sorobanscan.rumblefish.dev/v1`                          |
| Auth header          | `x-api-key: <key>`. `Authorization: Bearer` is rejected with 403.           |
| Spec (no key needed) | `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json` (OpenAPI 3.1) |
| Human docs           | https://sorobanscan.rumblefish.dev/api/docs                                 |

The spec describes every parameter and field in detail. Fetch it whenever
something is not covered here.

**Asset identifiers** (path parameter `{id}`):

- `native` for XLM.
- `CODE:ISSUER` for a classic asset, e.g.
  `USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN`. The code is
  case-sensitive: `yXLM` is not `YXLM`.
- A `C…` contract address for a Soroban token.

Do not pass the Stellar Asset Contract (SAC) address of a classic asset; it
usually returns 404. Use `CODE:ISSUER` for classic assets and `native` for XLM.
If you only know a code, find the issuer with `GET /v1/assets?search=CODE`.

## Endpoints

| Method and path                  | What it returns                                                                                  | Key params                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| `GET /v1/assets/{id}/price`      | Current price, 24h VWAP, change, volume, per-venue `sources`                                     | `min_volume_usd`                                            |
| `POST /v1/prices/batch`          | Current prices for 1–100 assets (`prices`), plus `not_found`                                     | body `{"assets": [...]}`                                    |
| `GET /v1/assets`                 | Paginated asset list with prices                                                                 | `type`, `search`, `sort`, `order`, `limit`, `cursor`        |
| `GET /v1/assets/{id}`            | Metadata: `asset_kind` (`native`/`credit`/`contract`), `code`, `issuer`, `contract`, `is_active` | —                                                           |
| `GET /v1/assets/{id}/ohlcv`      | Candles in ascending time order                                                                  | `timeframe`, `granularity`, `start`, `end`, `base_currency` |
| `GET /v1/oracles/{id}`           | Latest reading per oracle (Reflector); empty `oracles` if none covers the asset                  | —                                                           |
| `GET /v1/backfill/status`        | Progress of the history backfill; `earliest_data_available` is network-wide, not per asset       | —                                                           |
| `GET /health` (no `/v1`, no key) | Liveness                                                                                         | —                                                           |

The list endpoint names the metadata fields differently: `asset_type`,
`asset_code`, `issuer_address`, `contract_address`.

Parameter values for `GET /v1/assets`:

- `type`: `classic`, `soroban` or `all` (default).
- `search`: case-sensitive prefix of a classic asset's code. It never matches
  Soroban tokens.
- `sort`: `price`, `volume_24h` (default), `change_24h` or `code`.
- `order`: `asc` or `desc` (default).
- `limit`: 1–200, default 50.

Parameter values for `GET /v1/assets/{id}/ohlcv`:

- `timeframe`: `1h`, `24h` (default), `7d`, `30d`, `1y` or `all`.
- `granularity`: `1m`, `15m`, `1h`, `4h`, `1d`, `1w` or `1M`. It is
  case-sensitive: `1m` is one minute, `1M` is one month. If you omit it, the API
  always picks one that fits the window.
- `start` replaces the start of `timeframe`. `end` alone shifts the `timeframe`
  window to end there. Both take `YYYY-MM-DD` (00:00 UTC), ISO 8601, or a Unix
  epoch in seconds (milliseconds if 13+ digits). Both ends are inclusive.
- With an explicit `granularity`, a window of more than 5000 candles returns
  400 `invalid_query`. Use a coarser granularity or a narrower window.
- `base_currency`: `USD` (default) or `XLM`. They are different series. `USD`
  converts every trade to USD. `XLM` returns only the asset's trades against
  XLM, not the USD series converted to XLM.

## Runbook

The first lines resolve the key the same way as the check above. Shell state
does not carry over between tool calls, so repeat them in every call. The curl
options send the `stellar-prices-skill/1` user agent and make an HTTP error
visible (`curl: (22) … 403`) instead of letting `jq` print nulls.

```bash
API=https://prices-api.sorobanscan.rumblefish.dev/v1
KEY=${STELLAR_PRICES_API_KEY:-$(sed -n 's/^\(export \)*STELLAR_PRICES_API_KEY=//p' .env 2>/dev/null | head -1 | tr -d "\"'\r")}
H=(-sS --fail-with-body -H "x-api-key: $KEY" -A "stellar-prices-skill/1")
USDC=USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN

# Current XLM price
curl "${H[@]}" "$API/assets/native/price" | jq '{price_usd, change_24h_pct, as_of, price_status}'

# One classic asset
curl "${H[@]}" "$API/assets/$USDC/price"

# Find an asset's identifier from its code (case-sensitive prefix)
curl "${H[@]}" "$API/assets?search=AQUA&limit=5" | jq '.data[] | {asset_code, issuer_address, price_usd, volume_24h_usd}'

# Top 10 Soroban tokens by 24h volume
curl "${H[@]}" "$API/assets?type=soroban&sort=volume_24h&limit=10"

# Hourly USD candles for the last 7 days
curl "${H[@]}" "$API/assets/native/ohlcv?timeframe=7d&granularity=1h" | jq '.data[-3:]'

# Daily candles for a date range, from the asset's trades against XLM
curl "${H[@]}" "$API/assets/$USDC/ohlcv?start=2026-01-01&end=2026-03-31&granularity=1d&base_currency=XLM"

# Several prices in one request (max 100)
curl "${H[@]}" -X POST "$API/prices/batch" -H "content-type: application/json" \
  -d "{\"assets\": [\"native\", \"$USDC\"]}" | jq '{prices: [.prices[] | {asset, price_usd}], not_found}'

# Oracle readings only; fetch /price to compare with the traded price
curl "${H[@]}" "$API/oracles/native"
```

**Paginating `/v1/assets`.** Only this endpoint pages. Keep passing the returned
`cursor`, with the same `sort` and `order`, while `has_more` is true. The loop
below works in both bash and zsh:

```bash
cursor=""
while :; do
  args=(--data-urlencode limit=200)
  [ -n "$cursor" ] && args+=(--data-urlencode "cursor=$cursor")
  page=$(curl "${H[@]}" -G "$API/assets" "${args[@]}") || break
  echo "$page" | jq -c '.data[]'
  [ "$(echo "$page" | jq -r .has_more)" = true ] || break
  cursor=$(echo "$page" | jq -r .cursor)
  sleep 1   # 1 request/second per key
done
```

## Reading the response

- **Prices, volumes and percentages are decimal strings**, e.g.
  `"price_usd": "0.2209"`. Parse them with a decimal type. Some assets trade
  below 1e-7, where a float loses digits.
- **Counts are integers.** Candle price fields can be `null`. That happens when
  the bucket had no price-forming trade, or when the USD conversion has not yet
  caught up with the newest buckets.
- **`price_usd` is the latest traded close.** `vwap_24h` is the 24h VWAP across
  venues. `price_xlm` is `price_usd` divided by the XLM close.
- **`"0"` is a sentinel, not a zero price. It means different things in
  different fields:**
  - `price_usd: "0"` means there is no price. `price_status` is then
    `unpriced`, and `method` and `as_of` are `""`.
  - `vwap_24h: "0"` means no venue qualified. This is always the case for
    USDC.
  - `change_24h_pct: "0"` can mean there is no baseline.

  In a batch, `not_found` lists every asset with no current price: an
  unknown identifier, or a tracked asset not priced yet. Check the identifier
  with `?search=` before assuming it is wrong.

- **Judge freshness by `as_of`, not `updated_at`.** `updated_at` is refreshed
  every minute whether or not the price moved. `as_of` is the trading minute
  the price was read from. For USDC it is the oracle reading's time.
- **`price_status`** is one of:
  - `priced`: the newest price available.
  - `carried`: a real price, but a newer trade has not been priced yet.
  - `unpriced`: no price.
  - `""`: transient, briefly during a deploy. `price_usd` is still a real
    price, but `as_of` may be `""` beside it, so its age is unknown.

  "No price" means `price_status: unpriced`. An empty `as_of` on its own does
  not mean no price.

  USD values are recomputed hourly, so `carried` for up to about an hour is
  normal for an active asset. Do not treat it as an error or reject it with a
  tight freshness threshold.

- **`method`** is one of:
  - `traded`: from the asset's own trades.
  - `oracle`: a rate, currently used only for USDC.
  - `""`: there is no price.
- **`volume_24h_usd`** counts every trade the asset was in, as base or quote,
  unfiltered. `sources[*].volume_24h` counts only base-side trades on venues
  that passed the filters, so the sources sum to less.

## Errors and limits

| Status      | Body                                                                          | What to do                                                                                                                     |
| ----------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| 400         | `{"code": "invalid_id" \| "invalid_query" \| "invalid_body", "message": ...}` | `message` names the bad parameter. Fix it; do not retry as-is.                                                                 |
| 403         | `{"message": "Forbidden"}`                                                    | Missing, wrong or disabled key. Check the key is set and sent as `x-api-key`. Do not suggest Regenerate. No 401 in production. |
| 404         | `{"code": "not_found", "message": ...}`                                       | Unknown asset, one not priced yet, or a wrong path (`"no such route"`). Try `?search=`.                                        |
| 429         | `{"message": "Too Many Requests"}`                                            | Over 1 request/second. There is no `Retry-After` header: wait at least 1 s and retry.                                          |
| 429         | `{"message": "Limit Exceeded"}`                                               | Monthly quota spent. Retrying does not help; it resets on the 1st at 00:00 UTC.                                                |
| 500/502/504 | `{"code": "db_error", ...}`, or a gateway `{"message": ...}`                  | Retry with exponential backoff.                                                                                                |
| 503         | `{"code": "quote_unavailable", ...}` (ohlcv only)                             | The reference asset for that `base_currency` is not tracked. Do not retry: try the other `base_currency` or tell the user.     |

Each key allows 100,000 requests per month and 1 request per second. The
monthly quota resets on the 1st at 00:00 UTC. If a project needs more, the user
can get in touch with Rumble Fish at https://www.rumblefish.dev/contact/.

Keep request volume down:

- Use the batch endpoint instead of looping over `/price`.
- Cache on your side. Prices refresh once a minute, so polling faster gains
  nothing. At the API, `/price` is cached for 10 s and the other GET routes for
  60 s; batch is not cached.
