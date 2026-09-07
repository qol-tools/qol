use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::Sender;

use crate::audio::{AudioFormat, AudioFrame};
use crate::transcribe::{
    AudioSubmit, Transcriber, TranscriptionError, TranscriptionEvent, TranscriptionSession,
};
use crate::turn::TurnId;

use super::{ListenError, ListenEvent, ListenMessage};

pub(super) struct TranscriptionBridge {
    worker: Option<JoinHandle<()>>,
}

pub(super) struct RecognitionController {
    transcription: Option<Arc<dyn TranscriptionSession>>,
    segments: Arc<Mutex<RecognitionSegments>>,
}

struct RecognitionSegments {
    next_turn_id: u64,
    active: Option<TurnId>,
    awaiting_finals: VecDeque<TurnId>,
}

impl RecognitionController {
    pub(super) fn new(transcription: Option<Arc<dyn TranscriptionSession>>) -> Self {
        Self {
            transcription,
            segments: Arc::new(Mutex::new(RecognitionSegments {
                next_turn_id: 1,
                active: None,
                awaiting_finals: VecDeque::new(),
            })),
        }
    }

    pub(super) fn begin_turn(&self) -> Result<TurnId, TranscriptionError> {
        let mut segments = self.segments()?;
        if segments.active.is_some() {
            return Err(TranscriptionError::StreamClosed(
                "recognition already has an active user turn".to_owned(),
            ));
        }
        let turn_id = TurnId(segments.next_turn_id);
        segments.next_turn_id = segments.next_turn_id.checked_add(1).ok_or_else(|| {
            TranscriptionError::StreamClosed("user turn identifier space is exhausted".to_owned())
        })?;
        segments.active = Some(turn_id);
        Ok(turn_id)
    }

    pub(super) fn submit_audio(
        &self,
        frame: AudioFrame,
    ) -> Result<Option<AudioSubmit>, TranscriptionError> {
        let segments = self.segments()?;
        if segments.active.is_none() {
            return Ok(None);
        }
        let Some(transcription) = &self.transcription else {
            return Ok(None);
        };
        transcription.submit_audio(frame).map(Some)
    }

    pub(super) fn finalize_turn(&self, turn_id: TurnId) -> Result<(), TranscriptionError> {
        let mut segments = self.segments()?;
        if segments.awaiting_finals.contains(&turn_id) {
            return Ok(());
        }
        if segments.active != Some(turn_id) {
            return Err(TranscriptionError::StreamClosed(format!(
                "user turn {} is not active",
                turn_id.0
            )));
        }
        let Some(transcription) = &self.transcription else {
            segments.active = None;
            return Ok(());
        };
        transcription.finalize_user_turn()?;
        segments.active = None;
        segments.awaiting_finals.push_back(turn_id);
        Ok(())
    }

    fn segments(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RecognitionSegments>, TranscriptionError> {
        lock_segments(&self.segments)
    }
}

impl Drop for TranscriptionBridge {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = worker.join();
    }
}

pub(super) fn start_transcription(
    transcriber: Option<&dyn Transcriber>,
    format: AudioFormat,
    session_started_at: Instant,
    events: Sender<ListenMessage>,
) -> Result<(Arc<RecognitionController>, Option<TranscriptionBridge>), ListenError> {
    let Some(transcriber) = transcriber else {
        return Ok((Arc::new(RecognitionController::new(None)), None));
    };
    let (transcription_events, receiver) = std::sync::mpsc::channel();
    let session = transcriber
        .start(format, session_started_at, transcription_events)
        .map_err(|error| ListenError::TranscriptionFailed(error.to_string()))?;
    let session = Arc::<dyn TranscriptionSession>::from(session);
    let recognition = Arc::new(RecognitionController::new(Some(session)));
    let bridge_segments = recognition.segments.clone();
    let worker = thread::Builder::new()
        .name("qol-voice-transcription-events".to_owned())
        .spawn(move || forward_transcription_events(receiver, events, bridge_segments))
        .map_err(|error| ListenError::TranscriptionFailed(error.to_string()))?;
    Ok((
        recognition,
        Some(TranscriptionBridge {
            worker: Some(worker),
        }),
    ))
}

fn forward_transcription_events(
    receiver: Receiver<Result<TranscriptionEvent, TranscriptionError>>,
    events: Sender<ListenMessage>,
    segments: Arc<Mutex<RecognitionSegments>>,
) {
    while let Ok(message) = receiver.recv() {
        let message = match message {
            Ok(event) => correlate_transcription_event(&segments, event).map(|(turn_id, event)| {
                Some(ListenEvent::TranscriptHypothesis {
                    turn_id,
                    observed_at_ms: event.observed_at_ms,
                    text: event.text,
                    confidence_permille: event.confidence_permille,
                    final_result: event.final_result,
                })
            }),
            Err(error) => Err(error),
        };
        let message = message.map_err(|error| ListenError::TranscriptionFailed(error.to_string()));
        if events.send(message).is_err() {
            return;
        }
    }
}

fn correlate_transcription_event(
    segments: &Mutex<RecognitionSegments>,
    event: TranscriptionEvent,
) -> Result<(TurnId, TranscriptionEvent), TranscriptionError> {
    let mut segments = lock_segments(segments)?;
    let turn_id = if event.final_result {
        segments
            .awaiting_finals
            .pop_front()
            .or_else(|| segments.active.take())
    } else {
        segments
            .awaiting_finals
            .front()
            .copied()
            .or(segments.active)
    };
    let Some(turn_id) = turn_id else {
        return Err(TranscriptionError::ProtocolFailed(
            "transcription event has no user turn".to_owned(),
        ));
    };
    Ok((turn_id, event))
}

fn lock_segments(
    segments: &Mutex<RecognitionSegments>,
) -> Result<std::sync::MutexGuard<'_, RecognitionSegments>, TranscriptionError> {
    segments.lock().map_err(|_| {
        TranscriptionError::StreamClosed("recognition segment state is unavailable".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use crate::audio::AudioFrame;
    use crate::transcribe::{AudioSubmit, TranscriptionError, TranscriptionSession};

    use super::RecognitionController;

    struct CountingTranscriptionSession {
        finalizations: Arc<AtomicU64>,
    }

    impl TranscriptionSession for CountingTranscriptionSession {
        fn submit_audio(&self, _frame: AudioFrame) -> Result<AudioSubmit, TranscriptionError> {
            Ok(AudioSubmit::Accepted)
        }

        fn finalize_user_turn(&self) -> Result<(), TranscriptionError> {
            self.finalizations.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn finalization_is_idempotent_per_turn() {
        let finalizations = Arc::new(AtomicU64::new(0));
        let recognition =
            RecognitionController::new(Some(Arc::new(CountingTranscriptionSession {
                finalizations: finalizations.clone(),
            })));
        let turn_id = recognition.begin_turn().unwrap();

        recognition.finalize_turn(turn_id).unwrap();
        recognition.finalize_turn(turn_id).unwrap();

        assert_eq!(finalizations.load(Ordering::Relaxed), 1);
    }
}
