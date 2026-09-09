SELECT asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct,
       volume_24h_usd, market_cap_usd, vwap_24h, sources, updated_at
FROM prices.current_prices FINAL
WHERE asset_id = 4
FORMAT CSVWithNames
