use std::sync::{Arc, Mutex};

use crate::turn::Effect;
use crate::voice_session::{
    VoiceSession, VoiceSessionCause, VoiceSessionEvent, VoiceSessionUpdate,
};

use super::delivery::DeliveryDispatcher;
use super::events::SessionEventLog;
use super::routing::{ConversationRouter, RoutingControl};
use super::status::{assistant_state, user_state, LifecycleState, SessionStatus};

pub(super) fn run_session(
    session: &mut VoiceSession,
    status: Arc<Mutex<SessionStatus>>,
    events: SessionEventLog,
    routing_control: Arc<Mutex<RoutingControl>>,
    mut router: ConversationRouter,
    mut routing_revision: u64,
    delivery_dispatcher: DeliveryDispatcher,
) {
    loop {
        match session.receive() {
            Ok(Some(event)) => {
                sync_router(&routing_control, &mut routing_revision, &mut router);
                observe_event(&status, &events, &mut router, &delivery_dispatcher, &event);
            }
            Ok(None) => {
                if let Ok(mut status) = status.lock() {
                    *status = SessionStatus::default();
                }
                return;
            }
            Err(error) => {
                if let Ok(mut status) = status.lock() {
                    status.state = LifecycleState::Failed;
                    status.error = Some(error.to_string());
                }
                qol_runtime::probe!("VOICE_SESSION", "event=failed error={}", error);
                return;
            }
        }
    }
}

fn observe_event(
    status: &Mutex<SessionStatus>,
    events: &SessionEventLog,
    router: &mut ConversationRouter,
    delivery_dispatcher: &DeliveryDispatcher,
    event: &VoiceSessionEvent,
) {
    match event {
        VoiceSessionEvent::Update(update) => {
            if let Err(error) = events.record(update) {
                qol_runtime::probe!("VOICE_SESSION", "event=event_log_failed error={}", error);
            }
            observe_update(status, update);
            for intent in router.observe(update) {
                delivery_dispatcher.dispatch(intent);
            }
        }
        VoiceSessionEvent::AudioFramesDropped { stage, count, .. } => {
            qol_runtime::probe!(
                "VOICE_SESSION",
                "event=audio_frames_dropped stage={stage:?} count={count}"
            );
        }
    }
}

fn observe_update(status: &Mutex<SessionStatus>, update: &VoiceSessionUpdate) {
    if let Ok(mut status) = status.lock() {
        status.last_sequence = Some(update.snapshot.last_sequence);
        status.assistant_state = Some(assistant_state(&update.snapshot));
        status.user_state = Some(user_state(&update.snapshot));
    }
    let cause = match &update.cause {
        VoiceSessionCause::Observation(_) => "observation",
        VoiceSessionCause::AssistantTurnRequest(_) => "assistant_turn_request",
    };
    let effect_kinds = update
        .effects
        .effects
        .iter()
        .map(effect_kind)
        .collect::<Vec<_>>()
        .join(",");
    qol_runtime::probe!(
        "VOICE_SESSION",
        "session={} sequence={} event={} assistant={} user={} effects={}",
        update.snapshot.session_id.0,
        update.snapshot.last_sequence,
        cause,
        assistant_state(&update.snapshot).as_str(),
        user_state(&update.snapshot).as_str(),
        effect_kinds
    );
}

fn effect_kind(effect: &Effect) -> &'static str {
    match effect {
        Effect::Playback(_) => "playback",
        Effect::Synthesis(_) => "synthesis",
        Effect::Agent(_) => "agent",
        Effect::Recognition(_) => "recognition",
        Effect::Conversation(_) => "conversation",
    }
}

fn sync_router(
    control: &Mutex<RoutingControl>,
    revision: &mut u64,
    router: &mut ConversationRouter,
) {
    let Ok(control) = control.lock() else {
        return;
    };
    let decision = control.decision();
    if decision.revision == *revision {
        return;
    }
    *revision = decision.revision;
    router.configure(decision);
}
