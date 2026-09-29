use gpui::{div, point, px, size, Context, Render, TestAppContext, Window};
use std::sync::atomic::{AtomicUsize, Ordering};

struct ErrorCounter(AtomicUsize);

static ERRORS: ErrorCounter = ErrorCounter(AtomicUsize::new(0));

impl log::Log for ErrorCounter {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() == log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {}
}

#[derive(Default)]
struct BoundsObserver {
    updates: usize,
}

impl Render for BoundsObserver {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
    }
}

fn observed_window(cx: &mut TestAppContext) -> gpui::WindowHandle<BoundsObserver> {
    cx.add_window(|window, cx| {
        cx.observe_window_bounds(window, |this: &mut BoundsObserver, _, _| {
            this.updates += 1;
        })
        .detach();
        BoundsObserver::default()
    })
}

#[gpui::test]
fn resize_and_move_during_update_are_coalesced_and_applied(cx: &mut TestAppContext) {
    let handle = observed_window(cx);
    let mut platform = cx.test_window(handle.into());
    cx.run_until_parked();
    for move_only in [false, true] {
        let before = cx.read(|cx| handle.read(cx).unwrap().updates);
        handle
            .update(cx, |_, _, _| {
                if !move_only {
                    platform.simulate_resize(size(px(420.), px(240.)));
                    platform.simulate_resize(size(px(640.), px(480.)));
                }
                platform.simulate_move(point(px(80.), px(90.)));
                platform.simulate_move(point(px(120.), px(150.)));
            })
            .unwrap();
        assert_eq!(cx.read(|cx| handle.read(cx).unwrap().updates), before);
        cx.run_until_parked();
        handle
            .update(cx, |view, window, _| {
                assert_eq!(view.updates, before + 1, "move_only={move_only}");
                assert_eq!(window.viewport_size(), size(px(640.), px(480.)));
                assert_eq!(window.bounds().origin, point(px(120.), px(150.)));
            })
            .unwrap();
    }
    platform.simulate_resize(size(px(320.), px(200.)));
    handle
        .update(cx, |view, window, _| {
            assert_eq!(view.updates, 3);
            assert_eq!(window.viewport_size(), size(px(320.), px(200.)));
        })
        .unwrap();
}

#[gpui::test]
fn closing_window_before_deferred_bounds_update_is_safe(cx: &mut TestAppContext) {
    let handle = observed_window(cx);
    let mut platform = cx.test_window(handle.into());
    cx.run_until_parked();
    log::set_logger(&ERRORS).unwrap();
    log::set_max_level(log::LevelFilter::Error);
    handle
        .update(cx, |_, window, _| {
            platform.simulate_resize(size(px(420.), px(240.)));
            platform.simulate_move(point(px(80.), px(90.)));
            window.remove_window();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(handle.update(cx, |_, _, _| ()).is_err());
    assert_eq!(ERRORS.0.load(Ordering::Relaxed), 0);
}
