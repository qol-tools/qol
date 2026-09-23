mod app;

qol_conventions::declare_build_identity!(Host);

fn main() -> anyhow::Result<()> {
    qol_runtime::probe!("HOST_ENTRY", "phase=start");
    register_build_identity();
    scrub_inherited_daemon_handoff_env();
    app::run()
}

/// qol-tray hands pre-bound listener fds to the one daemon it spawns them for.
/// Any such variable already in qol-tray's own environment was leaked by an
/// ancestor (a terminal launched from a plugin daemon, for example) and names
/// an fd that does not exist here; every child would inherit it and adopt a
/// bogus listener instead of binding its own socket. Drop them before anything
/// can be spawned.
fn scrub_inherited_daemon_handoff_env() {
    for key in qol_conventions::daemon_handoff_env_keys() {
        std::env::remove_var(key);
    }
}
