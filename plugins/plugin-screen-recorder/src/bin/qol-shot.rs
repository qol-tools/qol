use std::process::ExitCode;

fn main() -> ExitCode {
    screen_recorder::cli::exit_code_for_binary("qol-shot", std::env::args().skip(1))
}
