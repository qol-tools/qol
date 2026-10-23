use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xfixes::{self, ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    self, Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Property,
    SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::capture::{Capture, CaptureError, Clipboard, Content, Payload, Support, MAX_CLIP_BYTES};

const BACKEND: &str = "x11-xfixes";
const CONVERT_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_STEP: Duration = Duration::from_millis(5);
const TRANSFER_PROPERTY: &[u8] = b"QOL_CLIPBOARD_TRANSFER";
const PNG: &str = "image/png";
const URI_LIST: &str = "text/uri-list";
const PASSWORD_HINT: &[u8] = b"x-kde-passwordManagerHint";
const PASSWORD_HINT_VALUE: &[u8] = b"secret";
const TEXT_TARGETS: [&str; 4] = ["UTF8_STRING", "text/plain;charset=utf-8", "STRING", "TEXT"];

pub(crate) fn open() -> Result<Arc<dyn Clipboard>, CaptureError> {
    let (conn, screen) = connect()?;
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id().map_err(failed)?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(failed)?;
    let atoms = Atoms::intern(&conn)?;
    conn.xfixes_select_selection_input(
        window,
        atoms.clipboard,
        SelectionEventMask::SET_SELECTION_OWNER,
    )
    .map_err(failed)?;
    conn.flush().map_err(failed)?;
    Ok(Arc::new(X11Clipboard {
        conn,
        window,
        atoms,
        offer: Mutex::new(None),
        pending: Mutex::new(VecDeque::new()),
    }))
}

pub(crate) fn probe() -> Support {
    match connect() {
        Ok(_) => Support::Ready { backend: BACKEND },
        Err(error) => Support::Unavailable(error),
    }
}

fn connect() -> Result<(RustConnection, usize), CaptureError> {
    let (conn, screen) = x11rb::connect(None).map_err(|error| {
        log::warn!("no X11 display ({error}); Wayland data-control capture is not built yet");
        CaptureError::Unavailable(
            "Clipboard history is not available in this desktop session.".to_string(),
        )
    })?;
    if conn
        .extension_information(xfixes::X11_EXTENSION_NAME)
        .map_err(failed)?
        .is_none()
    {
        return Err(CaptureError::Unavailable(
            "Clipboard history is not available in this desktop session.".to_string(),
        ));
    }
    conn.xfixes_query_version(5, 0)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    Ok((conn, screen))
}

fn failed(error: impl Display) -> CaptureError {
    CaptureError::Failed(error.to_string())
}

struct Atoms {
    clipboard: Atom,
    targets: Atom,
    incr: Atom,
    transfer: Atom,
    png: Atom,
    utf8_string: Atom,
    text_plain: Atom,
    password_hint: Atom,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Self, CaptureError> {
        let intern = |name: &[u8]| -> Result<Atom, CaptureError> {
            Ok(conn
                .intern_atom(false, name)
                .map_err(failed)?
                .reply()
                .map_err(failed)?
                .atom)
        };
        Ok(Self {
            clipboard: intern(b"CLIPBOARD")?,
            targets: intern(b"TARGETS")?,
            incr: intern(b"INCR")?,
            transfer: intern(TRANSFER_PROPERTY)?,
            png: intern(PNG.as_bytes())?,
            utf8_string: intern(b"UTF8_STRING")?,
            text_plain: intern(b"text/plain;charset=utf-8")?,
            password_hint: intern(PASSWORD_HINT)?,
        })
    }
}

struct X11Clipboard {
    conn: RustConnection,
    window: Window,
    atoms: Atoms,
    offer: Mutex<Option<Payload>>,
    pending: Mutex<VecDeque<Event>>,
}

enum Wanted {
    Png,
    Files,
    Text,
}

impl Clipboard for X11Clipboard {
    fn backend(&self) -> &'static str {
        BACKEND
    }

    fn next_change(&self) -> Result<Capture, CaptureError> {
        loop {
            let notify = match self.next_event()? {
                Event::XfixesSelectionNotify(notify) => notify,
                event => {
                    self.handle_background(event)?;
                    continue;
                }
            };
            if notify.selection != self.atoms.clipboard
                || notify.owner == x11rb::NONE
                || notify.owner == self.window
            {
                continue;
            }
            if let Some(capture) = self.read_selection(notify.owner, notify.selection_timestamp)? {
                return Ok(capture);
            }
        }
    }

    fn write(&self, payload: Payload) -> Result<(), CaptureError> {
        let size = match &payload {
            Payload::Text(text) | Payload::Secret { text, .. } => text.len(),
            Payload::Png(png) => png.len(),
        };
        if !self.servable(size) {
            log::warn!("a {size} byte clip is too large to put back on the X11 clipboard");
            return Err(CaptureError::Failed(
                "This clip is too large to copy again.".to_string(),
            ));
        }
        *lock(&self.offer) = Some(payload);
        self.conn
            .set_selection_owner(self.window, self.atoms.clipboard, x11rb::CURRENT_TIME)
            .map_err(failed)?;
        self.conn.flush().map_err(failed)?;
        let owner = self
            .conn
            .get_selection_owner(self.atoms.clipboard)
            .map_err(failed)?
            .reply()
            .map_err(failed)?
            .owner;
        if owner != self.window {
            return Err(CaptureError::Failed(
                "Another app took the clipboard. Try copying again.".to_string(),
            ));
        }
        Ok(())
    }

    fn clear_expired_secret(&self) -> Result<(), CaptureError> {
        {
            let mut offer = lock(&self.offer);
            if !offer.as_ref().is_some_and(Payload::is_expired_secret) {
                return Ok(());
            }
            *offer = None;
        }
        let owner = self
            .conn
            .get_selection_owner(self.atoms.clipboard)
            .map_err(failed)?
            .reply()
            .map_err(failed)?
            .owner;
        if owner == self.window {
            self.conn
                .set_selection_owner(x11rb::NONE, self.atoms.clipboard, x11rb::CURRENT_TIME)
                .map_err(failed)?;
            self.conn.flush().map_err(failed)?;
        }
        Ok(())
    }
}

impl X11Clipboard {
    fn next_event(&self) -> Result<Event, CaptureError> {
        if let Some(event) = lock(&self.pending).pop_front() {
            return Ok(event);
        }
        self.conn.wait_for_event().map_err(failed)
    }

    fn handle_background(&self, event: Event) -> Result<(), CaptureError> {
        match event {
            Event::SelectionRequest(request) => self.serve(request),
            Event::SelectionClear(clear) if clear.selection == self.atoms.clipboard => {
                *lock(&self.offer) = None;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn read_selection(&self, owner: Window, time: u32) -> Result<Option<Capture>, CaptureError> {
        let Some(raw) = self.convert(self.atoms.targets, time)? else {
            log::debug!("clipboard owner did not answer TARGETS");
            return Ok(None);
        };
        let targets = self.target_names(&atoms_of(&raw))?;
        let sensitive = crate::secrets::flagged_by_source(
            &targets
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>(),
        );
        let Some((target, wanted)) = choose_target(&targets) else {
            log::debug!("clipboard offers no text, image or file target");
            return Ok(None);
        };
        let Some(bytes) = self.convert(target, time)? else {
            log::debug!("clipboard owner did not answer the chosen target");
            return Ok(None);
        };
        let content = match wanted {
            Wanted::Png => Content::Png(bytes),
            Wanted::Files => {
                Content::Files(super::uri::parse_uri_list(&String::from_utf8_lossy(&bytes)))
            }
            Wanted::Text => Content::Text(String::from_utf8_lossy(&bytes).into_owned()),
        };
        Ok(Some(Capture {
            content,
            app: self.owner_app(owner),
            sensitive,
        }))
    }

    fn convert(&self, target: Atom, time: u32) -> Result<Option<Vec<u8>>, CaptureError> {
        self.conn
            .convert_selection(
                self.window,
                self.atoms.clipboard,
                target,
                self.atoms.transfer,
                time,
            )
            .map_err(failed)?;
        self.conn.flush().map_err(failed)?;
        let window = self.window;
        let clipboard = self.atoms.clipboard;
        let notify = self.wait_for(Instant::now() + CONVERT_TIMEOUT, |event| {
            matches!(event, Event::SelectionNotify(notify)
                if notify.requestor == window
                    && notify.selection == clipboard
                    && notify.target == target)
        })?;
        let Some(Event::SelectionNotify(notify)) = notify else {
            return Ok(None);
        };
        if notify.property == x11rb::NONE {
            return Ok(None);
        }
        let reply = self.take_transfer()?;
        if reply.type_ == self.atoms.incr {
            return self.read_incremental();
        }
        Ok(Some(reply.value))
    }

    fn read_incremental(&self) -> Result<Option<Vec<u8>>, CaptureError> {
        let window = self.window;
        let transfer = self.atoms.transfer;
        let mut data = Vec::new();
        loop {
            let chunk = self.wait_for(Instant::now() + CONVERT_TIMEOUT, |event| {
                matches!(event, Event::PropertyNotify(change)
                    if change.window == window
                        && change.atom == transfer
                        && change.state == Property::NEW_VALUE)
            })?;
            if chunk.is_none() {
                return Ok(None);
            }
            let value = self.take_transfer()?.value;
            if value.is_empty() {
                return Ok(Some(data));
            }
            data.extend_from_slice(&value);
            if data.len() > MAX_CLIP_BYTES {
                log::debug!("clipboard transfer exceeds {MAX_CLIP_BYTES} bytes; skipped");
                return Ok(None);
            }
        }
    }

    fn take_transfer(&self) -> Result<xproto::GetPropertyReply, CaptureError> {
        self.conn
            .get_property(
                true,
                self.window,
                self.atoms.transfer,
                AtomEnum::ANY,
                0,
                (MAX_CLIP_BYTES / 4) as u32,
            )
            .map_err(failed)?
            .reply()
            .map_err(failed)
    }

    fn wait_for(
        &self,
        deadline: Instant,
        wanted: impl Fn(&Event) -> bool,
    ) -> Result<Option<Event>, CaptureError> {
        loop {
            match self.conn.poll_for_event().map_err(failed)? {
                Some(event) if wanted(&event) => return Ok(Some(event)),
                Some(Event::SelectionRequest(request)) => self.serve(request)?,
                Some(event) => lock(&self.pending).push_back(event),
                None if Instant::now() >= deadline => return Ok(None),
                None => std::thread::sleep(POLL_STEP),
            }
        }
    }

    fn target_names(&self, atoms: &[Atom]) -> Result<Vec<(Atom, String)>, CaptureError> {
        let cookies = atoms
            .iter()
            .map(|atom| self.conn.get_atom_name(*atom).map(|cookie| (*atom, cookie)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        Ok(cookies
            .into_iter()
            .filter_map(|(atom, cookie)| {
                let reply = cookie.reply().ok()?;
                Some((atom, String::from_utf8_lossy(&reply.name).into_owned()))
            })
            .collect())
    }

    fn owner_app(&self, owner: Window) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, owner, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        let parts: Vec<&[u8]> = reply
            .value
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .collect();
        let class = parts.get(1).or_else(|| parts.first())?;
        Some(String::from_utf8_lossy(class).into_owned())
    }

    fn serve(&self, request: SelectionRequestEvent) -> Result<(), CaptureError> {
        let property = if request.property == x11rb::NONE {
            request.target
        } else {
            request.property
        };
        let answered = {
            let offer = lock(&self.offer);
            match offer.as_ref() {
                Some(payload) if request.selection == self.atoms.clipboard => {
                    self.answer(&request, property, payload)?
                }
                _ => false,
            }
        };
        let event = SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if answered { property } else { x11rb::NONE },
        };
        self.conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, event)
            .map_err(failed)?;
        self.conn.flush().map_err(failed)
    }

    // Larger payloads need the INCR protocol, which this owner does not serve.
    fn servable(&self, bytes: usize) -> bool {
        bytes + 64 <= self.conn.maximum_request_bytes()
    }

    fn answer(
        &self,
        request: &SelectionRequestEvent,
        property: Atom,
        payload: &Payload,
    ) -> Result<bool, CaptureError> {
        if payload.is_expired_secret() {
            return Ok(false);
        }
        let text_targets = [
            self.atoms.utf8_string,
            self.atoms.text_plain,
            AtomEnum::STRING.into(),
        ];
        let (offered, bytes): (Vec<Atom>, &[u8]) = match payload {
            Payload::Text(text) => (text_targets.to_vec(), text.as_bytes()),
            Payload::Secret { text, .. } => {
                let mut offered = text_targets.to_vec();
                offered.push(self.atoms.password_hint);
                (offered, text.as_bytes())
            }
            Payload::Png(png) => (vec![self.atoms.png], png.as_slice()),
        };
        let bytes = if request.target == self.atoms.password_hint {
            PASSWORD_HINT_VALUE
        } else {
            bytes
        };
        if request.target == self.atoms.targets {
            let mut targets = vec![self.atoms.targets];
            targets.extend(&offered);
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &targets,
                )
                .map_err(failed)?;
            return Ok(true);
        }
        if !offered.contains(&request.target) {
            return Ok(false);
        }
        if !self.servable(bytes.len()) {
            log::debug!(
                "clipboard payload of {} bytes needs INCR; not served",
                bytes.len()
            );
            return Ok(false);
        }
        self.conn
            .change_property8(
                PropMode::REPLACE,
                request.requestor,
                property,
                request.target,
                bytes,
            )
            .map_err(failed)?;
        Ok(true)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn atoms_of(raw: &[u8]) -> Vec<Atom> {
    let (chunks, _) = raw.as_chunks::<4>();
    chunks.iter().copied().map(u32::from_ne_bytes).collect()
}

fn choose_target(targets: &[(Atom, String)]) -> Option<(Atom, Wanted)> {
    let find = |name: &str| {
        targets
            .iter()
            .find(|(_, offered)| offered == name)
            .map(|(atom, _)| *atom)
    };
    if let Some(atom) = find(PNG) {
        return Some((atom, Wanted::Png));
    }
    if let Some(atom) = find(URI_LIST) {
        return Some((atom, Wanted::Files));
    }
    TEXT_TARGETS
        .iter()
        .find_map(|name| find(name))
        .map(|atom| (atom, Wanted::Text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(names: &[&str]) -> Vec<(Atom, String)> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| (index as Atom + 1, (*name).to_string()))
            .collect()
    }

    #[test]
    fn target_choice_prefers_image_then_files_then_utf8_text() {
        let cases: &[(&[&str], Option<Atom>)] = &[
            (&["TARGETS", "UTF8_STRING", "image/png"], Some(3)),
            (&["STRING", "text/uri-list", "UTF8_STRING"], Some(2)),
            (&["STRING", "UTF8_STRING"], Some(2)),
            (&["TEXT", "STRING"], Some(2)),
            (&["TARGETS", "application/x-qt-image"], None),
        ];
        for (names, atom) in cases {
            let chosen = choose_target(&offered(names)).map(|(atom, _)| atom);
            assert_eq!(chosen, *atom, "names: {names:?}");
        }
    }
}
