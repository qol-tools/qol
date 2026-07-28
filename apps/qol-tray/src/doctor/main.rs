fn main() -> anyhow::Result<()> {
    if qol_process::process_tree_guardian_requested() {
        qol_process::run_process_tree_guardian_entry()?;
        return Ok(());
    }
    let code = qol_tray::doctor::run_cli_from_env()?;
    std::process::exit(code);
}
