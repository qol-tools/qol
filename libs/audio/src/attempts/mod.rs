//! Who may repair an output, and how often.
//!
//! This module picks no numbers of its own: the caller passes its own cooldown
//! and cap, so counting stays bookkeeping and the policy stays with the plugin
//! that owns it.

pub mod lease;
pub mod record;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::AudioError;

#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Default)]
pub struct AudioWatchState {
    attempts: u32,
    last_attempt: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairDecision {
    Repair,
    Cooldown,
    Exhausted,
}

impl AudioWatchState {
    pub fn decide(&self, now: Instant, cooldown: Duration, max_attempts: u32) -> RepairDecision {
        if self.attempts >= max_attempts {
            return RepairDecision::Exhausted;
        }
        if self
            .last_attempt
            .is_some_and(|last| now.saturating_duration_since(last) < cooldown)
        {
            return RepairDecision::Cooldown;
        }
        RepairDecision::Repair
    }

    pub fn attempted(&mut self, now: Instant) {
        self.attempts = self.attempts.saturating_add(1);
        self.last_attempt = Some(now);
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

pub(crate) const STATE_SUBDIR: &str = "sound";

pub(crate) fn state_root() -> Result<PathBuf, AudioError> {
    #[cfg(test)]
    {
        if let Some(root) = current_test_root() {
            return Ok(root);
        }
    }
    qol_config::data_subdir(STATE_SUBDIR).ok_or_else(|| {
        AudioError::Operation("no local data directory is available for sound state".to_string())
    })
}

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
pub(crate) const TEST_ROOT_ENV: &str = "QOL_AUDIO_TEST_STATE_DIR";

#[cfg(test)]
static TEST_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
static TEST_ROOT_EXCLUSIVE: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) struct IsolatedRoot {
    _exclusive: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
}

#[cfg(test)]
impl IsolatedRoot {
    pub(crate) fn new() -> Self {
        let exclusive = TEST_ROOT_EXCLUSIVE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::TempDir::new().expect("an isolated sound state directory");
        *TEST_ROOT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(dir.path().to_path_buf());
        Self {
            _exclusive: exclusive,
            dir,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }
}

#[cfg(test)]
impl Drop for IsolatedRoot {
    fn drop(&mut self) {
        *TEST_ROOT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

#[cfg(test)]
pub(crate) fn current_test_root() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(TEST_ROOT_ENV) {
        return Some(PathBuf::from(value));
    }
    TEST_ROOT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COOLDOWN: Duration = Duration::from_secs(60);

    #[test]
    fn a_fresh_state_repairs() {
        let state = AudioWatchState::default();
        assert_eq!(
            state.decide(Instant::now(), COOLDOWN, 3),
            RepairDecision::Repair
        );
    }

    #[test]
    fn an_attempt_cooldowns_until_the_window_elapses() {
        let now = Instant::now();
        let mut state = AudioWatchState::default();
        state.attempted(now);
        assert_eq!(state.attempts(), 1);
        assert_eq!(
            state.decide(now + COOLDOWN - Duration::from_millis(1), COOLDOWN, 3),
            RepairDecision::Cooldown
        );
        assert_eq!(
            state.decide(now + COOLDOWN, COOLDOWN, 3),
            RepairDecision::Repair
        );
    }

    #[test]
    fn exhausting_the_attempts_cap_beats_elapsed_time() {
        let now = Instant::now();
        let mut state = AudioWatchState::default();
        for _ in 0..3 {
            state.attempted(now);
        }
        assert_eq!(state.attempts(), 3);
        assert_eq!(
            state.decide(now + COOLDOWN * 10, COOLDOWN, 3),
            RepairDecision::Exhausted
        );
    }

    #[test]
    fn zero_max_attempts_is_exhausted_immediately() {
        let state = AudioWatchState::default();
        assert_eq!(
            state.decide(Instant::now(), COOLDOWN, 0),
            RepairDecision::Exhausted
        );
    }
}
