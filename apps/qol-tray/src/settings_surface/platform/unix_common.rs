use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context};
use gpui::{App, AppContext, Application};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::settings_panel::SettingsActivation;
use qol_gpui::settings_panel::{SettingsPanel, SettingsRuntime, SettingsWindowHost};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use super::super::HostBoot;

#[derive(Debug)]
enum Command {
    Open(String),
    Kill,
}

fn config() -> DaemonConfig {
    DaemonConfig {
        socket: SocketSource::Path(
            crate::paths::runtime_dir()
                .join("sockets")
                .join(qol_conventions::SETTINGS_SURFACE_SOCKET_FILE),
        ),
        support_replace_existing: false,
    }
}

pub(in crate::settings_surface) fn request(plugin_id: &str) -> anyhow::Result<bool> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        bail!("invalid plugin ID for native settings: {plugin_id}");
    }
    if forward_open(plugin_id) {
        qol_runtime::probe!(
            "SURFACE_ACTIVATION",
            "plugin={plugin_id} phase=dispatch outcome=forwarded"
        );
        return Ok(true);
    }
    spawn_host(Some(plugin_id)).context("failed to launch the native settings surface host")?;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=dispatch outcome=spawned"
    );
    Ok(true)
}

const PREWARM_WATCH_WINDOW: Duration = Duration::from_secs(30);
const PREWARM_WATCH_INTERVAL: Duration = Duration::from_secs(1);

pub(in crate::settings_surface) fn prewarm() {
    if crate::dev_generation::is_shadow() {
        return;
    }
    std::thread::spawn(|| {
        stop();
        let deadline = std::time::Instant::now() + PREWARM_WATCH_WINDOW;
        loop {
            let token_ready =
                crate::features::plugin_store::server::security::current_token().is_some();
            if token_ready && !core_daemon::send_ping(&config()) {
                let outcome = if spawn_host(None).is_ok() {
                    "spawned"
                } else {
                    "spawn_failed"
                };
                #[cfg(not(debug_assertions))]
                let _ = &outcome;
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin=none phase=prewarm outcome={outcome}"
                );
            }
            if std::time::Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(PREWARM_WATCH_INTERVAL);
        }
    });
}

fn spawn_host(plugin_id: Option<&str>) -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("failed to locate qol-tray executable")?;
    let mut command = std::process::Command::new(executable);
    command.arg(super::super::HOST_ARGUMENT);
    if let Some(plugin_id) = plugin_id {
        command.arg(plugin_id);
    }
    command
        .env(
            qol_conventions::ENV_PLUGIN_ID,
            qol_conventions::SETTINGS_SURFACE_APP_ID,
        )
        .env(
            qol_conventions::ENV_STATE_SOCKET,
            crate::dev_generation::state_socket_path(),
        )
        .env_remove(qol_conventions::ENV_DAEMON_SOCKET);
    if let Some(token) = crate::features::plugin_store::server::security::current_token() {
        command.env(qol_conventions::ENV_HTTP_TOKEN, token);
    }
    crate::features::theme::apply_accent_env(&mut command);
    crate::features::theme::apply_theme_name_env(&mut command);
    qol_process::spawn_detached(&mut command)?;
    Ok(())
}

pub(in crate::settings_surface) fn stop() {
    let config = config();
    let stopped = core_daemon::send_kill(&config);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while stopped && core_daemon::send_ping(&config) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let outcome = if !stopped {
        "not_running"
    } else if core_daemon::send_ping(&config) {
        "timeout"
    } else {
        "stopped"
    };
    #[cfg(not(debug_assertions))]
    let _ = &outcome;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin=none phase=stop outcome={outcome}"
    );
}

pub(in crate::settings_surface) fn run(boot: HostBoot) -> anyhow::Result<()> {
    match boot {
        HostBoot::Warm => run_host(None),
        HostBoot::Open(plugin_id) => {
            let result = run_host(Some(plugin_id.clone()));
            if let Err(error) = &result {
                #[cfg(not(debug_assertions))]
                let _ = error;
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin={plugin_id} phase=host outcome=failed error={error}"
                );
                open_browser_fallback(&plugin_id, "host_failed");
            }
            result
        }
    }
}

fn run_host(initial: Option<String>) -> anyhow::Result<()> {
    if let Some(plugin_id) = &initial {
        if !crate::paths::is_safe_path_component(plugin_id) {
            bail!("invalid plugin ID for native settings: {plugin_id}");
        }
        if forward_open(plugin_id) {
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=courier outcome=forwarded"
            );
            return Ok(());
        }
    }

    let config = config();
    let socket_path =
        core_daemon::socket_path(&config).context("native settings socket path is unavailable")?;
    let socket_parent = socket_path
        .parent()
        .context("native settings socket path has no parent")?;
    qol_fs::create_private_dir(socket_parent)
        .context("failed to create the native settings socket directory")?;
    let (command_tx, command_rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&config, command_tx, parse_request) {
        return match &initial {
            Some(plugin_id) => {
                if forward_open(plugin_id) {
                    qol_runtime::probe!(
                        "SURFACE_ACTIVATION",
                        "plugin={plugin_id} phase=courier outcome=forwarded_after_race"
                    );
                    return Ok(());
                }
                bail!("native settings surface singleton could not start or accept a request")
            }
            None => {
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin=none phase=prewarm outcome=already_running"
                );
                Ok(())
            }
        };
    }

    let boot_label = initial.as_deref().unwrap_or("none");
    #[cfg(not(debug_assertions))]
    let _ = &boot_label;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={boot_label} phase=host outcome=started"
    );
    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        qol_gpui::keepalive::open_keepalive(cx, Some(qol_conventions::SETTINGS_SURFACE_APP_ID));
        let tracker = MonitorTracker::start(cx);
        let host = Rc::new(RefCell::new(SettingsWindowHost::default()));
        if let Some(plugin_id) = initial {
            let activation_host = host.clone();
            let activation_tracker = tracker.clone();
            cx.spawn(async move |cx| {
                activate(activation_host, activation_tracker, plugin_id, cx).await;
            })
            .detach();
        }
        spawn_command_loop(command_rx, host, tracker, cx);
    });
    core_daemon::cleanup(&config);
    Ok(())
}

fn spawn_command_loop(
    command_rx: mpsc::Receiver<Command>,
    host: Rc<RefCell<SettingsWindowHost>>,
    tracker: MonitorTracker,
    cx: &mut App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, command_rx, move |cx, command| {
        let host = host.clone();
        let tracker = tracker.clone();
        async move {
            match command {
                Command::Open(plugin_id) => {
                    qol_runtime::probe!(
                        "SURFACE_ACTIVATION",
                        "plugin={plugin_id} phase=command outcome=received"
                    );
                    activate(host, tracker, plugin_id, &cx).await;
                    LoopFlow::Continue
                }
                Command::Kill => LoopFlow::Stop,
            }
        }
    });
}

async fn activate(
    host: Rc<RefCell<SettingsWindowHost>>,
    tracker: MonitorTracker,
    plugin_id: String,
    cx: &gpui::AsyncApp,
) {
    let focused_host = host.clone();
    let focused_tracker = tracker.clone();
    let focused_plugin_id = plugin_id.clone();
    match cx.update(move |cx| {
        focused_host
            .borrow_mut()
            .present_active(&focused_plugin_id, &focused_tracker, cx)
    }) {
        Ok(true) => {
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=activate outcome=focused visible_windows=1"
            );
            return;
        }
        Ok(false) => {}
        Err(error) => {
            #[cfg(not(debug_assertions))]
            let _ = &error;
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=activate outcome=app_update_failed error={error}"
            );
            open_browser_fallback(&plugin_id, "activation_failed");
            return;
        }
    }

    let load_plugin_id = plugin_id.clone();
    let loaded = cx
        .background_spawn(async move { load_panel(&load_plugin_id) })
        .await;
    let activation = match loaded {
        Ok((panel, runtime)) => {
            match qol_gpui::settings_panel::prepare_from_async(panel, runtime, cx).await {
                Ok(prepared) => {
                    let activation_host = host.clone();
                    cx.update(move |cx| {
                        activation_host
                            .borrow_mut()
                            .activate_prepared(prepared, &tracker, cx)
                    })
                    .and_then(|result| result)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    match activation {
        Ok(activation) => {
            #[cfg(not(debug_assertions))]
            let _ = &activation;
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=activate outcome={} visible_windows=1",
                activation_name(activation)
            )
        }
        Err(error) => {
            #[cfg(not(debug_assertions))]
            let _ = &error;
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=activate outcome=failed error={error}"
            );
            open_browser_fallback(&plugin_id, "activation_failed");
        }
    }
}

fn load_panel(plugin_id: &str) -> anyhow::Result<(SettingsPanel, SettingsRuntime)> {
    let plugin_root = crate::plugins::paths::resolve_plugin_root(plugin_id)?;
    let manifest = crate::plugins::manifest::PluginManifest::read_from_dir(&plugin_root)?;
    if !manifest.capabilities.gpui {
        bail!("plugin does not opt into GPUI settings");
    }
    let Some((_, runtime_contract)) =
        crate::plugins::config::load_combined_contracts_from_root(&plugin_root)?
    else {
        bail!("plugin has no settings contract");
    };
    let contract =
        std::fs::read_to_string(crate::plugins::paths::config_contract_path(&plugin_root))
            .context("failed to read the plugin settings contract")?;
    let panel = SettingsPanel {
        plugin_id: plugin_id.to_string(),
        contract,
        heading: format!("{} Settings", manifest.plugin.name),
    };
    let mut runtime = SettingsRuntime::tray(plugin_id);
    if let Some(runtime_contract) = runtime_contract.as_ref() {
        for (name, query) in &runtime_contract.queries {
            runtime = runtime.poll_query_every(name, Duration::from_millis(query.poll_interval_ms));
        }
    }
    Ok((panel, runtime))
}

fn forward_open(plugin_id: &str) -> bool {
    let config = config();
    matches!(
        core_daemon::send_request(
            &config,
            "open",
            serde_json::json!({ "plugin_id": plugin_id }),
            Duration::from_millis(500),
        ),
        Ok(DaemonResponse::Handled { .. })
    )
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        "open" => request
            .input
            .get("plugin_id")
            .and_then(serde_json::Value::as_str)
            .filter(|plugin_id| crate::paths::is_safe_path_component(plugin_id))
            .map(|plugin_id| ReadResult::Command(Command::Open(plugin_id.to_string())))
            .unwrap_or_else(|| ReadResult::Error("open requires a valid plugin_id".into())),
        _ => ReadResult::Fallback,
    }
}

fn activation_name(activation: SettingsActivation) -> &'static str {
    match activation {
        SettingsActivation::Focused => "focused",
        SettingsActivation::Opened => "opened",
        SettingsActivation::Replaced => "replaced",
    }
}

fn open_browser_fallback(plugin_id: &str, reason: &str) {
    let result = crate::paths::open_url(&qol_conventions::settings_url(plugin_id));
    #[cfg(not(debug_assertions))]
    let _ = (&reason, &result);
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=fallback reason={reason} outcome={}",
        if result.is_ok() { "opened" } else { "failed" }
    );
}

#[cfg(test)]
mod tests {
    use qol_plugin_daemon::daemon::ReadResult;
    use qol_runtime::protocol::DaemonRequest;

    use super::{config, parse_request, Command};

    #[test]
    fn settings_socket_lives_in_the_private_runtime_tree() {
        let root = tempfile::tempdir().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        let path = qol_plugin_daemon::daemon::socket_path(&config()).unwrap();

        assert_eq!(
            path,
            crate::paths::runtime_dir()
                .join("sockets")
                .join(qol_conventions::SETTINGS_SURFACE_SOCKET_FILE)
        );
    }

    #[test]
    fn activation_protocol_rejects_untrusted_plugin_ids() {
        let cases = [
            ("plugin-a", true),
            ("../plugin-a", false),
            ("plugin/a", false),
            ("", false),
        ];
        for (plugin_id, valid) in cases {
            let request = DaemonRequest {
                action: "open".into(),
                input: serde_json::json!({ "plugin_id": plugin_id }),
            };
            let parsed = parse_request(&request);
            assert_eq!(
                matches!(parsed, ReadResult::Command(Command::Open(_))),
                valid,
                "plugin_id={plugin_id:?}"
            );
        }
    }
}
