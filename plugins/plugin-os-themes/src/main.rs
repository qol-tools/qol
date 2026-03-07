mod cursor;
mod theme;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        None | Some("run") => cursor::run(),
        Some("settings") => cursor::open_settings(),
        Some(action) => {
            eprintln!("Unknown action: {action}");
            return ExitCode::from(1);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str =
            std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest =
            toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");
    }
}
