use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::Sender;

use crate::audio::{AudioFormat, AudioFrame};
use crate::transcribe::AudioSubmit;
use crate::turn::TurnId;

use super::recognition::RecognitionController;
use super::utterance::{SegmentedFrame, UtteranceSegmenter};
use super::{
    AudioDropStage, ListenConfig, ListenError, ListenEvent, ListenMessage, UtteranceEndReason,
};

const CAPTURE_QUEUE_CAPACITY: usize = 8;

pub(super) struct AudioPipeline {
    worker: Option<JoinHandle<()>>,
}

impl AudioPipeline {
    pub(super) fn start(
        config: ListenConfig,
        format: AudioFormat,
        frames: Receiver<AudioFrame>,
        capture_drops: Arc<AtomicU64>,
        recognition: Arc<RecognitionController>,
        events: Sender<ListenMessage>,
    ) -> Result<Self, ListenError> {
        let worker = thread::Builder::new()
            .name("qol-voice-audio-pipeline".to_owned())
            .spawn(move || {
                process_audio(config, format, frames, capture_drops, recognition, events);
            })
            .map_err(|error| ListenError::CaptureFailed(error.to_string()))?;
        Ok(Self {
            worker: Some(worker),
        })
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = worker.join();
    }
}

pub(super) fn audio_frame_channel() -> (SyncSender<AudioFrame>, Receiver<AudioFrame>, Arc<AtomicU64>)
{
    let (frames, receiver) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    (frames, receiver, dropped)
}

fn process_audio(
    config: ListenConfig,
    format: AudioFormat,
    frames: Receiver<AudioFrame>,
    capture_drops: Arc<AtomicU64>,
    recognition: Arc<RecognitionController>,
    events: Sender<ListenMessage>,
) {
    let mut segmenter = UtteranceSegmenter::new(config, format);
    let mut acoustic_turn = None;
    let mut transcription_drops = 0_u64;
    while let Ok(frame) = frames.recv() {
        if !report_capture_drops(&capture_drops, frame.observed_at_ms, &events) {
            return;
        }
        let segmented = segmenter.observe(frame);
        if !process_segmented_frame(
            segmented,
            &mut acoustic_turn,
            &mut transcription_drops,
            &recognition,
            &events,
        ) {
            return;
        }
    }
}

fn report_capture_drops(
    capture_drops: &AtomicU64,
    observed_at_ms: u64,
    events: &Sender<ListenMessage>,
) -> bool {
    let count = capture_drops.swap(0, Ordering::Relaxed);
    if count == 0 {
        return true;
    }
    send_event(
        events,
        ListenEvent::AudioFramesDropped {
            observed_at_ms,
            stage: AudioDropStage::Capture,
            count,
        },
    )
}

fn process_segmented_frame(
    segmented: SegmentedFrame,
    acoustic_turn: &mut Option<TurnId>,
    transcription_drops: &mut u64,
    recognition: &RecognitionController,
    events: &Sender<ListenMessage>,
) -> bool {
    match segmented {
        SegmentedFrame::Idle => true,
        SegmentedFrame::Started {
            observed_at_ms,
            level_permille,
            frames,
        } => start_acoustic_turn(
            observed_at_ms,
            level_permille,
            frames,
            acoustic_turn,
            transcription_drops,
            recognition,
            events,
        ),
        SegmentedFrame::Active(frame) => {
            submit_recognition_frame(frame, transcription_drops, recognition, events)
        }
        SegmentedFrame::Ended {
            frame,
            level_permille,
            reason,
        } => end_acoustic_turn(
            frame,
            level_permille,
            reason,
            acoustic_turn,
            transcription_drops,
            recognition,
            events,
        ),
    }
}

fn start_acoustic_turn(
    observed_at_ms: u64,
    level_permille: u16,
    frames: Vec<AudioFrame>,
    acoustic_turn: &mut Option<TurnId>,
    transcription_drops: &mut u64,
    recognition: &RecognitionController,
    events: &Sender<ListenMessage>,
) -> bool {
    let turn_id = match recognition.begin_turn() {
        Ok(turn_id) => turn_id,
        Err(error) => return send_transcription_error(events, error.to_string()),
    };
    *acoustic_turn = Some(turn_id);
    if !send_event(
        events,
        ListenEvent::VoiceActivityStarted {
            turn_id,
            observed_at_ms,
            level_permille,
        },
    ) {
        return false;
    }
    frames
        .into_iter()
        .all(|frame| submit_recognition_frame(frame, transcription_drops, recognition, events))
}

fn end_acoustic_turn(
    frame: AudioFrame,
    level_permille: u16,
    reason: UtteranceEndReason,
    acoustic_turn: &mut Option<TurnId>,
    transcription_drops: &mut u64,
    recognition: &RecognitionController,
    events: &Sender<ListenMessage>,
) -> bool {
    let observed_at_ms = frame.observed_at_ms;
    if !submit_recognition_frame(frame, transcription_drops, recognition, events) {
        return false;
    }
    let Some(turn_id) = acoustic_turn.take() else {
        return send_capture_error(
            events,
            "voice activity ended without an active user turn".to_owned(),
        );
    };
    if let Err(error) = recognition.finalize_turn(turn_id) {
        return send_transcription_error(events, error.to_string());
    }
    send_event(
        events,
        ListenEvent::VoiceActivityEnded {
            turn_id,
            observed_at_ms,
            level_permille,
            reason,
        },
    )
}

fn submit_recognition_frame(
    frame: AudioFrame,
    transcription_drops: &mut u64,
    recognition: &RecognitionController,
    events: &Sender<ListenMessage>,
) -> bool {
    let observed_at_ms = frame.observed_at_ms;
    match recognition.submit_audio(frame) {
        Ok(Some(AudioSubmit::Accepted)) if *transcription_drops > 0 => {
            let count = std::mem::take(transcription_drops);
            send_event(
                events,
                ListenEvent::AudioFramesDropped {
                    observed_at_ms,
                    stage: AudioDropStage::Transcription,
                    count,
                },
            )
        }
        Ok(Some(AudioSubmit::Dropped)) => {
            *transcription_drops = transcription_drops.saturating_add(1);
            true
        }
        Ok(Some(AudioSubmit::Accepted)) | Ok(None) => true,
        Err(error) => send_transcription_error(events, error.to_string()),
    }
}

fn send_capture_error(events: &Sender<ListenMessage>, message: String) -> bool {
    let _ = events.send(Err(ListenError::CaptureFailed(message)));
    false
}

fn send_transcription_error(events: &Sender<ListenMessage>, message: String) -> bool {
    let _ = events.send(Err(ListenError::TranscriptionFailed(message)));
    false
}

fn send_event(events: &Sender<ListenMessage>, event: ListenEvent) -> bool {
    events.send(Ok(Some(event))).is_ok()
}
