pub mod activation;
pub mod focus;

#[cfg(unix)]
pub mod daemon;

// Plugin daemons MUST exit when the qol-tray host dies, or they orphan to PID 1
// (force-quit, SIGTERM, crash). start_listener arms qol_runtime's host-death
// watchdog to enforce that, but the watchdog is implemented for Unix only
// (Unix-domain-socket lifeline + getppid). Adding a non-Unix target to a daemon
// plugin's `platforms` without porting the watchdog first would ship a silent
// daemon leak, so fail the build loudly instead.
#[cfg(not(unix))]
compile_error!(
    "qol-plugin-daemon: the host-death watchdog that stops plugin daemons from \
     leaking when qol-tray exits is implemented for Unix only. Building daemon \
     support for this target would orphan daemons on host exit. Port \
     qol_runtime::spawn_host_death_watchdog to this platform before adding it to \
     a daemon plugin's `platforms`."
);
