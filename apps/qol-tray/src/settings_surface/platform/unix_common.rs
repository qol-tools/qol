use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context};
use gpui::{App, AppContext, Application};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;
#[cfg(debug_assertions)]
use qol_gpui::settings_panel::SettingsActivation;
use qol_gpui::settings_panel::{SettingsPanel, SettingsRuntime, SettingsWindowHost};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::Fallback {
        default_socket_name: qol_conventions::SETTINGS_SURFACE_SOCKET_NAME,
        use_tmpdir_env: false,
    },
    support_replace_existing: false,
};

#[derive(Debug)]
enum Command {
    Open(String),
    Kill,
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
    let executable = std::env::current_exe().context("failed to locate qol-tray executable")?;
    let mut command = std::process::Command::new(executable);
    command
        .arg(super::super::HOST_ARGUMENT)
        .arg(plugin_id)
        .env(qol_conventions::ENV_PLUGIN_ID, "qol-settings-surface")
        .env(
            qol_conventions::ENV_STATE_SOCKET,
            crate::dev_generation::state_socket_path(),
        )
        .env_remove(qol_conventions::ENV_DAEMON_SOCKET);
    crate::features::theme::apply_accent_env(&mut command);
    crate::features::theme::apply_theme_name_env(&mut command);
    qol_process::spawn_detached(&mut command)
        .context("failed to launch the native settings surface host")?;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=dispatch outcome=spawned"
    );
    Ok(true)
}

pub(in crate::settings_surface) fn stop() {
    let stopped = core_daemon::send_kill(&CONFIG);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while stopped && core_daemon::send_ping(&CONFIG) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let outcome = if !stopped {
        "not_running"
    } else if core_daemon::send_ping(&CONFIG) {
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

pub(in crate::settings_surface) fn run(plugin_id: String) -> anyhow::Result<()> {
    let result = run_host(plugin_id.clone());
    if result.is_err() {
        open_browser_fallback(&plugin_id, "host_failed");
    }
    result
}

fn run_host(plugin_id: String) -> anyhow::Result<()> {
    if !crate::paths::is_safe_path_component(&plugin_id) {
        bail!("invalid plugin ID for native settings: {plugin_id}");
    }
    if forward_open(&plugin_id) {
        qol_runtime::probe!(
            "SURFACE_ACTIVATION",
            "plugin={plugin_id} phase=courier outcome=forwarded"
        );
        return Ok(());
    }

    std::fs::create_dir_all(crate::paths::runtime_dir())
        .context("failed to create the qol runtime directory")?;
    let (command_tx, command_rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&CONFIG, command_tx, parse_request) {
        if forward_open(&plugin_id) {
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=courier outcome=forwarded_after_race"
            );
            return Ok(());
        }
        bail!("native settings surface singleton could not start or accept a request");
    }

    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=host outcome=started"
    );
    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        qol_gpui::keepalive::open_keepalive(cx, Some("qol-settings-surface"));
        let tracker = MonitorTracker::start(cx);
        let host = Rc::new(RefCell::new(SettingsWindowHost::default()));
        let activation_host = host.clone();
        let activation_tracker = tracker.clone();
        cx.spawn(async move |cx| {
            activate(activation_host, activation_tracker, plugin_id, cx).await;
        })
        .detach();
        spawn_command_loop(command_rx, host, tracker, cx);
    });
    core_daemon::cleanup(&CONFIG);
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
    let poll_interval = runtime_contract
        .as_ref()
        .and_then(|runtime| {
            runtime
                .queries
                .values()
                .map(|query| query.poll_interval_ms)
                .min()
        })
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(2));
    let panel = SettingsPanel {
        plugin_id: plugin_id.to_string(),
        contract,
        heading: format!("{} Settings", manifest.plugin.name),
    };
    let runtime = SettingsRuntime::tray(plugin_id).poll_every(poll_interval);
    Ok((panel, runtime))
}

fn forward_open(plugin_id: &str) -> bool {
    matches!(
        core_daemon::send_request(
            &CONFIG,
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

#[cfg(debug_assertions)]
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

    use super::{parse_request, Command};

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
