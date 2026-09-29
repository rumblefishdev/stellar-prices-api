---
name: stellar-prices-api
description: Use when you need the USD or XLM price of a Stellar asset — XLM, USDC, any classic CODE:ISSUER asset or Soroban token — current prices, 24h change and volume, OHLCV candles / price history, oracle readings, or many prices in one call. Covers the Rumble Fish Stellar Prices API (VWAP aggregated across SDEX, Soroswap, Aquarius, Phoenix and SushiSwap), its free API key, endpoints, curl recipes and error handling.
---

# Stellar Prices API

REST API with USD and XLM prices for every asset traded on Stellar: classic
assets, XLM and Soroban tokens. Rumble Fish runs it for the Stellar ecosystem.

**Free: 100,000 requests/month per key. A key takes about a minute to get.**

## When to use it instead of Horizon or RPC

Horizon and RPC return raw trades and order books. They give no USD value and no
cross-venue aggregate. Use this API when the task needs:

- **A USD price**, ready to use. Deriving one from Horizon means chaining pairs
  to a USD stablecoin yourself.
- **One price across venues**: a 24h VWAP over SDEX and the Soroban AMMs
  (Soroswap, Aquarius, Phoenix, SushiSwap), with low-volume venues and outliers
  filtered out, plus a per-venue breakdown.
- **Price history**: OHLCV candles from 1 minute to 1 month, in USD or XLM.
  `GET /v1/backfill/status` shows how far back the history reaches.
- **Many assets at once**: up to 100 prices in one request.
- **An oracle cross-check**: the latest Reflector reading next to the traded
  price.

Use Horizon or RPC for balances, transactions, order placement and contract
calls. This API only reads prices.

## Step 1: get the API key (do this first)

Every `/v1` endpoint needs an API key. Look for it in the `STELLAR_PRICES_API_KEY`
environment variable before the first call. Check without printing it:

```bash
[ -n "$STELLAR_PRICES_API_KEY" ] && echo "key set" || echo "key missing"
```

**If the key is missing, stop and ask the user for it.** Do not call the API
without a key, and do not guess or invent prices. Tell the user how to get one:

1. Open https://sorobanscan.rumblefish.dev/api/?ref=stellar-skill
2. Click **Sign in with Discord**. The account must be a member of the
   [Stellar Developers Discord](https://discord.gg/stellardev), must have passed
   the server's membership screening, and must be older than 5 minutes.
3. Click **Get my API key**. The key is issued instantly and is free.

Then have the user export it, e.g. `export STELLAR_PRICES_API_KEY=...`, or put
it in the project's `.env`.

Handle the key like a secret:

- Never print it, log it, or commit it.
- Never put it in client-side or browser code. Call the API from a backend and
  read the key from the environment there.
- Each Discord account gets one key. Replacing it on the dashboard disables the
  old key immediately.

## Basics

|                      |                                                                             |
| -------------------- | --------------------------------------------------------------------------- |
| Base URL             | `https://prices-api.sorobanscan.rumblefish.dev/v1`                          |
| Auth header          | `x-api-key: <key>`. `Authorization: Bearer` is rejected with 403.           |
| Spec (no key needed) | `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json` (OpenAPI 3.1) |
| Human docs           | https://sorobanscan.rumblefish.dev/api/docs                                 |

The spec is the source of truth. Fetch it whenever a parameter or field is not
covered here.

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

| Method and path                  | What it returns                                              | Key params                                                  |
| -------------------------------- | ------------------------------------------------------------ | ----------------------------------------------------------- |
| `GET /v1/assets/{id}/price`      | Current price, 24h VWAP, change, volume, per-venue `sources` | `min_volume_usd`                                            |
| `POST /v1/prices/batch`          | Current prices for 1–100 assets, plus `not_found`            | body `{"assets": [...]}`                                    |
| `GET /v1/assets`                 | Paginated asset list with prices                             | `type`, `search`, `sort`, `order`, `limit`, `cursor`        |
| `GET /v1/assets/{id}`            | Asset metadata (code, issuer, contract, kind)                | —                                                           |
| `GET /v1/assets/{id}/ohlcv`      | Candles in ascending time order                              | `timeframe`, `granularity`, `start`, `end`, `base_currency` |
| `GET /v1/oracles/{id}`           | Latest reading per oracle (Reflector)                        | —                                                           |
| `GET /v1/backfill/status`        | How far back history is available                            | —                                                           |
| `GET /health` (no `/v1`, no key) | Liveness                                                     | —                                                           |

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
  case-sensitive: `1m` is one minute, `1M` is one month. When omitted, the API
  picks a sensible granularity for the window.
- `start` / `end`: `YYYY-MM-DD`, ISO 8601, or Unix epoch seconds. They override
  `timeframe`.
- A window of more than 5000 candles returns 400. Use a coarser `granularity`.
- `base_currency`: `USD` (default) or `XLM`.

## Runbook

Every recipe assumes `STELLAR_PRICES_API_KEY` is set. Send the
`stellar-prices-skill/1` user agent so we can tell agent traffic apart.

```bash
API=https://prices-api.sorobanscan.rumblefish.dev/v1
H=(-H "x-api-key: $STELLAR_PRICES_API_KEY" -A "stellar-prices-skill/1")
USDC=USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN

# Current XLM price
curl -s "${H[@]}" "$API/assets/native/price" | jq '{price_usd, change_24h_pct, as_of, price_status}'

# One classic asset
curl -s "${H[@]}" "$API/assets/$USDC/price"

# Find an asset's identifier from its code (case-sensitive prefix)
curl -s "${H[@]}" "$API/assets?search=AQUA&limit=5" | jq '.data[] | {asset_code, issuer_address, price_usd, volume_24h_usd}'

# Top 10 Soroban tokens by 24h volume
curl -s "${H[@]}" "$API/assets?type=soroban&sort=volume_24h&limit=10"

# Hourly candles for the last 7 days
curl -s "${H[@]}" "$API/assets/native/ohlcv?timeframe=7d&granularity=1h" | jq '.data[-3:]'

# Daily candles for a date range, priced in XLM
curl -s "${H[@]}" "$API/assets/$USDC/ohlcv?start=2026-01-01&end=2026-03-31&granularity=1d&base_currency=XLM"

# Several prices in one request (max 100)
curl -s "${H[@]}" -X POST "$API/prices/batch" -H "content-type: application/json" \
  -d "{\"assets\": [\"native\", \"$USDC\"]}" | jq '{prices: [.prices[] | {asset, price_usd}], not_found}'

# Oracle reading next to the traded price
curl -s "${H[@]}" "$API/oracles/native"
```

**Paginating `/v1/assets`.** Only this endpoint pages. Keep passing the returned
`cursor`, with the same `sort` and `order`, while `has_more` is true:

```bash
cursor=""
while :; do
  page=$(curl -s -G "${H[@]}" "$API/assets" --data-urlencode limit=200 \
    ${cursor:+--data-urlencode "cursor=$cursor"})
  echo "$page" | jq -c '.data[]'
  [ "$(echo "$page" | jq -r .has_more)" = true ] || break
  cursor=$(echo "$page" | jq -r .cursor)
  sleep 1   # 1 request/second per key
done
```

## Reading the response

- **Numbers are decimal strings**, e.g. `"price_usd": "0.2209"`. Parse them with
  a decimal type. Some assets trade below 1e-7, where a float loses digits.
- **`"0"` means "no price"**, not a price of zero. `price_status` is then
  `unpriced`.
- **Judge freshness by `as_of`, not `updated_at`.** `updated_at` is refreshed
  every minute whether or not the price moved. `as_of` is the time of the trade
  the price comes from.
- **`price_status`** is one of:
  - `priced`: the newest price available.
  - `carried`: a real price, but a newer trade has not been priced yet.
  - `unpriced`: no price.

  USD values are recomputed hourly, so `carried` for up to about an hour is
  normal for an active asset. Do not treat it as an error or reject it with a
  tight freshness threshold.

- **`method`** is `traded` (from the asset's own trades) or `oracle` (a rate,
  currently used only for USDC).
- **`volume_24h_usd`** counts both sides of every trade, so it is larger than
  the sum of `sources[*].volume_24h`.

## Errors and limits

| Status | Body                                                                          | What to do                                                                                                |
| ------ | ----------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| 400    | `{"code": "invalid_id" \| "invalid_query" \| "invalid_body", "message": ...}` | `message` names the bad parameter. Fix it; do not retry as-is.                                            |
| 403    | `{"message": "Forbidden"}`                                                    | Missing, wrong or disabled key. Ask the user to check the key. This API never answers 401.                |
| 404    | `{"code": "not_found", "message": ...}`                                       | Unknown asset, no price yet, or a wrong path (`"no such route"`). Look the asset up with `?search=`.      |
| 429    | `{"message": "Too Many Requests"}`                                            | Rate or monthly quota exceeded. There is no `Retry-After` header: back off exponentially starting at 1 s. |
| 5xx    | —                                                                             | Retry with exponential backoff.                                                                           |

Each key allows 100,000 requests per month and 1 request per second. The
monthly quota resets on the 1st at 00:00 UTC. If a project needs more, the user
can get in touch with Rumble Fish at https://www.rumblefish.dev/contact/.

Keep request volume down:

- Use the batch endpoint instead of looping over `/price`.
- Cache responses on your side. Prices are cached for 10 s at the API, and
  everything else for 60 s.
