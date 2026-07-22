mod app;
mod command;
mod config;
mod discovery;
mod input;
mod network;
mod qol;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "kill") {
        if app::daemon::send_kill() {
            eprintln!("[pointz] kill sent");
        } else {
            eprintln!("[pointz] no daemon running");
        }
        return;
    }

    let action = if args.iter().any(|a| a == "--action") {
        args.iter().skip_while(|a| *a != "--action").nth(1).cloned()
    } else {
        args.first().cloned()
    };

    if let Some(action) = action {
        if app::daemon::send_action(&action) {
            eprintln!("[pointz] action '{}' sent", action);
        } else {
            eprintln!("[pointz] no daemon running, handling locally");
            if action == "settings" {
                qol::open_settings();
            }
        }
        return;
    }

    app::run();
}

#[cfg(test)]
mod tests {
    qol_plugin_api::assert_plugin_toml_valid!();
}
