mod app;
mod config;
mod cursor;
mod daemon;
mod theme;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let action = env::args().nth(1);
    app::run(action.as_deref())
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
