mod cli;
mod platform;

fn main() -> std::process::ExitCode {
    cli::exit_code(std::env::args().skip(1))
}
