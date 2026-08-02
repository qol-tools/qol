pub mod drain;
mod resolver;
mod runtime_cache;
mod scope;
mod store;

pub use resolver::{classify_os_bucket, resolve_plugin_config, PluginConfigResolution};
pub(crate) use runtime_cache::{ActionCacheStatus, ManifestIdentity};
pub use scope::{merge_slices, split_by_declarations, split_by_scope, ConfigSlices};

#[cfg(test)]
mod tests;

use crate::features::profile::scope_store::PluginConfigSlicePaths;
use crate::features::profile::ProfileScopeStore;
use crate::paths;
use crate::paths::is_safe_path_component;
use crate::plugins::manifest::PluginUid;
use crate::plugins::paths as plugin_paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
const RUNTIME_CONFIG_RETRY_TIMEOUT: Duration = Duration::from_millis(750);
const RUNTIME_CONFIG_RETRY_INTERVAL: Duration = Duration::from_millis(16);
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfigs {
    #[serde(flatten)]
    pub configs: HashMap<String, serde_json::Value>,
}

pub struct PluginConfigManager {
    scope_store: ProfileScopeStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeConfigCacheResult {
    Hit,
    Materialized,
}

struct MaterializedRuntimeConfig {
    value: Option<serde_json::Value>,
}

pub(crate) fn manifest_identity(
    manifest: &crate::plugins::manifest::PluginManifest,
) -> ManifestIdentity {
    ManifestIdentity::from(manifest)
}

pub(crate) fn begin_runtime_config_mutation(
    scope_store: &ProfileScopeStore,
) -> runtime_cache::MutationGuard {
    runtime_cache::begin_mutation(&scope_store.dir())
}

pub(crate) fn begin_runtime_config_global_mutation() -> runtime_cache::MutationGuard {
    runtime_cache::begin_global_mutation()
}

pub(crate) fn begin_runtime_config_mutation_for_active_profile(
) -> Result<runtime_cache::MutationGuard> {
    let scope_dir = ProfileScopeStore::from_active()?.dir();
    Ok(runtime_cache::begin_mutation(&scope_dir))
}

#[cfg(test)]
pub(crate) fn runtime_config_cache_hit(
    manager: &PluginConfigManager,
    plugin_id: &str,
    identity: &ManifestIdentity,
) -> bool {
    !matches!(
        runtime_config_cache_status(manager, plugin_id, identity),
        ActionCacheStatus::Miss
    )
}

pub(crate) fn runtime_config_cache_status(
    manager: &PluginConfigManager,
    plugin_id: &str,
    identity: &ManifestIdentity,
) -> ActionCacheStatus {
    runtime_cache::action_cache_status(&manager.scope_store, plugin_id, identity)
}

#[cfg(debug_assertions)]
pub(crate) fn runtime_config_generation() -> u64 {
    runtime_cache::generation()
}

#[cfg(debug_assertions)]
pub(crate) fn should_sample_runtime_cache_hit() -> bool {
    runtime_cache::should_sample_cache_hit()
}

static PROFILE_CONFIG_LOCK: OnceLock<RwLock<()>> = OnceLock::new();
static PROFILE_CONFIG_GENERATION: AtomicU64 = AtomicU64::new(0);
static PROFILE_CONFIG_INVALIDATIONS: OnceLock<Mutex<ProfileConfigInvalidationState>> =
    OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfileConfigInvalidation {
    All,
    Plugins(Vec<String>),
}

#[derive(Default)]
struct ProfileConfigInvalidationState {
    all_generation: u64,
    plugin_generations: HashMap<String, u64>,
}

#[cfg(test)]
thread_local! {
    static PROFILE_CONFIG_READS: Cell<u64> = const { Cell::new(0) };
}

fn profile_config_lock() -> &'static RwLock<()> {
    PROFILE_CONFIG_LOCK.get_or_init(|| RwLock::new(()))
}

fn profile_config_invalidations() -> &'static Mutex<ProfileConfigInvalidationState> {
    PROFILE_CONFIG_INVALIDATIONS
        .get_or_init(|| Mutex::new(ProfileConfigInvalidationState::default()))
}

pub(crate) struct ProfileConfigReadGuard {
    _guard: RwLockReadGuard<'static, ()>,
    generation: u64,
}

impl ProfileConfigReadGuard {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

pub(crate) struct ProfileConfigWriteGuard {
    _guard: RwLockWriteGuard<'static, ()>,
}

impl ProfileConfigWriteGuard {
    pub(crate) fn mark_changed(&self, scope: ProfileConfigInvalidation) -> u64 {
        mark_profile_config_changed(scope)
    }
}

pub(crate) fn profile_config_read_guard() -> ProfileConfigReadGuard {
    let guard = profile_config_lock()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(test)]
    PROFILE_CONFIG_READS.with(|reads| reads.set(reads.get() + 1));
    let generation = PROFILE_CONFIG_GENERATION.load(Ordering::Acquire);
    ProfileConfigReadGuard {
        _guard: guard,
        generation,
    }
}

pub(crate) fn profile_config_write_guard() -> ProfileConfigWriteGuard {
    profile_config_write_guard_for_scope(ProfileConfigInvalidation::All)
}

pub(crate) fn profile_config_write_guard_for_plugin(plugin_id: &str) -> ProfileConfigWriteGuard {
    profile_config_write_guard_for_scope(ProfileConfigInvalidation::Plugins(vec![
        plugin_id.to_string()
    ]))
}

pub(crate) fn profile_config_write_guard_for_plugins(
    plugin_ids: impl IntoIterator<Item = String>,
) -> ProfileConfigWriteGuard {
    profile_config_write_guard_for_scope(ProfileConfigInvalidation::Plugins(
        plugin_ids.into_iter().collect(),
    ))
}

pub(crate) fn profile_config_write_guard_unmarked() -> ProfileConfigWriteGuard {
    let guard = profile_config_lock()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ProfileConfigWriteGuard { _guard: guard }
}

fn profile_config_write_guard_for_scope(
    scope: ProfileConfigInvalidation,
) -> ProfileConfigWriteGuard {
    let guard = profile_config_write_guard_unmarked();
    guard.mark_changed(scope);
    guard
}

fn mark_profile_config_changed(scope: ProfileConfigInvalidation) -> u64 {
    let mut invalidations = profile_config_invalidations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let generation = PROFILE_CONFIG_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    match scope {
        ProfileConfigInvalidation::All => {
            invalidations.all_generation = generation;
            invalidations.plugin_generations.clear();
        }
        ProfileConfigInvalidation::Plugins(plugin_ids) => {
            for plugin_id in plugin_ids {
                invalidations
                    .plugin_generations
                    .insert(plugin_id, generation);
            }
        }
    }
    generation
}

pub(crate) fn profile_config_invalidation_since(
    last_generation: u64,
) -> Option<(u64, ProfileConfigInvalidation)> {
    let invalidations = profile_config_invalidations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let current_generation = current_profile_config_generation();
    if current_generation == last_generation {
        return None;
    }
    if invalidations.all_generation > last_generation {
        return Some((current_generation, ProfileConfigInvalidation::All));
    }
    let mut plugin_ids = invalidations
        .plugin_generations
        .iter()
        .filter(|(_, generation)| **generation > last_generation)
        .map(|(plugin_id, _)| plugin_id.clone())
        .collect::<Vec<_>>();
    plugin_ids.sort_unstable();
    Some((
        current_generation,
        ProfileConfigInvalidation::Plugins(plugin_ids),
    ))
}

pub(crate) fn profile_config_plugin_generation(plugin_id: &str) -> u64 {
    profile_config_invalidations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .plugin_generations
        .get(plugin_id)
        .copied()
        .unwrap_or_default()
}

pub(crate) fn current_profile_config_generation() -> u64 {
    PROFILE_CONFIG_GENERATION.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn reset_profile_config_read_count() {
    PROFILE_CONFIG_READS.with(|reads| reads.set(0));
}

#[cfg(test)]
pub(crate) fn profile_config_read_count() -> u64 {
    PROFILE_CONFIG_READS.with(Cell::get)
}

pub(crate) fn redacted_profile_fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
        .chars()
        .take(12)
        .collect()
}

struct RuntimeConfigTrace<'a> {
    scope: &'a str,
    generation: u64,
    fingerprint: &'a str,
    lock_load_outcome: &'a str,
    fallback_reason: Option<&'a str>,
    daemon_spawn_generation: Option<u64>,
}

struct RuntimeCacheMaterialization {
    outcome: &'static str,
    cache_load_outcome: &'static str,
}

struct RuntimeConfigLockSnapshot {
    lock_entries: HashMap<String, crate::features::profile::core::PluginLockEntry>,
    fingerprint: String,
    lock_load_outcome: &'static str,
    fallback_reason: Option<&'static str>,
}

pub(crate) struct RuntimeConfigContext {
    manager: PluginConfigManager,
    lock_entries: HashMap<String, crate::features::profile::core::PluginLockEntry>,
    generation: u64,
    fingerprint: String,
    lock_load_outcome: &'static str,
    fallback_reason: Option<&'static str>,
}

impl RuntimeConfigContext {
    pub(crate) fn new() -> Result<Self> {
        let profile_guard = profile_config_read_guard();
        let manager = PluginConfigManager::new()?;
        let snapshot = load_runtime_config_lock_snapshot(&manager);
        let generation = profile_guard.generation();
        drop(profile_guard);
        Ok(Self {
            manager,
            lock_entries: snapshot.lock_entries,
            generation,
            fingerprint: snapshot.fingerprint,
            lock_load_outcome: snapshot.lock_load_outcome,
            fallback_reason: snapshot.fallback_reason,
        })
    }

    #[cfg(test)]
    pub(crate) fn materialize_runtime_config_for_manifest(
        &mut self,
        plugin_id: &str,
        manifest: &crate::plugins::manifest::PluginManifest,
    ) -> Result<Option<serde_json::Value>> {
        let profile_guard = profile_config_read_guard();
        let result =
            self.materialize_runtime_config_with_guard(&profile_guard, plugin_id, manifest, None);
        drop(profile_guard);
        result
    }

    pub(crate) fn prepare_runtime_config_for_spawn(
        &mut self,
        plugin_id: &str,
        manifest: &crate::plugins::manifest::PluginManifest,
    ) -> Result<ProfileConfigReadGuard> {
        let profile_guard = profile_config_read_guard();
        self.materialize_runtime_config_with_guard(
            &profile_guard,
            plugin_id,
            manifest,
            Some(profile_guard.generation()),
        )?;
        Ok(profile_guard)
    }

    fn materialize_runtime_config_with_guard(
        &mut self,
        profile_guard: &ProfileConfigReadGuard,
        plugin_id: &str,
        manifest: &crate::plugins::manifest::PluginManifest,
        daemon_spawn_generation: Option<u64>,
    ) -> Result<Option<serde_json::Value>> {
        if profile_guard.generation() != self.generation {
            let snapshot = load_runtime_config_lock_snapshot(&self.manager);
            self.lock_entries = snapshot.lock_entries;
            self.generation = profile_guard.generation();
            self.fingerprint = snapshot.fingerprint;
            self.lock_load_outcome = snapshot.lock_load_outcome;
            self.fallback_reason = snapshot.fallback_reason;
        }
        let lock_entry = self.lock_entries.get(plugin_id);
        let trace = RuntimeConfigTrace {
            scope: plugin_id,
            generation: self.generation,
            fingerprint: &self.fingerprint,
            lock_load_outcome: self.lock_load_outcome,
            fallback_reason: self.fallback_reason,
            daemon_spawn_generation,
        };
        self.manager.materialize_runtime_config_with_trace(
            plugin_id,
            lock_entry,
            Some(manifest),
            Some(trace),
        )
    }
}

fn load_runtime_config_lock_snapshot(manager: &PluginConfigManager) -> RuntimeConfigLockSnapshot {
    match crate::features::profile::core::load_plugins_lock() {
        Ok(lock) => {
            let fingerprint = fingerprint_lock(&lock);
            let mut lock_entries = HashMap::with_capacity(lock.plugins.len());
            for entry in lock.plugins {
                lock_entries.entry(entry.id.clone()).or_insert(entry);
            }
            RuntimeConfigLockSnapshot {
                lock_entries,
                fingerprint,
                lock_load_outcome: "loaded",
                fallback_reason: None,
            }
        }
        Err(error) => {
            log::warn!(
                "Profile lock unavailable during daemon autostart; using manifest identity fallback: {error:#}"
            );
            RuntimeConfigLockSnapshot {
                lock_entries: HashMap::new(),
                fingerprint: fingerprint_path(&manager.store().plugins_lock_path()),
                lock_load_outcome: "fallback",
                fallback_reason: Some("lock_read_failed"),
            }
        }
    }
}

fn fingerprint_lock(lock: &crate::features::profile::core::PluginsLock) -> String {
    serde_json::to_vec(lock)
        .map(|bytes| redacted_profile_fingerprint(&bytes))
        .unwrap_or_else(|_| "unavailable".to_string())
}

fn fingerprint_path(path: &Path) -> String {
    std::fs::read(path)
        .map(|bytes| redacted_profile_fingerprint(&bytes))
        .unwrap_or_else(|_| "unavailable".to_string())
}

impl PluginConfigManager {
    pub fn new() -> Result<Self> {
        Ok(Self::with_store(runtime_cache::active_scope_store()?))
    }

    pub fn with_store(scope_store: ProfileScopeStore) -> Self {
        Self { scope_store }
    }

    pub fn store(&self) -> &ProfileScopeStore {
        &self.scope_store
    }

    fn plugin_config_path(plugin_id: &str) -> Result<PathBuf> {
        if !is_safe_path_component(plugin_id) {
            anyhow::bail!("Invalid plugin ID: {}", plugin_id);
        }
        Ok(paths::plugins_dir()?.join(plugin_id).join("config.json"))
    }

    pub fn load_configs(&self) -> Result<PluginConfigs> {
        let configs = store::load_configs(&self.scope_store.core_plugin_configs_dir())?;
        Ok(PluginConfigs { configs })
    }

    pub fn save_configs(&self, configs: &PluginConfigs) -> Result<()> {
        let _profile_guard = profile_config_write_guard();
        let _mutation = begin_runtime_config_mutation(&self.scope_store);
        store::save_configs(
            &self.scope_store.core_plugin_configs_dir(),
            &configs.configs,
        )
    }

    pub fn get_config(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let lock = load_lock_entry_for(plugin_id);
        let manifest = try_load_plugin_manifest(plugin_id);
        self.get_config_with(plugin_id, lock.as_ref(), manifest.as_ref())
    }

    pub fn get_config_with(
        &self,
        plugin_id: &str,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
    ) -> Result<Option<serde_json::Value>> {
        self.materialize_runtime_config_with(plugin_id, lock_entry, manifest)
    }

    pub fn materialize_runtime_config(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let lock = load_lock_entry_for(plugin_id);
        let manifest = try_load_plugin_manifest(plugin_id);
        self.materialize_runtime_config_with(plugin_id, lock.as_ref(), manifest.as_ref())
    }

    pub fn materialize_runtime_config_for_manifest(
        &self,
        plugin_id: &str,
        manifest: &crate::plugins::manifest::PluginManifest,
    ) -> Result<Option<serde_json::Value>> {
        if let Some(value) = runtime_cache::lookup(&self.scope_store, plugin_id, manifest) {
            return Ok(value);
        }
        let lock = load_lock_entry_for(plugin_id);
        self.materialize_runtime_config_with(plugin_id, lock.as_ref(), Some(manifest))
    }

    pub(crate) fn ensure_runtime_config_for_manifest(
        &self,
        plugin_id: &str,
        manifest: &crate::plugins::manifest::PluginManifest,
    ) -> Result<RuntimeConfigCacheResult> {
        let started = Instant::now();
        let generation = runtime_cache::generation();
        if runtime_cache::is_fresh(&self.scope_store, plugin_id, manifest) {
            trace_runtime_cache_check(
                plugin_id,
                RuntimeConfigCacheResult::Hit,
                generation,
                started,
            );
            return Ok(RuntimeConfigCacheResult::Hit);
        }
        let lock = load_lock_entry_for(plugin_id);
        self.materialize_runtime_config_with(plugin_id, lock.as_ref(), Some(manifest))
            .map(|_| {
                trace_runtime_cache_check(
                    plugin_id,
                    RuntimeConfigCacheResult::Materialized,
                    generation,
                    started,
                );
                RuntimeConfigCacheResult::Materialized
            })
    }

    pub fn materialize_installed_runtime_configs(&self) -> Result<usize> {
        let _profile_guard = profile_config_read_guard();
        let lock = crate::features::profile::core::load_plugins_lock()?;
        let generation = current_profile_config_generation();
        let fingerprint = fingerprint_lock(&lock);
        let mut count = 0;
        for entry in lock.plugins {
            let Some(manifest) = try_load_plugin_manifest(&entry.id) else {
                continue;
            };
            let trace = RuntimeConfigTrace {
                scope: &entry.id,
                generation,
                fingerprint: &fingerprint,
                lock_load_outcome: "loaded",
                fallback_reason: None,
                daemon_spawn_generation: None,
            };
            self.materialize_runtime_config_with_trace(
                &entry.id,
                Some(&entry),
                Some(&manifest),
                Some(trace),
            )?;
            count += 1;
        }
        Ok(count)
    }

    pub fn set_config(&self, plugin_id: &str, config: serde_json::Value) -> Result<()> {
        let lock = load_lock_entry_for(plugin_id);
        let manifest = try_load_plugin_manifest(plugin_id);
        self.set_config_with(plugin_id, config, lock.as_ref(), manifest.as_ref())
    }

    pub fn set_config_with(
        &self,
        plugin_id: &str,
        config: serde_json::Value,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
    ) -> Result<()> {
        let uid = uid_from_lock_manifest_or_id(lock_entry, manifest, plugin_id);
        let runtime_path = Self::plugin_config_path(plugin_id)?;
        let _profile_guard = profile_config_write_guard_for_plugin(plugin_id);
        let _mutation = begin_runtime_config_mutation(&self.scope_store);
        store::write_plugin_config(&runtime_path, &config)?;
        save_plugin_config_split_unlocked(&self.scope_store, &uid, &config, lock_entry, manifest)
    }
}

impl PluginConfigManager {
    fn materialize_runtime_config_with(
        &self,
        plugin_id: &str,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
    ) -> Result<Option<serde_json::Value>> {
        if let Some(manifest) = manifest {
            if let Some(value) = runtime_cache::lookup(&self.scope_store, plugin_id, manifest) {
                return Ok(value);
            }
            return self.materialize_cached_runtime_config(plugin_id, manifest);
        }
        Ok(self
            .materialize_runtime_config_once(plugin_id, lock_entry, None)?
            .value)
    }

    fn materialize_cached_runtime_config(
        &self,
        plugin_id: &str,
        manifest: &crate::plugins::manifest::PluginManifest,
    ) -> Result<Option<serde_json::Value>> {
        let scope_dir = self.scope_store.dir();
        if !runtime_cache::cache_available() && !runtime_cache::mutation_active(&scope_dir) {
            let lock = load_lock_entry_for(plugin_id);
            return Ok(self
                .materialize_runtime_config_once(plugin_id, lock.as_ref(), Some(manifest))?
                .value);
        }
        let deadline = Instant::now() + RUNTIME_CONFIG_RETRY_TIMEOUT;
        let mut observed_epoch = runtime_cache::mutation_epoch();
        let mut wait_duration = Duration::ZERO;
        let mut wait_scope = "none";
        loop {
            if let Some(value) = runtime_cache::lookup(&self.scope_store, plugin_id, manifest) {
                trace_runtime_cache_wait(plugin_id, wait_scope, wait_duration, false);
                return Ok(value);
            }
            let Some(version) = runtime_cache::stable_cache_version(&scope_dir) else {
                let wait_deadline =
                    std::cmp::min(deadline, Instant::now() + RUNTIME_CONFIG_RETRY_INTERVAL);
                if !wait_for_runtime_cache_change(
                    &scope_dir,
                    &mut observed_epoch,
                    wait_deadline,
                    &mut wait_duration,
                    &mut wait_scope,
                ) && Instant::now() >= deadline
                {
                    trace_runtime_cache_wait(plugin_id, wait_scope, wait_duration, true);
                    anyhow::bail!(
                        "profile mutation did not settle while materializing runtime config for {plugin_id}"
                    );
                }
                continue;
            };
            let lock = load_lock_entry_for(plugin_id);
            let uid = uid_from_lock_manifest_or_id(lock.as_ref(), Some(manifest), plugin_id);
            let paths =
                self.scope_store
                    .plugin_config_slice_paths(&uid, lock.as_ref(), Some(manifest))?;
            let source_before = runtime_cache::source_revision(&self.scope_store, &paths)?;
            let materialized =
                self.materialize_runtime_config_once(plugin_id, lock.as_ref(), Some(manifest))?;
            let source_after = runtime_cache::source_revision(&self.scope_store, &paths)?;
            #[cfg(test)]
            runtime_cache::before_publish();
            if runtime_cache::publish_if_unchanged(
                &self.scope_store,
                plugin_id,
                manifest,
                source_before,
                source_after,
                version,
                materialized.value.clone(),
            ) {
                trace_runtime_cache_wait(plugin_id, wait_scope, wait_duration, false);
                return Ok(materialized.value);
            }
            if !runtime_cache::cache_available() {
                trace_runtime_cache_wait(plugin_id, wait_scope, wait_duration, false);
                return Ok(materialized.value);
            }
            let wait_deadline =
                std::cmp::min(deadline, Instant::now() + RUNTIME_CONFIG_RETRY_INTERVAL);
            if !wait_for_runtime_cache_change(
                &scope_dir,
                &mut observed_epoch,
                wait_deadline,
                &mut wait_duration,
                &mut wait_scope,
            ) && Instant::now() >= deadline
            {
                trace_runtime_cache_wait(plugin_id, wait_scope, wait_duration, true);
                anyhow::bail!("profile changed while materializing runtime config for {plugin_id}");
            }
        }
    }

    fn materialize_runtime_config_with_trace(
        &self,
        plugin_id: &str,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
        trace: Option<RuntimeConfigTrace<'_>>,
    ) -> Result<Option<serde_json::Value>> {
        if let Some(manifest) = manifest {
            if let Some(value) = runtime_cache::lookup(&self.scope_store, plugin_id, manifest) {
                return Ok(value);
            }
        }
        Ok(self
            .materialize_runtime_config_once_with_trace(plugin_id, lock_entry, manifest, trace)?
            .value)
    }

    fn materialize_runtime_config_once(
        &self,
        plugin_id: &str,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
    ) -> Result<MaterializedRuntimeConfig> {
        self.materialize_runtime_config_once_with_trace(plugin_id, lock_entry, manifest, None)
    }

    fn materialize_runtime_config_once_with_trace(
        &self,
        plugin_id: &str,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
        trace: Option<RuntimeConfigTrace<'_>>,
    ) -> Result<MaterializedRuntimeConfig> {
        #[cfg(test)]
        runtime_cache::record_materialization();
        let uid = uid_from_lock_manifest_or_id(lock_entry, manifest, plugin_id);
        let merged = load_plugin_config_merged(&self.scope_store, &uid, lock_entry, manifest)?;
        let runtime_path = Self::plugin_config_path(plugin_id)?;
        let materialization = materialize_runtime_cache(&runtime_path, &merged)?;
        trace_runtime_materialization(
            plugin_id,
            materialization.outcome,
            materialization.cache_load_outcome,
            trace,
        );
        Ok(MaterializedRuntimeConfig {
            value: (!merged.as_object().is_some_and(|m| m.is_empty())).then_some(merged),
        })
    }
}

fn wait_for_runtime_cache_change(
    scope_dir: &Path,
    observed_epoch: &mut u64,
    deadline: Instant,
    wait_duration: &mut Duration,
    wait_scope: &mut &'static str,
) -> bool {
    let mutation_scope = runtime_cache::mutation_scope(scope_dir);
    let started = Instant::now();
    let changed = runtime_cache::wait_for_mutation_change(observed_epoch, deadline);
    *wait_duration += started.elapsed();
    if mutation_scope != "none" {
        *wait_scope = mutation_scope;
    }
    changed
}

fn materialize_runtime_cache(
    path: &Path,
    merged: &serde_json::Value,
) -> Result<RuntimeCacheMaterialization> {
    if merged.as_object().is_some_and(|m| m.is_empty()) {
        let cache_load_outcome = if path.exists() {
            "empty_existing"
        } else {
            "empty_missing"
        };
        return Ok(RuntimeCacheMaterialization {
            outcome: remove_runtime_cache(path)?,
            cache_load_outcome,
        });
    }
    let cache_load_outcome = match store::load_plugin_config(path) {
        Ok(value) if value == *merged => "hit",
        Ok(_) => "mismatch",
        Err(_) if path.exists() => "error",
        Err(_) => "missing",
    };
    if cache_load_outcome == "hit" {
        return Ok(RuntimeCacheMaterialization {
            outcome: "unchanged",
            cache_load_outcome,
        });
    }
    store::write_plugin_config(path, merged)?;
    Ok(RuntimeCacheMaterialization {
        outcome: "written",
        cache_load_outcome,
    })
}

fn remove_runtime_cache(path: &Path) -> Result<&'static str> {
    if !path.exists() {
        return Ok("empty_missing");
    }
    std::fs::remove_file(path)?;
    Ok("removed")
}

fn trace_runtime_materialization(
    plugin_id: &str,
    outcome: &str,
    cache_load_outcome: &str,
    trace: Option<RuntimeConfigTrace<'_>>,
) {
    let (
        scope,
        generation,
        fingerprint,
        lock_load_outcome,
        fallback_reason,
        daemon_spawn_generation,
    ) = trace.map_or(
        (
            "none",
            0,
            "unknown",
            "direct",
            "none",
            "unknown".to_string(),
        ),
        |trace| {
            (
                trace.scope,
                trace.generation,
                trace.fingerprint,
                trace.lock_load_outcome,
                trace.fallback_reason.unwrap_or("none"),
                trace
                    .daemon_spawn_generation
                    .map_or_else(|| "none".to_string(), |generation| generation.to_string()),
            )
        },
    );
    #[cfg(debug_assertions)]
    {
        qol_runtime::probe!(
            "PROFILE_CONFIG_MATERIALIZE",
            "plugin={:?} scope={scope:?} cache_load_outcome={cache_load_outcome} materialize_outcome={outcome} profile_generation={generation} fingerprint={fingerprint:?} lock_load_outcome={lock_load_outcome} fallback_reason={fallback_reason} consumed_generation={daemon_spawn_generation} acknowledged_generation=none",
            plugin_id,
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (
        plugin_id,
        scope,
        outcome,
        cache_load_outcome,
        generation,
        fingerprint,
        lock_load_outcome,
        fallback_reason,
        daemon_spawn_generation,
    );
}

fn trace_runtime_cache_wait(
    plugin_id: &str,
    mutation_scope: &str,
    wait_duration: Duration,
    timed_out: bool,
) {
    if wait_duration.is_zero() && !timed_out {
        return;
    }
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "PROFILE_CONFIG_MATERIALIZE",
        "plugin={:?} cache=miss last_known_good=false mutation_scope={} wait_us={} timeout={}",
        plugin_id,
        mutation_scope,
        wait_duration.as_micros(),
        timed_out
    );
    #[cfg(not(debug_assertions))]
    let _ = (plugin_id, mutation_scope, wait_duration, timed_out);
}

#[cfg(debug_assertions)]
fn trace_runtime_cache_check(
    plugin_id: &str,
    result: RuntimeConfigCacheResult,
    generation: u64,
    started: Instant,
) {
    if result == RuntimeConfigCacheResult::Hit && !runtime_cache::should_sample_cache_hit() {
        return;
    }
    let cache = match result {
        RuntimeConfigCacheResult::Hit => "hit",
        RuntimeConfigCacheResult::Materialized => "miss",
    };
    qol_runtime::probe!(
        "PROFILE_CONFIG_MATERIALIZE",
        "plugin={:?} cache={} last_known_good=false mutation_scope=none wait_us=0 timeout=false generation={} validation_us={}",
        plugin_id,
        cache,
        generation,
        started.elapsed().as_micros()
    );
}

#[cfg(not(debug_assertions))]
fn trace_runtime_cache_check(
    plugin_id: &str,
    result: RuntimeConfigCacheResult,
    generation: u64,
    started: Instant,
) {
    let _ = (plugin_id, result, generation, started);
}

fn load_lock_entry_for(plugin_id: &str) -> Option<crate::features::profile::core::PluginLockEntry> {
    #[cfg(test)]
    runtime_cache::record_profile_read();
    let lock = crate::features::profile::core::load_plugins_lock().ok()?;
    lock.plugins.into_iter().find(|entry| entry.id == plugin_id)
}

fn try_load_plugin_manifest(plugin_id: &str) -> Option<crate::plugins::manifest::PluginManifest> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id).ok()?;
    crate::plugins::manifest::PluginManifest::read_from_dir(&plugin_root).ok()
}

pub(crate) fn daemon_port(plugin_id: &str) -> Option<u16> {
    try_load_plugin_manifest(plugin_id)?.daemon?.port
}

pub fn load_plugin_config_merged(
    scope_store: &ProfileScopeStore,
    uid: &PluginUid,
    lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
    manifest: Option<&crate::plugins::manifest::PluginManifest>,
) -> Result<serde_json::Value> {
    #[cfg(test)]
    runtime_cache::record_profile_read();
    let paths = scope_store.plugin_config_slice_paths(uid, lock_entry, manifest)?;
    let slices = store::read_scoped_slices(&paths)?;
    Ok(merge_slices(&slices))
}

pub fn save_plugin_config_split(
    scope_store: &ProfileScopeStore,
    uid: &PluginUid,
    config: &serde_json::Value,
    lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
    manifest: Option<&crate::plugins::manifest::PluginManifest>,
) -> Result<()> {
    let plugin_id = lock_entry
        .map(|entry| entry.id.clone())
        .or_else(|| {
            manifest
                .and_then(|manifest| manifest.plugin.id.as_ref())
                .map(|plugin_id| plugin_id.as_str().to_string())
        })
        .unwrap_or_else(|| uid.as_str().to_string());
    let _profile_guard = profile_config_write_guard_for_plugin(&plugin_id);
    let _mutation = begin_runtime_config_mutation(scope_store);
    save_plugin_config_split_unlocked(scope_store, uid, config, lock_entry, manifest)
}

fn save_plugin_config_split_unlocked(
    scope_store: &ProfileScopeStore,
    uid: &PluginUid,
    config: &serde_json::Value,
    lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
    manifest: Option<&crate::plugins::manifest::PluginManifest>,
) -> Result<()> {
    let paths: PluginConfigSlicePaths =
        scope_store.plugin_config_slice_paths(uid, lock_entry, manifest)?;
    let default_decl = crate::plugins::manifest::ConfigDeclarations::default();
    let decl = manifest.map(|m| &m.config).unwrap_or(&default_decl);
    let slices = split_by_declarations(config, decl);
    store::write_scoped_slices(&paths, &slices)
}

fn uid_from_lock_manifest_or_id(
    lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
    manifest: Option<&crate::plugins::manifest::PluginManifest>,
    plugin_id: &str,
) -> PluginUid {
    let manifest_uid = manifest.and_then(|m| m.plugin.uid.clone());
    if let Some(entry) = lock_entry {
        if entry.uid.as_str() != entry.id.as_str() || manifest_uid.is_none() {
            return entry.uid.clone();
        }
    }
    manifest_uid.unwrap_or_else(|| PluginUid::new(plugin_id))
}

pub(crate) fn load_config_contract(
    plugin_id: &str,
) -> Result<Option<qol_config::contract::ConfigSpec>> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)?;
    load_config_contract_from_root(&plugin_root)
}

pub(crate) fn load_config_contract_from_root(
    plugin_root: &std::path::Path,
) -> Result<Option<qol_config::contract::ConfigSpec>> {
    let contract_path = plugin_paths::config_contract_path(plugin_root);
    if !is_regular_contract_file(&contract_path) {
        return Ok(None);
    }
    qol_config::contract::parse_spec(&contract_path)
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub(crate) fn load_runable_contract_from_root(
    plugin_root: &std::path::Path,
) -> Result<Option<qol_config::contract::RuntimeSpec>> {
    let runtime_path = plugin_paths::runable_contract_path(plugin_root);
    if !is_regular_contract_file(&runtime_path) {
        return Ok(None);
    }
    qol_config::contract::parse_runtime_spec(&runtime_path)
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub(crate) fn load_runable_contract(
    plugin_id: &str,
) -> Result<Option<qol_config::contract::RuntimeSpec>> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)?;
    load_runable_contract_from_root(&plugin_root)
}

pub(crate) fn load_combined_contracts_from_root(
    plugin_root: &std::path::Path,
) -> Result<
    Option<(
        qol_config::contract::ConfigSpec,
        Option<qol_config::contract::RuntimeSpec>,
    )>,
> {
    let Some(config) = load_config_contract_from_root(plugin_root)? else {
        return Ok(None);
    };
    let runtime = load_runable_contract_from_root(plugin_root)?;
    qol_config::contract::validate_contracts(&config, runtime.as_ref()).map_err(|errors| {
        anyhow::anyhow!(
            "contract validation failed:\n{}",
            format_validation_errors(errors)
        )
    })?;
    Ok(Some((config, runtime)))
}

pub(crate) fn load_combined_contracts(
    plugin_id: &str,
) -> Result<
    Option<(
        qol_config::contract::ConfigSpec,
        Option<qol_config::contract::RuntimeSpec>,
    )>,
> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)?;
    load_combined_contracts_from_root(&plugin_root)
}

/// Default traits served when a plugin manifest does not declare any.
/// Matches the frontend fallback in `ui/components/App.js`.
pub(crate) fn default_plugin_traits() -> serde_json::Value {
    serde_json::json!({ "confined": {} })
}

pub(crate) fn load_plugin_traits_from_root(plugin_root: &std::path::Path) -> serde_json::Value {
    read_manifest_traits(plugin_root).unwrap_or_else(default_plugin_traits)
}

pub(crate) fn load_plugin_traits(plugin_id: &str) -> serde_json::Value {
    let plugin_root = match plugin_paths::resolve_plugin_root(plugin_id) {
        Ok(root) => root,
        Err(_) => return default_plugin_traits(),
    };
    load_plugin_traits_from_root(&plugin_root)
}

fn read_manifest_traits(plugin_root: &std::path::Path) -> Option<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct TraitsOnly {
        traits: Option<serde_json::Value>,
    }
    let manifest_path = plugin_root.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let parsed: TraitsOnly = toml::from_str(&content).ok()?;
    let traits = parsed.traits?;
    if !traits.is_object() {
        return None;
    }
    Some(traits)
}

pub(crate) fn validate_config_value(
    spec: &qol_config::contract::ConfigSpec,
    config: &serde_json::Value,
) -> std::result::Result<(), Vec<qol_config::validation::ValidationError>> {
    let errors = match qol_config::normalized::resolve_config(spec, config) {
        Ok(_) => strict_validation_errors(spec, config),
        Err(errors) => errors,
    };
    if errors.is_empty() {
        return Ok(());
    }
    Err(errors)
}

pub(crate) fn format_validation_errors(
    errors: Vec<qol_config::validation::ValidationError>,
) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_regular_contract_file(path: &std::path::Path) -> bool {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    metadata.is_file()
}

fn strict_validation_errors(
    spec: &qol_config::contract::ConfigSpec,
    config: &serde_json::Value,
) -> Vec<qol_config::validation::ValidationError> {
    let mut errors = Vec::new();
    for (id, field) in &spec.fields {
        let config_key = field.config_key.as_deref().unwrap_or(id.as_str());
        let Some(raw) = config_override_value(config, config_key) else {
            continue;
        };
        let Some(value) = field_default_from_override(field.kind, raw) else {
            errors.push(qol_config::validation::ValidationError::new(
                format!("overrides.{id}"),
                format!("value does not match field type {}", field.kind.name()),
            ));
            continue;
        };
        errors.extend(qol_config::validation::validate_field_value(
            &format!("overrides.{id}"),
            field,
            &value,
        ));
    }
    errors
}

fn config_override_value<'a>(
    overrides: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = overrides;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn field_default_from_override(
    kind: qol_config::contract::FieldKind,
    raw: &serde_json::Value,
) -> Option<qol_config::contract::FieldDefault> {
    let value = serde_json::from_value::<qol_config::contract::FieldDefault>(raw.clone()).ok()?;
    field_default_matches_kind(kind, &value).then_some(value)
}

fn field_default_matches_kind(
    kind: qol_config::contract::FieldKind,
    value: &qol_config::contract::FieldDefault,
) -> bool {
    use qol_config::contract::{FieldDefault, FieldKind};
    match kind {
        FieldKind::Boolean => matches!(value, FieldDefault::Boolean(_)),
        FieldKind::String | FieldKind::Select | FieldKind::Color => {
            matches!(value, FieldDefault::String(_))
        }
        FieldKind::Number => matches!(value, FieldDefault::Number(_)),
        FieldKind::StringArray => matches!(value, FieldDefault::StringArray(_)),
        FieldKind::ObjectArray => matches!(value, FieldDefault::ObjectArray(_)),
        FieldKind::ObjectMap => matches!(value, FieldDefault::ObjectMap(_)),
        FieldKind::Action
        | FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad => false,
    }
}
