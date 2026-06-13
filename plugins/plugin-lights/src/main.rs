use std::process::ExitCode;

fn main() -> ExitCode {
    match plugin_lights::runtime::entrypoint(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
