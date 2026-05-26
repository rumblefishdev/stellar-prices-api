---
id: "0049"
title: "Rewrite design docs (overview + database-schema companion) to match ADR 0007 (Hetzner CH live sink)"
type: DOCS
status: completed
related_adr: ["0001", "0003", "0004", "0007"]
related_tasks: ["0044", "0045", "0046", "0047", "0048", "0011", "0038", "0039", "0040"]
tags: [layer-docs, priority-high, effort-medium, clickhouse, hetzner, overview, design-doc, database-schema, mermaid]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../../../docs/database-schema/amm-trades-schema.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md"
history:
  - date: 2026-05-20
    status: backlog
    who: okarcz
    note: >
      Spawned as the immediate follow-up to task 0045's closure.
      ADR 0007 (live data sink on shared Hetzner CH) is now
      accepted; docs/prices-api-general-overview.md still describes
      RDS Postgres as the live sink throughout §1.1, §2.1, §3,
      §5.2, §6, §8, §10, §11. This task rewrites those sections to
      match ADR 0007's architectural commitment and adds a
      Revision History row recording the change. The local
      backfill sections (Stream 1 ADR 0001, Stream 2 ADR 0005)
      are already up to date and are not in scope.
  - date: 2026-05-20
    status: active
    who: okarcz
    note: >
      Promoted to active immediately on creation — follows
      directly from task 0045's PR #25 (close 0045, accept
      ADR 0007). Branched off lore-0045/close-be-agreement so the
      ADR 0007 accepted state is in the working base; new branch
      is lore-0049/overview-rewrite-adr-0007.
  - date: 2026-05-20
    status: completed
    who: okarcz
    note: >
      Closed. All 12 acceptance criteria met. Doc rewrite touched
      §0, §1.1, §1.2, §2.1, §2.3, §3 (full schema rewrite to
      ClickHouse DDL), §4.5, §5.2, §5.3, §5.4, §5.6, §6, §7, §8,
      §9 Tranche 1, §10 (cost lines), §11.1/§11.2/§11.3/§11.4. Net
      +166 lines (1284 → 1450). Eight sub-task harness steps
      tracked the section groupings. Final grep verified no
      live-state RDS / NAT Gateway / OHLCV Rollup / sqlx
      references remain — only historical context in the
      Revision History and explicit "removed" callouts.
  - date: 2026-05-20
    status: active
    who: okarcz
    note: >
      Reopened from archive. Scope expanded to cover the
      `docs/database-schema/` companion folder, which the original
      task did not include:
      (a) `database-schema-overview.md` — 1540-line companion that
          mirrors the main overview; describes RDS Postgres
          throughout (66 RDS/Postgres refs, 0 ClickHouse, 14
          mermaid blocks). Needs the same ADR 0007 alignment as
          the main overview.
      (b) `amm-trades-schema.md` — describes a now-obsolete
          pre-ADR-0001 design (custom AMM trades table on BE's
          RDS, supposedly created and populated by BE for the
          prices-api backfill). Superseded by ADR 0001 (BE's
          `backfill-runner --target=clickhouse` populating a local
          CH copy of `soroban_events`); should be marked
          superseded with a clear banner.
      (c) `clickhouse-prod-schema.sql` — BE's `default.*`
          production schema reference. Not prices-api's schema;
          no change.
      All Appendix A (ER diagram) and Appendix B (full system
      diagram) mermaid blocks in `database-schema-overview.md`
      need to flip from PostgreSQL types and partitioning to
      ClickHouse engines and sort keys.
      Title updated to reflect the expanded scope. Continues on
      the lore-0049/overview-rewrite-adr-0007 branch (PR #26).
  - date: 2026-05-20
    status: completed
    who: okarcz
    note: >
      Closed (expanded scope). All 12 + 18 acceptance criteria met
      across the main overview rewrite (already committed in
      d1ee0e5 / rebased as 20bb744 on PR #26) and the
      database-schema/ companion rewrite landing in a follow-up
      commit on the same branch. Companion file changes:
      `amm-trades-schema.md` superseded banner added; full
      `database-schema-overview.md` rewrite (1540 → 1906 lines,
      net +366) covers all 13 main sections + Appendices A & B; all
      14 mermaid blocks flipped to ClickHouse types, engines, sort
      keys, MV chain, mTLS edge, and the workstation-local backfill
      topology. Final grep verified no live-state RDS / sqlx /
      PostgreSQL 16 / OHLCV Rollup / ECS Fargate references
      remain in current-state text. Total work for the expanded
      0049: main overview (788 +, 384 -) plus database-schema
      (~1500 + with full rewrites of §3, §7, §8, §10, Appendix B).
---

# Rewrite docs/prices-api-general-overview.md to match ADR 0007

## Summary

ADR 0007 transitioned proposed → accepted (2026-05-20). The design
doc's body still describes the prior RDS-Postgres-shaped
architecture. This task brings every section into alignment with
ADR 0007's Decision points and records the change in the Revision
History.

## Context

- ADR 0007 §3 Decision points: separate `prices` database in BE's
  Hetzner CH; S3 → SNS → Lambda fan-out; per-source
  `ReplacingMergeTree(version)` rows on per-granularity tables;
  CH MV chain replaces the OHLCV Rollup Lambda; mTLS via Caddy:443;
  Lambdas outside any VPC; prices-api owns `prices.*` schema
  unilaterally.
- ADR 0007 §Consequences: no Prices-api VPC / NAT / SG; one fewer
  Lambda; mTLS cert lifecycle added; cross-cloud network path
  (~80-130 ms RTT); `ReplacingMergeTree` is eventually consistent
  (read path uses `FINAL` or `argMax/argMin + GROUP BY`).
- Task 0046's empirical numbers (~0.45 GB/yr, ~74 bytes/ledger,
  14.8× compression) feed the §10 cost section.
- Task 0048's decoder spec already added the §5.5 L1/L2/L3
  layering callout — that part stays.
- Local backfill sections (Stream 1 ADR 0001, Stream 2 ADR 0005)
  are unchanged by this refactor; the cloud push targets shift
  from RDS → CH but the backfill itself remains workstation-local.

## Sections to rewrite

| Section | Change |
|---|---|
| Revision History | Add a new row for today's rewrite, referencing ADR 0007 |
| §0 Deployment & AWS Account | Note CH data plane lives on Hetzner (BE-owned); AWS account scope shrinks (no RDS, no VPC) |
| §1.1 API Layer diagram | Replace "RDS PostgreSQL" with "Hetzner ClickHouse (BE-shared) via HTTPS-mTLS" |
| §1.2 Data Ingestion Layer diagram | Add SNS fan-out between S3 and Lambdas; replace RDS box with CH box; remove OHLCV Rollup Lambda (replaced by CH MV chain) |
| §2.1 Components Hosted | Remove: RDS, NAT Gateway (no VPC), OHLCV Rollup Lambda. Add: SNS topic subscription (one-time BE CDK change), Secrets Manager mTLS material |
| §2.3 Shared with BE | Add: Hetzner CH data plane (separate `prices` DB). Remove: VPC, NAT Gateway (no longer needed prices-side). Update: S3 → SNS fan-out instead of direct S3 → Lambda |
| §3 Database Schema | Rewrite from PostgreSQL native range partitioning to ClickHouse `ReplacingMergeTree(version)` per-source, per-granularity tables (`price_ohlcv_1m`, `_15m`, ..., `_1M`). Backfill progress on `ReplacingMergeTree(updated_at)`. Cleanup becomes `ALTER TABLE … DROP PARTITION` |
| §4.5 GET /backfill/status | Endpoint contract unchanged; underlying read targets CH not Postgres |
| §5.2 Prices Ledger Processor | Write semantics: UPSERT → ReplacingMergeTree INSERT (idempotency via `version` column); HTTPS-mTLS to Caddy:443 instead of in-VPC sqlx |
| §5.3 Ingestion Workers | Remove OHLCV Rollup row (CH MV chain). Retarget other workers from RDS to CH |
| §5.4 EventBridge Scheduler Rules | Remove `ohlcv-rollup` rule |
| §5.5 VWAP layering | Keep — already correct (added by task 0048) |
| §5.6 Historical backfill | Update cloud-push targets from RDS to CH; the local backfill itself is unchanged |
| §6 Performance & Scaling | Replace "RDS Sizing" sub-section with "Hetzner CH (shared)" notes; remove Multi-AZ / read replica / RDS Proxy upgrade path. Add cross-cloud latency mitigation (warm connection reuse, batched per-ledger writes) |
| §7 Security | Update RDS references; add mTLS section (per-env certs, 1-year rotation, CA revocation) |
| §8 Tech Stack | sqlx → `clickhouse` Rust crate; remove "PostgreSQL 16"; add ClickHouse + Caddy mTLS row |
| §9 Delivery Plan (Tranches) | Tranche 1 reflects no RDS, no VPC; Lambdas deploy outside VPC; CDK stack simpler |
| §10 Cost Estimate | Remove RDS line ($12). Add Hetzner CH cost-share line (~$1-2/env/mo per task 0046's empirical numbers). Remove NAT Gateway line (already $0). Remove RDS upgrade-during-backfill line. Note cross-cloud network path is included |
| §11 Infrastructure Sharing | §11.1: add Hetzner CH data plane row. §11.2: add agreement record reference. §11.4: update Stream 1 risk discussion to reflect ADR 0007 |

## Acceptance Criteria

- [x] Revision History gains a 2026-05-20 row linking ADR 0007 and task 0045
- [x] §1.1 and §1.2 diagrams show ClickHouse + SNS, not RDS + direct S3
- [x] §2.1 component list no longer includes RDS, NAT Gateway, or OHLCV Rollup Lambda (these moved to a new "Components no longer in the Prices API budget" sub-list at the end of §2.1)
- [x] §2.3 lists Hetzner CH data plane and the SNS fan-out + mTLS CA as shared components; VPC and NAT Gateway moved to a "no longer shared because not needed" callout
- [x] §3 schema is ClickHouse DDL (`ReplacingMergeTree(version)`, per-granularity tables, MV chain, monthly partitions via `toYYYYMM`) — no PostgreSQL types or partition syntax remain in the live schema sections
- [x] §5 workers list no longer includes OHLCV Rollup; §5.4 EventBridge rules no longer include `ohlcv-rollup`; §5.3 has an explicit "Worker removed: OHLCV Rollup Lambda" callout
- [x] §8 tech stack lists `clickhouse` crate not `sqlx`; ClickHouse on Hetzner + Caddy:443 mTLS not PostgreSQL 16; CDK row notes no VPC/RDS/NAT in synth output
- [x] §10 cost line items reflect ~$1-2/env/mo CH share (per task 0046) instead of $12 RDS; the "Scaled Up" RDS escalation ladder is replaced with the D12-clause and sidecar-CH fallback
- [x] §11 sharing tables include the CH data plane and mTLS CA; agreement record is linked from §11.1, §11.2, and §11.4
- [x] No references to `db.t4g.micro`, `db.m6g.large`, `db.r6g.large`, RDS Proxy, Multi-AZ, or RDS read replica remain in current-state text. RDS occurrences in the body are either (a) explicit "removed" or "no longer" callouts, (b) historical Revision History rows describing prior states, or (c) the §2.3 paragraph documenting the obsolete table rows that previously existed
- [x] §5.5 L1/L2/L3 VWAP layering callout is preserved unchanged (added by task 0048)
- [x] Local backfill sections (Stream 1, Stream 2) preserve their ADR 0001 / ADR 0005 alignment; only the cloud-push target (RDS → Hetzner CH `prices.*`) and §5.6 metric table sink-during-backfill notes changed

### Expanded scope (database-schema/ folder)

- [x] `docs/database-schema/amm-trades-schema.md` carries a clear "SUPERSEDED" banner pointing at ADR 0001, ADR 0007, and BE ADRs 0044/0045
- [x] `docs/database-schema/database-schema-overview.md` Revision History row added (2026-05-20, ADR 0007 driver)
- [x] §1.1 / §1.2 mermaid diagrams show ClickHouse + SNS + mTLS Caddy edge, not RDS + direct S3
- [x] §2 Database Tech Stack lists CH on Hetzner, `clickhouse` Rust crate, MV-chain rollups, mTLS — no PostgreSQL 16, no sqlx
- [x] §3.0 ER overview mermaid block uses CH types (`Decimal_38_14`, `LowCardinality_S`, `FixedString12`) and shows engines / `PARTITION_BY` / `ORDER_BY` pseudo-rows; MV-chain edges between per-granularity OHLCV tables
- [x] §3.1–§3.5 DDL rewritten to ClickHouse: per-granularity `price_ohlcv_*` tables on `ReplacingMergeTree(version)`, MV chain sketch, `prices.assets` / `current_prices` / `oracle_prices` / `backfill_progress` on appropriate engines
- [x] §4 Retention is `ALTER TABLE … DROP PARTITION` per per-granularity table (not row DELETE)
- [x] §5 Indexing reframed as "Sort Keys & Query Patterns" — CH sort key, partition pruning, projections-not-B-tree-indexes
- [x] §6 Workers — OHLCV Rollup Lambda row removed; MV chain noted as the replacement; `ohlcv-rollup` EventBridge rule gone; `FINAL` and `argMax/argMin` noted on the readers table
- [x] §7 Backfill — both streams' diagrams and metric tables reflect local CLIs + Hetzner CH push targets; `task_healthy` / `last_heartbeat` swapped for `last_push_at` everywhere; §7.6 example response uses the new field shape
- [x] §8 Sizing — "RDS Sizing, Performance, Scaling" → "Sizing, Performance, Scaling (Hetzner ClickHouse, shared with BE)"; cross-cloud latency mitigation; sidecar-CH fallback; RDS escalation ladder removed
- [x] §9 Security — mTLS sub-bullets added (per-env certs, 1-year rotation, CA revocation, NotAfter alarm); no-VPC framing; Borg RPO trade-off
- [x] §10 Cross-Service Dependency — rewritten around Hetzner shared tenancy (ADR 0007), not BE-RDS read-only edge; mermaid block shows Hetzner box with `default.*` + `prices.*` co-tenancy, mTLS at Caddy
- [x] §11 What Is Not Shared — RDS PostgreSQL instance row replaced with `prices.*` schema + onboarding portal rows
- [x] §12 Tranche-1 DB Acceptance Criteria — flipped to ClickHouse / no-VPC / SNS / mTLS / push-cadence
- [x] §13 Quick Reference — engine + sort-key + partition columns; one row per per-granularity table
- [x] Appendix A (ER diagram) — full rewrite to CH types, engines, MV-chain edges, no SQL FKs
- [x] Appendix B (full system diagram) — full rewrite: SNS bucket fan-out, mTLS edge, Hetzner box with shared CH (default.* + prices.*), workstation backfill subgraph (BE backfill-runner, local CH, local Postgres, cloud-push tools), MV chain dotted edges, push-freshness alarm
- [x] Final grep on `database-schema-overview.md` shows no live-state RDS / sqlx / PostgreSQL 16 / OHLCV Rollup Lambda / ECS Fargate backfill references — only historical context, comparative-to-prior-design framing, and explicit "removed" callouts
- [x] `clickhouse-prod-schema.sql` left as-is (BE's `default.*` reference, not prices-api's schema)

## Out of scope

- Re-validating the empirical sizing numbers (already done in task 0046)
- Implementation work on 0011/0038/0039/0040 (still blocked on 0047)
- ADR 0007 itself — already accepted by PR #25

## Notes

- Reference the agreement record at
  `lore/1-tasks/archive/0045_.../notes/G-be-agreement-record.md`
  for cross-team commitments cited in §11.
- Reference task 0046's storage estimate at
  `lore/1-tasks/archive/0046_.../notes/G-empirical-storage-estimate.md`
  for the §10 numbers.
- Reference ADR 0007 §3 for the canonical Decision points.
- Reference task 0048's decoder spec for §5.2 wire-format details
  (kept as-is in this rewrite).

## Implementation Notes

- Single-PR rewrite spanning the full doc body (16 sections touched
  per the Revision History row). 1284 → 1450 lines net (+166).
  Larger sections of net additions: §3 schema rewrite, §6/§7/§8
  rewrites, §11 sharing table refresh.
- Empirical numbers in §10 sourced from
  [task 0046's G-note](../archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md):
  ~0.45 GB/yr flat-growth footprint, 14.8× compression, ~74
  bytes/ledger. Cost-share opening proposal ~1-2% pro-rata
  ($1-2/env/mo) — basis lives in
  [task 0045's agreement record](../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md)
  Cluster D.
- Sub-task harness tracker recorded 8 sub-steps (`#7`–`#14`)
  matching the section groupings.
- §5.5 L1/L2/L3 VWAP layering callout (task 0048) was preserved
  byte-for-byte across the rewrite.

## Issues Encountered

- **Line-number drift across edits.** The rewrite touches sections
  whose line numbers shift after each preceding edit. Resolved by
  re-running `grep -n "## N"` between major edits and using
  Edit's unique-old-string matching rather than line-based
  insertion.
- **Cross-project path convention (`../../../../soroban-block-explorer/`).**
  These resolve via convention rather than filesystem walk; the
  established 4-dot pattern was preserved verbatim from earlier
  archived tasks even though strict path math would suggest 5
  dots from the README inside a task directory. Not a regression —
  matches all other lore artefacts.

## Design Decisions

### From Plan

1. **Schema engine: `ReplacingMergeTree(version)` for OHLCV,
   `ReplacingMergeTree(updated_at)` for `current_prices` /
   `assets` / `backfill_progress`, `MergeTree` for `oracle_prices`.**
   Direct read of ADR 0007 §3.3 + ADR 0004; no judgment call.

2. **Per-granularity tables (`price_ohlcv_1m`/`_15m`/…/`_1M`)
   with an MV chain.** ADR 0007 §3.4 calls this explicitly.

3. **Preserve the §5.5 L1/L2/L3 layering callout unchanged.**
   Task 0048 added it; the rewrite kept it byte-for-byte (only
   verifying the references still make sense in the new context).

### Emerged

4. **Frame removed components as an explicit sub-list under §2.1
   rather than just deleting the rows.** A reader pattern-matching
   "where's RDS / NAT Gateway?" should find the answer in one
   scroll, not by realising they're absent. Added a "Components no
   longer in the Prices API budget" mini-list at the end of §2.1
   with one bullet per removed component.

5. **Title and intro preamble left as-is.** The document title
   ("Post-2nd-Review") and the intro paragraph mentioning
   `sqlx` describe the document's *starting point*, not its
   current state. The Revision History is the authoritative
   record of subsequent evolution. Rewriting the preamble would
   be a stylistic change beyond ADR 0007's scope.

6. **Cost section absorbed both the steady-state and the
   backfill-period sub-tables.** §10's backfill sub-table now
   shows ~$0 AWS-billed cost (all writes hit Hetzner, not RDS),
   so the prior "~$30 RDS upgrade during push windows" line
   collapses. Kept the sub-table structure for traceability
   rather than deleting it outright.

7. **§6 "ClickHouse Sizing (shared Hetzner cluster, BE-owned)"
   sub-section is intentionally short.** Hardware sizing is BE's
   concern, not prices-api's. Documented the sidecar-CH fallback
   from ADR 0007 Alternative 3 as the only branch worth
   describing in this doc.

8. **§7 mTLS section added as a bullet group, not a new
   sub-section.** Keeps the security list at parity with the
   prior bullet-list shape; readability over structure.

9. **§11.3 "What Is Not Shared" pruned to remove the obsolete
   RDS row.** Replaced with the `prices.*` schema row and the
   onboarding portal row.

## Future Work

- **Schema reference doc.** `docs/database-schema/clickhouse-prod-schema.sql`
  is referenced from §3.2 but does not yet exist on the
  prices-api side. The full DDL (engines, MV chain, sample data
  fixtures) should land as a companion task before task 0011's
  CDK bootstrap rewrites against it. Backlog candidate.
- **OpenAPI spec.** §4 documents the API surface; the OpenAPI
  source artifact mentioned in Tranche 3 should be drafted from
  this section as the canonical contract. Not in scope here.
- **Cross-link from the wiki.** `lore/3-wiki/` may reference the
  prior RDS-shaped design in places; a sweep of the wiki for
  RDS / sqlx / Postgres references would catch any inconsistency.
  Quick task, not done here.
