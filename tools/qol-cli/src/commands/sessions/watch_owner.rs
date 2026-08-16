use qol_terminal_sessions::{SessionInventory, TerminalSessionService};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use super::bridge::PendingBridgeStore;

pub(super) struct ClientWatcher {
    dir: PathBuf,
    owner_key: String,
    program: PathBuf,
    child: Arc<Mutex<Option<Child>>>,
}

const WATCH_PROGRAM: &str = "qol";

#[cfg(test)]
const ALWAYS_PRESENT_TEST_PROGRAM: &str = "/bin/echo";

fn sessions_dir() -> PathBuf {
    qol_config::data_subdir("sessions").unwrap_or_else(|| ".".into())
}

fn sanitize(token: &str) -> String {
    token.replace([':', '.'], "_")
}
impl ClientWatcher {
    pub(super) fn for_terminal(terminals: &TerminalSessionService) -> Self {
        let owner_key = terminals
            .discover()
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|facts| facts.binding().ok())
            .filter(|binding| terminals.is_current(binding).unwrap_or(false))
            .map(|binding| sanitize(&binding.token()))
            .next()
            .unwrap_or_else(|| format!("unknown-{}", std::process::id()));
        Self {
            dir: sessions_dir(),
            owner_key,
            program: PathBuf::from(WATCH_PROGRAM),
            child: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub(super) fn with_dir(dir: PathBuf, owner_key: String) -> Self {
        Self {
            dir,
            owner_key,
            program: PathBuf::from(ALWAYS_PRESENT_TEST_PROGRAM),
            child: Arc::new(Mutex::new(None)),
        }
    }

    fn state_file(&self) -> PathBuf {
        self.dir
            .join(format!("watch-owner-{}.json", self.owner_key))
    }

    pub(super) fn wake_debug_log(&self, line: &str) {
        let log_path = self.dir.join(format!("wake-debug-{}.log", self.owner_key));
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            let _ = writeln!(file, "{} {}", chrono::Utc::now().to_rfc3339(), line);
        }
    }

    pub(super) fn read_tokens(&self) -> Vec<String> {
        match fs::read_to_string(self.state_file()) {
            Ok(encoded) => serde_json::from_str::<Vec<String>>(&encoded).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub(super) fn record_token(&self, token: &str, pending: &PendingBridgeStore) {
        let mut tokens = self.read_tokens();
        if !tokens.iter().any(|candidate| candidate == token) {
            tokens.push(token.to_owned());
        }
        tokens.retain(|candidate| {
            candidate
                .parse::<qol_terminal_sessions::SessionBinding>()
                .ok()
                .and_then(|binding| pending.pending_round(&binding).ok().flatten())
                .is_some_and(|round| !round.completed)
        });
        if let Err(error) = fs::create_dir_all(&self.dir) {
            self.wake_debug_log(&format!("state dir failed: {error}"));
            return;
        }
        match fs::write(
            self.state_file(),
            serde_json::to_string(&tokens).unwrap_or_default(),
        ) {
            Ok(()) => self.wake_debug_log(&format!("token recorded tokens={}", tokens.len())),
            Err(error) => self.wake_debug_log(&format!("state write failed: {error}")),
        }
        self.restart();
    }

    pub(super) fn start(&self, pending: &PendingBridgeStore) {
        let mut tokens = self.read_tokens();
        tokens.retain(|candidate| {
            candidate
                .parse::<qol_terminal_sessions::SessionBinding>()
                .ok()
                .and_then(|binding| pending.pending_round(&binding).ok().flatten())
                .is_some_and(|round| !round.completed)
        });
        if tokens.is_empty() {
            return;
        }
        self.spawn_watcher(&tokens);
    }

    pub(super) fn restart(&self) {
        let tokens = self.read_tokens();
        self.stop();
        self.spawn_watcher(&tokens);
    }

    pub(super) fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn spawn_watcher(&self, tokens: &[String]) {
        if tokens.is_empty() {
            return;
        }
        let mut child = match Command::new(&self.program)
            .args(["sessions", "watch"])
            .args(tokens)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                self.wake_debug_log(&format!("watch spawn failed: {error}"));
                return;
            }
        };
        self.wake_debug_log(&format!(
            "watch spawn pid={} tokens={}",
            child.id(),
            tokens.len()
        ));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let slot = Arc::clone(&self.child);
        let debug = self.dir.join(format!("wake-debug-{}.log", self.owner_key));
        *slot.lock().unwrap() = Some(child);
        let owner = self.owner_key.clone();
        std::thread::spawn(move || {
            if let Some(stdout) = stdout {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                        continue;
                    };
                    let event_name = event
                        .get("event")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("?");
                    let session = event
                        .get("session")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("?");
                    let delivered = event
                        .get("delivered")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let error = event
                        .get("wake_error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let mut line =
                        format!("event={event_name} session={session} delivered={delivered}");
                    if !error.is_empty() {
                        line.push_str(&format!(" error={error}"));
                    }
                    if let Ok(mut file) = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&debug)
                    {
                        let _ = writeln!(file, "{} {line}", chrono::Utc::now().to_rfc3339());
                    }
                    let _ = owner.len();
                }
            }
            if let Some(mut stderr) = stderr {
                let mut text = String::new();
                let _ = std::io::Read::read_to_string(&mut stderr, &mut text);
                if !text.trim().is_empty() {
                    if let Ok(mut file) = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&debug)
                    {
                        let _ = writeln!(
                            file,
                            "{} watch stderr: {}",
                            chrono::Utc::now().to_rfc3339(),
                            text.trim()
                        );
                    }
                }
            }
            let status = slot
                .lock()
                .unwrap()
                .as_mut()
                .map(|child| child.wait())
                .transpose();
            if let Ok(Some(status)) = status {
                if let Ok(mut file) = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&debug)
                {
                    let _ = writeln!(
                        file,
                        "{} watch child exit code={}",
                        chrono::Utc::now().to_rfc3339(),
                        status
                            .code()
                            .map_or_else(|| "signal".to_owned(), |code| code.to_string())
                    );
                }
            }
            *slot.lock().unwrap() = None;
        });
    }
}

impl Drop for ClientWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::SessionBinding;

    fn store(root: &tempfile::TempDir) -> PendingBridgeStore {
        PendingBridgeStore::with_dir(root.path().join("pending-bridge"))
    }

    #[test]
    fn record_keeps_only_open_rounds_and_prunes_collected_tokens() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let watcher = ClientWatcher::with_dir(root.path().to_path_buf(), "owner-test".to_owned());
        let open: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let done: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(&open, "QOL_BRIDGE_DONE_open", "v1:fake:9:900", false)
            .unwrap();
        pending
            .start(&done, "QOL_BRIDGE_DONE_done", "v1:fake:9:900", false)
            .unwrap();
        pending
            .observe(&done, "QOL_BRIDGE_DONE_done", true)
            .unwrap();

        watcher.record_token(&open.token(), &pending);
        watcher.record_token(&done.token(), &pending);
        let tokens = watcher.read_tokens();
        assert_eq!(
            tokens,
            vec![open.token()],
            "collected rounds must be pruned"
        );
    }

    #[test]
    fn record_spawns_the_watcher_and_stop_clears_the_child_slot() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let watcher = ClientWatcher::with_dir(root.path().to_path_buf(), "owner-spawn".to_owned());
        let open: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&open, "QOL_BRIDGE_DONE_open", "v1:fake:9:900", false)
            .unwrap();
        watcher.record_token(&open.token(), &pending);
        let log = root.path().join("wake-debug-owner-spawn.log");
        assert!(
            std::fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("watch spawn pid="),
            "recording a token must spawn the watcher"
        );
        watcher.stop();
        assert!(
            watcher.child.lock().unwrap().is_none(),
            "stop must clear the watcher child"
        );
    }

    #[test]
    fn start_respawns_for_lingering_tokens_after_a_client_restart() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let watcher =
            ClientWatcher::with_dir(root.path().to_path_buf(), "owner-restart".to_owned());
        let open: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&open, "QOL_BRIDGE_DONE_open", "v1:fake:9:900", false)
            .unwrap();
        watcher.record_token(&open.token(), &pending);
        watcher.stop();
        let first_log = std::fs::read_to_string(root.path().join("wake-debug-owner-restart.log"))
            .unwrap_or_default();

        let restarted =
            ClientWatcher::with_dir(root.path().to_path_buf(), "owner-restart".to_owned());
        restarted.start(&pending);
        let log = std::fs::read_to_string(root.path().join("wake-debug-owner-restart.log"))
            .unwrap_or_default();
        assert!(
            log != first_log && log.contains("watch spawn pid="),
            "a restarted client must re-watch its lingering tokens"
        );
    }

    #[test]
    fn stop_never_touches_foreign_owner_files() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let watcher = ClientWatcher::with_dir(root.path().to_path_buf(), "owner-a".to_owned());
        let open: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&open, "QOL_BRIDGE_DONE_open", "v1:fake:9:900", false)
            .unwrap();
        watcher.record_token(&open.token(), &pending);
        watcher.stop();

        let foreign = ClientWatcher::with_dir(root.path().to_path_buf(), "owner-b".to_owned());
        assert!(foreign.read_tokens().is_empty());
        foreign.start(&pending);
        assert!(!root.path().join("wake-debug-owner-b.log").exists());
    }
}
