SELECT timestamp, asset_id, quote_asset_id, source, close, close_usd, volume_quote_usd
FROM prices.price_ohlcv_1m FINAL
WHERE asset_id IN (4,5,70,108,430,741)
  AND timestamp >= toDateTime('2026-08-25 13:22:00')
  AND timestamp <= toDateTime('2026-08-26 13:22:00')
ORDER BY asset_id, source, timestamp
FORMAT CSVWithNames
