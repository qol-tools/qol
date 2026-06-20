use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("open") => ExitCode::SUCCESS,
        Some("scan") | Some("remove") => ExitCode::SUCCESS,
        Some(other) => {
            eprintln!("removeapp: unknown subcommand {other:?}");
            ExitCode::from(2)
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
