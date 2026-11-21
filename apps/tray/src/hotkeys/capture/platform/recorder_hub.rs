use crate::daemon::{DaemonEvent, EventBus};
use qol_sync::Mutex;
use std::sync::Arc;

struct ActiveRecording {
    session_id: u64,
    events: Arc<EventBus>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Ignore,
    Cancel,
    Complete(String),
}

pub(super) struct RecorderHub {
    backend: &'static str,
    alive: fn() -> bool,
    active: Mutex<Option<ActiveRecording>>,
}

impl RecorderHub {
    pub(super) const fn new(backend: &'static str, alive: fn() -> bool) -> Self {
        Self {
            backend,
            alive,
            active: Mutex::new(None),
        }
    }

    pub(super) fn start(&self, session_id: u64, events: Arc<EventBus>) -> bool {
        if !(self.alive)() {
            return false;
        }
        *self.active.lock() = Some(ActiveRecording { session_id, events });
        qol_runtime::probe!(
            "HOTKEY_RECORD_ARM",
            "session={session_id} backend={}",
            self.backend
        );
        true
    }

    pub(super) fn cancel(&self, session_id: u64) {
        let mut active = self.active.lock();
        if active
            .as_ref()
            .is_none_or(|recording| recording.session_id != session_id)
        {
            return;
        }
        *active = None;
        qol_runtime::probe!("HOTKEY_RECORD_CANCEL", "session={session_id}");
    }

    pub(super) fn recording(&self) -> bool {
        self.active.lock().is_some()
    }

    pub(super) fn settle(&self, decision: Decision) {
        let mut active = self.active.lock();
        let Some(recording) = active.as_ref() else {
            return;
        };
        match decision {
            Decision::Ignore => {}
            Decision::Cancel => {
                let (session_id, events) = (recording.session_id, recording.events.clone());
                *active = None;
                events.send(DaemonEvent::HotkeyRecordingCanceled { session_id });
                qol_runtime::probe!("HOTKEY_RECORD_CANCEL", "session={session_id} source=escape");
            }
            Decision::Complete(key) => {
                let (session_id, events) = (recording.session_id, recording.events.clone());
                *active = None;
                qol_runtime::probe!(
                    "HOTKEY_RECORD_COMPLETE",
                    "session={session_id} key={key} backend={}",
                    self.backend
                );
                events.send(DaemonEvent::HotkeyRecorded { session_id, key });
            }
        }
    }
}
