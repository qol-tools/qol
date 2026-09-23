use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::turn::{Observation, SessionId, TurnId};
use crate::voice_session::VoiceSessionCause;
use crate::voice_session::VoiceSessionUpdate;

const EVENT_CAPACITY: usize = 256;
const TRANSCRIPT_CAPACITY: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub cursor: u64,
    pub update: VoiceSessionUpdate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventPage {
    pub events: Vec<SessionEvent>,
    pub next_cursor: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TranscriptItem {
    id: String,
    text: String,
    detail: String,
    accent: &'static str,
}

#[derive(Clone)]
pub(super) struct SessionEventLog {
    inner: Arc<Mutex<EventJournal>>,
}

struct EventJournal {
    capacity: usize,
    next_cursor: u64,
    events: VecDeque<SessionEvent>,
}

impl Default for SessionEventLog {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventJournal::new(EVENT_CAPACITY))),
        }
    }
}

impl SessionEventLog {
    pub(super) fn record(&self, update: &VoiceSessionUpdate) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("voice-session event log is unavailable"))?
            .record(update)
    }

    pub(super) fn page(&self, after: u64) -> Result<SessionEventPage> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("voice-session event log is unavailable"))?
            .page(after))
    }

    pub(super) fn transcripts(&self) -> Result<Vec<TranscriptItem>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| anyhow!("voice-session event log is unavailable"))?
            .transcripts())
    }
}

impl EventJournal {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next_cursor: 1,
            events: VecDeque::with_capacity(capacity),
        }
    }

    fn record(&mut self, update: &VoiceSessionUpdate) -> Result<()> {
        let cursor = self.next_cursor;
        self.next_cursor = self
            .next_cursor
            .checked_add(1)
            .ok_or_else(|| anyhow!("voice-session event cursor space is exhausted"))?;
        self.events.push_back(SessionEvent {
            cursor,
            update: update.clone(),
        });
        if self.events.len() > self.capacity {
            self.events.pop_front();
        }
        Ok(())
    }

    fn page(&self, after: u64) -> SessionEventPage {
        let oldest = self
            .events
            .front()
            .map_or(self.next_cursor, |event| event.cursor);
        let events = self
            .events
            .iter()
            .filter(|event| event.cursor > after)
            .cloned()
            .collect();
        SessionEventPage {
            events,
            next_cursor: self.next_cursor.saturating_sub(1),
            truncated: after.saturating_add(1) < oldest,
        }
    }

    fn transcripts(&self) -> Vec<TranscriptItem> {
        let mut turns = HashSet::<(SessionId, TurnId)>::new();
        self.events
            .iter()
            .rev()
            .filter_map(transcript_observation)
            .filter(|(key, _)| turns.insert(*key))
            .map(|(_, item)| item)
            .take(TRANSCRIPT_CAPACITY)
            .collect()
    }
}

fn transcript_observation(event: &SessionEvent) -> Option<((SessionId, TurnId), TranscriptItem)> {
    let VoiceSessionCause::Observation(envelope) = &event.update.cause else {
        return None;
    };
    let Observation::TranscriptHypothesis {
        turn_id,
        text,
        final_result,
        ..
    } = &envelope.observation
    else {
        return None;
    };
    let text = match text.trim() {
        "" => "(No speech recognized)",
        value => value,
    };
    let detail = if *final_result {
        elapsed_label(envelope.observed_at_ms)
    } else {
        format!("Transcribing · {}", elapsed_label(envelope.observed_at_ms))
    };
    let accent = if *final_result { "success" } else { "accent" };
    Some((
        (envelope.session_id, *turn_id),
        TranscriptItem {
            id: format!("{}-{}", envelope.session_id.0, turn_id.0),
            text: text.to_owned(),
            detail,
            accent,
        },
    ))
}

fn elapsed_label(milliseconds: u64) -> String {
    format!("{:.1}s", milliseconds as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use crate::turn::{
        EffectBatch, Observation, ObservationEnvelope, ResponsiveTurnPolicy, SessionId,
        TurnCoordinator,
    };
    use crate::voice_session::{VoiceSessionCause, VoiceSessionEvidence, VoiceSessionUpdate};

    use super::{EventJournal, TranscriptItem};

    #[test]
    fn bounded_pages_report_replay_gaps_and_monotonic_cursors() {
        let mut journal = EventJournal::new(2);
        for sequence in 1..=3 {
            journal.record(&update(sequence)).unwrap();
        }

        let page = journal.page(0);

        assert!(page.truncated);
        assert_eq!(page.next_cursor, 3);
        assert_eq!(
            page.events
                .iter()
                .map(|event| event.cursor)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(journal.page(3).events.len(), 0);
    }

    #[test]
    fn transcripts_keep_the_latest_hypothesis_per_turn_in_recent_first_order() {
        let mut journal = EventJournal::new(8);
        journal
            .record(&transcript_update(1, 1, 1, 1000, "hel", false))
            .unwrap();
        journal
            .record(&transcript_update(2, 1, 1, 1600, "hello", true))
            .unwrap();
        journal
            .record(&transcript_update(3, 1, 2, 2400, "next", false))
            .unwrap();

        let transcripts = journal.transcripts();
        assert_eq!(
            transcripts,
            vec![
                TranscriptItem {
                    id: "1-2".into(),
                    text: "next".into(),
                    detail: "Transcribing · 2.4s".into(),
                    accent: "accent",
                },
                TranscriptItem {
                    id: "1-1".into(),
                    text: "hello".into(),
                    detail: "1.6s".into(),
                    accent: "success",
                },
            ]
        );
        assert_eq!(
            serde_json::to_value(transcripts).unwrap(),
            serde_json::json!([
                {
                    "id": "1-2",
                    "text": "next",
                    "detail": "Transcribing · 2.4s",
                    "accent": "accent"
                },
                {
                    "id": "1-1",
                    "text": "hello",
                    "detail": "1.6s",
                    "accent": "success"
                }
            ])
        );
    }

    fn update(sequence: u64) -> VoiceSessionUpdate {
        let session_id = SessionId(1);
        let envelope = ObservationEnvelope {
            session_id,
            sequence,
            observed_at_ms: sequence,
            observation: Observation::VoiceActivityStarted {
                turn_id: crate::turn::TurnId(sequence),
            },
        };
        let mut coordinator = TurnCoordinator::new(session_id, ResponsiveTurnPolicy::default());
        coordinator.observe(envelope.clone()).unwrap();
        VoiceSessionUpdate {
            cause: VoiceSessionCause::Observation(envelope),
            evidence: VoiceSessionEvidence::default(),
            snapshot: coordinator.snapshot(),
            effects: EffectBatch::new(session_id, sequence, Vec::new()),
        }
    }

    fn transcript_update(
        sequence: u64,
        session_id: u64,
        turn_id: u64,
        observed_at_ms: u64,
        text: &str,
        final_result: bool,
    ) -> VoiceSessionUpdate {
        let session_id = SessionId(session_id);
        let envelope = ObservationEnvelope {
            session_id,
            sequence,
            observed_at_ms,
            observation: Observation::TranscriptHypothesis {
                turn_id: crate::turn::TurnId(turn_id),
                text: text.to_owned(),
                confidence_permille: None,
                final_result,
            },
        };
        let mut coordinator = TurnCoordinator::new(session_id, ResponsiveTurnPolicy::default());
        coordinator.observe(envelope.clone()).unwrap();
        VoiceSessionUpdate {
            cause: VoiceSessionCause::Observation(envelope),
            evidence: VoiceSessionEvidence::default(),
            snapshot: coordinator.snapshot(),
            effects: EffectBatch::new(session_id, sequence, Vec::new()),
        }
    }
}
