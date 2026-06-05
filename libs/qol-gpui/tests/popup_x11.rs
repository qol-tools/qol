#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

struct TestWindow {
    conn: RustConnection,
    screen_num: usize,
    root: u32,
    wid: u32,
    title: String,
}

impl TestWindow {
    fn spawn(title: &str) -> Option<Self> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let wid = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            wid,
            root,
            0,
            0,
            200,
            150,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(screen.black_pixel)
                .event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .ok()?;

        let win = Self {
            conn,
            screen_num,
            root,
            wid,
            title: title.to_string(),
        };
        win.set_title();
        win.set_pid();
        win.conn.map_window(wid).ok()?;
        win.conn.flush().ok()?;
        win.wait_managed().then_some(win)
    }

    fn set_title(&self) {
        let net_name = self.intern(b"_NET_WM_NAME");
        let utf8 = self.intern(b"UTF8_STRING");
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            self.wid,
            net_name,
            utf8,
            self.title.as_bytes(),
        );
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            self.wid,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            self.title.as_bytes(),
        );
    }

    fn set_pid(&self) {
        let pid_atom = self.intern(b"_NET_WM_PID");
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.wid,
            pid_atom,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        );
    }

    fn intern(&self, name: &[u8]) -> u32 {
        self.conn
            .intern_atom(false, name)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
            .unwrap_or(0)
    }

    fn atom_list(&self, prop: u32) -> Vec<u32> {
        self.conn
            .get_property(false, self.wid, prop, AtomEnum::ATOM, 0, 32)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    fn cardinal(&self, prop: u32) -> Option<u32> {
        self.conn
            .get_property(false, self.wid, prop, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()))
    }

    fn wm_state(&self) -> Option<u32> {
        let atom = self.intern(b"WM_STATE");
        self.conn
            .get_property(false, self.wid, atom, atom, 0, 2)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()))
    }

    fn compositor_running(&self) -> bool {
        let atom = self.intern(format!("_NET_WM_CM_S{}", self.screen_num).as_bytes());
        self.conn
            .get_selection_owner(atom)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.owner != 0)
            .unwrap_or(false)
    }

    fn wait_managed(&self) -> bool {
        let list_atom = self.intern(b"_NET_CLIENT_LIST");
        poll(Duration::from_secs(2), || {
            self.conn
                .get_property(false, self.root, list_atom, AtomEnum::WINDOW, 0, 1024)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().map(|mut v| v.any(|id| id == self.wid)))
                .unwrap_or(false)
        })
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.wid);
        let _ = self.conn.flush();
    }
}

fn poll(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    cond()
}

fn display_available() -> bool {
    std::env::var_os("DISPLAY").is_some()
}

fn wm_running() -> bool {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return false;
    };
    let root = conn.setup().roots[screen_num].root;
    let Ok(atom) = conn.intern_atom(false, b"_NET_SUPPORTING_WM_CHECK") else {
        return false;
    };
    let Some(atom) = atom.reply().ok().map(|r| r.atom) else {
        return false;
    };
    let check = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().and_then(|mut v| v.next()));
    let Some(check) = check else {
        return false;
    };
    conn.get_property(false, check, atom, AtomEnum::WINDOW, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().and_then(|mut v| v.next()))
        == Some(check)
}

#[test]
#[ignore = "needs a real X11 display + WM; run with `cargo test -p qol-gpui --test popup_x11 -- --ignored`"]
fn popup_lifecycle_writes_expected_x11_state() {
    if !display_available() {
        eprintln!("[popup_x11] no DISPLAY; skipping");
        return;
    }
    if !wm_running() {
        eprintln!("[popup_x11] no usable window manager on $DISPLAY; skipping");
        return;
    }
    let title = format!("qol-gpui-popup-x11-{}", std::process::id());
    let Some(win) = TestWindow::spawn(&title) else {
        panic!("WM did not manage the test window (is a window manager running on $DISPLAY?)");
    };

    assert!(
        qol_gpui::popup_window::configure_popup_window(&title),
        "configure_popup_window should find and configure the test window"
    );

    let dock = win.intern(b"_NET_WM_WINDOW_TYPE_DOCK");
    let type_atom = win.intern(b"_NET_WM_WINDOW_TYPE");
    assert!(
        poll(Duration::from_secs(2), || win
            .atom_list(type_atom)
            .contains(&dock)),
        "configure should set _NET_WM_WINDOW_TYPE_DOCK"
    );

    qol_gpui::popup_window::set_ghost_debug(Some(0.5), None);
    qol_gpui::popup_window::hide_window_by_title(&title);
    let opacity_atom = win.intern(b"_NET_WM_WINDOW_OPACITY");

    if win.compositor_running() {
        let ghosted = poll(Duration::from_secs(2), || {
            win.cardinal(opacity_atom)
                .map(|v| (v as f64 / u32::MAX as f64 - 0.5).abs() < 0.02)
                .unwrap_or(false)
        });
        assert!(
            ghosted,
            "hidden ghost should carry ~0.5 opacity under a compositor"
        );

        qol_gpui::popup_window::show_window_by_title(&title);
        let revealed = poll(Duration::from_secs(2), || {
            win.cardinal(opacity_atom).is_none()
        });
        assert!(revealed, "show should clear the opacity property");
    } else {
        const ICONIC: u32 = 3;
        let iconified = poll(Duration::from_secs(2), || win.wm_state() == Some(ICONIC));
        assert!(
            iconified,
            "without a compositor, hide should iconify the window"
        );
    }
}
