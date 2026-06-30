//! Shared HTTP primitives, ported/adapted from BE's `api/src/common`. Each
//! submodule is framework-level (no resource-specific logic). Phase 0 lands the
//! error envelope + cache-control tiers; cursor pagination, extractors, and
//! conditional GET (ETag/304) follow in the phases that first need them
//! (`/assets` listing, response caching).

pub mod cache_control;
pub mod errors;
