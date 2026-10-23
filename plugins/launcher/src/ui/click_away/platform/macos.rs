use std::ptr::NonNull;
use std::sync::mpsc;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask};
use objc2_foundation::MainThreadMarker;

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
    window_is_shown(title) && !qol_gpui::popup_window::pointer_over_window_by_title(title)
}

fn window_is_shown(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .any(|win| win.title().to_string() == title && win.isVisible() && win.alphaValue() > 0.0)
}
