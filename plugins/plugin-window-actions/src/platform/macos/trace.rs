#[inline]
pub(super) fn timed_opt<T>(op: &'static str, pid: i32, f: impl FnOnce() -> Option<T>) -> Option<T> {
    #[cfg(not(debug_assertions))]
    let _ = (op, pid);
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    let result = f();
    #[cfg(debug_assertions)]
    emit(op, pid, start, if result.is_some() { "ok" } else { "miss" });
    result
}

#[inline]
pub(super) fn timed_bool(op: &'static str, pid: i32, f: impl FnOnce() -> bool) -> bool {
    #[cfg(not(debug_assertions))]
    let _ = (op, pid);
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    let result = f();
    #[cfg(debug_assertions)]
    emit(op, pid, start, if result { "ok" } else { "fail" });
    result
}

#[inline]
pub(super) fn timed_pred(op: &'static str, pid: i32, f: impl FnOnce() -> bool) -> bool {
    #[cfg(not(debug_assertions))]
    let _ = (op, pid);
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    let result = f();
    #[cfg(debug_assertions)]
    emit(op, pid, start, if result { "yes" } else { "no" });
    result
}

#[inline]
pub(super) fn timed_pid(
    op: &'static str,
    source: &'static str,
    f: impl FnOnce() -> Option<i32>,
) -> Option<i32> {
    #[cfg(not(debug_assertions))]
    let _ = (op, source);
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    let result = f();
    #[cfg(debug_assertions)]
    emit_pid(op, source, result, start);
    result
}

#[inline]
pub(super) fn trace_geometry(
    pid: i32,
    expected: super::screen::Rect,
    actual: Option<super::screen::Rect>,
    matches: bool,
) {
    #[cfg(not(debug_assertions))]
    let _ = (pid, expected, actual, matches);
    #[cfg(debug_assertions)]
    {
        let actual = actual.unwrap_or(super::screen::Rect {
            x: f64::NAN,
            y: f64::NAN,
            w: f64::NAN,
            h: f64::NAN,
        });
        qol_runtime::probe!(
            "WINACT_AX",
            "op=verify_geometry pid={pid} expected={:.1},{:.1},{:.1},{:.1} actual={:.1},{:.1},{:.1},{:.1} outcome={}",
            expected.x,
            expected.y,
            expected.w,
            expected.h,
            actual.x,
            actual.y,
            actual.w,
            actual.h,
            if matches { "ok" } else { "mismatch" }
        );
    }
}

#[cfg(debug_assertions)]
fn emit(op: &str, pid: i32, start: std::time::Instant, outcome: &str) {
    qol_runtime::probe!(
        "WINACT_AX",
        "op={op} pid={pid} dur_ms={} outcome={outcome}",
        start.elapsed().as_millis()
    );
}

#[cfg(debug_assertions)]
fn emit_pid(op: &str, source: &str, pid: Option<i32>, start: std::time::Instant) {
    qol_runtime::probe!(
        "WINACT_AX",
        "op={op} pid={} source={source} dur_ms={} outcome={}",
        pid.unwrap_or(0),
        start.elapsed().as_millis(),
        if pid.is_some() { "ok" } else { "miss" }
    );
}
