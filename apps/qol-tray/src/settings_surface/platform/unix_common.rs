use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context};
use gpui::{App, AppContext, Application};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::settings_panel::SettingsActivation;
use qol_gpui::settings_panel::{PanelSource, SettingsPanel, SettingsRuntime, SettingsWindowHost};
use qol_gpui::toast::{Toast, ToastHost, ToastLayout, ToastTone};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse, NotificationLayout};

use super::super::HostBoot;
use super::native_tools::NativeToolsHost;

#[derive(Debug)]
enum Command {
    Open(String),
    Toast {
        title: String,
        body: String,
        level: String,
        action: Option<(String, String)>,
        artifact: Option<String>,
        layout: Option<NotificationLayout>,
    },
    ThemeChanged,
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
        let mut handover_broken = stop();
        let deadline = std::time::Instant::now() + PREWARM_WATCH_WINDOW;
        loop {
            let token_ready =
                crate::features::plugin_store::server::security::current_token().is_some();
            if token_ready
                && spawn_replacement_after_handover(
                    core_daemon::send_ping(&config()),
                    handover_broken,
                )
            {
                let outcome = if spawn_host(None).is_ok() {
                    if handover_broken {
                        "challenged"
                    } else {
                        "spawned"
                    }
                } else {
                    "spawn_failed"
                };
                #[cfg(not(debug_assertions))]
                let _ = &outcome;
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin=none phase=prewarm outcome={outcome}"
                );
                handover_broken = false;
            }
            if std::time::Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(PREWARM_WATCH_INTERVAL);
        }
    });
}

fn spawn_replacement_after_handover(ping_alive: bool, handover_broken: bool) -> bool {
    !ping_alive || handover_broken
}

fn spawn_host(plugin_id: Option<&str>) -> anyhow::Result<()> {
    let spawn_started = std::time::Instant::now();
    let executable = std::env::current_exe().context("failed to locate qol-tray executable")?;
    let resolved_plugin = plugin_id.unwrap_or("none");
    #[cfg(not(debug_assertions))]
    let _ = &resolved_plugin;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={resolved_plugin} phase=resolve outcome=resolved elapsed_ms={}",
        spawn_started.elapsed().as_millis()
    );
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
        .env_remove(qol_conventions::ENV_DAEMON_SOCKET)
        .env_remove(qol_conventions::ENV_DAEMON_LISTENER_FD)
        .env_remove(qol_conventions::ENV_DAEMON_REPLACE_EXISTING)
        .env_remove("XMODIFIERS");
    if let Some(token) = crate::features::plugin_store::server::security::current_token() {
        command.env(qol_conventions::ENV_HTTP_TOKEN, token);
    }
    crate::features::theme::apply_accent_env(&mut command);
    crate::features::theme::apply_theme_name_env(&mut command);
    let spawned = qol_process::spawn_detached(&mut command);
    let outcome = if spawned.is_ok() {
        "spawned"
    } else {
        "spawn_failed"
    };
    #[cfg(not(debug_assertions))]
    let _ = &outcome;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={resolved_plugin} phase=spawn outcome={outcome} elapsed_ms={}",
        spawn_started.elapsed().as_millis()
    );
    spawned?;
    Ok(())
}

#[cfg(target_os = "linux")]
const USER_HZ: u64 = 100;

#[cfg(target_os = "linux")]
fn process_elapsed_ms() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let start_ticks = stat
        .rsplit_once(')')?
        .1
        .split_ascii_whitespace()
        .nth(19)?
        .parse::<u64>()
        .ok()?;
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let uptime_ms = (uptime
        .split_ascii_whitespace()
        .next()?
        .parse::<f64>()
        .ok()?
        * 1000.0) as u64;
    uptime_ms.checked_sub(start_ticks * 1000 / USER_HZ)
}

#[cfg(target_os = "macos")]
fn process_elapsed_ms() -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            i32::try_from(size).ok()?,
        )
    };
    if read != i32::try_from(size).ok()? {
        return None;
    }
    let started = std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(
        info.pbi_start_tvsec,
        u32::try_from(info.pbi_start_tvusec * 1000).ok()?,
    ))?;
    std::time::SystemTime::now()
        .duration_since(started)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}

pub(in crate::settings_surface) fn apply_theme(native: &str, accent: &str) -> bool {
    core_daemon::send_action(&config(), &format!("theme {native} {accent}"), true)
}

pub(in crate::settings_surface) fn stop() -> bool {
    let config = config();
    let stopped = core_daemon::send_kill(&config);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while stopped && core_daemon::send_ping(&config) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let handover_broken = stopped && core_daemon::send_ping(&config);
    let outcome = if !stopped {
        "not_running"
    } else if handover_broken {
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
    handover_broken
}

pub(in crate::settings_surface) fn run(boot: HostBoot) -> anyhow::Result<()> {
    match boot {
        HostBoot::Warm => {
            let result = run_host(None);
            if let Err(error) = &result {
                #[cfg(not(debug_assertions))]
                let _ = error;
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin=none phase=host outcome=failed error={error}"
                );
            }
            result
        }
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

const HOST_LISTENER_RETRIES: usize = 5;
const HOST_LISTENER_RETRY_INTERVAL: Duration = Duration::from_millis(200);

fn start_listener_with_retry(config: &DaemonConfig, command_tx: &mpsc::Sender<Command>) -> bool {
    for attempt in 1..=HOST_LISTENER_RETRIES {
        if core_daemon::start_request_listener(config, command_tx.clone(), parse_request) {
            return true;
        }
        if attempt < HOST_LISTENER_RETRIES {
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin=none phase=bind outcome=lost attempt={attempt}"
            );
            std::thread::sleep(HOST_LISTENER_RETRY_INTERVAL);
        }
    }
    false
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
    if !start_listener_with_retry(&config, &command_tx) {
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
        "plugin={boot_label} phase=host outcome=started elapsed_ms={}",
        process_elapsed_ms().map_or_else(|| "unavailable".to_owned(), |ms| ms.to_string())
    );
    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        qol_gpui::keepalive::open_keepalive(cx, Some(qol_conventions::SETTINGS_SURFACE_APP_ID));
        let tracker = MonitorTracker::start(cx);
        let toast_host = ToastHost::new(tracker.clone());
        let host = Rc::new(RefCell::new(SettingsWindowHost::default()));
        let tools = Rc::new(RefCell::new(NativeToolsHost::default()));
        if let Some(plugin_id) = initial {
            let activation_host = host.clone();
            let activation_tools = tools.clone();
            let activation_tracker = tracker.clone();
            cx.spawn(async move |cx| {
                activate(
                    activation_host,
                    activation_tools,
                    activation_tracker,
                    plugin_id,
                    cx,
                )
                .await;
            })
            .detach();
        }
        spawn_command_loop(command_rx, host, tools, tracker, toast_host, cx);
    });
    core_daemon::cleanup(&config);
    Ok(())
}

fn spawn_command_loop(
    command_rx: mpsc::Receiver<Command>,
    host: Rc<RefCell<SettingsWindowHost>>,
    tools: Rc<RefCell<NativeToolsHost>>,
    tracker: MonitorTracker,
    toast_host: ToastHost,
    cx: &mut App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, command_rx, move |cx, command| {
        let host = host.clone();
        let tools = tools.clone();
        let tracker = tracker.clone();
        let toast_host = toast_host.clone();
        async move {
            match command {
                Command::Open(plugin_id) => {
                    qol_runtime::probe!(
                        "SURFACE_ACTIVATION",
                        "plugin={plugin_id} phase=command outcome=received"
                    );
                    activate(host, tools, tracker, plugin_id, &cx).await;
                    LoopFlow::Continue
                }
                Command::Toast {
                    title,
                    body,
                    level,
                    action,
                    artifact,
                    layout,
                } => {
                    show_toast_in_host(
                        toast_host, title, body, level, action, artifact, layout, &cx,
                    );
                    LoopFlow::Continue
                }
                Command::ThemeChanged => {
                    let _ = cx.update(|cx| cx.refresh_windows());
                    LoopFlow::Continue
                }
                Command::Kill => LoopFlow::Stop,
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn show_toast_in_host(
    toast_host: ToastHost,
    title: String,
    body: String,
    level: String,
    action: Option<(String, String)>,
    artifact: Option<String>,
    layout: Option<NotificationLayout>,
    cx: &gpui::AsyncApp,
) {
    let result = cx.update(move |cx| {
        let (anchor, width, height, style) = layout
            .as_ref()
            .map(|layout| {
                (
                    layout.anchor.as_deref(),
                    layout.width,
                    layout.height,
                    layout.style.as_deref(),
                )
            })
            .unwrap_or((None, None, None, None));
        let mut toast = Toast::new(
            title,
            body,
            ToastLayout::for_push(anchor, width, height, style),
        )
        .tone(toast_tone(&level));
        if let Some(path) = artifact {
            toast = toast.artifact(path);
        } else if let Some((_, payload)) = action {
            toast = toast.on_activate(move |_cx| crate::paths::open_url(&payload));
        }
        if let Err(error) = toast_host.show(toast, cx) {
            log::warn!("[toast] render failed: {error:#}");
        }
    });
    if let Err(error) = result {
        log::warn!("[toast] app update failed: {error:#}");
    }
}

fn toast_tone(level: &str) -> ToastTone {
    match level {
        "info" => ToastTone::Info,
        "warn" => ToastTone::Warning,
        "error" => ToastTone::Danger,
        _ => ToastTone::Neutral,
    }
}

async fn activate(
    host: Rc<RefCell<SettingsWindowHost>>,
    tools: Rc<RefCell<NativeToolsHost>>,
    tracker: MonitorTracker,
    plugin_id: String,
    cx: &gpui::AsyncApp,
) {
    if let Some(tool) = super::super::CoreTool::from_wire_id(&plugin_id) {
        let activation = cx.update(move |cx| {
            host.borrow_mut().hide_active(cx);
            tools.borrow_mut().activate(tool, &tracker, cx)
        });
        match activation {
            Ok(Ok(())) => qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "tool={} phase=activate outcome=opened visible_windows=1",
                tool.wire_id()
            ),
            Ok(Err(error)) => {
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "tool={} phase=activate outcome=failed error={error}",
                    tool.wire_id()
                );
                open_browser_fallback(&plugin_id, "activation_failed");
            }
            Err(error) => {
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "tool={} phase=activate outcome=app_update_failed error={error}",
                    tool.wire_id()
                );
                open_browser_fallback(&plugin_id, "activation_failed");
            }
        }
        return;
    }
    let dismiss_tools = tools.clone();
    let _ = cx.update(move |cx| dismiss_tools.borrow_mut().dismiss(cx));
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

    let loaded = cx
        .background_spawn(async move { load_unified_panel() })
        .await;
    let activation = match loaded {
        Ok((mut panel, runtimes)) => {
            if !panel
                .sources
                .iter()
                .any(|source| source.plugin_id == plugin_id)
            {
                Err(match load_panel(&plugin_id) {
                    Ok(_) => {
                        anyhow::anyhow!("plugin is not available in the unified settings panel")
                    }
                    Err(error) => error,
                })
            } else {
                panel.focus = Some(plugin_id.clone());
                match qol_gpui::settings_panel::prepare_many_from_async(panel, runtimes, cx).await {
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
    if plugin_id == qol_conventions::CORE_PANEL_ID {
        return Ok(load_core_panel());
    }
    load_plugin_panel(plugin_id)
}

fn load_core_panel() -> (SettingsPanel, SettingsRuntime) {
    let mut panel = SettingsPanel::single(
        qol_conventions::CORE_PANEL_ID.to_string(),
        include_str!("core-config.toml").to_string(),
        qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME.to_string(),
    );
    panel.sources[0].heading = "General".to_string();
    let runtime = SettingsRuntime::tray_core();
    (panel, runtime)
}

fn load_plugin_panel(plugin_id: &str) -> anyhow::Result<(SettingsPanel, SettingsRuntime)> {
    let (source, runtime) = load_plugin_source(plugin_id)?;
    let mut panel = SettingsPanel::single(
        plugin_id.to_string(),
        source.contract,
        format!("{} Settings", source.heading),
    );
    panel.sources[0].heading = source.heading;
    Ok((panel, runtime))
}

struct PluginSettingsSource {
    contract: String,
    heading: String,
}

fn load_plugin_source(plugin_id: &str) -> anyhow::Result<(PluginSettingsSource, SettingsRuntime)> {
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
    let mut runtime = SettingsRuntime::tray(plugin_id);
    if let Some(runtime_contract) = runtime_contract.as_ref() {
        for (name, query) in &runtime_contract.queries {
            runtime = runtime.poll_query_every(name, Duration::from_millis(query.poll_interval_ms));
        }
    }
    Ok((
        PluginSettingsSource {
            contract,
            heading: manifest.plugin.name.clone(),
        },
        runtime,
    ))
}

fn load_unified_panel() -> anyhow::Result<(SettingsPanel, Vec<SettingsRuntime>)> {
    let (core_panel, core_runtime) = load_core_panel();
    let mut sources = vec![core_panel
        .sources
        .into_iter()
        .next()
        .expect("core panel has one source")];
    let mut runtimes = vec![core_runtime];
    let mut eligible = Vec::new();
    for resolved in resolved_plugins()? {
        let manifest = match crate::plugins::manifest::PluginManifest::read_from_dir(&resolved.path)
        {
            Ok(manifest) => manifest,
            Err(error) => {
                log::warn!("unified settings skips {}: {error:#}", resolved.id);
                continue;
            }
        };
        if !manifest.capabilities.gpui || !crate::plugins::paths::has_config(&resolved.path) {
            continue;
        }
        eligible.push((manifest.plugin.name.clone(), resolved.id.to_string()));
    }
    eligible.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, plugin_id) in eligible {
        match load_plugin_source(&plugin_id) {
            Ok((source, runtime)) => {
                sources.push(PanelSource {
                    plugin_id,
                    contract: source.contract,
                    heading: source.heading,
                });
                runtimes.push(runtime);
            }
            Err(error) => {
                log::warn!("unified settings skips {plugin_id}: {error:#}");
            }
        }
    }
    Ok((
        SettingsPanel {
            sources,
            heading: qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME.to_string(),
            focus: None,
        },
        runtimes,
    ))
}

fn resolved_plugins() -> anyhow::Result<Vec<crate::plugins::resolver::ResolvedPlugin>> {
    let plugins_dir = crate::plugins::PluginLoader::ensure_plugin_dir()?;
    let config_dir = crate::paths::shared_config_dir()?;
    let registry = crate::plugins::registry::ensure_registry_initialized(&config_dir, &plugins_dir)
        .map_err(|error| anyhow::anyhow!("plugin registry is unavailable: {error}"))?;
    Ok(crate::plugins::resolver::resolve_effective_registry(&registry, &config_dir).plugins)
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

pub(in crate::settings_surface) fn show_toast(
    title: &str,
    body: &str,
    level: &str,
    action: Option<(&str, &str)>,
    artifact: Option<&str>,
    layout: Option<NotificationLayout>,
) -> anyhow::Result<bool> {
    let config = config();
    let action =
        action.map(|(label, payload)| serde_json::json!({ "label": label, "payload": payload }));
    Ok(matches!(
        core_daemon::send_request(
            &config,
            "toast",
            serde_json::json!({
                "title": title,
                "body": body,
                "level": level,
                "action": action,
                "artifact": artifact,
                "layout": layout,
            }),
            Duration::from_millis(500),
        ),
        Ok(DaemonResponse::Handled { .. })
    ))
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        action if action == "theme" || action.starts_with("theme ") => {
            ReadResult::Command(Command::ThemeChanged)
        }
        "open" => request
            .input
            .get("plugin_id")
            .and_then(serde_json::Value::as_str)
            .filter(|plugin_id| crate::paths::is_safe_path_component(plugin_id))
            .map(|plugin_id| ReadResult::Command(Command::Open(plugin_id.to_string())))
            .unwrap_or_else(|| ReadResult::Error("open requires a valid plugin_id".into())),
        "toast" => {
            let title = request
                .input
                .get("title")
                .and_then(serde_json::Value::as_str);
            let body = request
                .input
                .get("body")
                .and_then(serde_json::Value::as_str);
            let level = request
                .input
                .get("level")
                .and_then(serde_json::Value::as_str);
            let action = request
                .input
                .get("action")
                .and_then(serde_json::Value::as_object)
                .and_then(validated_action);
            let artifact = request
                .input
                .get("artifact")
                .and_then(serde_json::Value::as_str)
                .filter(|artifact| crate::paths::is_existing_absolute_path(artifact))
                .map(str::to_string);
            let layout = request
                .input
                .get("layout")
                .and_then(|value| serde_json::from_value::<NotificationLayout>(value.clone()).ok());
            match (title, body, level) {
                (Some(title), Some(body), Some(level)) => ReadResult::Command(Command::Toast {
                    title: title.to_string(),
                    body: body.to_string(),
                    level: level.to_string(),
                    action,
                    artifact,
                    layout,
                }),
                _ => ReadResult::Error("toast requires title, body and level".into()),
            }
        }
        _ => ReadResult::Fallback,
    }
}

fn validated_action(
    action: &serde_json::Map<String, serde_json::Value>,
) -> Option<(String, String)> {
    let label = action.get("label").and_then(serde_json::Value::as_str)?;
    let payload = action.get("payload").and_then(serde_json::Value::as_str)?;
    if label.trim().is_empty() {
        return None;
    }
    if !crate::paths::is_existing_absolute_path(payload) {
        return None;
    }
    Some((label.to_string(), payload.to_string()))
}

fn activation_name(activation: SettingsActivation) -> &'static str {
    match activation {
        SettingsActivation::Focused => "focused",
        SettingsActivation::Opened => "opened",
        SettingsActivation::Replaced => "replaced",
    }
}

fn open_browser_fallback(plugin_id: &str, reason: &str) {
    let url = super::super::CoreTool::from_wire_id(plugin_id)
        .map(|tool| {
            qol_conventions::local_hash_url(tool.fallback_route(), qol_conventions::DEFAULT_PORT)
        })
        .unwrap_or_else(|| qol_conventions::settings_url(plugin_id));
    let result = crate::paths::open_url(&url);
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

    use super::{config, parse_request, spawn_replacement_after_handover, Command};

    #[test]
    fn core_panel_carries_every_wired_field_in_a_valid_contract() {
        let (panel, _runtime) = super::load_core_panel();
        assert_eq!(panel.primary_plugin_id(), qol_conventions::CORE_PANEL_ID);
        assert_eq!(
            panel.heading,
            qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME
        );
        let spec = qol_config::contract::parse_spec_str(&panel.sources[0].contract)
            .expect("core contract must parse");
        for (name, kind) in [
            ("native_theme", qol_config::contract::FieldKind::Select),
            ("theme", qol_config::contract::FieldKind::Select),
            ("accent", qol_config::contract::FieldKind::Select),
            ("profile", qol_config::contract::FieldKind::Select),
            ("residency", qol_config::contract::FieldKind::Boolean),
        ] {
            let field = spec.field(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(field.kind, kind, "wrong kind for {name}");
        }
        assert_eq!(
            spec.field("profile").and_then(|field| field.query.clone()),
            Some("profiles".to_string())
        );
        qol_config::normalized::resolve_config(&spec, &serde_json::json!({}))
            .expect("core contract must resolve with no stored values");
    }

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

    #[test]
    fn toast_protocol_accepts_complete_payloads_and_drops_invalid_actions() {
        let existing = tempfile::tempdir().unwrap();
        let existing_path = existing.path().join("target");
        std::fs::write(&existing_path, "x").unwrap();
        let payload = existing_path.to_str().unwrap().to_string();
        let request = DaemonRequest {
            action: "toast".into(),
            input: serde_json::json!({
                "title": "title",
                "body": "body",
                "level": "warn",
                "action": { "label": "open", "payload": payload },
            }),
        };
        match parse_request(&request) {
            ReadResult::Command(Command::Toast {
                title,
                body,
                level,
                action,
                artifact,
                layout,
            }) => {
                assert_eq!(title, "title");
                assert_eq!(body, "body");
                assert_eq!(level, "warn");
                assert_eq!(action, Some(("open".to_string(), payload)));
                assert_eq!(artifact, None);
                assert_eq!(layout, None);
            }
            _ => panic!("toast request did not parse as a command"),
        }

        let incomplete = DaemonRequest {
            action: "toast".into(),
            input: serde_json::json!({ "title": "title" }),
        };
        assert!(matches!(parse_request(&incomplete), ReadResult::Error(_)));

        let missing_payload = DaemonRequest {
            action: "toast".into(),
            input: serde_json::json!({
                "title": "title",
                "body": "body",
                "level": "info",
                "action": { "label": "open", "payload": "/does/not/exist" },
            }),
        };
        match parse_request(&missing_payload) {
            ReadResult::Command(Command::Toast { action, .. }) => assert_eq!(action, None),
            _ => panic!("toast request did not parse as a command"),
        }
    }

    #[test]
    fn toast_protocol_carries_a_semantic_layout_override() {
        let request = DaemonRequest {
            action: "toast".into(),
            input: serde_json::json!({
                "title": "title",
                "body": "body",
                "level": "info",
                "layout": {
                    "anchor": "bottom-right",
                    "width": 400.0,
                    "height": 84.0,
                    "style": "compact",
                },
            }),
        };
        match parse_request(&request) {
            ReadResult::Command(Command::Toast { layout, .. }) => {
                let layout = layout.expect("layout override parsed");
                assert_eq!(layout.anchor.as_deref(), Some("bottom-right"));
                assert_eq!(layout.width, Some(400.0));
                assert_eq!(layout.height, Some(84.0));
                assert_eq!(layout.style.as_deref(), Some("compact"));
            }
            _ => panic!("toast request did not parse as a command"),
        }

        let malformed = DaemonRequest {
            action: "toast".into(),
            input: serde_json::json!({
                "title": "title",
                "body": "body",
                "level": "info",
                "layout": "bottom-right",
            }),
        };
        match parse_request(&malformed) {
            ReadResult::Command(Command::Toast { layout, .. }) => assert_eq!(layout, None),
            _ => panic!("toast request did not parse as a command"),
        }
    }

    #[test]
    fn toast_protocol_resolves_the_artifact_only_for_an_existing_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("shot.png");
        std::fs::write(&file, b"x").unwrap();
        let existing = file.to_str().unwrap().to_string();
        let request_for = |artifact: &str| DaemonRequest {
            action: "toast".into(),
            input: serde_json::json!({
                "title": "title",
                "body": "body",
                "level": "info",
                "artifact": artifact,
            }),
        };

        match parse_request(&request_for(&existing)) {
            ReadResult::Command(Command::Toast { artifact, .. }) => {
                assert_eq!(artifact, Some(existing));
            }
            _ => panic!("toast request did not parse as a command"),
        }

        let missing = tmp.path().join("gone.png").to_string_lossy().into_owned();
        match parse_request(&request_for(&missing)) {
            ReadResult::Command(Command::Toast { artifact, .. }) => {
                assert_eq!(artifact, None);
            }
            _ => panic!("toast request did not parse as a command"),
        }
    }

    #[test]
    fn a_timed_out_handover_still_spawns_a_replacement() {
        assert!(spawn_replacement_after_handover(false, false));
        assert!(!spawn_replacement_after_handover(true, false));
        assert!(
            spawn_replacement_after_handover(true, true),
            "a host that accepted the kill but never died must still get a challenger"
        );
    }
}
