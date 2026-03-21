# Production Logging (Phase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add production file logging with rate-limited error deduplication, build info embedding, and daily rotation.

**Architecture:** A `log_error!` macro writes structured entries to a daily log file via a thread-safe writer. A rate limiter deduplicates errors by signature key, suppressing after 5 occurrences. Suppression state persists across restarts and auto-resets on version change. Platform-specific modules resolve the log directory path. A `build.rs` script embeds the git commit hash at compile time. Plugin manifests gain an optional `[build]` section for their own commit hashes.

**Tech Stack:** Rust, `chrono` (timestamps), `log` crate (existing), platform `dirs` crate (existing)

**Spec:** `docs/superpowers/specs/2026-03-21-state-logging-design.md` (Phase 2)

**Out of scope (separate tasks):**
- Plugin CI pipeline updates to emit `[build] commit` into `plugin.toml` — cross-repo work, not qol-tray code
- Detailed OS version detection, display server (X11/Wayland), monitor count in startup line — requires platform-specific detection, follow-up task

---

### Task 1: Platform Log Directory Paths

Create the platform strategy for resolving the log file directory.

**Files:**
- Create: `src/logging/platform/mod.rs`
- Create: `src/logging/platform/linux.rs`
- Create: `src/logging/platform/macos.rs`
- Create: `src/logging/platform/windows.rs`
- Modify: `src/paths.rs` — promote `base_data_dir` to `pub(crate)`

- [ ] **Step 1: Write tests in platform/mod.rs**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_is_absolute_and_ends_with_logs() {
        let dir = log_dir();
        assert!(dir.is_absolute(), "log dir {:?} should be absolute", dir);
        assert!(dir.ends_with("logs"), "log dir {:?} should end with 'logs'", dir);
    }

    #[test]
    fn log_dir_contains_app_name() {
        let dir = log_dir();
        let path_str = dir.to_string_lossy();
        assert!(
            path_str.contains("qol-tray"),
            "log dir {:?} should contain qol-tray",
            dir
        );
    }
}
```

- [ ] **Step 2: Implement platform modules**

`src/logging/platform/mod.rs`:
```rust
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::path::PathBuf;

pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    return linux::log_dir();
    #[cfg(target_os = "macos")]
    return macos::log_dir();
    #[cfg(target_os = "windows")]
    return windows::log_dir();
}

#[cfg(test)]
mod tests { ... }
```

`src/logging/platform/linux.rs`:
```rust
use std::path::PathBuf;

pub(super) fn log_dir() -> PathBuf {
    crate::paths::base_data_dir()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/qol-tray/logs"))
}
```

`src/logging/platform/macos.rs`:
```rust
use std::path::PathBuf;

pub(super) fn log_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Library/Logs/qol-tray"))
        .unwrap_or_else(|| PathBuf::from("/tmp/qol-tray/logs"))
}
```

`src/logging/platform/windows.rs`:
```rust
use std::path::PathBuf;

pub(super) fn log_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|d| d.join("qol-tray/logs"))
        .unwrap_or_else(|| PathBuf::from("C:/Temp/qol-tray/logs"))
}
```

- [ ] **Step 3: Promote base_data_dir to pub(crate)**

In `src/paths.rs`, change `fn base_data_dir()` to `pub(crate) fn base_data_dir()`.

- [ ] **Step 4: Add platform module to logging**

In `src/logging/mod.rs`, add:
```rust
mod platform;
```

- [ ] **Step 5: Run tests, commit**

```bash
git commit -m "feat: add platform-specific log directory paths"
```

---

### Task 2: Build Info Embedding

Create `build.rs` to embed git commit hash at compile time.

**Files:**
- Create: `build.rs` (crate root)

- [ ] **Step 1: Create build.rs**

```rust
use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", hash);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");
}
```

Available at runtime as `env!("GIT_COMMIT_HASH")`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: builds with GIT_COMMIT_HASH set

- [ ] **Step 3: Commit**

```bash
git commit -m "feat: embed git commit hash at compile time via build.rs"
```

---

### Task 3: Log File Writer

Thread-safe writer that appends entries to daily log files with rotation.

**Files:**
- Create: `src/logging/writer.rs`
- Add: `chrono` to Cargo.toml dependencies

- [ ] **Step 1: Add chrono dependency**

In `Cargo.toml` under `[dependencies]`:
```toml
chrono = { version = "0.4", default-features = false, features = ["clock"] }
```

- [ ] **Step 2: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn log_file_name_contains_date() {
        let name = log_file_name();
        assert!(name.starts_with("qol-tray-"), "name: {}", name);
        assert!(name.ends_with(".log"), "name: {}", name);
        assert!(name.len() > 20, "name should contain date: {}", name);
    }

    #[test]
    fn write_entry_creates_file_and_appends() {
        let tmp = TempDir::new().unwrap();
        let writer = LogWriter::new(tmp.path().to_path_buf());

        writer.write("first line\n");
        writer.write("second line\n");

        let files: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 1, "should have one log file");

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("first line"), "content: {}", content);
        assert!(content.contains("second line"), "content: {}", content);
    }

    #[test]
    fn rotate_removes_old_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        for day in 1..=10 {
            let name = format!("qol-tray-2026-03-{:02}.log", day);
            std::fs::write(dir.join(&name), "old").unwrap();
        }

        rotate_old_logs(dir, 7);

        let remaining: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".log"))
            .collect();
        assert_eq!(remaining.len(), 7, "should keep 7 most recent");
    }

    #[test]
    fn rotate_ignores_non_log_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("other.txt"), "keep").unwrap();
        std::fs::write(tmp.path().join("qol-tray-2020-01-01.log"), "old").unwrap();

        rotate_old_logs(tmp.path(), 7);

        assert!(tmp.path().join("other.txt").exists());
    }
}
```

- [ ] **Step 3: Implement LogWriter**

```rust
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) struct LogWriter {
    dir: PathBuf,
    file: Mutex<Option<(String, File)>>,
}

impl LogWriter {
    pub(crate) fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Self {
            dir,
            file: Mutex::new(None),
        }
    }

    pub(crate) fn write(&self, entry: &str) {
        let today = log_file_name();
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let file = match guard.as_mut() {
            Some((name, f)) if *name == today => f,
            _ => {
                let path = self.dir.join(&today);
                let f = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path);
                let Ok(f) = f else { return };
                *guard = Some((today, f));
                &mut guard.as_mut().unwrap().1
            }
        };
        let _ = file.write_all(entry.as_bytes());
    }
}

fn log_file_name() -> String {
    let now = chrono::Local::now();
    format!("qol-tray-{}.log", now.format("%Y-%m-%d"))
}

pub(crate) fn rotate_old_logs(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.starts_with("qol-tray-") && s.ends_with(".log")
        })
        .collect();

    if logs.len() <= keep {
        return;
    }

    logs.sort_by_key(|e| e.file_name());
    for entry in &logs[..logs.len() - keep] {
        let _ = fs::remove_file(entry.path());
    }
}
```

- [ ] **Step 4: Add module to logging/mod.rs**

```rust
mod writer;
```

- [ ] **Step 5: Run tests, commit**

```bash
git commit -m "feat: add log file writer with daily rotation"
```

---

### Task 4: Rate Limiter

Deduplication by signature key with suppression after 5 occurrences.

**Files:**
- Create: `src/logging/rate_limiter.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(version: &str) -> RateLimiter {
        RateLimiter::new(version.to_string())
    }

    #[test]
    fn first_occurrence_is_allowed() {
        let rl = limiter("1.0.0");
        assert!(rl.check("err.key").is_allowed());
    }

    #[test]
    fn occurrences_up_to_threshold_are_allowed() {
        let rl = limiter("1.0.0");
        for _ in 0..4 {
            assert!(rl.check("err.key").is_allowed());
        }
    }

    #[test]
    fn fifth_occurrence_returns_suppressed() {
        let rl = limiter("1.0.0");
        for _ in 0..4 {
            rl.check("err.key");
        }
        let result = rl.check("err.key");
        assert!(result.is_suppressed(), "5th should suppress");
    }

    #[test]
    fn after_suppression_is_rejected() {
        let rl = limiter("1.0.0");
        for _ in 0..6 {
            rl.check("err.key");
        }
        let result = rl.check("err.key");
        assert!(result.is_rejected(), "6th+ should reject");
    }

    #[test]
    fn different_keys_are_independent() {
        let rl = limiter("1.0.0");
        for _ in 0..5 {
            rl.check("key.a");
        }
        assert!(rl.check("key.b").is_allowed());
    }

    #[test]
    fn check_returns_occurrence_count() {
        let rl = limiter("1.0.0");
        let cases = [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 5)];
        for (call, expected_count) in cases {
            let result = rl.check("err.key");
            assert_eq!(
                result.count(), expected_count,
                "call {} should have count {}", call, expected_count
            );
        }
    }

    #[test]
    fn load_and_save_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("suppressed.json");

        let rl = limiter("1.0.0");
        for _ in 0..5 {
            rl.check("err.key");
        }
        rl.save(&path);

        let rl2 = RateLimiter::load(&path, "1.0.0".to_string());
        assert!(rl2.check("err.key").is_rejected());
    }

    #[test]
    fn version_change_resets_suppression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("suppressed.json");

        let rl = limiter("1.0.0");
        for _ in 0..5 {
            rl.check("err.key");
        }
        rl.save(&path);

        let rl2 = RateLimiter::load(&path, "2.0.0".to_string());
        assert!(rl2.check("err.key").is_allowed(), "new version should reset");
    }
}
```

- [ ] **Step 2: Implement RateLimiter**

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

const SUPPRESS_THRESHOLD: u32 = 5;

pub(crate) struct RateLimiter {
    version: String,
    state: Mutex<HashMap<String, EntryState>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct EntryState {
    count: u32,
    suppressed: bool,
    version: String,
    first_seen: String,
    last_seen: String,
    last_message: Option<String>,
    source: Option<String>,
    location: Option<String>,
}

pub(crate) enum CheckResult {
    Allowed { count: u32 },
    Suppressed { count: u32 },
    Rejected,
}

impl CheckResult {
    pub(crate) fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    pub(crate) fn is_suppressed(&self) -> bool {
        matches!(self, Self::Suppressed { .. })
    }

    pub(crate) fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected)
    }

    pub(crate) fn count(&self) -> u32 {
        match self {
            Self::Allowed { count } | Self::Suppressed { count } => *count,
            Self::Rejected => SUPPRESS_THRESHOLD,
        }
    }
}

impl RateLimiter {
    pub(crate) fn new(version: String) -> Self {
        Self {
            version,
            state: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn load(path: &Path, version: String) -> Self {
        let mut state: HashMap<String, EntryState> = std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();

        state.retain(|_, entry| entry.version == version);

        Self {
            version,
            state: Mutex::new(state),
        }
    }

    pub(crate) fn check(&self, key: &str) -> CheckResult {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return CheckResult::Allowed { count: 1 },
        };

        let entry = state.entry(key.to_string()).or_insert_with(|| EntryState {
            count: 0,
            suppressed: false,
            version: self.version.clone(),
            first_seen: now_iso(),
            last_seen: now_iso(),
            last_message: None,
            source: None,
            location: None,
        });

        if entry.suppressed {
            entry.count += 1;
            entry.last_seen = now_iso();
            return CheckResult::Rejected;
        }

        entry.count += 1;
        entry.last_seen = now_iso();

        if entry.count >= SUPPRESS_THRESHOLD {
            entry.suppressed = true;
            return CheckResult::Suppressed { count: entry.count };
        }

        CheckResult::Allowed { count: entry.count }
    }

    pub(crate) fn update_entry_context(
        &self,
        key: &str,
        message: &str,
        source: &str,
        location: &str,
    ) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(entry) = state.get_mut(key) {
            entry.last_message = Some(message.to_string());
            entry.source = Some(source.to_string());
            entry.location = Some(location.to_string());
        }
    }

    pub(crate) fn save(&self, path: &Path) {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let suppressed: HashMap<_, _> = state
            .iter()
            .filter(|(_, e)| e.suppressed)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if suppressed.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let Ok(content) = serde_json::to_string_pretty(&suppressed) else {
            return;
        };
        let _ = std::fs::write(path, content);
    }
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}
```

- [ ] **Step 3: Add module to logging/mod.rs**

```rust
mod rate_limiter;
```

- [ ] **Step 4: Run tests, commit**

```bash
git commit -m "feat: add rate limiter with signature-based suppression"
```

---

### Task 5: log_error! Macro and Production Logger

Define the macro and initialize the production logger with writer + rate limiter.

**Files:**
- Create: `src/logging/prod.rs`
- Modify: `src/logging/mod.rs` — add prod module, public API
- Modify: `src/main.rs` — replace env_logger init
- Modify: `src/paths.rs` — add `suppressed_errors_path()`

- [ ] **Step 1: Add suppressed_errors_path to paths.rs**

```rust
pub fn suppressed_errors_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("suppressed-errors.json"))
}
```

- [ ] **Step 2: Create src/logging/prod.rs**

This module holds the global prod logger state and the `log_error!` macro.

```rust
use std::sync::OnceLock;

use super::rate_limiter::{CheckResult, RateLimiter};
use super::writer::{self, LogWriter};

struct ProdLogger {
    writer: LogWriter,
    limiter: RateLimiter,
    version_tag: String,
}

static LOGGER: OnceLock<ProdLogger> = OnceLock::new();

pub(crate) fn init() {
    let version = format!(
        "v{}@{}",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_COMMIT_HASH")
    );
    let log_dir = super::platform::log_dir();
    let suppressed_path = crate::paths::suppressed_errors_path()
        .unwrap_or_else(|_| log_dir.join("suppressed-errors.json"));

    writer::rotate_old_logs(&log_dir, 7);

    let limiter = RateLimiter::load(&suppressed_path, version.clone());
    let writer = LogWriter::new(log_dir);

    let _ = LOGGER.set(ProdLogger {
        writer,
        limiter,
        version_tag: version,
    });
}

pub(crate) fn log_entry(key: &str, source: &str, message: &str, file: &str, line: u32) {
    let Some(logger) = LOGGER.get() else {
        return;
    };

    let result = logger.limiter.check(key);
    match result {
        CheckResult::Rejected => return,
        CheckResult::Allowed { count } => {
            let count_suffix = if count > 1 {
                format!(" (x{})", count)
            } else {
                String::new()
            };
            let entry = format_entry(
                &logger.version_tag,
                source,
                key,
                message,
                file,
                line,
                &count_suffix,
            );
            logger.writer.write(&entry);
        }
        CheckResult::Suppressed { count } => {
            let entry = format_entry(
                &logger.version_tag,
                source,
                key,
                message,
                file,
                line,
                &format!(" (x{}, suppressed)", count),
            );
            logger.writer.write(&entry);
            save_suppressed(&logger.limiter);
        }
    }

    logger
        .limiter
        .update_entry_context(key, message, source, &format!("{}:{}", file, line));
}

fn format_entry(
    version: &str,
    source: &str,
    key: &str,
    message: &str,
    file: &str,
    line: u32,
    suffix: &str,
) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    format!(
        "[{}] [{}] [{}] ERROR {} — {}{} ({}:{})\n",
        timestamp, version, source, key, message, suffix, file, line
    )
}

fn save_suppressed(limiter: &RateLimiter) {
    if let Ok(path) = crate::paths::suppressed_errors_path() {
        limiter.save(&path);
    }
}

pub(crate) fn log_startup(info: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let entry = format!(
        "[{}] [{}] [core] STARTUP — {}\n",
        timestamp, logger.version_tag, info
    );
    logger.writer.write(&entry);
}

#[macro_export]
macro_rules! log_error {
    ($key:expr, source = $source:expr, $($arg:tt)+) => {
        $crate::logging::prod::log_entry(
            $key,
            &$source,
            &format!($($arg)+),
            file!(),
            line!(),
        )
    };
    ($key:expr, $($arg:tt)+) => {
        $crate::logging::prod::log_entry(
            $key,
            "core",
            &format!($($arg)+),
            file!(),
            line!(),
        )
    };
}
```

- [ ] **Step 3: Update logging/mod.rs**

Add:
```rust
pub mod prod;
```

- [ ] **Step 4: Update main.rs to initialize prod logger**

Replace the prod logger initialization:
```rust
#[cfg(not(feature = "dev"))]
{
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    qol_tray::logging::prod::init();
}
```

Keep `env_logger` for the `log::info!` / `log::warn!` calls that exist throughout the codebase (they write to stderr). The prod file logger is additive — `log_error!` macro writes to the file, `log::*` macros continue to write to stderr via env_logger.

- [ ] **Step 5: Run tests, commit**

```bash
git commit -m "feat: add log_error! macro and production file logger"
```

---

### Task 6: Plugin Manifest Build Section

Add optional `[build]` section with commit hash to PluginManifest.

**Files:**
- Modify: `src/plugins/manifest/schema.rs`
- Modify: `src/plugins/manifest/mod.rs` — re-export BuildInfo

- [ ] **Step 1: Add BuildInfo struct and field**

In `src/plugins/manifest/schema.rs`, add struct:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BuildInfo {
    #[serde(default)]
    pub commit: Option<String>,
}
```

Add field to `PluginManifest`:
```rust
#[serde(default)]
pub build: BuildInfo,
```

- [ ] **Step 2: Update mod.rs re-exports**

Add `BuildInfo` to the `pub use schema::{ ... }` line.

- [ ] **Step 3: Verify existing tests still pass**

Existing manifest parsing tests should pass because `BuildInfo` has `Default` and `serde(default)`.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat: add optional [build] section to plugin manifest"
```

---

### Task 7: Startup Context Line

Log a single STARTUP entry with environment info on every boot.

**Files:**
- Modify: `src/main.rs` — add startup log call after plugins loaded

- [ ] **Step 1: Build startup info string**

After `plugin_manager.load_plugins()` succeeds in `async_init_inner`, gather context and call `log_startup`:

```rust
#[cfg(not(feature = "dev"))]
{
    let startup_info = build_startup_info(&plugin_manager);
    qol_tray::logging::prod::log_startup(&startup_info);
}
```

Add helper function in main.rs:

```rust
#[cfg(not(feature = "dev"))]
fn build_startup_info(pm: &std::sync::Arc<std::sync::Mutex<qol_tray::plugins::PluginManager>>) -> String {
    let plugins_desc = match pm.lock() {
        Ok(manager) => manager
            .plugins()
            .map(|p| {
                let id = &p.id;
                let version = &p.manifest.plugin.version;
                let commit = p.manifest.build.commit.as_deref().unwrap_or("?");
                format!("{}@{}@{}", id, version, commit)
            })
            .collect::<Vec<_>>()
            .join(", "),
        Err(_) => "unknown".to_string(),
    };

    let os_info = std::env::consts::OS;
    format!("{}, plugins: [{}]", os_info, plugins_desc)
}
```

Note: This logs `std::env::consts::OS` (e.g., "linux") and plugin list. OS version, display server (X11/Wayland), and monitor count are spec requirements but need platform-specific detection — deferred to a follow-up task to avoid blocking the core logging infrastructure.

- [ ] **Step 2: Commit**

```bash
git commit -m "feat: log startup context with OS and loaded plugins"
```

---

### Task 8: Wire Daemon Relay to Prod Logger

Connect daemon stderr relay to the production log file so plugin errors are captured.

**Files:**
- Modify: `src/logging/relay.rs` — add prod log sink
- Modify: `src/plugins/daemon_lifecycle/spawn.rs` — pipe stderr in prod

- [ ] **Step 1: Add prod log sink to relay**

In `src/logging/relay.rs`, add a function that routes daemon error lines through `log_error!`:

```rust
pub(crate) fn attach_with_prod_log(
    plugin_id: &str,
    plugin_version: &str,
    plugin_commit: Option<&str>,
    stderr: Option<impl Read + Send + 'static>,
) {
    let Some(stderr) = stderr else { return };
    let source = build_source(plugin_id, plugin_version, plugin_commit);
    let key = format!("plugin.{}.daemon_stderr", plugin_id);
    let id = plugin_id.to_string();
    std::thread::spawn(move || {
        let mut buf = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            let n = buf.read_line(&mut line).unwrap_or(0);
            if n == 0 {
                break;
            }
            let trimmed = line.trim_end();
            if is_error_line(trimmed) {
                crate::log_error!(
                    &key,
                    source = source,
                    "[{}] {}", id, trimmed
                );
            }
            eprint!("{}", line);
        }
    });
}

fn build_source(plugin_id: &str, version: &str, commit: Option<&str>) -> String {
    match commit {
        Some(c) => format!("plugin:{}@{}@{}", plugin_id, version, c),
        None => format!("plugin:{}@{}", plugin_id, version),
    }
}

fn is_error_line(line: &str) -> bool {
    line.contains("ERROR")
        || line.contains("error")
        || line.contains("FATAL")
        || line.contains("panic")
        || line.contains("PANIC")
}
```

- [ ] **Step 2: Update spawn.rs prod path**

In `src/plugins/daemon_lifecycle/spawn.rs`, update the `#[cfg(not(feature = "dev"))]` branch in `spawn_daemon`:

```rust
#[cfg(not(feature = "dev"))]
{
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let version = plugin.manifest.plugin.version.clone();
    let commit = plugin.manifest.build.commit.clone();
    crate::logging::relay::attach_with_prod_log(
        plugin.id.as_str(),
        &version,
        commit.as_deref(),
        child.stderr.take(),
    );
    return Ok(child);
}
```

This pipes stderr in prod (for error capture) while keeping stdout inherited (for normal output). The `return Ok(child)` intentionally short-circuits past the shared `relay::attach` call at the bottom of `spawn_daemon` — the prod path handles its own relay via `attach_with_prod_log`. The dev path continues to the shared `relay::attach` as before. Read the full `spawn_daemon` function before making changes to understand the control flow.

- [ ] **Step 3: Commit**

```bash
git commit -m "feat: capture daemon errors in production log file"
```
