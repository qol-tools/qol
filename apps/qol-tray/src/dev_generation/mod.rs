use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};

use qol_conventions::{
    DEFAULT_PORT, DEV_GENERATION_MODE_SHADOW, ENV_DEV_GENERATION_ID, ENV_DEV_GENERATION_MODE,
    ENV_DEV_READY_FILE, ENV_DEV_ROLLING_RESTART, ENV_DEV_UI_PORT, STATE_SOCKET_FILE,
};

#[cfg(feature = "dev")]
mod platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationContext {
    id: Option<String>,
    mode: GenerationMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenerationMode {
    Stable,
    Shadow,
}

impl GenerationContext {
    pub fn current() -> Self {
        if PROMOTED_TO_STABLE.load(Ordering::Acquire) {
            return Self::stable();
        }
        let mode = match std::env::var(ENV_DEV_GENERATION_MODE) {
            Ok(value) if value == DEV_GENERATION_MODE_SHADOW => GenerationMode::Shadow,
            _ => GenerationMode::Stable,
        };
        let id = std::env::var(ENV_DEV_GENERATION_ID)
            .ok()
            .and_then(|value| generation_id(value.as_str()));
        Self { id, mode }
    }

    pub fn generation_id(&self) -> Option<String> {
        self.id.clone()
    }

    pub fn stable() -> Self {
        Self {
            id: None,
            mode: GenerationMode::Stable,
        }
    }

    pub fn shadow(id: &str) -> Self {
        Self {
            id: Some(generation_id(id).unwrap_or_else(|| format!("dev-{}", std::process::id()))),
            mode: GenerationMode::Shadow,
        }
    }

    pub fn is_shadow(&self) -> bool {
        self.mode == GenerationMode::Shadow
    }

    pub fn ui_bind_port(&self) -> u16 {
        if !self.is_shadow() {
            return DEFAULT_PORT;
        }
        std::env::var(ENV_DEV_UI_PORT)
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(0)
    }

    pub fn state_socket_path(&self) -> PathBuf {
        let path = crate::paths::runtime_dir()
            .join("sockets")
            .join(STATE_SOCKET_FILE);
        if !self.is_shadow() {
            return path;
        }
        namespaced_socket(&path, self.id())
    }

    pub fn daemon_socket_path(&self, socket: &str) -> PathBuf {
        let file_name = Path::new(socket)
            .file_name()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| std::ffi::OsStr::new("qol-plugin.sock"));
        let path = crate::paths::runtime_dir().join("sockets").join(file_name);
        if !self.is_shadow() {
            return path;
        }
        namespaced_socket(&path, self.id())
    }

    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("dev")
    }
}

pub fn current() -> GenerationContext {
    GenerationContext::current()
}

pub fn is_shadow() -> bool {
    current().is_shadow()
}

pub fn is_rolling_restart() -> bool {
    std::env::var(ENV_DEV_ROLLING_RESTART)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

// A shadow generation boots while the predecessor generation is still fully
// running. Namespaced socket paths keep the two generations' IPC apart, but
// daemons that grab exclusive host resources (event taps, hotkeys, fixed UDP
// ports, gpui UIs) cannot coexist with their predecessor twins - so daemon
// autostart is held until promotion, when the predecessor is already gone.
static DAEMON_AUTOSTART_RELEASED: AtomicBool = AtomicBool::new(false);
static PROMOTED_TO_STABLE: AtomicBool = AtomicBool::new(false);

const MIN_DISTINGUISHING_GENERATION_ID_CHARS: usize = 8;

#[cfg(feature = "dev")]
const PREDECESSOR_DAEMON_EXIT_GRACE: Duration = Duration::from_secs(2);
#[cfg(feature = "dev")]
const PREDECESSOR_DAEMON_TERM_GRACE: Duration = Duration::from_millis(500);
#[cfg(feature = "dev")]
const PREDECESSOR_DAEMON_POLL: Duration = Duration::from_millis(50);

#[cfg(feature = "dev")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PredecessorDaemon {
    plugin_id: String,
    pid: u32,
}

pub fn daemon_autostart_held() -> bool {
    is_shadow() && !DAEMON_AUTOSTART_RELEASED.load(Ordering::Acquire)
}

pub fn promote_to_stable() {
    PROMOTED_TO_STABLE.store(true, Ordering::Release);
    DAEMON_AUTOSTART_RELEASED.store(true, Ordering::Release);
    std::env::remove_var(ENV_DEV_GENERATION_MODE);
    std::env::remove_var(ENV_DEV_GENERATION_ID);
    std::env::remove_var(ENV_DEV_UI_PORT);
    std::env::remove_var(ENV_DEV_READY_FILE);
}

pub fn state_socket_path() -> PathBuf {
    current().state_socket_path()
}

pub fn daemon_socket_path(socket: &str) -> PathBuf {
    current().daemon_socket_path(socket)
}

#[cfg(feature = "dev")]
pub fn drain_predecessor_daemons_for_promotion() -> anyhow::Result<()> {
    let predecessor_daemons = tracked_predecessor_daemons();
    if predecessor_daemons.is_empty() {
        crate::plugins::daemon_tracker::registry::clear_all(&crate::paths::runtime_pids_dir());
        return Ok(());
    }
    log::info!(
        "Promoted dev generation: waiting for predecessor daemons to exit: {}",
        format_predecessor_daemons(&predecessor_daemons)
    );
    let remaining =
        wait_for_predecessor_daemons_to_exit(predecessor_daemons, PREDECESSOR_DAEMON_EXIT_GRACE);
    if remaining.is_empty() {
        crate::plugins::daemon_tracker::registry::clear_all(&crate::paths::runtime_pids_dir());
        log::info!("Promoted dev generation: predecessor daemons exited cleanly");
        return Ok(());
    }
    log::warn!(
        "Promoted dev generation: terminating predecessor daemons still alive: {}",
        format_predecessor_daemons(&remaining)
    );
    for daemon in &remaining {
        crate::process_utils::terminate_group(daemon.pid as i32, PREDECESSOR_DAEMON_TERM_GRACE);
    }
    let remaining = wait_for_predecessor_daemons_to_exit(remaining, PREDECESSOR_DAEMON_TERM_GRACE);
    if remaining.is_empty() {
        crate::plugins::daemon_tracker::registry::clear_all(&crate::paths::runtime_pids_dir());
        log::info!("Promoted dev generation: predecessor daemons stopped after TERM");
        return Ok(());
    }
    anyhow::bail!(
        "predecessor daemons still alive after promotion drain: {}",
        format_predecessor_daemons(&remaining)
    )
}

#[cfg(feature = "dev")]
fn tracked_predecessor_daemons() -> Vec<PredecessorDaemon> {
    predecessor_daemons_from_pid_files(&crate::paths::runtime_pids_dir())
        .into_iter()
        .filter(|daemon| process_holds_handoff_resources(daemon.pid))
        .collect()
}

#[cfg(feature = "dev")]
fn predecessor_daemons_from_pid_files(pids_dir: &Path) -> Vec<PredecessorDaemon> {
    let mut daemons: Vec<_> = crate::plugins::daemon_tracker::registry::tracked_pids(pids_dir)
        .map(|(plugin_id, pid)| PredecessorDaemon { plugin_id, pid })
        .collect();
    daemons.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then(left.pid.cmp(&right.pid))
    });
    daemons.dedup();
    daemons
}

#[cfg(feature = "dev")]
fn wait_for_predecessor_daemons_to_exit(
    mut daemons: Vec<PredecessorDaemon>,
    timeout: Duration,
) -> Vec<PredecessorDaemon> {
    let deadline = Instant::now() + timeout;
    loop {
        daemons.retain(|daemon| process_holds_handoff_resources(daemon.pid));
        if daemons.is_empty() || Instant::now() >= deadline {
            return daemons;
        }
        std::thread::sleep(PREDECESSOR_DAEMON_POLL);
    }
}

#[cfg(feature = "dev")]
fn process_holds_handoff_resources(pid: u32) -> bool {
    platform::process_holds_handoff_resources(pid)
}

#[cfg(feature = "dev")]
fn format_predecessor_daemons(daemons: &[PredecessorDaemon]) -> String {
    daemons
        .iter()
        .map(|daemon| format!("{}={}", daemon.plugin_id, daemon.pid))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn write_ready_file(port: u16) -> std::io::Result<()> {
    let Some(path) = std::env::var_os(ENV_DEV_READY_FILE).map(PathBuf::from) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let context = current();
    let payload = serde_json::json!({
        "generation": if context.is_shadow() { DEV_GENERATION_MODE_SHADOW } else { "stable" },
        "id": context.id.as_deref(),
        "port": port,
        "stateSocket": context.state_socket_path().to_string_lossy().to_string(),
    });
    std::fs::write(path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(())
}

fn generation_id(value: &str) -> Option<String> {
    let filtered: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

fn namespaced_socket(path: &Path, id: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("socket");
    let stem = file_name.strip_suffix(".sock").unwrap_or(file_name);
    let candidate = parent.join(format!("{stem}.{id}.sock"));
    let overflow = candidate
        .as_os_str()
        .len()
        .saturating_sub(qol_runtime::local_ipc::MAX_SOCKET_PATH_BYTES);
    if overflow == 0 {
        return candidate;
    }
    let kept = id.chars().count().saturating_sub(overflow);
    if kept < MIN_DISTINGUISHING_GENERATION_ID_CHARS {
        log::error!(
            "Socket directory leaves only {} characters for the dev generation id at {}; \
             shadow generations may collide on the same socket",
            kept,
            parent.display()
        );
    }
    let id: String = id.chars().take(kept).collect();
    parent.join(format!("{stem}.{id}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_generation_state() {
        PROMOTED_TO_STABLE.store(false, Ordering::Release);
        DAEMON_AUTOSTART_RELEASED.store(false, Ordering::Release);
    }

    #[test]
    fn stable_generation_keeps_public_addresses() {
        let ctx = GenerationContext::stable();
        let sockets = crate::paths::runtime_dir().join("sockets");

        assert_eq!(ctx.ui_bind_port(), DEFAULT_PORT);
        assert_eq!(ctx.state_socket_path(), sockets.join(STATE_SOCKET_FILE));
        assert_eq!(
            ctx.daemon_socket_path("/tmp/qol-launcher.sock"),
            sockets.join("qol-launcher.sock")
        );
    }

    #[test]
    fn shadow_generation_namespaces_socket_files() {
        let ctx = GenerationContext::shadow("blue-1");
        let sockets = crate::paths::runtime_dir().join("sockets");

        assert_eq!(ctx.ui_bind_port(), 0);
        assert_eq!(
            ctx.state_socket_path(),
            sockets.join("qol-tray-state.blue-1.sock")
        );
        assert_eq!(
            ctx.daemon_socket_path("/tmp/qol-launcher.sock"),
            sockets.join("qol-launcher.blue-1.sock")
        );
    }

    const STAGED_RUNTIME_DIGEST: &str =
        "dba93b5e5332db99863e2afe0dff8cf71976ae918091ccf9a7592429cea97807";

    fn bounded_socket_path_limit() -> Option<usize> {
        let limit = qol_runtime::local_ipc::MAX_SOCKET_PATH_BYTES;
        (limit < usize::MAX).then_some(limit)
    }

    #[test]
    fn content_digest_generation_id_stays_within_the_platform_socket_limit() {
        let Some(limit) = bounded_socket_path_limit() else {
            return;
        };
        let sockets = Path::new("/home/a-developer/.local/share/qol-tray/runtime/sockets");

        let socket = namespaced_socket(&sockets.join(STATE_SOCKET_FILE), STAGED_RUNTIME_DIGEST);

        assert!(
            socket.as_os_str().len() <= limit,
            "staged runtime digests are 64 characters, so an unbounded namespace \
             produces an unbindable address: {} bytes for {}",
            socket.as_os_str().len(),
            socket.display()
        );
        assert!(socket
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("qol-tray-state.dba93b5e")));
    }

    #[test]
    fn namespaced_socket_of_a_near_limit_directory_still_binds() {
        let Some(limit) = bounded_socket_path_limit() else {
            return;
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let headroom = STATE_SOCKET_FILE.len() + STAGED_RUNTIME_DIGEST.len();
        let padding = limit.saturating_sub(tmp.path().as_os_str().len() + headroom);
        let sockets = tmp.path().join("d".repeat(padding));
        std::fs::create_dir_all(&sockets).unwrap();

        let socket = namespaced_socket(&sockets.join(STATE_SOCKET_FILE), STAGED_RUNTIME_DIGEST);

        let listener = qol_runtime::local_ipc::bind_listener(&socket)
            .unwrap_or_else(|error| panic!("{} did not bind: {error}", socket.display()));
        drop(listener);
    }

    #[test]
    fn generation_id_is_shell_safe() {
        let ctx = GenerationContext::shadow("a/b c:$d");

        assert_eq!(
            ctx.daemon_socket_path("/tmp/qol.sock"),
            crate::paths::runtime_dir()
                .join("sockets")
                .join("qol.abcd.sock")
        );
    }

    #[test]
    fn promoted_shadow_uses_stable_addresses() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        reset_generation_state();
        std::env::set_var(ENV_DEV_GENERATION_MODE, DEV_GENERATION_MODE_SHADOW);
        std::env::set_var(ENV_DEV_GENERATION_ID, "blue-1");

        assert!(is_shadow());
        assert_eq!(
            state_socket_path(),
            crate::paths::runtime_dir()
                .join("sockets")
                .join("qol-tray-state.blue-1.sock")
        );

        promote_to_stable();

        assert!(!is_shadow());
        assert_eq!(
            state_socket_path(),
            crate::paths::runtime_dir()
                .join("sockets")
                .join(STATE_SOCKET_FILE)
        );
        assert_eq!(
            daemon_socket_path("/tmp/qol-launcher.sock"),
            crate::paths::runtime_dir()
                .join("sockets")
                .join("qol-launcher.sock")
        );
        reset_generation_state();
    }

    #[cfg(feature = "dev")]
    #[test]
    fn predecessor_daemons_from_pid_files_reads_valid_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("plugin-z.pid"), "222").unwrap();
        std::fs::write(tmp.path().join("plugin-a.pid"), "111\n").unwrap();
        std::fs::write(tmp.path().join("plugin-b.pid"), "not a pid").unwrap();
        std::fs::write(tmp.path().join("plugin-c.txt"), "333").unwrap();

        let daemons = predecessor_daemons_from_pid_files(tmp.path());

        assert_eq!(
            daemons,
            vec![
                PredecessorDaemon {
                    plugin_id: "plugin-a".to_string(),
                    pid: 111
                },
                PredecessorDaemon {
                    plugin_id: "plugin-z".to_string(),
                    pid: 222
                },
            ]
        );
    }

    #[cfg(feature = "dev")]
    #[test]
    fn format_predecessor_daemons_matches_logs() {
        let daemons = vec![
            PredecessorDaemon {
                plugin_id: "plugin-a".to_string(),
                pid: 111,
            },
            PredecessorDaemon {
                plugin_id: "plugin-z".to_string(),
                pid: 222,
            },
        ];

        assert_eq!(
            format_predecessor_daemons(&daemons),
            "plugin-a=111, plugin-z=222"
        );
    }
}
