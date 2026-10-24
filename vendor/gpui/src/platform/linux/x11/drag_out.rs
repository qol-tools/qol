use std::path::PathBuf;

use http_client::Url;
use util::ResultExt;
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        self, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode,
        SelectionNotifyEvent, SelectionRequestEvent,
    },
    wrapper::ConnectionExt as _,
    xcb_ffi::XCBConnection,
};

use super::XcbAtoms;

const XDND_VERSION: u32 = 5;

pub(crate) struct DragOut {
    uri_list: String,
    owns_selection: bool,
    target: Option<Target>,
    dropped: bool,
}

struct Target {
    window: xproto::Window,
    proxy: xproto::Window,
    accepted: bool,
    awaiting_status: bool,
    pending: Option<Position>,
}

#[derive(Clone, Copy)]
pub(crate) struct Position {
    pub root_x: i16,
    pub root_y: i16,
    pub time: u32,
    pub action: xproto::Atom,
}

pub(crate) struct Link<'a> {
    pub xcb: &'a XCBConnection,
    pub atoms: &'a XcbAtoms,
    pub root: xproto::Window,
    pub source: xproto::Window,
}

impl DragOut {
    pub fn new(paths: &[PathBuf]) -> Self {
        let uri_list = paths
            .iter()
            .filter_map(|path| Url::from_file_path(path).ok())
            .map(|url| url.to_string())
            .collect::<Vec<_>>()
            .join("\r\n");
        Self {
            uri_list,
            owns_selection: false,
            target: None,
            dropped: false,
        }
    }

    pub fn is_dropped(&self) -> bool {
        self.dropped
    }

    pub fn motion(&mut self, link: &Link, inside: bool, position: Position) {
        if self.dropped {
            return;
        }
        if inside {
            self.leave(link);
            return;
        }
        if !self.owns_selection {
            link.xcb
                .set_selection_owner(link.source, link.atoms.XdndSelection, position.time)
                .log_err();
            self.owns_selection = true;
        }
        let found = find_target(link, position.root_x, position.root_y);
        if self.target.as_ref().map(|target| target.window) != found.map(|found| found.window) {
            self.leave(link);
            if let Some(Found {
                window,
                proxy,
                version,
            }) = found
            {
                send(
                    link,
                    window,
                    proxy,
                    link.atoms.XdndEnter,
                    [
                        link.source,
                        version << 24,
                        link.atoms.TextUriList,
                        link.atoms.UTF8_STRING,
                        link.atoms.TEXT_PLAIN,
                    ],
                );
                self.target = Some(Target {
                    window,
                    proxy,
                    accepted: false,
                    awaiting_status: false,
                    pending: None,
                });
            }
        }
        let Some(target) = self.target.as_mut() else {
            return;
        };
        if target.awaiting_status {
            target.pending = Some(position);
            return;
        }
        send_position(link, target, position);
    }

    pub fn status(&mut self, link: &Link, from: xproto::Window, flags: u32) {
        let Some(target) = self.target.as_mut().filter(|target| target.window == from) else {
            return;
        };
        target.accepted = flags & 1 == 1;
        target.awaiting_status = false;
        if let Some(position) = target.pending.take() {
            send_position(link, target, position);
        }
    }

    pub fn release(&mut self, link: &Link, time: u32) -> bool {
        match self.target.as_ref() {
            Some(target) if target.accepted && !self.dropped => {
                send(
                    link,
                    target.window,
                    target.proxy,
                    link.atoms.XdndDrop,
                    [link.source, 0, time, 0, 0],
                );
                self.dropped = true;
                true
            }
            _ => {
                self.leave(link);
                false
            }
        }
    }

    pub fn uri_list(&self) -> &str {
        &self.uri_list
    }

    fn leave(&mut self, link: &Link) {
        if let Some(target) = self.target.take() {
            send(
                link,
                target.window,
                target.proxy,
                link.atoms.XdndLeave,
                [link.source, 0, 0, 0, 0],
            );
        }
    }
}

pub(crate) fn answer(link: &Link, request: &SelectionRequestEvent, uri_list: Option<&str>) {
    let atoms = link.atoms;
    let offered = [
        atoms.TextUriList,
        atoms.UTF8_STRING,
        atoms.TEXT_PLAIN,
        atoms.TEXT_PLAIN_UTF8,
        atoms.STRING,
        atoms.TEXT,
    ];
    let property = if request.property == x11rb::NONE {
        request.target
    } else {
        request.property
    };
    let ours = request.selection == atoms.XdndSelection && uri_list.is_some();
    let answered = if ours && request.target == atoms.TARGETS {
        let mut targets = vec![atoms.TARGETS];
        targets.extend(offered);
        link.xcb
            .change_property32(
                PropMode::REPLACE,
                request.requestor,
                property,
                AtomEnum::ATOM,
                &targets,
            )
            .log_err()
            .is_some()
    } else {
        ours && offered.contains(&request.target)
            && link
                .xcb
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    request.target,
                    uri_list.unwrap_or_default().as_bytes(),
                )
                .log_err()
                .is_some()
    };
    let notify = SelectionNotifyEvent {
        response_type: xproto::SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: request.time,
        requestor: request.requestor,
        selection: request.selection,
        target: request.target,
        property: if answered { property } else { x11rb::NONE },
    };
    link.xcb
        .send_event(false, request.requestor, EventMask::NO_EVENT, notify)
        .log_err();
    link.xcb.flush().log_err();
}

fn send_position(link: &Link, target: &mut Target, position: Position) {
    let packed = ((position.root_x as u16 as u32) << 16) | position.root_y as u16 as u32;
    send(
        link,
        target.window,
        target.proxy,
        link.atoms.XdndPosition,
        [link.source, 0, packed, position.time, position.action],
    );
    target.awaiting_status = true;
}

fn send(
    link: &Link,
    window: xproto::Window,
    proxy: xproto::Window,
    kind: xproto::Atom,
    data: [u32; 5],
) {
    let message = ClientMessageEvent::new(32, window, kind, data);
    link.xcb
        .send_event(false, proxy, EventMask::NO_EVENT, message)
        .log_err();
    link.xcb.flush().log_err();
}

#[derive(Clone, Copy)]
struct Found {
    window: xproto::Window,
    proxy: xproto::Window,
    version: u32,
}

fn find_target(link: &Link, root_x: i16, root_y: i16) -> Option<Found> {
    let mut window = link.root;
    loop {
        let child = link
            .xcb
            .translate_coordinates(link.root, window, root_x, root_y)
            .ok()?
            .reply()
            .ok()?
            .child;
        if child == x11rb::NONE || child == link.source {
            return None;
        }
        if let Some(version) = xdnd_version(link, child) {
            return Some(Found {
                window: child,
                proxy: xdnd_proxy(link, child).unwrap_or(child),
                version: version.min(XDND_VERSION),
            });
        }
        window = child;
    }
}

fn xdnd_version(link: &Link, window: xproto::Window) -> Option<u32> {
    link.xcb
        .get_property(false, window, link.atoms.XdndAware, AtomEnum::ATOM, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
        .filter(|version| *version >= 3)
}

fn xdnd_proxy(link: &Link, window: xproto::Window) -> Option<xproto::Window> {
    link.xcb
        .get_property(false, window, link.atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}
