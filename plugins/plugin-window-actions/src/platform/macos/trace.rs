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
pub(super) fn timed_unit(op: &'static str, pid: i32, f: impl FnOnce()) {
    #[cfg(not(debug_assertions))]
    let _ = (op, pid);
    #[cfg(debug_assertions)]
    let start = std::time::Instant::now();
    f();
    #[cfg(debug_assertions)]
    emit(op, pid, start, "done");
}

#[cfg(debug_assertions)]
fn emit(op: &str, pid: i32, start: std::time::Instant, outcome: &str) {
    qol_runtime::probe!(
        "WINACT_AX",
        "op={op} pid={pid} dur_ms={} outcome={outcome}",
        start.elapsed().as_millis()
    );
}
