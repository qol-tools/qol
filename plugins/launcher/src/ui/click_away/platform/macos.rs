use std::ptr::NonNull;
use std::sync::mpsc;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSWindow};
use objc2_foundation::MainThreadMarker;

#[derive(Clone, Copy)]
struct ScreenPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct ScreenFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn click_is_outside(frame: ScreenFrame, point: ScreenPoint) -> bool {
    point.x < frame.x
        || point.x > frame.x + frame.width
        || point.y < frame.y
        || point.y > frame.y + frame.height
}

pub(crate) struct Monitor {
    token: Retained<AnyObject>,
    _block: RcBlock<dyn Fn(NonNull<NSEvent>)>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            NSEvent::removeMonitor(&self.token);
        }
    }
}

pub(crate) fn start(window_title: String, tx: mpsc::Sender<()>) -> Option<Monitor> {
    let block = RcBlock::new(move |_event: NonNull<NSEvent>| {
        if click_is_outside_window(&window_title) {
            let _ = tx.send(());
        }
    });
    let block_ref: &DynBlock<dyn Fn(NonNull<NSEvent>)> = &block;
    let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown,
        block_ref,
    )?;
    Some(Monitor {
        token,
        _block: block,
    })
}

fn click_is_outside_window(title: &str) -> bool {
    let Some(window) = find_visible_window(title) else {
        return false;
    };
    let frame = window.frame();
    let point = NSEvent::mouseLocation();
    click_is_outside(
        ScreenFrame {
            x: frame.origin.x,
            y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
        },
        ScreenPoint {
            x: point.x,
            y: point.y,
        },
    )
}

fn find_visible_window(title: &str) -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    app.windows()
        .iter()
        .filter(|win| {
            win.title().to_string() == title && win.alphaValue() > 0.0 && !win.ignoresMouseEvents()
        })
        .last()
}

#[cfg(test)]
mod tests {
    use super::{click_is_outside, ScreenFrame, ScreenPoint};

    fn sample_frame() -> ScreenFrame {
        ScreenFrame {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        }
    }

    #[test]
    fn point_classification_respects_frame_edges() {
        let cases = [
            (10.0, 20.0, false),
            (60.0, 45.0, false),
            (110.0, 70.0, false),
            (9.9, 30.0, true),
            (200.0, 30.0, true),
            (30.0, 19.9, true),
            (30.0, 70.1, true),
        ];
        for (x, y, expected) in cases {
            assert_eq!(
                click_is_outside(sample_frame(), ScreenPoint { x, y }),
                expected,
                "point=({x}, {y})"
            );
        }
    }
}
