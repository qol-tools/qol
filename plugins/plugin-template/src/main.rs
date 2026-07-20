mod platform;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        None | Some("run") => {
            println!("Hello from My Plugin");
            Ok(())
        }
        Some("settings") => platform::open_settings(),
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
    qol_plugin_api::assert_plugin_toml_valid!();
}
