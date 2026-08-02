use crate::features::profile::scope_store::{PluginConfigSlicePaths, ProfileScopeStore};
use crate::plugins::manifest::PluginManifest;
use anyhow::{bail, Result};
#[cfg(not(test))]
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(test))]
use std::sync::Arc;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static CACHE_AVAILABLE: AtomicBool = AtomicBool::new(cfg!(test));
static CACHE: OnceLock<Mutex<HashMap<CacheKey, CacheEntry>>> = OnceLock::new();
static ACTION_READINESS: OnceLock<Mutex<HashMap<CacheKey, ManifestIdentity>>> = OnceLock::new();
static MUTATION_STATE: OnceLock<Mutex<MutationState>> = OnceLock::new();
static ACTIVE_SCOPE_STORE: OnceLock<Mutex<Option<ProfileScopeStore>>> = OnceLock::new();
static MUTATION_NOTIFIER: OnceLock<MutationNotifier> = OnceLock::new();
#[cfg(not(test))]
static WATCHER: OnceLock<Mutex<Option<WatcherState>>> = OnceLock::new();
#[cfg(not(test))]
static WATCHER_REARM: AtomicBool = AtomicBool::new(true);

#[cfg(test)]
type BeforePublishHook = std::sync::Arc<dyn Fn() + Send + Sync>;
#[cfg(test)]
static BEFORE_PUBLISH: OnceLock<Mutex<Option<BeforePublishHook>>> = OnceLock::new();
#[cfg(test)]
static BEFORE_WAIT: OnceLock<Mutex<Option<BeforePublishHook>>> = OnceLock::new();
#[cfg(test)]
type ActiveMarkerSnapshot = (PathBuf, Option<Vec<u8>>);
#[cfg(test)]
static ACTIVE_MARKER: OnceLock<Mutex<Option<ActiveMarkerSnapshot>>> = OnceLock::new();
#[cfg(debug_assertions)]
static CACHE_HIT_SAMPLES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[derive(Default)]
struct TestStats {
    source_revision_reads: usize,
    profile_reads: usize,
    materializations: usize,
    value_clones: usize,
}

#[cfg(test)]
thread_local! {
    static TEST_STATS: RefCell<TestStats> = const { RefCell::new(TestStats {
        source_revision_reads: 0,
        profile_reads: 0,
        materializations: 0,
        value_clones: 0,
    }) };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    profile_dir: PathBuf,
    os_bucket: String,
    plugin_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheEntry {
    version: CacheVersion,
    manifest_identity: ManifestIdentity,
    value: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheVersion {
    generation: u64,
    scope_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionCacheStatus {
    Miss,
    Fresh,
    LastKnownGood { mutation_scope: &'static str },
}

#[derive(Debug, Default)]
struct ScopeState {
    generation: u64,
    writers: usize,
}

#[derive(Debug, Default)]
struct MutationState {
    global_writers: usize,
    scopes: HashMap<PathBuf, ScopeState>,
}

struct MutationNotifier {
    epoch: Mutex<u64>,
    changed: Condvar,
}

#[cfg(not(test))]
#[derive(Debug)]
struct WatcherState {
    root: PathBuf,
    _watcher: RecommendedWatcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ManifestIdentity {
    fingerprint: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceRevision {
    lock: Option<[u8; 32]>,
    core: Option<[u8; 32]>,
    os: Option<[u8; 32]>,
    device: Option<[u8; 32]>,
}

#[derive(Debug)]
pub(crate) struct MutationGuard {
    scope: MutationScope,
}

#[derive(Debug)]
enum MutationScope {
    Profile(PathBuf),
    Global,
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        match &self.scope {
            MutationScope::Profile(scope_dir) => finish_profile_mutation(scope_dir),
            MutationScope::Global => finish_global_mutation(),
        }
    }
}

pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub(crate) fn cache_available() -> bool {
    CACHE_AVAILABLE.load(Ordering::Acquire)
}

#[cfg(not(test))]
fn set_cache_available(available: bool) {
    CACHE_AVAILABLE.store(available, Ordering::Release);
    if !available {
        clear_all_entries();
    }
}

pub(crate) fn begin_mutation(scope_dir: &Path) -> MutationGuard {
    {
        let mut mutation = mutation_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let scope = mutation.scopes.entry(scope_dir.to_path_buf()).or_default();
        scope.writers += 1;
        if scope.writers == 1 {
            scope.generation = scope.generation.wrapping_add(1);
        }
    }
    notify_mutation_change();
    MutationGuard {
        scope: MutationScope::Profile(scope_dir.to_path_buf()),
    }
}

pub(crate) fn begin_global_mutation() -> MutationGuard {
    {
        let mut mutation = mutation_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        mutation.global_writers += 1;
        if mutation.global_writers == 1 {
            GENERATION.fetch_add(1, Ordering::AcqRel);
        }
    }
    notify_mutation_change();
    MutationGuard {
        scope: MutationScope::Global,
    }
}

fn finish_profile_mutation(scope_dir: &Path) {
    let mut mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let should_clear = {
        let state = mutation
            .scopes
            .get_mut(scope_dir)
            .expect("runtime config mutation scope missing");
        state.writers = state
            .writers
            .checked_sub(1)
            .expect("runtime config mutation writer underflow");
        if state.writers == 0 {
            state.generation = state.generation.wrapping_add(1);
            true
        } else {
            false
        }
    };
    if should_clear {
        clear_scope_entries(scope_dir, mutation.global_writers > 0);
    }
    drop(mutation);
    if should_clear {
        notify_mutation_change();
    }
}

fn finish_global_mutation() {
    let mut mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    mutation.global_writers = mutation
        .global_writers
        .checked_sub(1)
        .expect("runtime config global mutation writer underflow");
    if mutation.global_writers != 0 {
        return;
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
    clear_all_entries_preserving_active_profiles(&mutation);
    drop(mutation);
    notify_mutation_change();
}

fn mutation_state() -> &'static Mutex<MutationState> {
    MUTATION_STATE.get_or_init(|| Mutex::new(MutationState::default()))
}

fn mutation_notifier() -> &'static MutationNotifier {
    MUTATION_NOTIFIER.get_or_init(|| MutationNotifier {
        epoch: Mutex::new(0),
        changed: Condvar::new(),
    })
}

fn notify_mutation_change() {
    let notifier = mutation_notifier();
    let mut epoch = notifier
        .epoch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *epoch = epoch.wrapping_add(1);
    notifier.changed.notify_all();
}

pub(crate) fn mutation_epoch() -> u64 {
    *mutation_notifier()
        .epoch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn wait_for_mutation_change(observed_epoch: &mut u64, deadline: Instant) -> bool {
    #[cfg(test)]
    before_wait();
    let notifier = mutation_notifier();
    let mut epoch = notifier
        .epoch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *epoch != *observed_epoch {
        *observed_epoch = *epoch;
        return true;
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return false;
    }
    let (next_epoch, timeout) = notifier
        .changed
        .wait_timeout(epoch, remaining)
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    epoch = next_epoch;
    let changed = *epoch != *observed_epoch;
    *observed_epoch = *epoch;
    changed || !timeout.timed_out()
}

pub(crate) fn stable_cache_version(scope_dir: &Path) -> Option<CacheVersion> {
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let generation = GENERATION.load(Ordering::Acquire);
    if !generation.is_multiple_of(2) {
        return None;
    }
    let state = mutation.scopes.get(scope_dir);
    match state {
        Some(state) if state.writers == 0 && state.generation.is_multiple_of(2) => {
            Some(CacheVersion {
                generation,
                scope_generation: state.generation,
            })
        }
        Some(_) => None,
        None => Some(CacheVersion {
            generation,
            scope_generation: 0,
        }),
    }
}

pub(crate) fn invalidate() {
    advance_generation(2, false);
}

pub(crate) fn invalidate_active_profile() {
    advance_generation(2, true);
    *active_scope_store_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    #[cfg(test)]
    {
        *active_marker_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

pub(crate) fn active_scope_store() -> Result<ProfileScopeStore> {
    #[cfg(test)]
    let marker = {
        let path = crate::paths::active_profile_marker_path()?;
        let content = std::fs::read(&path).ok();
        (path, content)
    };
    if let Some(store) = active_scope_store_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        #[cfg(not(test))]
        {
            ensure_watcher(&store);
            return Ok(store);
        }
        #[cfg(test)]
        if active_marker_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            == Some(&marker)
        {
            return Ok(store);
        }
    }

    let store = ProfileScopeStore::from_active()?;
    #[cfg(not(test))]
    ensure_watcher(&store);
    *active_scope_store_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(store.clone());
    #[cfg(test)]
    {
        *active_marker_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(marker);
    }
    Ok(store)
}

pub(crate) fn source_revision(
    scope_store: &ProfileScopeStore,
    paths: &PluginConfigSlicePaths,
) -> Result<SourceRevision> {
    Ok(SourceRevision {
        lock: file_digest(&scope_store.plugins_lock_path())?,
        core: file_digest(&paths.core)?,
        os: file_digest(&paths.os)?,
        device: file_digest(&paths.device)?,
    })
}

pub(crate) fn lookup(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    manifest: &PluginManifest,
) -> Option<Option<Value>> {
    let identity = ManifestIdentity::from(manifest);
    lookup_for_identity(scope_store, plugin_id, &identity)
}

pub(crate) fn lookup_for_identity(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    identity: &ManifestIdentity,
) -> Option<Option<Value>> {
    if !cache_available() {
        return None;
    }
    let key = cache_key(scope_store, plugin_id);
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = cache.get(&key)?;
    if !entry_is_fresh(entry, &key.profile_dir, identity, &mutation) {
        return None;
    }
    #[cfg(test)]
    TEST_STATS.with(|stats| stats.borrow_mut().value_clones += 1);
    Some(entry.value.clone())
}

pub(crate) fn is_fresh(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    manifest: &PluginManifest,
) -> bool {
    let identity = ManifestIdentity::from(manifest);
    is_fresh_for_identity(scope_store, plugin_id, &identity)
}

pub(crate) fn is_fresh_for_identity(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    identity: &ManifestIdentity,
) -> bool {
    if !cache_available() {
        return false;
    }
    let key = cache_key(scope_store, plugin_id);
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .get(&key)
        .is_some_and(|entry| entry_is_fresh(entry, &key.profile_dir, identity, &mutation))
}

pub(crate) fn action_cache_status(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    identity: &ManifestIdentity,
) -> ActionCacheStatus {
    if !cache_available() {
        return ActionCacheStatus::Miss;
    }
    let key = cache_key(scope_store, plugin_id);
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache_is_fresh = {
        let cache = cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match cache.get(&key) {
            None => false,
            Some(entry) if entry.manifest_identity != *identity => {
                return ActionCacheStatus::Miss;
            }
            Some(entry) => entry_is_fresh(entry, &key.profile_dir, identity, &mutation),
        }
    };
    if cache_is_fresh {
        return ActionCacheStatus::Fresh;
    }
    let mutation_scope = mutation_scope_for(&mutation, &key.profile_dir);
    if mutation_scope == "none" {
        return ActionCacheStatus::Miss;
    }
    let readiness = action_readiness()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if readiness
        .get(&key)
        .is_some_and(|cached_identity| cached_identity == identity)
    {
        ActionCacheStatus::LastKnownGood { mutation_scope }
    } else {
        ActionCacheStatus::Miss
    }
}

pub(crate) fn publish_if_unchanged(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    manifest: &PluginManifest,
    source_before: SourceRevision,
    source_after: SourceRevision,
    version: CacheVersion,
    value: Option<Value>,
) -> bool {
    if !cache_available() || source_before != source_after {
        return false;
    }
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if version.generation != GENERATION.load(Ordering::Acquire)
        || current_scope_generation(&mutation, &scope_store.dir()) != Some(version.scope_generation)
    {
        return false;
    }
    let key = cache_key(scope_store, plugin_id);
    let manifest_identity = ManifestIdentity::from(manifest);
    let mut readiness = action_readiness()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key.clone(),
        CacheEntry {
            version,
            manifest_identity,
            value,
        },
    );
    readiness.insert(key, manifest_identity);
    true
}

fn cache_key(scope_store: &ProfileScopeStore, plugin_id: &str) -> CacheKey {
    CacheKey {
        profile_dir: scope_store.dir(),
        os_bucket: scope_store.os_bucket().to_string(),
        plugin_id: plugin_id.to_string(),
    }
}

fn cache() -> &'static Mutex<HashMap<CacheKey, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn action_readiness() -> &'static Mutex<HashMap<CacheKey, ManifestIdentity>> {
    ACTION_READINESS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_scope_store_cache() -> &'static Mutex<Option<ProfileScopeStore>> {
    ACTIVE_SCOPE_STORE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn active_marker_cache() -> &'static Mutex<Option<ActiveMarkerSnapshot>> {
    ACTIVE_MARKER.get_or_init(|| Mutex::new(None))
}

#[cfg(not(test))]
fn watcher() -> &'static Mutex<Option<WatcherState>> {
    WATCHER.get_or_init(|| Mutex::new(None))
}

fn entry_is_fresh(
    entry: &CacheEntry,
    scope_dir: &Path,
    identity: &ManifestIdentity,
    mutation: &MutationState,
) -> bool {
    entry.version.generation == generation()
        && current_scope_generation(mutation, scope_dir) == Some(entry.version.scope_generation)
        && entry.manifest_identity == *identity
}

pub(crate) fn mutation_active(scope_dir: &Path) -> bool {
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    mutation_active_for(&mutation, scope_dir)
}

pub(crate) fn mutation_scope(scope_dir: &Path) -> &'static str {
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    mutation_scope_for(&mutation, scope_dir)
}

fn mutation_active_for(mutation: &MutationState, scope_dir: &Path) -> bool {
    mutation.global_writers > 0
        || mutation
            .scopes
            .get(scope_dir)
            .is_some_and(|state| state.writers > 0)
}

fn mutation_scope_for(mutation: &MutationState, scope_dir: &Path) -> &'static str {
    if mutation.global_writers > 0 {
        return "global";
    }
    if mutation
        .scopes
        .get(scope_dir)
        .is_some_and(|state| state.writers > 0)
    {
        return "profile";
    }
    "none"
}

fn current_scope_generation(mutation: &MutationState, scope_dir: &Path) -> Option<u64> {
    match mutation.scopes.get(scope_dir) {
        None => Some(0),
        Some(state) if state.writers == 0 => Some(state.generation),
        Some(_) => None,
    }
}

fn clear_scope_entries(scope_dir: &Path, preserve_readiness: bool) {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut readiness = action_readiness()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|key, _| key.profile_dir != scope_dir);
    if !preserve_readiness {
        readiness.retain(|key, _| key.profile_dir != scope_dir);
    }
}

fn clear_all_entries() {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut readiness = action_readiness()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.clear();
    readiness.clear();
}

fn clear_all_entries_preserving_active_profiles(mutation: &MutationState) {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut readiness = action_readiness()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.clear();
    readiness.retain(|key, _| {
        mutation
            .scopes
            .get(&key.profile_dir)
            .is_some_and(|state| state.writers > 0)
    });
}

fn advance_generation(step: u64, clear_readiness: bool) {
    let mutation = mutation_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut readiness = action_readiness()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    GENERATION.fetch_add(step, Ordering::AcqRel);
    cache.clear();
    if clear_readiness {
        readiness.clear();
    }
    if !clear_readiness && mutation.global_writers == 0 {
        readiness.retain(|key, _| {
            mutation
                .scopes
                .get(&key.profile_dir)
                .is_some_and(|state| state.writers > 0)
        });
    }
    drop(cache);
    drop(readiness);
    notify_mutation_change();
}

fn file_digest(path: &Path) -> Result<Option<[u8; 32]>> {
    #[cfg(test)]
    TEST_STATS.with(|stats| stats.borrow_mut().source_revision_reads += 1);
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "profile config source must not be a symlink: {}",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!(
            "profile config source is not a regular file: {}",
            path.display()
        );
    }
    let content = std::fs::read(path)?;
    let digest = Sha256::digest(content);
    let mut result = [0_u8; 32];
    result.copy_from_slice(&digest);
    Ok(Some(result))
}

#[cfg(not(test))]
fn ensure_watcher(store: &ProfileScopeStore) {
    let root = store.profile_root().to_path_buf();
    {
        let mut current = watcher()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !WATCHER_REARM.load(Ordering::Acquire)
            && current.as_ref().is_some_and(|state| state.root == root)
        {
            set_cache_available(true);
            if !WATCHER_REARM.load(Ordering::Acquire) {
                return;
            }
            set_cache_available(false);
        }
        set_cache_available(false);
        *current = None;
    }
    if !root.is_dir() {
        log::warn!(
            "profile config watcher root is unavailable: {}",
            root.display()
        );
        WATCHER_REARM.store(true, Ordering::Release);
        set_cache_available(false);
        return;
    }

    let active_marker = crate::paths::active_profile_marker_path().ok();
    let watcher_failed = Arc::new(AtomicBool::new(false));
    let watcher_failed_for_callback = Arc::clone(&watcher_failed);
    let callback = move |result: notify::Result<notify::Event>| match result {
        Ok(event) if relevant_event(&event.kind) => {
            if active_marker.as_ref().is_some_and(|marker| {
                event.paths.iter().any(|path| {
                    path == marker
                        || path.file_name() == marker.file_name()
                            && path.parent() == marker.parent()
                })
            }) {
                invalidate_active_profile();
            } else {
                invalidate();
            }
        }
        Ok(_) => {}
        Err(error) => {
            log::warn!("profile config watcher failed: {error}");
            watcher_failed_for_callback.store(true, Ordering::Release);
            WATCHER_REARM.store(true, Ordering::Release);
            set_cache_available(false);
            invalidate();
        }
    };
    let mut next = match notify::recommended_watcher(callback) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("failed to create profile config watcher: {error}");
            WATCHER_REARM.store(true, Ordering::Release);
            set_cache_available(false);
            return;
        }
    };
    if let Err(error) = next.watch(&root, RecursiveMode::Recursive) {
        log::warn!(
            "failed to watch profile config root {}: {error}",
            root.display()
        );
        WATCHER_REARM.store(true, Ordering::Release);
        set_cache_available(false);
        return;
    }
    if watcher_failed.load(Ordering::Acquire) {
        WATCHER_REARM.store(true, Ordering::Release);
        set_cache_available(false);
        return;
    }
    *watcher()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(WatcherState {
        root,
        _watcher: next,
    });
    WATCHER_REARM.store(false, Ordering::Release);
    set_cache_available(true);
    if watcher_failed.load(Ordering::Acquire) {
        WATCHER_REARM.store(true, Ordering::Release);
        set_cache_available(false);
        *watcher()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

#[cfg(not(test))]
fn relevant_event(kind: &EventKind) -> bool {
    kind.is_create() || kind.is_modify() || kind.is_remove()
}

impl From<&PluginManifest> for ManifestIdentity {
    fn from(manifest: &PluginManifest) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"qol-runtime-config-manifest-identity-v1");
        update_identity_part(
            &mut hasher,
            manifest.plugin.uid.as_ref().map(|uid| uid.as_str()),
        );
        match &manifest.plugin.platforms {
            Some(platforms) => {
                hasher.update([1]);
                hasher.update((platforms.len() as u64).to_le_bytes());
                for platform in platforms {
                    update_identity_part(&mut hasher, Some(platform));
                }
            }
            None => hasher.update([0]),
        }
        Self {
            fingerprint: hasher.finalize().into(),
        }
    }
}

fn update_identity_part(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update((value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

#[cfg(debug_assertions)]
pub(crate) fn should_sample_cache_hit() -> bool {
    CACHE_HIT_SAMPLES
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(32)
}

#[cfg(test)]
pub(super) fn record_profile_read() {
    TEST_STATS.with(|stats| stats.borrow_mut().profile_reads += 1);
}

#[cfg(test)]
pub(super) fn record_materialization() {
    TEST_STATS.with(|stats| stats.borrow_mut().materializations += 1);
}

#[cfg(test)]
pub(super) fn before_publish() {
    let hook = BEFORE_PUBLISH
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
pub(super) fn set_before_publish_hook(hook: Option<BeforePublishHook>) {
    *BEFORE_PUBLISH
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = hook;
}

#[cfg(test)]
pub(super) fn before_wait() {
    let hook = BEFORE_WAIT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
pub(super) fn set_before_wait_hook(hook: Option<BeforePublishHook>) {
    *BEFORE_WAIT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = hook;
}

#[cfg(test)]
pub(super) fn reset_for_tests() {
    CACHE_AVAILABLE.store(true, Ordering::Release);
    clear_all_entries();
    *active_scope_store_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    *active_marker_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    TEST_STATS.with(|stats| *stats.borrow_mut() = TestStats::default());
    if let Some(hook) = BEFORE_PUBLISH.get() {
        *hook.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
    if let Some(hook) = BEFORE_WAIT.get() {
        *hook.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

#[cfg(test)]
pub(super) fn source_revision_reads_for_tests() -> usize {
    TEST_STATS.with(|stats| stats.borrow().source_revision_reads)
}

#[cfg(test)]
pub(super) fn profile_reads_for_tests() -> usize {
    TEST_STATS.with(|stats| stats.borrow().profile_reads)
}

#[cfg(test)]
pub(super) fn materializations_for_tests() -> usize {
    TEST_STATS.with(|stats| stats.borrow().materializations)
}

#[cfg(test)]
pub(super) fn value_clones_for_tests() -> usize {
    TEST_STATS.with(|stats| stats.borrow().value_clones)
}
