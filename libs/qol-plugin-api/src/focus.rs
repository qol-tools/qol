#[cfg(target_os = "linux")]
mod linux {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;

    pub fn has_process_focus() -> bool {
        let Ok((conn, _)) = x11rb::connect(None) else {
            return true;
        };
        let Ok(reply) = conn.get_input_focus() else {
            return true;
        };
        let Ok(focus) = reply.reply() else {
            return true;
        };
        if focus.focus == 0 {
            return false;
        }
        owns_window(&conn, focus.focus, std::process::id())
    }

    fn owns_window(conn: &impl Connection, mut window: u32, target_pid: u32) -> bool {
        loop {
            if window_pid(conn, window) == Some(target_pid) {
                return true;
            }
            let Ok(reply) = conn.query_tree(window) else {
                return false;
            };
            let Ok(tree) = reply.reply() else {
                return false;
            };
            if tree.parent == 0 || tree.parent == tree.root {
                return false;
            }
            window = tree.parent;
        }
    }

    fn window_pid(conn: &impl Connection, window: u32) -> Option<u32> {
        let atom = conn
            .intern_atom(false, b"_NET_WM_PID")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let prop = conn
            .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        prop.value32().and_then(|mut v| v.next())
    }
}

pub fn has_process_focus() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::has_process_focus()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}
