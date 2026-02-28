fn main() -> anyhow::Result<()> {
    let code = qol_tray::doctor::run_cli_from_env()?;
    std::process::exit(code);
}
