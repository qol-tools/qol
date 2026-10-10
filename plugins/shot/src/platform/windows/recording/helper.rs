use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use super::audio;
use super::plan::CapturePlan;

pub(super) const HELPER_ENV: &str = "QOL_SHOT_WINDOWS_CAPTURE_REQUEST";
const COMPAT_LAYER_ENV: &str = "__COMPAT_LAYER";
const COMPAT_LAYER_DPI_AWARE: &str = "HighDpiAware";
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const QUIT_GRACE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum Step {
    Wait,
    Quit,
    Kill,
    Exit,
}

#[derive(Debug)]
pub(super) struct Lifecycle {
    grace: Duration,
    quit_sent: Option<Instant>,
    killed: bool,
}

impl Lifecycle {
    pub(super) fn new(grace: Duration) -> Self {
        Self {
            grace,
            quit_sent: None,
            killed: false,
        }
    }

    pub(super) fn next(&mut self, now: Instant, stop_requested: bool, exited: bool) -> Step {
        if exited {
            return Step::Exit;
        }
        match self.quit_sent {
            None if stop_requested => {
                self.quit_sent = Some(now);
                Step::Quit
            }
            None => Step::Wait,
            Some(sent) if !self.killed && now.saturating_duration_since(sent) >= self.grace => {
                self.killed = true;
                Step::Kill
            }
            Some(_) => Step::Wait,
        }
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
    if let Err(error) = qol_process::bind_to_host_lifetime(child.id()) {
        log::warn!("ffmpeg will not stop with the capture helper: {error}");
    }
    supervise(&mut child, &stop, audio.as_ref())
}

fn supervise(
    child: &mut Child,
    stop: &qol_process::CancellationToken,
    audio: Option<&audio::AudioFeed>,
) -> Result<()> {
    let mut lifecycle = Lifecycle::new(QUIT_GRACE);
    loop {
        let status = child.try_wait().context("failed to inspect ffmpeg")?;
        match lifecycle.next(Instant::now(), stop.is_cancelled(), status.is_some()) {
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
    use super::{Lifecycle, Step};
    use std::time::{Duration, Instant};

    #[test]
    fn lifecycle_quits_once_then_kills_after_the_grace_period() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut lifecycle = Lifecycle::new(Duration::from_secs(10));
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
            assert_eq!(lifecycle.next(now, stop, exited), expected, "{now:?}");
        }
    }

    #[test]
    fn lifecycle_exits_when_ffmpeg_ends_on_its_own() {
        let now = Instant::now();
        let cases = [(false, true), (true, true)];
        for (stop, exited) in cases {
            let mut lifecycle = Lifecycle::new(Duration::from_secs(10));
            assert_eq!(lifecycle.next(now, stop, exited), Step::Exit);
        }
    }
}
