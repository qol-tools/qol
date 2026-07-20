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
    qol_plugin_api::assert_plugin_toml_valid!();
}
