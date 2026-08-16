//! Shared profile sync engine for qol-tray and the `qol` CLI.
//!
//! Both entry points drive the same on-disk profile store and the same
//! GitHub-backed git repository. This crate owns every piece of that contract
//! so the tray's `SyncService` and `qol sync` cannot drift:
//!
//! - **Conflict model**: `types` (conflict, incident, status shapes) and
//!   `merge` (field-level three-way merge, per-plugin `plugins.lock.json`
//!   union).
//! - **State-file format**: `state` owns `SyncStateFile` and `SyncToggles`
//!   JSON shapes plus their paths, and `SyncTarget` (`profile/sync.json`).
//! - **Backup naming**: `state::write_conflict_backup` produces the
//!   `<timestamp>-conflict.json` names in the tracked backups dir, and
//!   `backup_file_path` enforces the name allowlist used by both entry
//!   points.
//! - **Allowlist**: `scope` owns which relative profile paths are synced and
//!   which participate in field-level merging.
//! - **Repository shape**: `git_repo` owns the `main` branch, `origin`
//!   remote, token-authenticated transport, and commit identity handling.
//!
//! Consumers stay thin adapters: they resolve the config directory and the
//! profile root, then pass plain paths and values into this crate. The crate
//! has no dependency on qol-tray and no tray-specific policy.
//!
//! # Cross-process sync lock
//!
//! The tray (an always-on daemon) and `qol sync` (a one-shot CLI) can run
//! concurrently against the same profile repo. Both must serialize their
//! pull/merge/push windows, or a fetch can interleave with a push and corrupt
//! the local checkout.
//!
//! [`lock::SyncLock`] is an advisory cross-process lock over a lockfile in
//! the per-device sync state dir (`<profile>/<name>/device/sync/sync.lock`).
//! It uses `std::fs::File::lock`, which maps to `flock(2)` on Unix and
//! `LockFileEx` on Windows; the OS releases it automatically when the
//! holding process exits, so there is no stale-lock problem.
//!
//! Both the tray's `SyncService` and `qol sync` acquire [`lock::SyncLock`]
//! at the start of every pull/merge/push window and hold it until the git
//! work is done. Acquisition is blocking: a CLI sync waits for an in-flight
//! tray sync (and vice versa) instead of failing. Consumers that already
//! serialize in-process (the tray's `operation_lock`) acquire the file lock
//! only around the git window, so holding it never covers unrelated I/O.
//! Use [`lock::SyncLock::try_acquire`] when a non-blocking probe is needed.
//!
//! The lockfile lives under `device/`, which the sync allowlist and the
//! repo `.gitignore` both exclude, so it never reaches the remote.

pub mod git_repo;
pub mod lock;
pub mod merge;
pub mod migrate;
pub mod promote;
pub mod reconcile;
pub mod scope;
pub mod state;
pub mod types;

pub use git_repo::{CommitInfo, GitRepo, PullOutcome, SignatureSpec};
pub use lock::SyncLock;
pub use merge::{
    merge_json, merge_json_resolved, merge_profile, merge_profile_with, ConflictResolver,
    FieldConflict, FileMerge, ProfileMerge, ProfileSnapshot,
};
pub use migrate::repair_profile_schema;
pub use promote::{promote_allowlisted_clone, promote_clone_git_dir, PromotionScope};
pub use reconcile::reconcile;
pub use scope::{
    device_local_dir, is_sync_allowlisted, mergeable_path, DeviceLocalDirError, GITIGNORE_CONTENTS,
};
pub use state::{
    backup_file_path, build_status, clear_sync_target, ensure_sync_dirs, filename_string,
    list_backup_entries, load_state_file, load_sync_target, load_toggles, now_rfc3339,
    save_state_file, save_sync_target, save_toggles, write_conflict_backup, SyncPaths,
    SyncStateFile, SyncToggles,
};
pub use types::{
    ConflictChoice, ResolvableConflict, Side, SyncActionResult, SyncBackupEntry, SyncBackupPreview,
    SyncConnectRequest, SyncHealth, SyncIncident, SyncIncidentKind, SyncStatus, SyncTarget,
};
