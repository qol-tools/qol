use x11rb::connection::Connection;

pub(super) fn activating(conn: &impl Connection, window_id: u32) {
    #[cfg(debug_assertions)]
    {
        let title = window_name(conn, window_id).unwrap_or_else(|| "Unknown".to_string());
        qol_gpui::probe::probe(
            "ACTIVATE_WIN",
            &format!("wid={window_id} title=\"{title}\" source=2 timestamp=0 requester_active=0"),
        );
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (conn, window_id);
    }
}

#[cfg(debug_assertions)]
fn window_name(conn: &impl Connection, wid: u32) -> Option<String> {
    use x11rb::protocol::xproto::AtomEnum;
    let props = [
        (
            super::intern(conn, b"_NET_WM_NAME")?,
            super::intern(conn, b"UTF8_STRING")?,
        ),
        (u32::from(AtomEnum::WM_NAME), u32::from(AtomEnum::STRING)),
    ];
    props
        .into_iter()
        .find_map(|(property, kind)| read_string(conn, wid, property, kind))
}

#[cfg(debug_assertions)]
fn read_string(conn: &impl Connection, wid: u32, property: u32, kind: u32) -> Option<String> {
    use x11rb::protocol::xproto::ConnectionExt;
    let reply = conn
        .get_property(false, wid, property, kind, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&reply.value)
            .trim_end_matches('\0')
            .to_string(),
    )
}
