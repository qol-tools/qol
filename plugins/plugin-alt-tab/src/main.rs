#[cfg(target_os = "linux")]
mod x11_utils;

#[cfg(not(target_os = "linux"))]
mod x11_utils {
    #[derive(Debug, Clone)]
    pub struct WindowInfo {
        pub id: u32,
        pub title: String,
        pub preview_path: Option<String>,
    }

    pub fn get_open_windows() -> Vec<WindowInfo> {
        Vec::new()
    }
}

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    list::{List, ListDelegate, ListItem, ListState},
    IndexPath,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use x11_utils::WindowInfo;

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-alt-tab/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum PreviewMode {
    #[default]
    BelowList,
    PreviewOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct DisplayConfig {
    preview_mode: PreviewMode,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            preview_mode: PreviewMode::BelowList,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct AltTabConfig {
    display: DisplayConfig,
}

fn load_alt_tab_config() -> AltTabConfig {
    for path in config_paths() {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };

        match serde_json::from_str::<AltTabConfig>(&contents) {
            Ok(config) => {
                println!("Loaded config from {}: {:?}", path.display(), config);
                return config;
            }
            Err(e) => {
                println!("Failed to parse config at {}: {}", path.display(), e);
            }
        }
    }

    println!("Using default config");
    AltTabConfig::default()
}

fn config_paths() -> Vec<PathBuf> {
    const INSTALL_RELATIVE_CONFIG_PATHS: [&str; 2] = [
        "plugins/plugin-alt-tab/config.json",
        "plugins/alt-tab/config.json",
    ];
    const LEGACY_RELATIVE_CONFIG_PATHS: [&str; 2] = [
        "qol-tray/plugins/plugin-alt-tab/config.json",
        "qol-tray/plugins/alt-tab/config.json",
    ];

    let mut paths = Vec::new();

    for root in install_config_roots() {
        for relative in INSTALL_RELATIVE_CONFIG_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    let mut roots = Vec::new();
    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg_config_home.trim().is_empty() {
            roots.push(PathBuf::from(xdg_config_home));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            roots.push(PathBuf::from(home).join(".config"));
        }
    }

    for root in roots {
        for relative in LEGACY_RELATIVE_CONFIG_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    paths
}

fn install_config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(base_data_dir) = base_data_dir() else {
        return roots;
    };

    if let Some(install_id) = install_id_from_env() {
        let candidate = base_data_dir.join("installs").join(install_id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }

    if let Some(install_id) = install_id_from_active_file(&base_data_dir) {
        let candidate = base_data_dir.join("installs").join(install_id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }

    roots
}

fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

fn install_id_from_env() -> Option<String> {
    let value = std::env::var("QOL_TRAY_INSTALL_ID").ok()?;
    let trimmed = value.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn install_id_from_active_file(base_data_dir: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn preview_tile(preview_path: &Option<String>, width: f32, height: f32) -> AnyElement {
    if let Some(path) = preview_path {
        img(path.clone())
            .w(px(width))
            .h(px(height))
            .into_any_element()
    } else {
        div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(0x22262f))
            .border_1()
            .border_color(rgb(0x3a4252))
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(rgb(0x7b8495))
            .child("No Preview")
            .into_any_element()
    }
}

struct WindowDelegate {
    windows: Vec<WindowInfo>,
    selected_index: Option<IndexPath>,
    preview_mode: PreviewMode,
}

impl WindowDelegate {
    fn new(windows: Vec<WindowInfo>, preview_mode: PreviewMode) -> Self {
        Self {
            windows,
            selected_index: Some(IndexPath::new(0)),
            preview_mode,
        }
    }
}

impl ListDelegate for WindowDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.windows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let is_selected = self.selected_index == Some(ix);
        let win = &self.windows[ix.row];

        let row = match self.preview_mode {
            PreviewMode::BelowList => div()
                .flex()
                .w_full()
                .items_center()
                .h(px(64.0))
                .px_4()
                .gap_3()
                .when(is_selected, |style: Div| style.bg(rgb(0x2d3342)))
                .child(preview_tile(&win.preview_path, 86.0, 48.0))
                .child(
                    div()
                        .flex_1()
                        .text_color(rgb(0xf2f5fb))
                        .text_sm()
                        .text_ellipsis()
                        .child(win.title.clone()),
                ),
            PreviewMode::PreviewOnly => div()
                .flex()
                .justify_center()
                .items_center()
                .w_full()
                .h(px(240.0))
                .p_4()
                .when(is_selected, |style: Div| style.bg(rgb(0x2d3342)))
                .child(preview_tile(&win.preview_path, 384.0, 216.0)),
        };

        Some(ListItem::new(("window", ix.row)).child(row))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        if let Some(ix) = self.selected_index {
            println!("Selected window ID: {}", self.windows[ix.row].id);
        }
    }
}

struct AltTabApp {
    list_state: Entity<ListState<WindowDelegate>>,
    focus_handle: FocusHandle,
    preview_mode: PreviewMode,
}

impl AltTabApp {
    fn new(window: &mut Window, cx: &mut Context<Self>, config: AltTabConfig) -> Self {
        let preview_mode = config.display.preview_mode;
        let windows = x11_utils::get_open_windows();
        let delegate = WindowDelegate::new(windows, preview_mode.clone());

        let list_state = cx.new(|cx| ListState::new(delegate, window, cx));
        let focus_handle = cx.focus_handle();

        Self {
            list_state,
            focus_handle,
            preview_mode,
        }
    }

    fn mode_label(&self) -> &'static str {
        match self.preview_mode {
            PreviewMode::BelowList => "Previews Below List",
            PreviewMode::PreviewOnly => "Preview-First Layout",
        }
    }
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AltTabApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .bg(rgb(0x131722))
            .w_full()
            .h_full()
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(0x303748))
                    .child(format!("Alt Tab • {}", self.mode_label())),
            )
            .child(List::new(&self.list_state).flex_1().w_full())
    }
}

fn run_app(config: AltTabConfig) {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        let config = config.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(600.0), px(440.0)),
                    cx,
                ))),
                titlebar: None,
                focus: true,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| AltTabApp::new(window, cx, config.clone()));
                window.focus(&view.focus_handle(cx));
                view
            },
        )
        .unwrap();
    });
}

fn maybe_open_settings(args: &[String]) -> bool {
    if !args.iter().any(|arg| arg == "--settings") {
        return false;
    }

    if let Err(error) = open::that(SETTINGS_URL) {
        eprintln!("Failed to open settings page: {}", error);
    }

    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if maybe_open_settings(&args) {
        return;
    }

    let config = load_alt_tab_config();
    run_app(config);
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str =
            std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest =
            toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");

        println!("Plugin contract passed successfully!");
    }
}
