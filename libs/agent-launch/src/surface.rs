use std::path::Path;

use anyhow::{anyhow, Result};
use qol_terminal_sessions::SpawnSurface;

pub const SURFACE_TAB: &str = "tab";
pub const SURFACE_OS_WINDOW: &str = "os-window";

pub fn parse_surface(token: &str) -> Option<SpawnSurface> {
    match token {
        SURFACE_TAB => Some(SpawnSurface::Tab),
        SURFACE_OS_WINDOW => Some(SpawnSurface::OsWindow),
        _ => None,
    }
}

pub fn surface_token(surface: SpawnSurface) -> &'static str {
    match surface {
        SpawnSurface::Tab => SURFACE_TAB,
        SpawnSurface::OsWindow => SURFACE_OS_WINDOW,
    }
}

pub fn config_surface() -> Result<Option<SpawnSurface>> {
    let Some(path) = crate::config::primary_config_path() else {
        return Ok(None);
    };
    config_surface_at(&path)
}

pub fn config_surface_at(path: &Path) -> Result<Option<SpawnSurface>> {
    let Some(config) = crate::config::read(path, "spawn surface config")? else {
        return Ok(None);
    };
    let Some(token) = config.spawn_surface else {
        return Ok(None);
    };
    let surface = parse_surface(&token).ok_or_else(|| {
        anyhow!(
            "invalid spawn_surface `{token}` in {}; expected `{SURFACE_TAB}` or `{SURFACE_OS_WINDOW}`",
            path.display()
        )
    })?;
    Ok(Some(surface))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn config_surface_parses_tokens_and_rejects_unknown_values() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions.toml");
        assert_eq!(config_surface_at(&path).unwrap(), None);

        fs::write(&path, "spawn_surface = \"tab\"\n").unwrap();
        assert_eq!(config_surface_at(&path).unwrap(), Some(SpawnSurface::Tab));

        fs::write(&path, "spawn_surface = \"os-window\"\n").unwrap();
        assert_eq!(
            config_surface_at(&path).unwrap(),
            Some(SpawnSurface::OsWindow)
        );

        fs::write(&path, "spawn_surface = \"floating\"\n").unwrap();
        let error = config_surface_at(&path).unwrap_err().to_string();
        assert!(
            error.contains("invalid spawn_surface `floating`"),
            "{error}"
        );
        assert!(error.contains("tab"), "{error}");

        fs::write(&path, "spawn_surface =\n").unwrap();
        assert!(config_surface_at(&path).is_err());
    }
}
