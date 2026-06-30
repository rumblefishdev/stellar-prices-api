//! Shared in-process cache primitive (moka). Ported from BE's `api/src/cache.rs`.
//!
//! `ttl_cache` is the one bounded TTL builder the resource handlers compose
//! (Phase 2+). Values are wrapped in `Arc<V>` so a cache hit is a refcount bump
//! rather than a clone. This in-process cache is the first of the two caching
//! layers in front of ClickHouse (the API Gateway stage cache is the second);
//! together they keep the `/price` load test off the database (ADR 0008).

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;

/// Build a bounded, time-to-live cache mapping `K → Arc<V>`.
///
/// `max_capacity` is an explicit entry bound (moka evicts under pressure); every
/// caller picks one suited to its key space.
pub fn ttl_cache<K, V>(ttl: Duration, max_capacity: u64) -> Cache<K, Arc<V>>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    Cache::builder()
        .time_to_live(ttl)
        .max_capacity(max_capacity)
        .build()
}
