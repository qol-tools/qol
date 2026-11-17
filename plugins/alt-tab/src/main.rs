mod actions;
mod app;
mod capture;
mod config;
mod discovery;
mod picker;
mod platform;
mod preview_plane;
mod rendering;
mod runtime;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

fn main() -> std::process::ExitCode {
    qol_log::init_stderr();
    runtime::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
