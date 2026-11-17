#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;

qol_conventions::declare_build_identity!(Host);

fn main() -> anyhow::Result<()> {
    qol_runtime::probe!("HOST_ENTRY", "phase=start");
    register_build_identity();
    app::scrub_inherited_daemon_handoff_env();
    app::drop_launching_claude_session_env();
    app::run()
}
