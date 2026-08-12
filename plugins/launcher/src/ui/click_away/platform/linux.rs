use std::sync::{mpsc, Arc};
use std::thread;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::record::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self};
use x11rb::rust_connection::RustConnection;
use x11rb::x11_utils::TryParse;

const LEFT_CLICK: u8 = 1;
const RECORD_FROM_SERVER: u8 = 0;
const EVENT_HEADER_BYTES: usize = 32;

type Frame = (i32, i32, i32, i32);

pub(crate) struct Monitor {
    ctrl: Arc<RustConnection>,
    context: record::Context,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self
            .ctrl
            .record_disable_context(self.context)
            .ok()
            .and_then(|cookie| cookie.check().ok());
        let _ = self.ctrl.flush();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub(crate) fn start(window_title: String, tx: mpsc::Sender<()>) -> Option<Monitor> {
    let (ctrl, _screen) = x11rb::connect(None).ok()?;
    let (data, _screen) = x11rb::connect(None).ok()?;
    let record_available = ctrl
        .extension_information(record::X11_EXTENSION_NAME)
        .ok()?
        .is_some();
    if !record_available {
        return None;
    }
    let _ = ctrl
        .record_query_version(
            record::X11_XML_VERSION.0 as _,
            record::X11_XML_VERSION.1 as _,
        )
        .ok()?
        .reply()
        .ok()?;
    let context = ctrl.generate_id().ok()?;
    let empty = record::Range8 { first: 0, last: 0 };
    let empty_ext = record::ExtRange {
        major: empty,
        minor: record::Range16 { first: 0, last: 0 },
    };
    let range = record::Range {
        core_requests: empty,
        core_replies: empty,
        ext_requests: empty_ext,
        ext_replies: empty_ext,
        delivered_events: empty,
        device_events: record::Range8 {
            first: xproto::BUTTON_PRESS_EVENT,
            last: xproto::BUTTON_PRESS_EVENT,
        },
        errors: empty,
        client_started: false,
        client_died: false,
    };
    ctrl.record_create_context(context, 0, &[record::CS::ALL_CLIENTS.into()], &[range])
        .ok()?
        .check()
        .ok()?;
    let (x, y, width, height) =
        qol_gpui::popup_window::window_geometry_session(&window_title)?.bounds()?;
    let frame = (x, y, width as i32, height as i32);
    let ctrl = Arc::new(ctrl);
    let join = thread::spawn(move || {
        let Ok(cookie) = data.record_enable_context(context) else {
            return;
        };
        for item in cookie {
            let Ok(reply) = item else {
                break;
            };
            if reply.client_swapped || reply.category != RECORD_FROM_SERVER {
                continue;
            }
            if record_has_outside_left_click(&reply.data, frame) && tx.send(()).is_err() {
                break;
            }
        }
    });
    Some(Monitor {
        ctrl,
        context,
        join: Some(join),
    })
}

fn record_has_outside_left_click(data: &[u8], frame: Frame) -> bool {
    let mut remaining = data;
    while remaining.len() >= EVENT_HEADER_BYTES {
        if remaining[0] == xproto::BUTTON_PRESS_EVENT {
            let (event, rest) = match xproto::ButtonPressEvent::try_parse(remaining) {
                Ok(parsed) => parsed,
                Err(_) => break,
            };
            if event.detail == LEFT_CLICK
                && click_is_outside(frame, (i32::from(event.root_x), i32::from(event.root_y)))
            {
                return true;
            }
            remaining = rest;
        } else if remaining[0] == 0 {
            let Ok((length, _)) = u32::try_parse(&remaining[4..]) else {
                break;
            };
            let length = length as usize * 4 + EVENT_HEADER_BYTES;
            if length > remaining.len() {
                break;
            }
            remaining = &remaining[length..];
        } else {
            remaining = &remaining[EVENT_HEADER_BYTES..];
        }
    }
    false
}

fn click_is_outside(frame: Frame, point: (i32, i32)) -> bool {
    let (x, y, width, height) = frame;
    let (px, py) = point;
    px < x || px > x + width || py < y || py > y + height
}

#[cfg(test)]
mod tests {
    use super::{click_is_outside, record_has_outside_left_click, EVENT_HEADER_BYTES, LEFT_CLICK};

    fn sample_frame() -> (i32, i32, i32, i32) {
        (710, 559, 500, 298)
    }

    fn button_press_bytes(detail: u8, root_x: i16, root_y: i16) -> Vec<u8> {
        let mut bytes = vec![0u8; EVENT_HEADER_BYTES];
        bytes[0] = x11rb::protocol::xproto::BUTTON_PRESS_EVENT;
        bytes[1] = detail;
        bytes[20..22].copy_from_slice(&root_x.to_ne_bytes());
        bytes[22..24].copy_from_slice(&root_y.to_ne_bytes());
        bytes
    }

    #[test]
    fn outside_left_click_is_reported() {
        let bytes = button_press_bytes(LEFT_CLICK, 100, 100);
        assert!(record_has_outside_left_click(&bytes, sample_frame()));
    }

    #[test]
    fn click_on_launcher_window_is_ignored() {
        let bytes = button_press_bytes(LEFT_CLICK, 900, 700);
        assert!(!record_has_outside_left_click(&bytes, sample_frame()));
    }

    #[test]
    fn non_left_clicks_are_ignored() {
        let bytes = button_press_bytes(3, 100, 100);
        assert!(!record_has_outside_left_click(&bytes, sample_frame()));
    }

    #[test]
    fn multiple_events_in_one_record_are_scanned() {
        let mut bytes = button_press_bytes(LEFT_CLICK, 900, 700);
        bytes.extend(button_press_bytes(LEFT_CLICK, 100, 100));
        assert!(record_has_outside_left_click(&bytes, sample_frame()));
    }

    #[test]
    fn truncated_record_is_ignored() {
        let bytes = button_press_bytes(LEFT_CLICK, 100, 100);
        assert!(!record_has_outside_left_click(&bytes[..12], sample_frame()));
    }

    #[test]
    fn point_classification_respects_frame_edges() {
        let cases = [
            ((710, 559, 500, 298), (710, 559), false),
            ((710, 559, 500, 298), (1210, 857), false),
            ((710, 559, 500, 298), (709, 700), true),
            ((710, 559, 500, 298), (1211, 700), true),
            ((710, 559, 500, 298), (900, 558), true),
            ((710, 559, 500, 298), (900, 858), true),
        ];
        for (frame, point, expected) in cases {
            assert_eq!(
                click_is_outside(frame, point),
                expected,
                "frame={frame:?} point={point:?}"
            );
        }
    }
}
