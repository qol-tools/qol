use std::cell::RefCell;
use std::sync::Once;

struct ThreadLogCapture;

static CAPTURE: ThreadLogCapture = ThreadLogCapture;
static INSTALL: Once = Once::new();

thread_local! {
    static CAPTURED: RefCell<Option<(log::Level, Vec<String>)>> = const { RefCell::new(None) };
}

fn captures(level: log::Level) -> bool {
    CAPTURED
        .try_with(|captured| {
            captured
                .borrow()
                .as_ref()
                .is_some_and(|(threshold, _)| level <= *threshold)
        })
        .unwrap_or(false)
}

impl log::Log for ThreadLogCapture {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        captures(metadata.level())
    }

    fn log(&self, record: &log::Record) {
        if !captures(record.level()) {
            return;
        }
        let line = format!("{} {}", record.level(), record.args());
        let _ = CAPTURED.try_with(|captured| {
            if let Some((_, lines)) = captured.borrow_mut().as_mut() {
                lines.push(line);
            }
        });
    }

    fn flush(&self) {}
}

pub(crate) fn capture_logs<T>(threshold: log::Level, run: impl FnOnce() -> T) -> (T, Vec<String>) {
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).expect("unit tests install no other logger");
        log::set_max_level(log::LevelFilter::Trace);
    });
    CAPTURED.with(|captured| *captured.borrow_mut() = Some((threshold, Vec::new())));
    let result = run();
    let lines = CAPTURED.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .map(|(_, lines)| lines)
            .unwrap_or_default()
    });
    (result, lines)
}
