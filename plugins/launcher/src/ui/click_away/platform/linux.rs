use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::thread;

use x11rb::connection::Connection;
use x11rb::protocol::xinput;
use x11rb::protocol::xinput::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

const XI_ALL_MASTER_DEVICES: u16 = 1;
const STOP_BYTE: &[u8] = b"q";

pub(crate) struct Monitor {
    stop_write: i32,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            libc::write(self.stop_write, STOP_BYTE.as_ptr().cast(), 1);
            libc::close(self.stop_write);
        }
    }
}

pub(crate) fn start(window_title: String, tx: mpsc::Sender<()>) -> Option<Monitor> {
    let (conn, screen_num) = RustConnection::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let version = conn.xinput_xi_query_version(2, 0).ok()?.reply().ok()?;
    if version.major_version < 2 {
        return None;
    }
    conn.xinput_xi_select_events(
        root,
        &[xinput::EventMask {
            deviceid: XI_ALL_MASTER_DEVICES,
            mask: vec![xinput::XIEventMask::RAW_BUTTON_PRESS],
        }],
    )
    .ok()?
    .check()
    .ok()?;
    conn.flush().ok()?;

    let mut stop_fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe(stop_fds.as_mut_ptr()) } != 0 {
        return None;
    }
    let [stop_read, stop_write] = stop_fds;
    let fd = conn.stream().as_raw_fd();
    let spawned = thread::Builder::new()
        .name("qol-click-away".to_string())
        .spawn(move || monitor_loop(conn, fd, stop_read, window_title, tx))
        .is_ok();
    if !spawned {
        unsafe {
            libc::close(stop_write);
            libc::close(stop_read);
        }
        return None;
    }
    Some(Monitor { stop_write })
}

fn monitor_loop(
    conn: RustConnection,
    fd: i32,
    stop_read: i32,
    window_title: String,
    tx: mpsc::Sender<()>,
) {
    loop {
        let mut fds = [
            libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stop_read,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if ready < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if ready == 0 {
            continue;
        }
        if fds[1].revents != 0 {
            break;
        }
        if fds[0].revents & libc::POLLIN == 0 {
            break;
        }
        while let Ok(Some(event)) = conn.poll_for_event() {
            if matches!(event, Event::XinputRawButtonPress(_))
                && !qol_gpui::popup_window::pointer_over_window_by_title(&window_title)
            {
                let _ = tx.send(());
            }
        }
    }
    unsafe {
        libc::close(stop_read);
    }
}
