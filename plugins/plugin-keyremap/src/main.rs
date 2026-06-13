#[cfg(target_os = "macos")]
mod config;
#[cfg(target_os = "macos")]
mod daemon;
#[cfg(target_os = "macos")]
mod keycode;
mod platform;
#[cfg(target_os = "macos")]
mod remap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(platform::run(args));
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }

    #[test]
    fn manifest_declares_macos_only_platform() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let platforms = manifest
            .plugin
            .platforms
            .as_ref()
            .expect("plugin.toml must declare platforms = [\"macos\"]");
        assert_eq!(
            platforms,
            &vec!["macos".to_string()],
            "keyremap requires CGEventTap; manifest must restrict to macOS so the host never offers it elsewhere"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_run_exits_with_error_code() {
        let code = super::platform::run(Vec::new());
        assert_eq!(
            code, 1,
            "on non-macOS hosts keyremap must refuse to start with a non-zero exit code"
        );
    }
}
