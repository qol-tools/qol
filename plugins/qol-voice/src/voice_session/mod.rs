use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};

use crate::listen::{
    AudioDropStage, AudioInputInfo, AudioInputRequest, ListenConfig, ListenError, ListenEvent,
    ListenMessage, ListeningSession, UtteranceEndReason,
};
use crate::transcribe::{
    create_transcriber, TranscriberDescriptor, TranscriberRequest, TranscriptionError,
};
use crate::turn::{
    AssistantTurnRequest, AssistantTurnRequestEnvelope, ConversationalSignal, Effect, EffectBatch,
    Observation, ObservationEnvelope, RecognitionCommand, ResponseId, ResponsiveTurnPolicy,
    SessionId, TurnCoordinator, TurnError, TurnSnapshot, UtteranceId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceSessionConfig {
    pub session_id: SessionId,
    pub input: AudioInputRequest,
    pub listening: ListenConfig,
    pub transcription: Option<TranscriberRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceSessionInfo {
    pub input: AudioInputInfo,
    pub transcription: Option<TranscriberDescriptor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoiceSessionInput {
    AssistantSpeechStarted {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
    AssistantSpeechFinished {
        utterance_id: UtteranceId,
    },
    ConversationalSignalDetected {
        signal: ConversationalSignal,
        confidence_permille: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum VoiceSessionCause {
    Observation(ObservationEnvelope),
    AssistantTurnRequest(AssistantTurnRequestEnvelope),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSessionEvidence {
    pub level_permille: Option<u16>,
    pub utterance_end_reason: Option<UtteranceEndReason>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSessionUpdate {
    pub cause: VoiceSessionCause,
    pub evidence: VoiceSessionEvidence,
    pub snapshot: TurnSnapshot,
    pub effects: EffectBatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoiceSessionEvent {
    Update(VoiceSessionUpdate),
    AudioFramesDropped {
        observed_at_ms: u64,
        stage: AudioDropStage,
        count: u64,
    },
}

#[derive(Clone, Debug)]
pub enum VoiceSessionError {
    Listening(ListenError),
    Transcription(TranscriptionError),
    Turn(TurnError),
    SequenceExhausted,
    ControlClosed,
}

impl Display for VoiceSessionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Listening(error) => Display::fmt(error, formatter),
            Self::Transcription(error) => Display::fmt(error, formatter),
            Self::Turn(error) => Display::fmt(error, formatter),
            Self::SequenceExhausted => {
                write!(formatter, "voice session sequence space is exhausted")
            }
            Self::ControlClosed => write!(formatter, "voice session control channel is closed"),
        }
    }
}

impl Error for VoiceSessionError {}

impl From<ListenError> for VoiceSessionError {
    fn from(error: ListenError) -> Self {
        Self::Listening(error)
    }
}

impl From<TranscriptionError> for VoiceSessionError {
    fn from(error: TranscriptionError) -> Self {
        Self::Transcription(error)
    }
}

impl From<TurnError> for VoiceSessionError {
    fn from(error: TurnError) -> Self {
        Self::Turn(error)
    }
}

#[derive(Clone)]
pub struct VoiceSessionStopHandle {
    stop: Arc<dyn Fn() + Send + Sync>,
}

impl VoiceSessionStopHandle {
    fn new(stop: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            stop: Arc::new(stop),
        }
    }

    pub fn stop(&self) {
        (self.stop)();
    }
}

pub struct VoiceSession {
    recognition: Box<dyn RecognitionSession>,
    controls: Receiver<ControlRequest>,
    control_sender: Sender<ControlRequest>,
    coordinator: TurnCoordinator<ResponsiveTurnPolicy>,
    session_id: SessionId,
    next_sequence: u64,
    info: VoiceSessionInfo,
}

trait RecognitionSession: Send {
    fn stop_handle(&self) -> VoiceSessionStopHandle;
    fn event_receiver(&self) -> &Receiver<ListenMessage>;
    fn finalize_user_turn(&self, turn_id: crate::turn::TurnId) -> Result<(), ListenError>;
}

impl RecognitionSession for ListeningSession {
    fn stop_handle(&self) -> VoiceSessionStopHandle {
        let stop = ListeningSession::stop_handle(self);
        VoiceSessionStopHandle::new(move || stop.stop())
    }

    fn event_receiver(&self) -> &Receiver<ListenMessage> {
        ListeningSession::event_receiver(self)
    }

    fn finalize_user_turn(&self, turn_id: crate::turn::TurnId) -> Result<(), ListenError> {
        ListeningSession::finalize_user_turn(self, turn_id)
    }
}

enum ControlRequest {
    ObserveInput {
        observed_at_ms: u64,
        input: VoiceSessionInput,
        reply: Sender<Result<VoiceSessionUpdate, VoiceSessionError>>,
    },
    RequestAssistantTurn {
        requested_at_ms: u64,
        request: AssistantTurnRequest,
        reply: Sender<Result<VoiceSessionUpdate, VoiceSessionError>>,
    },
}

#[derive(Clone)]
pub struct VoiceSessionControlHandle {
    sender: Sender<ControlRequest>,
}

impl VoiceSessionControlHandle {
    pub fn observe_input(
        &self,
        observed_at_ms: u64,
        input: VoiceSessionInput,
    ) -> Result<VoiceSessionUpdate, VoiceSessionError> {
        let (reply, receiver) = bounded(1);
        self.sender
            .send(ControlRequest::ObserveInput {
                observed_at_ms,
                input,
                reply,
            })
            .map_err(|_| VoiceSessionError::ControlClosed)?;
        receiver
            .recv()
            .map_err(|_| VoiceSessionError::ControlClosed)?
    }

    pub fn request_assistant_turn(
        &self,
        requested_at_ms: u64,
        request: AssistantTurnRequest,
    ) -> Result<VoiceSessionUpdate, VoiceSessionError> {
        let (reply, receiver) = bounded(1);
        self.sender
            .send(ControlRequest::RequestAssistantTurn {
                requested_at_ms,
                request,
                reply,
            })
            .map_err(|_| VoiceSessionError::ControlClosed)?;
        receiver
            .recv()
            .map_err(|_| VoiceSessionError::ControlClosed)?
    }
}

impl VoiceSession {
    pub fn start(config: VoiceSessionConfig) -> Result<Self, VoiceSessionError> {
        let selected = config
            .transcription
            .as_ref()
            .map(create_transcriber)
            .transpose()?;
        let listening = match &selected {
            Some(selected) => ListeningSession::start_transcribed(
                config.input,
                config.listening,
                selected.transcriber.as_ref(),
            ),
            None => ListeningSession::start(config.input, config.listening),
        }?;
        let info = VoiceSessionInfo {
            input: listening.info().clone(),
            transcription: selected.map(|selected| selected.descriptor),
        };
        Ok(Self::from_recognition(
            config.session_id,
            info,
            Box::new(listening),
        ))
    }

    fn from_recognition(
        session_id: SessionId,
        info: VoiceSessionInfo,
        recognition: Box<dyn RecognitionSession>,
    ) -> Self {
        let (control_sender, controls) = unbounded();
        Self {
            recognition,
            controls,
            control_sender,
            coordinator: TurnCoordinator::new(session_id, ResponsiveTurnPolicy::default()),
            session_id,
            next_sequence: 1,
            info,
        }
    }

    pub fn info(&self) -> &VoiceSessionInfo {
        &self.info
    }

    pub fn stop_handle(&self) -> VoiceSessionStopHandle {
        self.recognition.stop_handle()
    }

    pub fn control_handle(&self) -> VoiceSessionControlHandle {
        VoiceSessionControlHandle {
            sender: self.control_sender.clone(),
        }
    }

    pub fn receive(&mut self) -> Result<Option<VoiceSessionEvent>, VoiceSessionError> {
        let recognition_events = self.recognition.event_receiver().clone();
        crossbeam_channel::select! {
            recv(recognition_events) -> message => {
                let message = message.map_err(|_| ListenError::EventChannelClosed)?;
                self.receive_listen_message(message)
            }
            recv(self.controls) -> request => {
                let request = request.map_err(|_| VoiceSessionError::ControlClosed)?;
                self.receive_control(request)
            }
        }
    }

    fn receive_listen_message(
        &mut self,
        message: ListenMessage,
    ) -> Result<Option<VoiceSessionEvent>, VoiceSessionError> {
        let Some(event) = message? else {
            return Ok(None);
        };
        if let ListenEvent::AudioFramesDropped {
            observed_at_ms,
            stage,
            count,
        } = event
        {
            return Ok(Some(VoiceSessionEvent::AudioFramesDropped {
                observed_at_ms,
                stage,
                count,
            }));
        }
        let (observed_at_ms, observation, evidence) =
            normalize_listen_event(event, self.info.transcription.is_some());
        let update = self.observe(observed_at_ms, observation, evidence)?;
        Ok(Some(VoiceSessionEvent::Update(update)))
    }

    fn receive_control(
        &mut self,
        request: ControlRequest,
    ) -> Result<Option<VoiceSessionEvent>, VoiceSessionError> {
        let (result, reply) = match request {
            ControlRequest::ObserveInput {
                observed_at_ms,
                input,
                reply,
            } => (self.observe_input(observed_at_ms, input), reply),
            ControlRequest::RequestAssistantTurn {
                requested_at_ms,
                request,
                reply,
            } => (self.request_assistant_turn(requested_at_ms, request), reply),
        };
        let _ = reply.send(result.clone());
        result.map(|update| Some(VoiceSessionEvent::Update(update)))
    }

    pub fn observe_input(
        &mut self,
        observed_at_ms: u64,
        input: VoiceSessionInput,
    ) -> Result<VoiceSessionUpdate, VoiceSessionError> {
        let observation = match input {
            VoiceSessionInput::AssistantSpeechStarted {
                response_id,
                utterance_id,
            } => Observation::AssistantSpeechStarted {
                response_id,
                utterance_id,
            },
            VoiceSessionInput::AssistantSpeechFinished { utterance_id } => {
                Observation::AssistantSpeechFinished { utterance_id }
            }
            VoiceSessionInput::ConversationalSignalDetected {
                signal,
                confidence_permille,
            } => Observation::ConversationalSignalDetected {
                signal,
                confidence_permille,
            },
        };
        self.observe(observed_at_ms, observation, VoiceSessionEvidence::default())
    }

    pub fn request_assistant_turn(
        &mut self,
        requested_at_ms: u64,
        request: AssistantTurnRequest,
    ) -> Result<VoiceSessionUpdate, VoiceSessionError> {
        let sequence = self.take_sequence()?;
        let envelope = AssistantTurnRequestEnvelope {
            session_id: self.session_id,
            sequence,
            requested_at_ms,
            request,
        };
        let effects = self.coordinator.request_assistant_turn(envelope.clone())?;
        self.dispatch_owned_effects(&effects)?;
        Ok(VoiceSessionUpdate {
            cause: VoiceSessionCause::AssistantTurnRequest(envelope),
            evidence: VoiceSessionEvidence::default(),
            snapshot: self.coordinator.snapshot(),
            effects,
        })
    }

    fn observe(
        &mut self,
        observed_at_ms: u64,
        observation: Observation,
        evidence: VoiceSessionEvidence,
    ) -> Result<VoiceSessionUpdate, VoiceSessionError> {
        let sequence = self.take_sequence()?;
        let envelope = ObservationEnvelope {
            session_id: self.session_id,
            sequence,
            observed_at_ms,
            observation,
        };
        let effects = self.coordinator.observe(envelope.clone())?;
        self.dispatch_owned_effects(&effects)?;
        Ok(VoiceSessionUpdate {
            cause: VoiceSessionCause::Observation(envelope),
            evidence,
            snapshot: self.coordinator.snapshot(),
            effects,
        })
    }

    fn dispatch_owned_effects(&self, batch: &EffectBatch) -> Result<(), VoiceSessionError> {
        for effect in &batch.effects {
            let Effect::Recognition(RecognitionCommand::FinalizeUserTurn { turn_id }) = effect
            else {
                continue;
            };
            self.recognition.finalize_user_turn(*turn_id)?;
        }
        Ok(())
    }

    fn take_sequence(&mut self) -> Result<u64, VoiceSessionError> {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(VoiceSessionError::SequenceExhausted)?;
        Ok(sequence)
    }
}

fn normalize_listen_event(
    event: ListenEvent,
    transcription_enabled: bool,
) -> (u64, Observation, VoiceSessionEvidence) {
    match event {
        ListenEvent::VoiceActivityStarted {
            turn_id,
            observed_at_ms,
            level_permille,
        } => (
            observed_at_ms,
            Observation::VoiceActivityStarted { turn_id },
            VoiceSessionEvidence {
                level_permille: Some(level_permille),
                utterance_end_reason: None,
            },
        ),
        ListenEvent::VoiceActivityEnded {
            turn_id,
            observed_at_ms,
            level_permille,
            reason,
        } => (
            observed_at_ms,
            Observation::VoiceActivityEnded {
                turn_id,
                awaiting_transcript: transcription_enabled,
            },
            VoiceSessionEvidence {
                level_permille: Some(level_permille),
                utterance_end_reason: Some(reason),
            },
        ),
        ListenEvent::TranscriptHypothesis {
            turn_id,
            observed_at_ms,
            text,
            confidence_permille,
            final_result,
        } => (
            observed_at_ms,
            Observation::TranscriptHypothesis {
                turn_id,
                text,
                confidence_permille,
                final_result,
            },
            VoiceSessionEvidence::default(),
        ),
        ListenEvent::AudioFramesDropped { .. } => {
            unreachable!("audio drop events are normalized before observations")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crossbeam_channel::{unbounded, Receiver};

    use crate::audio::{AudioEncoding, AudioFormat};
    use crate::listen::{AudioInputInfo, ListenEvent, UtteranceEndReason};
    use crate::turn::{
        AssistantTurnRequest, ConversationCommand, Effect, PlaybackCommand, RecognitionCommand,
        ResponseId, SessionId, TurnId, UserActivityState, UtteranceId,
    };

    use super::{
        RecognitionSession, VoiceSession, VoiceSessionCause, VoiceSessionEvent, VoiceSessionInfo,
        VoiceSessionStopHandle,
    };

    struct FakeRecognition {
        events: Receiver<Result<Option<ListenEvent>, crate::listen::ListenError>>,
        finalized: Arc<Mutex<Vec<TurnId>>>,
    }

    impl RecognitionSession for FakeRecognition {
        fn stop_handle(&self) -> VoiceSessionStopHandle {
            VoiceSessionStopHandle::new(|| {})
        }

        fn event_receiver(
            &self,
        ) -> &Receiver<Result<Option<ListenEvent>, crate::listen::ListenError>> {
            &self.events
        }

        fn finalize_user_turn(&self, turn_id: TurnId) -> Result<(), crate::listen::ListenError> {
            self.finalized.lock().unwrap().push(turn_id);
            Ok(())
        }
    }

    #[test]
    fn runtime_owns_sequence_and_dispatches_recognition_effects() {
        let turn_id = TurnId(41);
        let (events, receiver) = unbounded();
        events
            .send(Ok(Some(ListenEvent::VoiceActivityStarted {
                turn_id,
                observed_at_ms: 100,
                level_permille: 20,
            })))
            .unwrap();
        events
            .send(Ok(Some(ListenEvent::TranscriptHypothesis {
                turn_id,
                observed_at_ms: 200,
                text: "one more thing".to_owned(),
                confidence_permille: Some(900),
                final_result: false,
            })))
            .unwrap();
        let finalized = Arc::new(Mutex::new(Vec::new()));
        let recognition = FakeRecognition {
            events: receiver,
            finalized: finalized.clone(),
        };
        let mut session = VoiceSession::from_recognition(
            SessionId(7),
            VoiceSessionInfo {
                input: AudioInputInfo {
                    device_name: "test".to_owned(),
                    format: AudioFormat {
                        sample_rate: 16_000,
                        channels: 1,
                        encoding: AudioEncoding::PcmS16Le,
                    },
                },
                transcription: None,
            },
            Box::new(recognition),
        );

        let started = session.receive().unwrap().unwrap();
        let partial = session.receive().unwrap().unwrap();
        let interruption = session
            .request_assistant_turn(
                300,
                AssistantTurnRequest {
                    response_id: ResponseId(5),
                    utterance_id: UtteranceId(6),
                },
            )
            .unwrap();

        assert_eq!(sequence(&started), 1);
        assert_eq!(sequence(&partial), 2);
        assert_eq!(interruption.effects.caused_by_sequence, 3);
        assert_eq!(
            interruption.effects.effects,
            vec![
                Effect::Recognition(RecognitionCommand::FinalizeUserTurn { turn_id }),
                Effect::Playback(PlaybackCommand::Start {
                    response_id: ResponseId(5),
                    utterance_id: UtteranceId(6),
                }),
            ]
        );
        assert_eq!(*finalized.lock().unwrap(), vec![turn_id]);

        events
            .send(Ok(Some(ListenEvent::TranscriptHypothesis {
                turn_id,
                observed_at_ms: 400,
                text: "one more thing".to_owned(),
                confidence_permille: Some(900),
                final_result: true,
            })))
            .unwrap();
        let final_transcript = session.receive().unwrap().unwrap();
        let VoiceSessionEvent::Update(final_transcript) = final_transcript else {
            panic!("expected final transcript update");
        };
        assert_eq!(final_transcript.effects.caused_by_sequence, 4);
        assert_eq!(
            final_transcript.effects.effects,
            vec![Effect::Conversation(ConversationCommand::CommitUserTurn {
                turn_id,
                text: "one more thing".to_owned(),
            })]
        );
    }

    #[test]
    fn final_transcript_commits_after_voice_activity_ends() {
        let turn_id = TurnId(41);
        let (events, receiver) = unbounded();
        events
            .send(Ok(Some(ListenEvent::VoiceActivityStarted {
                turn_id,
                observed_at_ms: 100,
                level_permille: 20,
            })))
            .unwrap();
        events
            .send(Ok(Some(ListenEvent::VoiceActivityEnded {
                turn_id,
                observed_at_ms: 200,
                level_permille: 2,
                reason: UtteranceEndReason::Silence,
            })))
            .unwrap();
        events
            .send(Ok(Some(ListenEvent::TranscriptHypothesis {
                turn_id,
                observed_at_ms: 300,
                text: "send this to the terminal".to_owned(),
                confidence_permille: Some(900),
                final_result: true,
            })))
            .unwrap();
        let recognition = FakeRecognition {
            events: receiver,
            finalized: Arc::new(Mutex::new(Vec::new())),
        };
        let mut session = VoiceSession::from_recognition(
            SessionId(7),
            VoiceSessionInfo {
                input: AudioInputInfo {
                    device_name: "test".to_owned(),
                    format: AudioFormat {
                        sample_rate: 16_000,
                        channels: 1,
                        encoding: AudioEncoding::PcmS16Le,
                    },
                },
                transcription: crate::transcribe::transcriber_descriptors().next(),
            },
            Box::new(recognition),
        );

        session.receive().unwrap().unwrap();
        let ended = session.receive().unwrap().unwrap();
        let VoiceSessionEvent::Update(ended) = ended else {
            panic!("expected voice activity update");
        };
        assert!(matches!(
            ended.snapshot.user,
            UserActivityState::Finalizing {
                turn_id: TurnId(41),
                ..
            }
        ));

        let final_transcript = session.receive().unwrap().unwrap();
        let VoiceSessionEvent::Update(final_transcript) = final_transcript else {
            panic!("expected final transcript update");
        };
        assert_eq!(
            final_transcript.effects.effects,
            vec![Effect::Conversation(ConversationCommand::CommitUserTurn {
                turn_id,
                text: "send this to the terminal".to_owned(),
            })]
        );
        assert_eq!(final_transcript.snapshot.user, UserActivityState::Idle);
    }

    #[test]
    fn control_handle_wakes_an_idle_session_for_agent_interruption() {
        let (_events, receiver) = unbounded();
        let recognition = FakeRecognition {
            events: receiver,
            finalized: Arc::new(Mutex::new(Vec::new())),
        };
        let mut session = VoiceSession::from_recognition(
            SessionId(7),
            VoiceSessionInfo {
                input: AudioInputInfo {
                    device_name: "test".to_owned(),
                    format: AudioFormat {
                        sample_rate: 16_000,
                        channels: 1,
                        encoding: AudioEncoding::PcmS16Le,
                    },
                },
                transcription: None,
            },
            Box::new(recognition),
        );
        let control = session.control_handle();
        let worker = std::thread::spawn(move || session.receive());

        let update = control
            .request_assistant_turn(
                100,
                AssistantTurnRequest {
                    response_id: ResponseId(5),
                    utterance_id: UtteranceId(6),
                },
            )
            .unwrap();

        assert_eq!(
            update.effects.effects,
            vec![Effect::Playback(PlaybackCommand::Start {
                response_id: ResponseId(5),
                utterance_id: UtteranceId(6),
            })]
        );
        assert!(worker.join().unwrap().unwrap().is_some());
    }

    fn sequence(event: &VoiceSessionEvent) -> u64 {
        let VoiceSessionEvent::Update(update) = event else {
            panic!("expected voice session update");
        };
        match &update.cause {
            VoiceSessionCause::Observation(envelope) => envelope.sequence,
            VoiceSessionCause::AssistantTurnRequest(envelope) => envelope.sequence,
        }
    }
}
