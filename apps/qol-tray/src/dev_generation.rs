use std::path::{Path, PathBuf};

use qol_conventions::{
    DEFAULT_PORT, DEV_GENERATION_MODE_SHADOW, ENV_DEV_GENERATION_ID, ENV_DEV_GENERATION_MODE,
    ENV_DEV_READY_FILE, ENV_DEV_ROLLING_RESTART, ENV_DEV_UI_PORT, STATE_SOCKET_PATH,
};

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
        let mode = match std::env::var(ENV_DEV_GENERATION_MODE) {
            Ok(value) if value == DEV_GENERATION_MODE_SHADOW => GenerationMode::Shadow,
            _ => GenerationMode::Stable,
        };
        let id = std::env::var(ENV_DEV_GENERATION_ID)
            .ok()
            .and_then(|value| generation_id(value.as_str()));
        Self { id, mode }
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
        if !self.is_shadow() {
            return PathBuf::from(STATE_SOCKET_PATH);
        }
        namespaced_socket(Path::new(STATE_SOCKET_PATH), self.id())
    }

    pub fn daemon_socket_path(&self, socket: &str) -> PathBuf {
        let path = PathBuf::from(socket);
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

pub fn state_socket_path() -> PathBuf {
    current().state_socket_path()
}

pub fn daemon_socket_path(socket: &str) -> PathBuf {
    current().daemon_socket_path(socket)
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
    parent.join(format!("{stem}.{id}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_generation_keeps_public_addresses() {
        let ctx = GenerationContext::stable();

        assert_eq!(ctx.ui_bind_port(), DEFAULT_PORT);
        assert_eq!(ctx.state_socket_path(), PathBuf::from(STATE_SOCKET_PATH));
        assert_eq!(
            ctx.daemon_socket_path("/tmp/qol-launcher.sock"),
            PathBuf::from("/tmp/qol-launcher.sock")
        );
    }

    #[test]
    fn shadow_generation_namespaces_socket_files() {
        let ctx = GenerationContext::shadow("blue-1");

        assert_eq!(ctx.ui_bind_port(), 0);
        assert_eq!(
            ctx.state_socket_path(),
            PathBuf::from("/tmp/qol-tray-state.blue-1.sock")
        );
        assert_eq!(
            ctx.daemon_socket_path("/tmp/qol-launcher.sock"),
            PathBuf::from("/tmp/qol-launcher.blue-1.sock")
        );
    }

    #[test]
    fn generation_id_is_shell_safe() {
        let ctx = GenerationContext::shadow("a/b c:$d");

        assert_eq!(
            ctx.daemon_socket_path("/tmp/qol.sock"),
            PathBuf::from("/tmp/qol.abcd.sock")
        );
    }
}
