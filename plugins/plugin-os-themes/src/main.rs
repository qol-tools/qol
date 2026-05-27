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
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
