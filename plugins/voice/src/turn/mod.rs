mod model;
mod policy;

pub use model::{
    AgentCommand, AssistantOutputState, AssistantTurnRequest, AssistantTurnRequestEnvelope,
    ConversationCommand, ConversationalSignal, Effect, EffectBatch, Observation,
    ObservationEnvelope, PlaybackCommand, RecognitionCommand, ResponseId, SessionId,
    SynthesisCommand, TurnDecision, TurnError, TurnId, TurnSnapshot, UserActivityState,
    UtteranceId, CONTROL_SCHEMA_VERSION,
};
pub use policy::{ResponsiveTurnPolicy, TurnPolicy};

use model::TurnState;

#[derive(Debug)]
pub struct TurnCoordinator<P> {
    policy: P,
    state: TurnState,
}

impl<P> TurnCoordinator<P>
where
    P: TurnPolicy,
{
    pub fn new(session_id: SessionId, policy: P) -> Self {
        Self {
            policy,
            state: TurnState::new(session_id),
        }
    }

    pub fn observe(&mut self, envelope: ObservationEnvelope) -> Result<EffectBatch, TurnError> {
        self.state
            .validate(envelope.session_id, envelope.sequence)?;
        let snapshot = self.state.snapshot();
        let TurnDecision {
            assistant,
            user,
            effects,
        } = self.policy.decide(&snapshot, &envelope);
        self.state.apply(envelope.sequence, assistant, user);
        Ok(EffectBatch::new(
            envelope.session_id,
            envelope.sequence,
            effects,
        ))
    }

    pub fn request_assistant_turn(
        &mut self,
        envelope: AssistantTurnRequestEnvelope,
    ) -> Result<EffectBatch, TurnError> {
        self.state
            .validate(envelope.session_id, envelope.sequence)?;
        let snapshot = self.state.snapshot();
        let TurnDecision {
            assistant,
            user,
            effects,
        } = self.policy.request_assistant_turn(&snapshot, &envelope);
        self.state.apply(envelope.sequence, assistant, user);
        Ok(EffectBatch::new(
            envelope.session_id,
            envelope.sequence,
            effects,
        ))
    }

    pub fn snapshot(&self) -> TurnSnapshot {
        self.state.snapshot()
    }
}
