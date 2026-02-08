use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("run") => {
            println!("Hello from My Plugin");
            ExitCode::SUCCESS
        }
        Some(action) => {
            eprintln!("Unknown action: {action}");
            ExitCode::from(1)
        }
    }
}
