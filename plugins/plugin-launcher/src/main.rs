use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::{Read as IoRead, Write as IoWrite};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::Request;
use wry::WebViewBuilder;

const SOCKET_PATH: &str = "/tmp/qol-launcher.sock";

#[derive(Debug)]
enum UserEvent {
    SearchComplete(Vec<SearchResult>),
    Show,
}

fn handle_socket_message(stream: &mut UnixStream, proxy: &tao::event_loop::EventLoopProxy<UserEvent>) {
    let mut buf = [0u8; 16];
    let Ok(n) = stream.read(&mut buf) else { return };
    if &buf[..n] != b"show" { return }
    let _ = proxy.send_event(UserEvent::Show);
}

fn start_socket_listener(proxy: tao::event_loop::EventLoopProxy<UserEvent>) -> bool {
    if let Ok(mut stream) = UnixStream::connect(SOCKET_PATH) {
        let _ = stream.write_all(b"show");
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

fn parse_search_result(line: &str) -> SearchResult {
    let path = line.to_string();
    let name = PathBuf::from(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let is_dir = std::path::Path::new(&path).is_dir();
    SearchResult { path, name, is_dir }
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
    stdout.lines().filter(|l| !l.is_empty()).map(parse_search_result).collect()
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
    let _ = Command::new("xdg-open").arg(path).spawn();
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
                SearchResult { path: "/a".to_string(), name: "a".to_string(), is_dir: true },
                SearchResult { path: "/b".to_string(), name: "b".to_string(), is_dir: false },
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
}
