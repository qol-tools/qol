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
