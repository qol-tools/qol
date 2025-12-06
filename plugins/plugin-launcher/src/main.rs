use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Read as IoRead, Write as IoWrite};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::Request;
use wry::WebViewBuilder;

const HALF_LIFE_DAYS: f64 = 7.0;
const FREQUENCY_BONUS: i32 = 500;
const SOCKET_PATH: &str = "/tmp/qol-launcher.sock";

#[derive(Serialize, Deserialize, Clone, Default)]
struct FrequencyEntry {
    count: u32,
    last_accessed: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct FrequencyData {
    entries: HashMap<String, FrequencyEntry>,
}

fn get_frequency_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qol-launcher-frequency.json")
}

fn load_frequency() -> FrequencyData {
    fs::read_to_string(get_frequency_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_frequency(data: &FrequencyData) {
    let path = get_frequency_path();
    let _ = fs::write(path, serde_json::to_string(data).unwrap_or_default());
}

fn record_access(path: &str) {
    let mut data = load_frequency();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let entry = data.entries.entry(path.to_string()).or_default();
    entry.count += 1;
    entry.last_accessed = now;
    prune_frequency(&mut data);
    save_frequency(&data);
}

fn prune_frequency(data: &mut FrequencyData) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    data.entries.retain(|_, e| effective_count(e, now) > 0.1);
    if data.entries.len() > 1000 {
        let mut items: Vec<_> = data.entries.drain().collect();
        items.sort_by(|a, b| effective_count(&b.1, now).partial_cmp(&effective_count(&a.1, now)).unwrap());
        items.truncate(1000);
        data.entries = items.into_iter().collect();
    }
}

fn effective_count(entry: &FrequencyEntry, now: u64) -> f64 {
    let days_elapsed = (now - entry.last_accessed) as f64 / 86400.0;
    let decay = (-days_elapsed * 0.693 / HALF_LIFE_DAYS).exp();
    entry.count as f64 * decay
}

#[derive(Debug)]
enum UserEvent {
    SearchComplete(Vec<SearchResult>),
    Show,
}

fn handle_socket_message(stream: &mut UnixStream, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
    let mut buf = [0u8; 16];
    let Ok(n) = stream.read(&mut buf) else { return };
    match &buf[..n] {
        b"show" => { let _ = proxy.send_event(UserEvent::Show); }
        b"kill" => std::process::exit(0),
        _ => {}
    }
}

fn send_socket_command(cmd: &[u8]) -> bool {
    if let Ok(mut stream) = UnixStream::connect(SOCKET_PATH) {
        let _ = stream.write_all(cmd);
        return true;
    }
    false
}

fn start_socket_listener(proxy: tao::event_loop::EventLoopProxy<UserEvent>) -> bool {
    if send_socket_command(b"show") {
        return false;
    }

    let _ = fs::remove_file(SOCKET_PATH);
    let Ok(listener) = UnixListener::bind(SOCKET_PATH) else { return false };
    let _ = listener.set_nonblocking(true);

    std::thread::spawn(move || loop {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        };
        handle_socket_message(&mut stream, &proxy);
    });

    true
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum IpcMessage {
    #[serde(rename = "search")]
    Search { query: String },
    #[serde(rename = "execute")]
    Execute { path: String, action: String },
    #[serde(rename = "close")]
    Close,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct SearchResult {
    path: String,
    name: String,
    is_dir: bool,
    icon: Option<String>,
}

#[derive(Default)]
struct AppState {
    should_exit: bool,
}

#[cfg(target_os = "linux")]
fn get_focused_window_position() -> (i32, i32) {
    let output = Command::new("xdotool")
        .args(["getactivewindow", "getwindowgeometry", "--shell"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let Ok(out) = output else { return (0, 0) };
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut x = 0i32;
    let mut y = 0i32;
    for line in stdout.lines() {
        if let Some(val) = line.strip_prefix("X=") {
            x = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("Y=") {
            y = val.parse().unwrap_or(0);
        }
    }
    (x, y)
}

fn calculate_centered_position(
    monitor_width: u32, monitor_height: u32,
    window_width: u32, window_height: u32,
    monitor_x: i32, monitor_y: i32,
) -> (i32, i32) {
    assert!(window_width > 0 && window_height > 0, "window size must be non-zero");
    let x = monitor_x + ((monitor_width - window_width) / 2) as i32;
    let y = monitor_y + ((monitor_height - window_height) / 3) as i32;
    (x, y)
}

fn get_plugin_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn get_backend_script() -> &'static str {
    if cfg!(target_os = "linux") {
        "backends/linux.sh"
    } else if cfg!(target_os = "macos") {
        "backends/macos.sh"
    } else {
        "backends/windows.ps1"
    }
}

fn parse_desktop_file(path: &str) -> (Option<String>, Option<String>) {
    let Ok(content) = fs::read_to_string(path) else { return (None, None) };
    let name = content.lines()
        .find(|l| l.starts_with("Name="))
        .map(|l| l.trim_start_matches("Name=").to_string());
    let icon = content.lines()
        .find(|l| l.starts_with("Icon="))
        .map(|l| l.trim_start_matches("Icon="))
        .and_then(resolve_icon_path)
        .and_then(|p| icon_to_data_url(&p));
    (name, icon)
}

fn icon_to_data_url(path: &str) -> Option<String> {
    let data = fs::read(path).ok()?;
    let mime = if path.ends_with(".svg") { "image/svg+xml" } else { "image/png" };
    use base64::{Engine, engine::general_purpose::STANDARD};
    Some(format!("data:{};base64,{}", mime, STANDARD.encode(&data)))
}

fn resolve_icon_path(icon_name: &str) -> Option<String> {
    if std::path::Path::new(icon_name).is_absolute() {
        return Some(icon_name.to_string());
    }

    let home = env::var("HOME").unwrap_or_default();
    let candidates = build_icon_candidates(icon_name, &home);
    candidates.into_iter().find(|p| std::path::Path::new(p).exists())
}

fn build_icon_candidates(icon_name: &str, home: &str) -> Vec<String> {
    let data_dirs = ["/usr/share/icons", "/usr/local/share/icons", "/usr/share/pixmaps"];
    let local_dir = format!("{}/.local/share/icons", home);
    let themes = ["hicolor", "Papirus", "Adwaita", "breeze"];
    let sizes = ["256x256", "128x128", "64x64", "48x48", "scalable"];
    let categories = ["apps", "applications"];
    let extensions = ["png", "svg"];

    let mut candidates = Vec::new();

    for base in data_dirs.iter().chain(std::iter::once(&local_dir.as_str())) {
        add_themed_candidates(&mut candidates, base, icon_name, &themes, &sizes, &categories, &extensions);
        add_pixmap_candidates(&mut candidates, base, icon_name, &extensions);
    }

    candidates
}

fn add_themed_candidates(
    candidates: &mut Vec<String>, base: &str, icon: &str,
    themes: &[&str], sizes: &[&str], categories: &[&str], extensions: &[&str],
) {
    let combos = themes.iter()
        .flat_map(|t| sizes.iter().map(move |s| (*t, *s)))
        .flat_map(|(t, s)| categories.iter().map(move |c| (t, s, *c)))
        .flat_map(|(t, s, c)| extensions.iter().map(move |e| (t, s, c, *e)));

    for (theme, size, cat, ext) in combos {
        candidates.push(format!("{}/{}/{}/{}/{}.{}", base, theme, size, cat, icon, ext));
    }
}

fn add_pixmap_candidates(candidates: &mut Vec<String>, base: &str, icon: &str, extensions: &[&str]) {
    for ext in extensions {
        candidates.push(format!("{}/{}.{}", base, icon, ext));
    }
}

fn parse_search_result(line: &str) -> SearchResult {
    let path = line.to_string();
    let is_dir = std::path::Path::new(&path).is_dir();
    let (name, icon) = if path.ends_with(".desktop") {
        let (n, i) = parse_desktop_file(&path);
        (n.unwrap_or_else(|| extract_filename(&path)), i)
    } else {
        (extract_filename(&path), None)
    };
    SearchResult { path, name, is_dir, icon }
}

fn extract_filename(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn search(query: &str, plugin_dir: &std::path::Path) -> Vec<SearchResult> {
    let script = plugin_dir.join(get_backend_script());

    #[cfg(target_os = "windows")]
    let output = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg(query)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    #[cfg(not(target_os = "windows"))]
    let output = Command::new("bash")
        .arg(&script)
        .arg(query)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let Ok(out) = output else { return vec![] };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let results: Vec<_> = stdout.lines().filter(|l| !l.is_empty()).map(parse_search_result).collect();
    let freq = load_frequency();
    sort_by_relevance(dedupe_results(results), query, &freq)
}

fn dedupe_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    results.into_iter().filter(|r| {
        let key = if r.path.ends_with(".desktop") {
            extract_app_id(&r.path)
        } else {
            r.name.to_lowercase()
        };
        seen.insert(key)
    }).collect()
}

fn score_result(r: &SearchResult, query: &str, freq: &FrequencyData) -> i32 {
    let name = r.name.to_lowercase();
    let q = query.to_lowercase();
    let path = &r.path;

    let match_penalty = if name == q { 0 }
        else if name.starts_with(&q) { 100 }
        else if name.contains(&q) { 200 }
        else { 300 };

    let type_penalty = if path.ends_with(".desktop") { 0 } else { 1000 };

    let path_penalty = score_path_quality(path);

    let length_penalty = r.name.len() as i32;

    let frequency_bonus = calc_frequency_bonus(path, freq);

    match_penalty + type_penalty + path_penalty + length_penalty - frequency_bonus
}

fn calc_frequency_bonus(path: &str, freq: &FrequencyData) -> i32 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    freq.entries.get(path)
        .map(|e| (effective_count(e, now) * FREQUENCY_BONUS as f64) as i32)
        .unwrap_or(0)
}

fn score_path_quality(path: &str) -> i32 {
    let mut penalty = 0i32;

    let standard_dirs = ["/usr/share/applications", "/usr/lib", ".local/share/applications"];
    let is_standard = standard_dirs.iter().any(|d| path.contains(d));
    if !is_standard {
        penalty += 50;
    }

    if path.contains("/autostart/") || path.contains("/xdg/") {
        penalty += 30;
    }

    let depth = path.matches('/').count();
    penalty += (depth as i32) * 2;

    let hidden_count = path.split('/')
        .filter(|p| p.starts_with('.') && *p != ".local")
        .count();
    penalty += (hidden_count as i32) * 500;

    penalty
}

fn sort_by_relevance(mut results: Vec<SearchResult>, query: &str, freq: &FrequencyData) -> Vec<SearchResult> {
    results.sort_by_key(|r| score_result(r, query, freq));
    results
}

fn extract_app_id(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let after_dots = stem.split('.').last().unwrap_or(&stem);
    after_dots.split('_').last().unwrap_or(after_dots).to_lowercase()
}

fn get_dir(path: &str) -> String {
    if std::path::Path::new(path).is_dir() { return path.to_string() }
    std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn action_open(path: &str) {
    #[cfg(target_os = "linux")]
    {
        if path.ends_with(".desktop") {
            if let Some(name) = std::path::Path::new(path).file_stem() {
                let _ = Command::new("gtk-launch").arg(name).spawn();
                return;
            }
        }
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("explorer").arg(path).spawn();
}

fn action_terminal(path: &str) {
    let dir = get_dir(path);
    #[cfg(target_os = "linux")]
    for term in ["gnome-terminal", "konsole", "xfce4-terminal", "xterm"] {
        let found = Command::new("which").arg(term).output().map(|o| o.status.success()).unwrap_or(false);
        if !found { continue }
        let _ = Command::new(term).arg("--working-directory").arg(&dir).spawn();
        break;
    }
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").args(["-a", "Terminal", &dir]).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd").args(["/c", "start", "cmd", "/k", &format!("cd /d {}", dir)]).spawn();
}

fn action_folder(path: &str) {
    let dir = get_dir(path);
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(&dir).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(&dir).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("explorer").arg(&dir).spawn();
}

fn action_copy(path: &str) {
    #[cfg(target_os = "linux")]
    let _ = Command::new("sh").args(["-c", &format!("echo -n '{}' | xclip -selection clipboard", path)]).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("sh").args(["-c", &format!("echo -n '{}' | pbcopy", path)]).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("powershell").args(["-Command", &format!("Set-Clipboard '{}'", path)]).spawn();
}

fn execute_action(path: &str, action: &str) {
    record_access(path);
    match action {
        "open" => action_open(path),
        "terminal" => action_terminal(path),
        "folder" => action_folder(path),
        "copy" => action_copy(path),
        _ => {}
    }
}

const HTML: &str = include_str!("../ui/index.html");
const CSS: &str = include_str!("../ui/style.css");
const JS: &str = include_str!("../ui/app.js");

fn build_html() -> String {
    HTML.replace(r#"<link rel="stylesheet" href="style.css">"#, &format!("<style>{}</style>", CSS))
        .replace(r#"<script src="app.js"></script>"#, &format!("<script>{}</script>", JS))
}

fn reset_ui(webview: &wry::WebView) {
    let _ = webview.evaluate_script("document.getElementById('search').value = ''; window.onSearchResults([]);");
}

fn create_window(event_loop: &tao::event_loop::EventLoop<UserEvent>) -> tao::window::Window {
    WindowBuilder::new()
        .with_title("Launcher")
        .with_decorations(false)
        .with_always_on_top(true)
        .with_visible(false)
        .with_inner_size(tao::dpi::LogicalSize::new(600.0, 400.0))
        .with_resizable(false)
        .build(event_loop)
        .unwrap()
}

fn create_ipc_handler(
    proxy: tao::event_loop::EventLoopProxy<UserEvent>,
    plugin_dir: PathBuf,
    state: Arc<Mutex<AppState>>,
) -> impl Fn(Request<String>) + 'static {
    move |request: Request<String>| {
        let Ok(msg) = serde_json::from_str::<IpcMessage>(request.body()) else { return };
        match msg {
            IpcMessage::Search { query } => {
                let proxy = proxy.clone();
                let dir = plugin_dir.clone();
                std::thread::spawn(move || {
                    let _ = proxy.send_event(UserEvent::SearchComplete(search(&query, &dir)));
                });
            }
            IpcMessage::Execute { path, action } => {
                execute_action(&path, &action);
                state.lock().unwrap().should_exit = true;
            }
            IpcMessage::Close => {
                state.lock().unwrap().should_exit = true;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn show_window_linux(window: &tao::window::Window) {
    use gtk::gdk::prelude::MonitorExt;
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    use tao::platform::unix::WindowExtUnix;

    let gtk_win = window.gtk_window();
    let display = gtk_win.display();
    let (focus_x, focus_y) = get_focused_window_position();

    let gdk_monitor = display.monitor_at_point(focus_x, focus_y)
        .or_else(|| display.monitor_at_point(0, 0))
        .unwrap();
    let geom = gdk_monitor.geometry();
    let scale = gdk_monitor.scale_factor() as u32;

    let (win_w, win_h) = (600 * scale, 400 * scale);
    let (x, y) = calculate_centered_position(
        geom.width() as u32, geom.height() as u32,
        win_w, win_h,
        geom.x(), geom.y(),
    );

    window.set_outer_position(tao::dpi::PhysicalPosition::new(x, y));
    gtk_win.set_keep_above(true);
    window.set_visible(true);

    let timestamp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() & 0xFFFFFFFF) as u32;
    gtk_win.present_with_time(timestamp);
    gtk_win.grab_focus();
}

#[cfg(not(target_os = "linux"))]
fn show_window_other(window: &tao::window::Window) {
    if let Some(monitor) = window.current_monitor() {
        let mon_pos = monitor.position();
        let mon_size = monitor.size();
        let win_size = window.outer_size();
        let (x, y) = calculate_centered_position(
            mon_size.width, mon_size.height,
            win_size.width, win_size.height,
            mon_pos.x, mon_pos.y,
        );
        window.set_outer_position(tao::dpi::PhysicalPosition::new(x, y));
    }
    window.set_visible(true);
    window.set_focus();
}

fn main() {
    if env::args().any(|a| a == "--kill") {
        send_socket_command(b"kill");
        return;
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    if !start_socket_listener(proxy.clone()) {
        return;
    }

    let state = Arc::new(Mutex::new(AppState::default()));
    let window = create_window(&event_loop);
    let handler = create_ipc_handler(proxy.clone(), get_plugin_dir(), state.clone());

    let html = build_html();

    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "android"))]
    let webview = WebViewBuilder::new()
        .with_html(&html)
        .with_ipc_handler(handler)
        .build(&window)
        .unwrap();

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "android")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        WebViewBuilder::new()
            .with_html(&html)
            .with_ipc_handler(handler)
            .build_gtk(window.default_vbox().unwrap())
            .unwrap()
    };

    if !env::args().any(|a| a == "--preload") {
        let _ = proxy.send_event(UserEvent::Show);
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if std::mem::take(&mut state.lock().unwrap().should_exit) {
            window.set_visible(false);
            reset_ui(&webview);
        }

        match event {
            Event::UserEvent(UserEvent::Show) => {
                #[cfg(target_os = "linux")]
                show_window_linux(&window);
                #[cfg(not(target_os = "linux"))]
                show_window_other(&window);
                let _ = webview.evaluate_script("document.getElementById('search').focus();");
            }
            Event::UserEvent(UserEvent::SearchComplete(ref results)) => {
                let Ok(json) = serde_json::to_string(results) else { return };
                let _ = webview.evaluate_script(&format!("window.onSearchResults({})", json));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } |
            Event::WindowEvent { event: WindowEvent::Focused(false), .. } => {
                window.set_visible(false);
                reset_ui(&webview);
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    mod parse_search_result {
        use super::*;

        #[test]
        fn extracts_filename_from_absolute_path() {
            // Arrange
            let line = "/home/user/documents/file.txt";

            // Act
            let result = parse_search_result(line);

            // Assert
            assert_eq!(result.name, "file.txt");
            assert_eq!(result.path, "/home/user/documents/file.txt");
        }

        #[test]
        fn handles_path_with_spaces() {
            // Arrange
            let line = "/home/user/my documents/my file.txt";

            // Act
            let result = parse_search_result(line);

            // Assert
            assert_eq!(result.name, "my file.txt");
        }

        #[test]
        fn handles_root_path() {
            // Arrange
            let line = "/";

            // Act
            let result = parse_search_result(line);

            // Assert
            assert_eq!(result.name, "/");
            assert_eq!(result.path, "/");
        }

        #[test]
        fn handles_hidden_file() {
            // Arrange
            let line = "/home/user/.bashrc";

            // Act
            let result = parse_search_result(line);

            // Assert
            assert_eq!(result.name, ".bashrc");
        }

        #[test]
        fn handles_deeply_nested_path() {
            // Arrange
            let line = "/a/b/c/d/e/f/g/h/i/file.rs";

            // Act
            let result = parse_search_result(line);

            // Assert
            assert_eq!(result.name, "file.rs");
            assert_eq!(result.path, "/a/b/c/d/e/f/g/h/i/file.rs");
        }

        #[test]
        fn handles_unicode_filename() {
            // Arrange
            let line = "/home/user/文档/файл.txt";

            // Act
            let result = parse_search_result(line);

            // Assert
            assert_eq!(result.name, "файл.txt");
        }
    }

    mod get_dir {
        use super::*;
        use std::fs;
        use tempfile::tempdir;

        #[test]
        fn returns_path_if_directory_exists() {
            // Arrange
            let dir = tempdir().unwrap();
            let path = dir.path().to_str().unwrap();

            // Act
            let result = get_dir(path);

            // Assert
            assert_eq!(result, path);
        }

        #[test]
        fn returns_parent_for_file_path() {
            // Arrange
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test").unwrap();

            // Act
            let result = get_dir(file_path.to_str().unwrap());

            // Assert
            assert_eq!(result, dir.path().to_str().unwrap());
        }

        #[test]
        fn returns_parent_for_nonexistent_file() {
            // Arrange
            let path = "/some/nonexistent/path/file.txt";

            // Act
            let result = get_dir(path);

            // Assert
            assert_eq!(result, "/some/nonexistent/path");
        }

        #[test]
        fn returns_dot_for_root_file() {
            // Arrange
            let path = "file.txt";

            // Act
            let result = get_dir(path);

            // Assert
            assert_eq!(result, ".");
        }
    }

    mod get_backend_script {
        use super::*;

        #[test]
        #[cfg(target_os = "linux")]
        fn returns_linux_script_on_linux() {
            // Act
            let result = get_backend_script();

            // Assert
            assert_eq!(result, "backends/linux.sh");
        }

        #[test]
        #[cfg(target_os = "macos")]
        fn returns_macos_script_on_macos() {
            // Act
            let result = get_backend_script();

            // Assert
            assert_eq!(result, "backends/macos.sh");
        }

        #[test]
        #[cfg(target_os = "windows")]
        fn returns_windows_script_on_windows() {
            // Act
            let result = get_backend_script();

            // Assert
            assert_eq!(result, "backends/windows.ps1");
        }
    }

    mod ipc_message_deserialization {
        use super::*;

        #[test]
        fn deserializes_search_message() {
            // Arrange
            let json = r#"{"type": "search", "query": "test query"}"#;

            // Act
            let msg: IpcMessage = serde_json::from_str(json).unwrap();

            // Assert
            match msg {
                IpcMessage::Search { query } => assert_eq!(query, "test query"),
                _ => panic!("Expected Search variant"),
            }
        }

        #[test]
        fn deserializes_execute_message() {
            // Arrange
            let json = r#"{"type": "execute", "path": "/home/user/file.txt", "action": "open"}"#;

            // Act
            let msg: IpcMessage = serde_json::from_str(json).unwrap();

            // Assert
            match msg {
                IpcMessage::Execute { path, action } => {
                    assert_eq!(path, "/home/user/file.txt");
                    assert_eq!(action, "open");
                }
                _ => panic!("Expected Execute variant"),
            }
        }

        #[test]
        fn deserializes_close_message() {
            // Arrange
            let json = r#"{"type": "close"}"#;

            // Act
            let msg: IpcMessage = serde_json::from_str(json).unwrap();

            // Assert
            assert!(matches!(msg, IpcMessage::Close));
        }

        #[test]
        fn rejects_invalid_type() {
            // Arrange
            let json = r#"{"type": "invalid"}"#;

            // Act
            let result = serde_json::from_str::<IpcMessage>(json);

            // Assert
            assert!(result.is_err());
        }

        #[test]
        fn rejects_missing_query_in_search() {
            // Arrange
            let json = r#"{"type": "search"}"#;

            // Act
            let result = serde_json::from_str::<IpcMessage>(json);

            // Assert
            assert!(result.is_err());
        }

        #[test]
        fn handles_empty_query() {
            // Arrange
            let json = r#"{"type": "search", "query": ""}"#;

            // Act
            let msg: IpcMessage = serde_json::from_str(json).unwrap();

            // Assert
            match msg {
                IpcMessage::Search { query } => assert_eq!(query, ""),
                _ => panic!("Expected Search variant"),
            }
        }

        #[test]
        fn handles_query_with_special_characters() {
            // Arrange
            let json = r#"{"type": "search", "query": "test \"quoted\" path/with\\backslash"}"#;

            // Act
            let msg: IpcMessage = serde_json::from_str(json).unwrap();

            // Assert
            match msg {
                IpcMessage::Search { query } => {
                    assert_eq!(query, r#"test "quoted" path/with\backslash"#)
                }
                _ => panic!("Expected Search variant"),
            }
        }

        #[test]
        fn handles_all_action_types() {
            // Arrange
            let actions = ["open", "terminal", "folder", "copy"];

            for action in actions {
                // Act
                let json = format!(r#"{{"type": "execute", "path": "/test", "action": "{}"}}"#, action);
                let msg: IpcMessage = serde_json::from_str(&json).unwrap();

                // Assert
                match msg {
                    IpcMessage::Execute { action: a, .. } => assert_eq!(a, action),
                    _ => panic!("Expected Execute variant"),
                }
            }
        }
    }

    mod search_result_serialization {
        use super::*;

        #[test]
        fn serializes_to_json() {
            // Arrange
            let result = SearchResult {
                path: "/home/user/file.txt".to_string(),
                name: "file.txt".to_string(),
                is_dir: false,
                icon: None,
            };

            // Act
            let json = serde_json::to_string(&result).unwrap();

            // Assert
            assert!(json.contains(r#""path":"/home/user/file.txt""#));
            assert!(json.contains(r#""name":"file.txt""#));
            assert!(json.contains(r#""is_dir":false"#));
        }

        #[test]
        fn serializes_directory() {
            // Arrange
            let result = SearchResult {
                path: "/home/user/docs".to_string(),
                name: "docs".to_string(),
                is_dir: true,
                icon: None,
            };

            // Act
            let json = serde_json::to_string(&result).unwrap();

            // Assert
            assert!(json.contains(r#""is_dir":true"#));
        }

        #[test]
        fn serializes_vec_of_results() {
            // Arrange
            let results = vec![
                SearchResult { path: "/a".to_string(), name: "a".to_string(), is_dir: true, icon: None },
                SearchResult { path: "/b".to_string(), name: "b".to_string(), is_dir: false, icon: None },
            ];

            // Act
            let json = serde_json::to_string(&results).unwrap();

            // Assert
            assert!(json.starts_with('['));
            assert!(json.ends_with(']'));
            assert!(json.contains(r#""path":"/a""#));
            assert!(json.contains(r#""path":"/b""#));
        }
    }

    mod app_state {
        use super::*;

        #[test]
        fn defaults_to_not_exiting() {
            // Act
            let state = AppState::default();

            // Assert
            assert!(!state.should_exit);
        }

        #[test]
        fn can_set_should_exit() {
            // Arrange
            let mut state = AppState::default();

            // Act
            state.should_exit = true;

            // Assert
            assert!(state.should_exit);
        }
    }

    mod calculate_centered_position {
        use super::*;

        #[test]
        fn centers_horizontally_on_1920x1080_monitor() {
            // Arrange
            let monitor_width = 1920;
            let monitor_height = 1080;
            let window_width = 600;
            let window_height = 400;

            // Act
            let (x, y) = calculate_centered_position(
                monitor_width, monitor_height,
                window_width, window_height,
                0, 0,
            );

            // Assert
            assert_eq!(x, (1920 - 600) / 2);
            assert_eq!(y, (1080 - 400) / 3);
        }

        #[test]
        fn accounts_for_monitor_offset() {
            // Arrange
            let monitor_x = 1920;
            let monitor_y = 0;

            // Act
            let (x, y) = calculate_centered_position(
                1920, 1080,
                600, 400,
                monitor_x, monitor_y,
            );

            // Assert
            assert_eq!(x, 1920 + (1920 - 600) / 2);
            assert_eq!(y, 0 + (1080 - 400) / 3);
        }

        #[test]
        fn handles_small_monitor() {
            // Arrange
            let monitor_width = 800;
            let monitor_height = 600;
            let window_width = 600;
            let window_height = 400;

            // Act
            let (x, y) = calculate_centered_position(
                monitor_width, monitor_height,
                window_width, window_height,
                0, 0,
            );

            // Assert
            assert_eq!(x, (800 - 600) / 2);
            assert_eq!(y, (600 - 400) / 3);
        }

        #[test]
        fn handles_vertical_monitor_stack() {
            // Arrange
            let monitor_x = 0;
            let monitor_y = 1080;

            // Act
            let (x, y) = calculate_centered_position(
                1920, 1080,
                600, 400,
                monitor_x, monitor_y,
            );

            // Assert
            assert_eq!(x, (1920 - 600) / 2);
            assert_eq!(y, 1080 + (1080 - 400) / 3);
        }

        #[test]
        #[should_panic]
        fn panics_on_zero_window_size() {
            calculate_centered_position(1920, 1080, 0, 0, 0, 0);
        }
    }

    mod extract_app_id {
        use super::*;

        #[test]
        fn extracts_simple_name() {
            assert_eq!(extract_app_id("/usr/share/applications/firefox.desktop"), "firefox");
        }

        #[test]
        fn extracts_from_flatpak_path() {
            assert_eq!(extract_app_id("/var/lib/flatpak/exports/share/applications/org.mozilla.firefox.desktop"), "firefox");
        }

        #[test]
        fn extracts_from_snap_path() {
            assert_eq!(extract_app_id("/var/lib/snapd/desktop/applications/firefox_firefox.desktop"), "firefox");
        }

        #[test]
        fn handles_hyphenated_name() {
            assert_eq!(extract_app_id("/usr/share/applications/signal-desktop.desktop"), "signal-desktop");
        }

        #[test]
        fn handles_underscored_snap_name() {
            assert_eq!(extract_app_id("/usr/share/applications/discord_discord.desktop"), "discord");
        }

        #[test]
        fn handles_user_local_path() {
            assert_eq!(extract_app_id("/home/user/.local/share/applications/discord.desktop"), "discord");
        }
    }

    mod extract_filename {
        use super::*;

        #[test]
        fn extracts_from_absolute_path() {
            assert_eq!(extract_filename("/home/user/documents/file.txt"), "file.txt");
        }

        #[test]
        fn handles_path_with_spaces() {
            assert_eq!(extract_filename("/home/user/my documents/my file.txt"), "my file.txt");
        }

        #[test]
        fn returns_path_for_root() {
            assert_eq!(extract_filename("/"), "/");
        }

        #[test]
        fn handles_hidden_file() {
            assert_eq!(extract_filename("/home/user/.bashrc"), ".bashrc");
        }
    }

    mod dedupe_results {
        use super::*;

        fn make_result(path: &str, name: &str) -> SearchResult {
            SearchResult { path: path.to_string(), name: name.to_string(), is_dir: false, icon: None }
        }

        #[test]
        fn removes_duplicate_desktop_files_by_app_id() {
            // Arrange
            let results = vec![
                make_result("/usr/share/applications/firefox.desktop", "Firefox"),
                make_result("/home/user/.local/share/applications/firefox.desktop", "Firefox"),
            ];

            // Act
            let deduped = dedupe_results(results);

            // Assert
            assert_eq!(deduped.len(), 1);
            assert_eq!(deduped[0].path, "/usr/share/applications/firefox.desktop");
        }

        #[test]
        fn removes_flatpak_duplicates() {
            // Arrange
            let results = vec![
                make_result("/usr/share/applications/firefox.desktop", "Firefox Web Browser"),
                make_result("/var/lib/flatpak/exports/share/applications/org.mozilla.firefox.desktop", "Firefox"),
            ];

            // Act
            let deduped = dedupe_results(results);

            // Assert
            assert_eq!(deduped.len(), 1);
        }

        #[test]
        fn keeps_different_apps() {
            // Arrange
            let results = vec![
                make_result("/usr/share/applications/firefox.desktop", "Firefox"),
                make_result("/usr/share/applications/chrome.desktop", "Chrome"),
            ];

            // Act
            let deduped = dedupe_results(results);

            // Assert
            assert_eq!(deduped.len(), 2);
        }

        #[test]
        fn dedupes_non_desktop_by_name() {
            // Arrange
            let results = vec![
                make_result("/home/user/Documents", "Documents"),
                make_result("/media/drive/Documents", "Documents"),
            ];

            // Act
            let deduped = dedupe_results(results);

            // Assert
            assert_eq!(deduped.len(), 1);
        }

        #[test]
        fn keeps_first_occurrence() {
            // Arrange
            let results = vec![
                make_result("/first/path/discord.desktop", "Discord"),
                make_result("/second/path/discord.desktop", "Discord"),
            ];

            // Act
            let deduped = dedupe_results(results);

            // Assert
            assert_eq!(deduped[0].path, "/first/path/discord.desktop");
        }
    }

    mod score_path_quality {
        use super::*;

        #[test]
        fn standard_app_dir_has_low_penalty() {
            // Arrange
            let path = "/usr/share/applications/firefox.desktop";

            // Act
            let score = score_path_quality(path);

            // Assert
            assert!(score < 50);
        }

        #[test]
        fn autostart_dir_has_higher_penalty() {
            // Arrange
            let path = "/etc/xdg/autostart/something.desktop";

            // Act
            let score = score_path_quality(path);

            // Assert
            assert!(score >= 80);
        }

        #[test]
        fn hidden_dirs_heavily_penalized() {
            // Arrange
            let path = "/home/user/.config/autostart/app.desktop";

            // Act
            let score = score_path_quality(path);

            // Assert
            assert!(score >= 500);
        }

        #[test]
        fn deep_paths_penalized() {
            // Arrange
            let shallow = "/usr/share/applications/app.desktop";
            let deep = "/a/b/c/d/e/f/g/h/app.desktop";

            // Act
            let shallow_score = score_path_quality(shallow);
            let deep_score = score_path_quality(deep);

            // Assert
            assert!(deep_score > shallow_score);
        }

        #[test]
        fn user_local_apps_have_low_penalty() {
            // Arrange
            let path = "/home/user/.local/share/applications/discord.desktop";

            // Act
            let score = score_path_quality(path);

            // Assert
            assert!(score < 100);
        }
    }

    mod score_result {
        use super::*;

        fn make_result(path: &str, name: &str) -> SearchResult {
            SearchResult { path: path.to_string(), name: name.to_string(), is_dir: false, icon: None }
        }

        fn make_app(name: &str) -> SearchResult {
            make_result(
                &format!("/usr/share/applications/{}.desktop", name.to_lowercase()),
                name,
            )
        }

        #[test]
        fn exact_match_has_lowest_penalty() {
            // Arrange
            let r = make_app("foo");
            let freq = FrequencyData::default();

            // Act
            let score = score_result(&r, "foo", &freq);

            // Assert
            assert!(score < 50);
        }

        #[test]
        fn prefix_lower_than_contains() {
            // Arrange
            let prefix = make_app("foobar");
            let contains = make_app("xfoo");
            let freq = FrequencyData::default();

            // Act
            let prefix_score = score_result(&prefix, "foo", &freq);
            let contains_score = score_result(&contains, "foo", &freq);

            // Assert
            assert!(prefix_score < contains_score);
        }

        #[test]
        fn desktop_lower_than_folder() {
            // Arrange
            let desktop = make_app("foo");
            let folder = make_result("/some/path/foo", "foo");
            let freq = FrequencyData::default();

            // Act
            let desktop_score = score_result(&desktop, "foo", &freq);
            let folder_score = score_result(&folder, "foo", &freq);

            // Assert
            assert!(desktop_score < folder_score);
        }

        #[test]
        fn shorter_name_lower_score() {
            // Arrange
            let short = make_app("foob");
            let long = make_app("foobar");
            let freq = FrequencyData::default();

            // Act
            let short_score = score_result(&short, "foo", &freq);
            let long_score = score_result(&long, "foo", &freq);

            // Assert
            assert!(short_score < long_score);
        }

        #[test]
        fn standard_path_lower_than_autostart() {
            // Arrange
            let standard = make_app("foo");
            let autostart = make_result("/etc/xdg/autostart/foo.desktop", "foo");
            let freq = FrequencyData::default();

            // Act
            let standard_score = score_result(&standard, "foo", &freq);
            let autostart_score = score_result(&autostart, "foo", &freq);

            // Assert
            assert!(standard_score < autostart_score);
        }

        #[test]
        fn frequency_boost_lowers_score() {
            // Arrange
            let r = make_app("foo");
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let mut freq = FrequencyData::default();
            freq.entries.insert(r.path.clone(), FrequencyEntry { count: 5, last_accessed: now });

            // Act
            let score_with_freq = score_result(&r, "foo", &freq);
            let score_without_freq = score_result(&r, "foo", &FrequencyData::default());

            // Assert
            assert!(score_with_freq < score_without_freq);
        }
    }

    mod sort_by_relevance {
        use super::*;

        fn make_result(path: &str, name: &str) -> SearchResult {
            SearchResult { path: path.to_string(), name: name.to_string(), is_dir: false, icon: None }
        }

        fn make_app(name: &str) -> SearchResult {
            SearchResult {
                path: format!("/usr/share/applications/{}.desktop", name.to_lowercase()),
                name: name.to_string(),
                is_dir: false,
                icon: None,
            }
        }

        #[test]
        fn exact_match_beats_prefix_match() {
            // Arrange
            let results = vec![make_app("foobar"), make_app("foo")];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "foo", &freq);

            // Assert
            assert_eq!(sorted[0].name, "foo");
        }

        #[test]
        fn prefix_match_beats_contains_match() {
            // Arrange
            let results = vec![make_app("xfoo"), make_app("foobar")];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "foo", &freq);

            // Assert
            assert_eq!(sorted[0].name, "foobar");
        }

        #[test]
        fn desktop_beats_folder_same_name() {
            // Arrange
            let results = vec![
                make_result("/some/path/foo", "foo"),
                make_app("foo"),
            ];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "foo", &freq);

            // Assert
            assert!(sorted[0].path.ends_with(".desktop"));
        }

        #[test]
        fn shorter_name_wins_same_match_level() {
            // Arrange
            let results = vec![make_app("foobar"), make_app("foob")];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "foo", &freq);

            // Assert
            assert_eq!(sorted[0].name, "foob");
        }

        #[test]
        fn standard_path_beats_autostart() {
            // Arrange
            let results = vec![
                make_result("/etc/xdg/autostart/foo.desktop", "foo"),
                make_app("foo"),
            ];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "foo", &freq);

            // Assert
            assert!(sorted[0].path.contains("/usr/share/applications/"));
        }

        #[test]
        fn empty_results_returns_empty() {
            let freq = FrequencyData::default();
            assert!(sort_by_relevance(vec![], "test", &freq).is_empty());
        }

        #[test]
        fn single_result_unchanged() {
            // Arrange
            let results = vec![make_app("test")];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "test", &freq);

            // Assert
            assert_eq!(sorted.len(), 1);
        }

        #[test]
        fn case_insensitive_matching() {
            // Arrange
            let results = vec![make_app("FOO"), make_app("foo")];
            let freq = FrequencyData::default();

            // Act
            let sorted = sort_by_relevance(results, "Foo", &freq);

            // Assert
            assert_eq!(sorted.len(), 2);
        }

        #[test]
        fn frequent_app_beats_better_match() {
            // Arrange
            let results = vec![make_app("foobar"), make_app("xfoo")];
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let mut freq = FrequencyData::default();
            freq.entries.insert(
                "/usr/share/applications/xfoo.desktop".to_string(),
                FrequencyEntry { count: 10, last_accessed: now },
            );

            // Act
            let sorted = sort_by_relevance(results, "foo", &freq);

            // Assert
            assert_eq!(sorted[0].name, "xfoo");
        }
    }

    mod frequency {
        use super::*;

        #[test]
        fn effective_count_no_decay_at_zero_days() {
            // Arrange
            let now = 1000000u64;
            let entry = FrequencyEntry { count: 5, last_accessed: now };

            // Act
            let count = effective_count(&entry, now);

            // Assert
            assert!((count - 5.0).abs() < 0.01);
        }

        #[test]
        fn effective_count_halves_after_half_life() {
            // Arrange
            let now = 1000000u64;
            let half_life_secs = (HALF_LIFE_DAYS * 86400.0) as u64;
            let entry = FrequencyEntry { count: 10, last_accessed: now - half_life_secs };

            // Act
            let count = effective_count(&entry, now);

            // Assert
            assert!((count - 5.0).abs() < 0.1);
        }

        #[test]
        fn calc_frequency_bonus_returns_zero_for_unknown() {
            // Arrange
            let freq = FrequencyData::default();

            // Act
            let bonus = calc_frequency_bonus("/unknown/path", &freq);

            // Assert
            assert_eq!(bonus, 0);
        }

        #[test]
        fn calc_frequency_bonus_scales_with_count() {
            // Arrange
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let mut freq = FrequencyData::default();
            freq.entries.insert("/path1".to_string(), FrequencyEntry { count: 1, last_accessed: now });
            freq.entries.insert("/path2".to_string(), FrequencyEntry { count: 5, last_accessed: now });

            // Act
            let bonus1 = calc_frequency_bonus("/path1", &freq);
            let bonus2 = calc_frequency_bonus("/path2", &freq);

            // Assert
            assert!(bonus2 > bonus1);
        }
    }
}
