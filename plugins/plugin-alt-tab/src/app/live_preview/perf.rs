use crate::platform;
use std::collections::HashSet;
use std::time::Instant;

/// Total process CPU time (user + system) in microseconds via getrusage(RUSAGE_SELF).
fn process_cpu_us() -> u64 {
    #[repr(C)]
    struct Timeval { tv_sec: i64, tv_usec: i32 }
    #[repr(C)]
    struct Rusage { ru_utime: Timeval, ru_stime: Timeval, _pad: [u8; 200] }
    extern "C" { fn getrusage(who: i32, usage: *mut Rusage) -> i32; }
    let mut ru = Rusage { ru_utime: Timeval { tv_sec: 0, tv_usec: 0 }, ru_stime: Timeval { tv_sec: 0, tv_usec: 0 }, _pad: [0; 200] };
    unsafe { getrusage(0 /* RUSAGE_SELF */, &mut ru) };
    (ru.ru_utime.tv_sec as u64) * 1_000_000 + (ru.ru_utime.tv_usec as u64)
        + (ru.ru_stime.tv_sec as u64) * 1_000_000 + (ru.ru_stime.tv_usec as u64)
}

pub(super) struct PerfCounters {
    t: Instant,
    cpu: u64,
    ticks: u32,
    frames: u32,
    notify: u32,
    skip: u32,
    wids: HashSet<u32>,
}

impl PerfCounters {
    pub(super) fn new() -> Self {
        Self {
            t: Instant::now(),
            cpu: process_cpu_us(),
            ticks: 0, frames: 0, notify: 0, skip: 0,
            wids: HashSet::new(),
        }
    }

    pub(super) fn tick(&mut self) {
        self.ticks += 1;
        let elapsed = self.t.elapsed();
        if elapsed.as_secs() < 2 { return; }
        let now_cpu = process_cpu_us();
        let cpu_delta = now_cpu.saturating_sub(self.cpu) as f64;
        let wall = elapsed.as_micros() as f64;
        let pct = if wall > 0.0 { cpu_delta / wall * 100.0 } else { 0.0 };
        let (cb_calls, cb_stored, cb_null) = platform::sc_callback_stats();
        eprintln!(
            "[alt-tab/sc/perf] {:.1}s cpu={:.1}%: ticks={} frames={} notify={} skip={} wids={} | cb: calls={} stored={} null_img={}",
            elapsed.as_secs_f32(), pct,
            self.ticks, self.frames, self.notify, self.skip, self.wids.len(),
            cb_calls, cb_stored, cb_null,
        );
        self.t = Instant::now();
        self.cpu = now_cpu;
        self.ticks = 0;
        self.frames = 0;
        self.notify = 0;
        self.skip = 0;
        self.wids.clear();
    }

    pub(super) fn add_skip(&mut self) { self.skip += 1; }
    pub(super) fn add_frames(&mut self, n: u32) { self.frames += n; }
    pub(super) fn add_notify(&mut self) { self.notify += 1; }
    pub(super) fn add_frame_wids(&mut self, wids: impl Iterator<Item = u32>) { self.wids.extend(wids); }
}
