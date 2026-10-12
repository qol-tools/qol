use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime};

use super::audio;
use super::plan::CapturePlan;

pub(super) const HELPER_ENV: &str = "QOL_SHOT_WINDOWS_CAPTURE_REQUEST";
const COMPAT_LAYER_ENV: &str = "__COMPAT_LAYER";
const COMPAT_LAYER_DPI_AWARE: &str = "HighDpiAware";
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const FINALIZE_STALL_GRACE: Duration = Duration::from_secs(15);
const FINALIZE_CAP: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum Step {
    Wait,
    Quit,
    Kill,
    Exit,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(super) struct Progress {
    pub(super) len: u64,
    pub(super) modified: Option<SystemTime>,
}

impl Progress {
    fn of(path: &Path) -> Self {
        path.metadata()
            .map(|metadata| Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            })
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub(super) struct Lifecycle {
    stall_grace: Duration,
    finalize_cap: Duration,
    quit_sent: Option<Instant>,
    last_progress: Option<(Progress, Instant)>,
    killed: bool,
}

impl Lifecycle {
    pub(super) fn new(stall_grace: Duration, finalize_cap: Duration) -> Self {
        Self {
            stall_grace,
            finalize_cap,
            quit_sent: None,
            last_progress: None,
            killed: false,
        }
    }

    pub(super) fn next(
        &mut self,
        now: Instant,
        stop_requested: bool,
        exited: bool,
        progress: Progress,
    ) -> Step {
        if exited {
            return Step::Exit;
        }
        let Some(sent) = self.quit_sent else {
            if !stop_requested {
                return Step::Wait;
            }
            self.quit_sent = Some(now);
            self.last_progress = Some((progress, now));
            return Step::Quit;
        };
        if self.killed {
            return Step::Wait;
        }
        let since_progress = match self.last_progress {
            Some((last, at)) if last == progress => now.saturating_duration_since(at),
            _ => {
                self.last_progress = Some((progress, now));
                Duration::ZERO
            }
        };
        let finalizing = now.saturating_duration_since(sent);
        if since_progress >= self.stall_grace || finalizing >= self.finalize_cap {
            self.killed = true;
            return Step::Kill;
        }
        Step::Wait
    }
}

pub fn run_internal_capture_helper() -> Option<ExitCode> {
    let request = std::env::var(HELPER_ENV).ok()?;
    Some(match run(&request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("Windows capture helper failed: {error:#}");
            ExitCode::FAILURE
        }
    })
}

fn run(request: &str) -> Result<()> {
    let plan: CapturePlan =
        serde_json::from_str(request).context("invalid Windows capture request")?;
    let stop = qol_process::CancellationToken::install()
        .context("failed to listen for the recording stop request")?;
    let audio = audio::start(&plan.audio)?;
    let args = plan.ffmpeg_args(audio.as_ref().map(audio::AudioFeed::url));
    qol_runtime::probe!(
        "SHOT_RECORD_START_PLAN",
        "backend=gdigrab rect={}x{}+{},{} fps={} audio_sources={} audio_feed={}",
        plan.rect.w,
        plan.rect.h,
        plan.rect.x,
        plan.rect.y,
        plan.framerate,
        plan.audio.len(),
        audio.is_some()
    );
    let mut command = Command::new(&plan.ffmpeg);
    let mut child = qol_process::hide_console_window(&mut command)
        .args(&args)
        .env(COMPAT_LAYER_ENV, COMPAT_LAYER_DPI_AWARE)
        .env_remove(HELPER_ENV)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {}", plan.ffmpeg.display()))?;
    if let Some(audio) = &audio {
        audio.admit(child.id());
    }
    if let Err(error) = qol_process::bind_to_host_lifetime(child.id()) {
        log::warn!("ffmpeg will not stop with the capture helper: {error}");
    }
    supervise(&mut child, &stop, audio.as_ref(), &plan.output)
}

fn supervise(
    child: &mut Child,
    stop: &qol_process::CancellationToken,
    audio: Option<&audio::AudioFeed>,
    output: &Path,
) -> Result<()> {
    let mut lifecycle = Lifecycle::new(FINALIZE_STALL_GRACE, FINALIZE_CAP);
    loop {
        let status = child.try_wait().context("failed to inspect ffmpeg")?;
        match lifecycle.next(
            Instant::now(),
            stop.is_cancelled(),
            status.is_some(),
            Progress::of(output),
        ) {
            Step::Wait => std::thread::sleep(POLL_INTERVAL),
            Step::Quit => {
                qol_runtime::probe!("SHOT_RECORD_HELPER", "step=quit");
                send_quit(child);
                if let Some(audio) = audio {
                    audio.stop();
                }
            }
            Step::Kill => {
                qol_runtime::probe!("SHOT_RECORD_HELPER", "step=kill");
                let _ = child.kill();
            }
            Step::Exit => {
                let status = status.ok_or_else(|| anyhow!("ffmpeg exit status is unknown"))?;
                qol_runtime::probe!("SHOT_RECORD_HELPER", "step=exit status={status}");
                if stop.is_cancelled() || status.success() {
                    return Ok(());
                }
                return Err(anyhow!("ffmpeg exited with {status}"));
            }
        }
    }
}

fn send_quit(child: &mut Child) {
    let Some(mut stdin) = child.stdin.take() else {
        return;
    };
    if let Err(error) = stdin.write_all(b"q").and_then(|()| stdin.flush()) {
        log::warn!("failed to ask ffmpeg to finish: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{Lifecycle, Progress, Step};
    use std::time::{Duration, Instant, SystemTime};

    fn written(len: u64) -> Progress {
        Progress {
            len,
            modified: None,
        }
    }

    #[test]
    fn lifecycle_quits_once_then_kills_after_output_stalls_for_the_grace_period() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut lifecycle = Lifecycle::new(Duration::from_secs(10), Duration::from_secs(300));
        let cases = [
            (at(0), false, false, Step::Wait),
            (at(1), true, false, Step::Quit),
            (at(2), true, false, Step::Wait),
            (at(10), true, false, Step::Wait),
            (at(11), true, false, Step::Kill),
            (at(12), true, false, Step::Wait),
            (at(13), true, true, Step::Exit),
        ];
        for (now, stop, exited, expected) in cases {
            assert_eq!(
                lifecycle.next(now, stop, exited, written(7)),
                expected,
                "{now:?}"
            );
        }
    }

    #[test]
    fn lifecycle_keeps_waiting_while_ffmpeg_still_writes_the_file() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let touched = |seconds: u64| Progress {
            len: 7,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
        };
        let mut lifecycle = Lifecycle::new(Duration::from_secs(10), Duration::from_secs(300));
        let cases = [
            (at(0), written(7), Step::Quit),
            (at(9), written(9), Step::Wait),
            (at(18), written(12), Step::Wait),
            (at(27), touched(27), Step::Wait),
            (at(36), touched(27), Step::Wait),
            (at(37), touched(27), Step::Kill),
        ];
        for (now, progress, expected) in cases {
            assert_eq!(
                lifecycle.next(now, true, false, progress),
                expected,
                "{now:?}"
            );
        }
    }

    #[test]
    fn lifecycle_kills_at_the_cap_even_while_the_file_still_grows() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut lifecycle = Lifecycle::new(Duration::from_secs(10), Duration::from_secs(30));
        let cases = [
            (at(0), 1, Step::Quit),
            (at(9), 2, Step::Wait),
            (at(18), 3, Step::Wait),
            (at(27), 4, Step::Wait),
            (at(30), 5, Step::Kill),
            (at(31), 6, Step::Wait),
        ];
        for (now, len, expected) in cases {
            assert_eq!(
                lifecycle.next(now, true, false, written(len)),
                expected,
                "{now:?}"
            );
        }
    }

    #[test]
    fn lifecycle_exits_when_ffmpeg_ends_on_its_own() {
        let now = Instant::now();
        let cases = [(false, true), (true, true)];
        for (stop, exited) in cases {
            let mut lifecycle = Lifecycle::new(Duration::from_secs(10), Duration::from_secs(300));
            assert_eq!(lifecycle.next(now, stop, exited, written(0)), Step::Exit);
        }
    }
}
