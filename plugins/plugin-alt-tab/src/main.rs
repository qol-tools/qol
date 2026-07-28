mod actions;
mod app;
mod capture;
mod config;
mod discovery;
mod picker;
mod preview_plane;
mod rendering;
mod runtime;

fn main() -> std::process::ExitCode {
    runtime::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
