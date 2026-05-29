use axum::http::StatusCode;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;

use crate::plugins::PluginLoader;

use super::super::super::github::{
    current_timestamp, write_cache, GitHubClient, PluginCache, PluginMetadata,
    CACHE_FORMAT_VERSION, CACHE_TTL_SECS,
};
use super::super::helpers::{read_installed_plugin_dirs, read_plugin_version};
use super::super::types::{AppState, PluginInfo, PluginsResponse};

pub(super) fn list_plugins(
    state: &AppState,
    refresh: bool,
) -> Result<PluginsResponse, (StatusCode, String)> {
    log::info!("API /plugins called (refresh={})", refresh);
    let plugins_dir = plugins_dir()?;
    let installed_versions = installed_versions(&plugins_dir);
    let snapshot = read_cache_snapshot(state);
    let (metadata, cache_age, stale) = present_state(snapshot, refresh);
    maybe_spawn_revalidation(state, refresh, stale);
    let revalidating = state.plugins_revalidating.load(Ordering::SeqCst);
    log::info!("Got {} plugins (stale={stale})", metadata.len());
    Ok(PluginsResponse {
        plugins: metadata
            .into_iter()
            .filter(|m| m.supports_current_platform())
            .map(|m| plugin_info(m, &installed_versions))
            .collect(),
        cache_age_secs: cache_age,
        stale,
        revalidating,
    })
}

struct CacheSnapshot {
    plugins: Vec<PluginMetadata>,
    age_secs: u64,
    fresh: bool,
}

fn read_cache_snapshot(state: &AppState) -> Option<CacheSnapshot> {
    let guard = state.plugins_cache.read().ok()?;
    let cache = guard.as_ref()?;
    Some(snapshot_from_cache(cache, current_timestamp()))
}

fn snapshot_from_cache(cache: &PluginCache, now: u64) -> CacheSnapshot {
    let age = now.saturating_sub(cache.timestamp);
    let fresh = cache.format_version == CACHE_FORMAT_VERSION && age < CACHE_TTL_SECS;
    CacheSnapshot {
        plugins: cache.plugins.iter().cloned().map(Into::into).collect(),
        age_secs: age,
        fresh,
    }
}

fn is_stale(snapshot: Option<&CacheSnapshot>, refresh: bool) -> bool {
    if refresh {
        return true;
    }
    match snapshot {
        Some(s) => !s.fresh,
        None => true,
    }
}

fn present_state(
    snapshot: Option<CacheSnapshot>,
    refresh: bool,
) -> (Vec<PluginMetadata>, Option<u64>, bool) {
    let stale = is_stale(snapshot.as_ref(), refresh);
    match snapshot {
        Some(s) => (s.plugins, Some(s.age_secs), stale),
        None => (Vec::new(), None, stale),
    }
}

fn maybe_spawn_revalidation(state: &AppState, refresh: bool, stale: bool) {
    if !(stale || refresh) {
        return;
    }
    if state
        .plugins_revalidating
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let cache = state.plugins_cache.clone();
    let flag = state.plugins_revalidating.clone();
    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        let client = GitHubClient::new("qol-tools");
        let result = client.list_plugins().await;
        match result {
            Ok(plugins) if plugins.is_empty() => {
                log::warn!("Plugin revalidation returned empty list; keeping previous cache");
            }
            Ok(plugins) => {
                if let Err(error) = write_cache(&plugins) {
                    log::warn!("Failed to persist plugin cache: {}", error);
                }
                let new_cache = PluginCache {
                    format_version: CACHE_FORMAT_VERSION,
                    timestamp: current_timestamp(),
                    plugins: plugins.into_iter().map(Into::into).collect(),
                };
                if let Ok(mut guard) = cache.write() {
                    *guard = Some(new_cache);
                }
                events.send_plugins_changed();
            }
            Err(error) => {
                log::warn!("Plugin revalidation failed: {:#}", error);
            }
        }
        flag.store(false, Ordering::SeqCst);
    });
}

fn plugins_dir() -> Result<std::path::PathBuf, (StatusCode, String)> {
    PluginLoader::default_plugin_dir().map_err(|error| {
        log::error!("Failed to determine config directory: {}", error);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to determine plugin directory".to_string(),
        )
    })
}

fn installed_versions(plugins_dir: &Path) -> HashMap<String, String> {
    read_installed_plugin_dirs(plugins_dir)
        .into_iter()
        .filter_map(|(id, path)| read_plugin_version(&path).ok().map(|version| (id, version)))
        .collect()
}

fn plugin_info(
    metadata: PluginMetadata,
    installed_versions: &HashMap<String, String>,
) -> PluginInfo {
    let installed_version = installed_versions.get(&metadata.id).cloned();
    PluginInfo {
        id: metadata.id.clone(),
        name: metadata.name,
        description: metadata.description,
        version: metadata.version,
        installed: installed_version.is_some(),
        installed_version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::plugin_store::github::CachedPlugin;

    fn make_cache(timestamp: u64, version: u32) -> PluginCache {
        PluginCache {
            format_version: version,
            timestamp,
            plugins: vec![CachedPlugin {
                id: "plugin-a".to_string(),
                name: "A".to_string(),
                description: "x".to_string(),
                version: "1.0.0".to_string(),
                repo_url: "https://example.com".to_string(),
                platforms: None,
            }],
        }
    }

    fn fresh_snapshot() -> CacheSnapshot {
        CacheSnapshot {
            plugins: vec![],
            age_secs: 10,
            fresh: true,
        }
    }

    fn stale_snapshot() -> CacheSnapshot {
        CacheSnapshot {
            plugins: vec![],
            age_secs: CACHE_TTL_SECS + 1,
            fresh: false,
        }
    }

    #[test]
    fn is_stale_predicate_table() {
        let fresh = fresh_snapshot();
        let stale = stale_snapshot();
        let cases: &[(&str, Option<&CacheSnapshot>, bool, bool)] = &[
            ("no cache, no refresh", None, false, true),
            ("fresh cache, no refresh", Some(&fresh), false, false),
            ("fresh cache, force refresh", Some(&fresh), true, true),
            ("stale cache, no refresh", Some(&stale), false, true),
            ("no cache, force refresh", None, true, true),
            ("stale cache, force refresh", Some(&stale), true, true),
        ];
        for (name, snap, refresh, expected) in cases {
            assert_eq!(is_stale(*snap, *refresh), *expected, "case: {}", name);
        }
    }

    #[test]
    fn snapshot_marks_old_cache_stale() {
        let now = 10_000;
        let old = make_cache(now - CACHE_TTL_SECS - 1, CACHE_FORMAT_VERSION);
        let snap = snapshot_from_cache(&old, now);
        assert!(!snap.fresh);
        assert!(snap.age_secs > CACHE_TTL_SECS);
    }

    #[test]
    fn snapshot_marks_recent_cache_fresh() {
        let now = 10_000;
        let recent = make_cache(now - 5, CACHE_FORMAT_VERSION);
        let snap = snapshot_from_cache(&recent, now);
        assert!(snap.fresh);
        assert_eq!(snap.age_secs, 5);
    }

    #[test]
    fn snapshot_marks_format_mismatch_stale() {
        let now = 10_000;
        let bad = make_cache(now, CACHE_FORMAT_VERSION + 99);
        let snap = snapshot_from_cache(&bad, now);
        assert!(!snap.fresh);
    }

    #[test]
    fn snapshot_marks_clock_skew_as_zero_age_not_panic() {
        let now = 100;
        let future = make_cache(now + 50, CACHE_FORMAT_VERSION);
        let snap = snapshot_from_cache(&future, now);
        assert_eq!(snap.age_secs, 0);
        assert!(snap.fresh);
    }

    #[test]
    fn present_state_returns_empty_with_no_cache() {
        let (metadata, age, stale) = present_state(None, false);
        assert!(metadata.is_empty());
        assert!(age.is_none());
        assert!(stale);
    }

    #[test]
    fn present_state_returns_cached_plugins_with_age() {
        let snap = CacheSnapshot {
            plugins: vec![PluginMetadata {
                id: "plugin-x".to_string(),
                name: "X".to_string(),
                description: "d".to_string(),
                version: "1.0.0".to_string(),
                repo_url: "https://example.com".to_string(),
                platforms: None,
            }],
            age_secs: 42,
            fresh: true,
        };
        let (metadata, age, stale) = present_state(Some(snap), false);
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].id, "plugin-x");
        assert_eq!(age, Some(42));
        assert!(!stale);
    }

    #[tokio::test]
    async fn single_flight_guard_coalesces_concurrent_triggers() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let flag = Arc::new(AtomicBool::new(false));
        let mut won = 0;
        for _ in 0..10 {
            if flag
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                won += 1;
            }
        }
        assert_eq!(won, 1, "only one caller should win the single-flight race");
        flag.store(false, Ordering::SeqCst);
        assert!(flag
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok());
    }
}
