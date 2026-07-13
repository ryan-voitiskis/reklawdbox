//! Cache-first provider lookup workflow shared by MCP lookup tools.

use std::future::Future;

use serde::Serialize;

use super::model::{CacheLookupOutcome, ProviderLookupOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LookupPolicy {
    pub(crate) force_refresh: bool,
    pub(crate) cache_read_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CachedLookup {
    pub(crate) response_json: Option<String>,
    pub(crate) created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LookupCacheWrite {
    pub(crate) match_quality: String,
    pub(crate) response_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LookupResult {
    pub(crate) payload: serde_json::Value,
    pub(crate) cache_hit: bool,
    pub(crate) cached_at: Option<String>,
}

impl LookupResult {
    pub(crate) fn into_output(self) -> serde_json::Value {
        lookup_output_with_cache_metadata(self.payload, self.cache_hit, self.cached_at.as_deref())
    }
}

#[derive(Debug)]
pub(crate) enum LookupWorkflowError<C, L> {
    Cache(C),
    Lookup(L),
    Serialize(serde_json::Error),
}

pub(crate) async fn lookup_with_cache<T, C, L, ReadCache, WriteCache, LookupFuture, Quality>(
    policy: LookupPolicy,
    read_cache: ReadCache,
    write_cache: WriteCache,
    lookup: LookupFuture,
    quality: Quality,
) -> Result<LookupResult, LookupWorkflowError<C, L>>
where
    T: Serialize,
    ReadCache: FnOnce() -> Result<Option<CachedLookup>, C>,
    WriteCache: FnOnce(LookupCacheWrite) -> Result<(), C>,
    LookupFuture: Future<Output = Result<Option<T>, L>>,
    Quality: FnOnce(&T) -> &'static str,
{
    let cache = if policy.cache_read_enabled && !policy.force_refresh {
        match read_cache().map_err(LookupWorkflowError::Cache)? {
            Some(cached) => CacheLookupOutcome::Hit(cached),
            None => CacheLookupOutcome::Miss,
        }
    } else {
        CacheLookupOutcome::Miss
    };

    if let CacheLookupOutcome::Hit(cached) = cache {
        let payload = cached
            .response_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or(serde_json::Value::Null);
        return Ok(LookupResult {
            payload,
            cache_hit: true,
            cached_at: Some(cached.created_at),
        });
    }

    let outcome = match lookup.await {
        Ok(Some(result)) => ProviderLookupOutcome::Match(result),
        Ok(None) => ProviderLookupOutcome::NoMatch,
        Err(error) => ProviderLookupOutcome::Error(error),
    };

    let (payload, cache_write) = match outcome {
        ProviderLookupOutcome::Match(result) => {
            let match_quality = quality(&result).to_string();
            let response_json =
                serde_json::to_string(&result).map_err(LookupWorkflowError::Serialize)?;
            let payload = serde_json::to_value(result).map_err(LookupWorkflowError::Serialize)?;
            (
                payload,
                LookupCacheWrite {
                    match_quality,
                    response_json: Some(response_json),
                },
            )
        }
        ProviderLookupOutcome::NoMatch => (
            serde_json::Value::Null,
            LookupCacheWrite {
                match_quality: "none".to_string(),
                response_json: None,
            },
        ),
        ProviderLookupOutcome::Error(error) => {
            return Err(LookupWorkflowError::Lookup(error));
        }
    };

    write_cache(cache_write).map_err(LookupWorkflowError::Cache)?;
    Ok(LookupResult {
        payload,
        cache_hit: false,
        cached_at: None,
    })
}

pub(crate) fn lookup_output_with_cache_metadata(
    payload: serde_json::Value,
    cache_hit: bool,
    cached_at: Option<&str>,
) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut map) => {
            map.insert("cache_hit".to_string(), serde_json::json!(cache_hit));
            if let Some(cached_at) = cached_at {
                map.insert("cached_at".to_string(), serde_json::json!(cached_at));
            }
            serde_json::Value::Object(map)
        }
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".to_string(), other);
            map.insert("cache_hit".to_string(), serde_json::json!(cache_hit));
            if let Some(cached_at) = cached_at {
                map.insert("cached_at".to_string(), serde_json::json!(cached_at));
            }
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn enrichment_lookup_preserves_negative_cache_policy() {
        let remote_calls = AtomicUsize::new(0);
        let cache_writes = AtomicUsize::new(0);

        let result = lookup_with_cache::<serde_json::Value, String, String, _, _, _, _>(
            LookupPolicy {
                force_refresh: false,
                cache_read_enabled: true,
            },
            || {
                Ok(Some(CachedLookup {
                    response_json: None,
                    created_at: "2026-07-14T00:00:00Z".to_string(),
                }))
            },
            |_| {
                cache_writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            async {
                remote_calls.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            },
            |_| "exact",
        )
        .await
        .unwrap();

        assert_eq!(remote_calls.load(Ordering::SeqCst), 0);
        assert_eq!(cache_writes.load(Ordering::SeqCst), 0);
        assert_eq!(result.payload, serde_json::Value::Null);
        assert!(result.cache_hit);
        assert_eq!(result.cached_at.as_deref(), Some("2026-07-14T00:00:00Z"));
    }
}
