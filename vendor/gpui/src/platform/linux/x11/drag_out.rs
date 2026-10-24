use std::{path::PathBuf, rc::Rc};

use crate::window::premultiplied;
use http_client::Url;
use util::ResultExt;
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        self, AtomEnum, ClientMessageEvent, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _,
        CreateGCAux, CreateWindowAux, EventMask, ImageFormat, PropMode, SelectionNotifyEvent,
        SelectionRequestEvent, StackMode, WindowClass,
    },
    wrapper::ConnectionExt as _,
    xcb_ffi::XCBConnection,
};

use super::XcbAtoms;

const XDND_VERSION: u32 = 5;
const ICON_DEPTH: u8 = 32;

pub(crate) struct DragOut {
    uri_list: String,
    owns_selection: bool,
    target: Option<Target>,
    dropped: bool,
    icon: Option<DragIcon>,
}

struct Target {
    window: xproto::Window,
    proxy: xproto::Window,
    accepted: bool,
    action: xproto::Atom,
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
    pub fn new(paths: &[PathBuf], icon: Option<DragIcon>) -> Self {
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
            icon,
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
            if let Some(icon) = self.icon.as_mut() {
                icon.hide();
            }
            self.leave(link);
            return;
        }
        if let Some(icon) = self.icon.as_mut() {
            icon.follow(position.root_x, position.root_y);
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
                    action: x11rb::NONE,
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

    pub fn status(&mut self, link: &Link, from: xproto::Window, flags: u32, action: xproto::Atom) {
        let Some(target) = self.target.as_mut().filter(|target| target.window == from) else {
            return;
        };
        target.accepted = flags & 1 == 1;
        target.action = action;
        target.awaiting_status = false;
        if let Some(position) = target.pending.take() {
            send_position(link, target, position);
        }
    }

    pub fn release(&mut self, link: &Link, time: u32) -> bool {
        self.icon = None;
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

    pub fn accepted_action(&self) -> Option<xproto::Atom> {
        if self.dropped {
            return None;
        }
        self.target
            .as_ref()
            .filter(|target| target.accepted)
            .map(|target| target.action)
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

pub(crate) struct DragIcon {
    xcb: Rc<XCBConnection>,
    window: xproto::Window,
    pixmap: xproto::Pixmap,
    colormap: xproto::Colormap,
    offset: (i16, i16),
    shown: bool,
}

pub(crate) struct Picture<'a> {
    pub bgra: &'a [u8],
    pub width: u16,
    pub height: u16,
    pub offset: (i16, i16),
}

impl DragIcon {
    pub fn new(
        xcb: &Rc<XCBConnection>,
        root: xproto::Window,
        visual: xproto::Visualid,
        picture: Picture,
    ) -> Option<Self> {
        let Picture {
            bgra,
            width,
            height,
            offset,
        } = picture;
        if width == 0 || height == 0 || bgra.len() != width as usize * height as usize * 4 {
            return None;
        }
        let colormap = xcb.generate_id().log_err()?;
        xcb.create_colormap(ColormapAlloc::NONE, colormap, root, visual)
            .log_err()?;
        let pixmap = xcb.generate_id().log_err()?;
        xcb.create_pixmap(ICON_DEPTH, pixmap, root, width, height)
            .log_err()?;
        let gc = xcb.generate_id().log_err()?;
        xcb.create_gc(gc, pixmap, &CreateGCAux::new()).log_err()?;
        xcb.put_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            gc,
            width,
            height,
            0,
            0,
            0,
            ICON_DEPTH,
            &premultiplied(bgra),
        )
        .log_err()?;
        xcb.free_gc(gc).log_err();
        let window = xcb.generate_id().log_err()?;
        xcb.create_window(
            ICON_DEPTH,
            window,
            root,
            0,
            0,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixmap(pixmap)
                .border_pixel(0)
                .override_redirect(1)
                .colormap(colormap),
        )
        .log_err()?;
        Some(Self {
            xcb: xcb.clone(),
            window,
            pixmap,
            colormap,
            offset,
            shown: false,
        })
    }

    fn follow(&mut self, root_x: i16, root_y: i16) {
        let aux = ConfigureWindowAux::new()
            .x(i32::from(root_x) + i32::from(self.offset.0))
            .y(i32::from(root_y) + i32::from(self.offset.1))
            .stack_mode(StackMode::ABOVE);
        self.xcb.configure_window(self.window, &aux).log_err();
        if !self.shown {
            self.xcb.map_window(self.window).log_err();
            self.shown = true;
        }
        self.xcb.flush().log_err();
    }

    fn hide(&mut self) {
        if self.shown {
            self.xcb.unmap_window(self.window).log_err();
            self.xcb.flush().log_err();
            self.shown = false;
        }
    }
}

impl Drop for DragIcon {
    fn drop(&mut self) {
        self.xcb.destroy_window(self.window).log_err();
        self.xcb.free_pixmap(self.pixmap).log_err();
        self.xcb.free_colormap(self.colormap).log_err();
        self.xcb.flush().log_err();
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

#[cfg(test)]
mod tests {
    use super::premultiplied;

    #[test]
    fn premultiplies_each_channel_by_alpha_and_keeps_alpha() {
        let cases = [
            ([200, 100, 50, 255], [200, 100, 50, 255]),
            ([200, 100, 50, 0], [0, 0, 0, 0]),
            ([255, 255, 255, 128], [128, 128, 128, 128]),
            ([10, 20, 30, 51], [2, 4, 6, 51]),
        ];
        for (pixel, expected) in cases {
            assert_eq!(premultiplied(&pixel), expected, "{pixel:?}");
        }
    }
}
