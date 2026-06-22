use std::ptr::NonNull;
use std::sync::mpsc;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSWindow};
use objc2_foundation::MainThreadMarker;

use super::super::{click_is_outside, ScreenFrame, ScreenPoint};

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
