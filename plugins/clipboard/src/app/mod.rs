use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, SocketSource};

use crate::capture::{self, Clipboard, SecretEcho};
use crate::history::{now_ms, Clip, History, Recorded, SecretPolicy, Store};

pub mod detail;
pub mod host;
pub mod request;
pub mod rows;

pub const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const SWEEP_INTERVAL: Duration = Duration::from_secs(1);
const NO_SECRET: u64 = u64::MAX;

pub struct Daemon {
    history: History,
    clipboard: Option<Arc<dyn Clipboard>>,
    paused: Arc<Mutex<Option<String>>>,
}

impl Daemon {
    pub(crate) fn history(&self) -> &History {
        &self.history
    }

    pub(crate) fn clipboard(&self) -> Result<&Arc<dyn Clipboard>> {
        match (&self.clipboard, self.paused_reason()) {
            (Some(clipboard), None) => Ok(clipboard),
            (_, reason) => anyhow::bail!(
                "capture is paused: {}",
                reason.unwrap_or_else(|| "no clipboard backend".to_string())
            ),
        }
    }

    pub(crate) fn paused_reason(&self) -> Option<String> {
        lock(&self.paused).clone()
    }
}

pub fn run_daemon() -> Result<()> {
    let history = History::new(Store::resolve()?);
    let retention = crate::config::load().retention();
    let live = history.live(now_ms(), retention)?;
    let secret_deadline = Arc::new(AtomicU64::new(next_secret_deadline(
        &live,
        retention.secret_ttl,
    )));
    let paused = Arc::new(Mutex::new(None));
    let clipboard = match capture::open() {
        Ok(clipboard) => {
            spawn_watcher(
                Arc::clone(&clipboard),
                history.clone(),
                Arc::clone(&paused),
                Arc::clone(&secret_deadline),
            )?;
            log::info!("clipboard capture running on {}", clipboard.backend());
            Some(clipboard)
        }
        Err(error) => {
            log::warn!("{error}");
            *lock(&paused) = Some(error.to_string());
            None
        }
    };
    spawn_sweeper(history.clone(), secret_deadline)?;
    let daemon = Daemon {
        history,
        clipboard,
        paused,
    };
    core_daemon::run_stateful_request_listener(&DAEMON_CONFIG, daemon, request::handle)
        .context("qol-clipboard daemon listener failed")
}

fn spawn_watcher(
    clipboard: Arc<dyn Clipboard>,
    history: History,
    paused: Arc<Mutex<Option<String>>>,
    secret_deadline: Arc<AtomicU64>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("qol-clipboard-watch".to_owned())
        .spawn(move || {
            let mut echo = SecretEcho::default();
            loop {
                let capture = match clipboard.next_change() {
                    Ok(capture) => capture,
                    Err(error) => {
                        log::error!("clipboard capture stopped: {error}");
                        qol_runtime::probe!("QOL_CLIPBOARD", "event=capture_stopped error={error}");
                        *lock(&paused) = Some(error.to_string());
                        return;
                    }
                };
                if echo.repeats_secret(&capture) {
                    qol_runtime::probe!("QOL_CLIPBOARD", "event=dropped reason=secret_echo");
                    continue;
                }
                let config = crate::config::load();
                let now = now_ms();
                let policy = SecretPolicy {
                    keep: config.keep_secrets,
                };
                match history.record(capture, now, config.retention(), policy) {
                    Ok(Recorded::Added(clip)) | Ok(Recorded::Refreshed(clip)) => {
                        qol_runtime::probe!(
                            "QOL_CLIPBOARD",
                            "event=captured kind={} bytes={}",
                            clip.kind.id(),
                            clip.bytes
                        );
                        if clip.is_secret() {
                            let expires = now
                                .saturating_add(config.retention().secret_ttl.as_millis() as u64);
                            secret_deadline.fetch_min(expires, Ordering::SeqCst);
                        }
                    }
                    Ok(Recorded::Dropped(reason)) => {
                        qol_runtime::probe!("QOL_CLIPBOARD", "event=dropped reason={reason}");
                    }
                    Err(error) => log::warn!("failed to record a clip: {error:#}"),
                }
            }
        })
        .context("failed to start the clipboard watcher")?;
    Ok(())
}

fn spawn_sweeper(history: History, secret_deadline: Arc<AtomicU64>) -> Result<()> {
    std::thread::Builder::new()
        .name("qol-clipboard-sweep".to_owned())
        .spawn(move || loop {
            std::thread::sleep(SWEEP_INTERVAL);
            let now = now_ms();
            if secret_deadline.load(Ordering::SeqCst) > now {
                continue;
            }
            let retention = crate::config::load().retention();
            // Stored under the history lock, so a secret recorded after this sweep read the
            // history still lowers the deadline afterwards.
            let swept = history.with_live(now, retention, |live| {
                secret_deadline.store(
                    next_secret_deadline(live, retention.secret_ttl),
                    Ordering::SeqCst,
                )
            });
            match swept {
                Ok(()) => {}
                Err(error) => {
                    log::warn!("failed to clear expired secrets: {error:#}");
                    secret_deadline
                        .store(now + SWEEP_INTERVAL.as_millis() as u64, Ordering::SeqCst);
                }
            }
        })
        .context("failed to start the clipboard sweeper")?;
    Ok(())
}

fn next_secret_deadline(clips: &[Clip], secret_ttl: Duration) -> u64 {
    clips
        .iter()
        .filter(|clip| clip.is_secret())
        .map(|clip| {
            clip.copied_at_ms
                .saturating_add(secret_ttl.as_millis() as u64)
        })
        .min()
        .unwrap_or(NO_SECRET)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::ClipKind;

    fn clip(kind: ClipKind, copied_at_ms: u64) -> Clip {
        Clip {
            id: format!("c_{copied_at_ms}"),
            kind,
            text: Some("x".to_string()),
            image: None,
            bytes: 1,
            app: None,
            copied_at_ms,
            pinned: false,
        }
    }

    #[test]
    fn the_sweeper_wakes_for_the_earliest_secret() {
        let ttl = Duration::from_secs(30);
        let cases = [
            (vec![], NO_SECRET),
            (vec![clip(ClipKind::Text, 1_000)], NO_SECRET),
            (
                vec![clip(ClipKind::Secret, 5_000), clip(ClipKind::Secret, 2_000)],
                32_000,
            ),
            (
                vec![clip(ClipKind::Code, 0), clip(ClipKind::Secret, 9_000)],
                39_000,
            ),
        ];
        for (clips, deadline) in cases {
            assert_eq!(next_secret_deadline(&clips, ttl), deadline);
        }
    }
}
