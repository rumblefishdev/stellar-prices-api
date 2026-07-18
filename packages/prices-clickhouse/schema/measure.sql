-- Per-table size + row-count report for the prices DB (task 0060 measurement).
-- Usage: clickhouse over HTTP, FORMAT chosen by caller. Reports on-disk
-- (compressed) and uncompressed bytes after a forced merge (run OPTIMIZE …
-- FINAL first for stable numbers, or rely on background merges).
SELECT
    table,
    sum(rows)                                   AS rows,
    formatReadableSize(sum(data_compressed_bytes))   AS disk_compressed,
    formatReadableSize(sum(data_uncompressed_bytes)) AS uncompressed,
    sum(data_compressed_bytes)                  AS compressed_bytes
FROM system.parts
WHERE database = 'prices' AND active
GROUP BY table
ORDER BY compressed_bytes DESC;
