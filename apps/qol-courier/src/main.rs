qol_conventions::declare_build_identity!(Courier);

fn main() {
    qol_runtime::probe!("HOST_ENTRY", "phase=start");
    register_build_identity();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.as_slice() {
        [command, target, action] if command == "exec" => {
            qol_plugin_api::host_exec::run_exec(target, action)
        }
        [command] if matches!(command.as_str(), "help" | "--help" | "-h") => {
            print_usage();
            0
        }
        _ => {
            eprintln!("Usage: qol-courier exec <plugin-id> <action-id>");
            eprintln!("       qol-courier exec shortcut <id>");
            2
        }
    };
    std::process::exit(code);
}

fn print_usage() {
    println!("USAGE:");
    println!("    qol-courier exec <plugin-id> <action-id>    Trigger a plugin action via the running tray");
    println!("    qol-courier exec shortcut <id>              Run a shortcut via the running tray");
    println!("    qol-courier help, --help, -h                Print this message and exit");
}
