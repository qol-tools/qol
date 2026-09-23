use std::thread;

use anyhow::Result;
use qol_headless::{Command, OutputSink, EXIT_SUCCESS};

use crate::listen::AudioDropStage;
use crate::turn::{Observation, SessionId};
use crate::voice_session::{
    VoiceSession, VoiceSessionCause, VoiceSessionConfig, VoiceSessionEvent, VoiceSessionUpdate,
};

use super::reject_args;

pub(super) fn command() -> Command {
    Command::new("listen")
        .about("Run a local voice session without the tray daemon or audio playback.")
        .usage("qol-voice listen")
        .detail("Uses the plugin configuration and prints STT events.")
        .detail("Press Enter or Ctrl+C to stop.")
        .run_streaming(run)
}

fn run(context: &qol_headless::CommandContext, sink: &mut dyn OutputSink) -> Result<u8> {
    reject_args(context.args())?;
    let config = crate::config::load();
    let mut session = VoiceSession::start(VoiceSessionConfig {
        session_id: SessionId(1),
        input: config.input_request(),
        listening: config.listen_config(),
        transcription: config.transcriber_request(),
    })?;
    let info = session.info();
    sink.stdout(&format!(
        "listening: {} ({} Hz, {} channel(s), {})\n",
        info.input.device_name,
        info.input.format.sample_rate,
        info.input.format.channels,
        info.input.format.encoding.display_name()
    ));
    if let Some(provider) = &info.transcription {
        sink.stdout(&format!(
            "transcription: {} ({})\n",
            provider.name, provider.id
        ));
    } else {
        sink.stdout("transcription: disabled in plugin settings\n");
    }
    sink.stdout("speak into the microphone; press Enter or Ctrl+C to stop\n");
    let stop = session.stop_handle();
    thread::spawn(move || {
        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
        stop.stop();
    });
    while let Some(event) = session.receive()? {
        print_event(sink, event);
    }
    sink.stdout("listening stopped\n");
    Ok(EXIT_SUCCESS)
}

fn print_event(sink: &mut dyn OutputSink, event: VoiceSessionEvent) {
    match event {
        VoiceSessionEvent::AudioFramesDropped {
            observed_at_ms,
            stage,
            count,
        } => {
            let stage = match stage {
                AudioDropStage::Capture => "capture",
                AudioDropStage::Transcription => "transcription",
            };
            sink.stderr(&format!(
                "audio frames dropped: stage={stage} count={count} at {observed_at_ms}ms\n"
            ));
        }
        VoiceSessionEvent::Update(update) => print_update(sink, &update),
    }
}

fn print_update(sink: &mut dyn OutputSink, update: &VoiceSessionUpdate) {
    let VoiceSessionCause::Observation(envelope) = &update.cause else {
        return;
    };
    match &envelope.observation {
        Observation::VoiceActivityStarted { .. } => {
            sink.stdout(&format!(
                "voice activity started at {}ms\n",
                envelope.observed_at_ms
            ));
        }
        Observation::VoiceActivityEnded { .. } => {
            sink.stdout(&format!(
                "voice activity ended at {}ms\n",
                envelope.observed_at_ms
            ));
        }
        Observation::TranscriptHypothesis {
            text,
            confidence_permille,
            final_result,
            ..
        } => {
            let kind = if *final_result { "final" } else { "partial" };
            let confidence = confidence_permille
                .map(|value| format!(", confidence={value}‰"))
                .unwrap_or_default();
            sink.stdout(&format!("transcript ({kind}{confidence}): {text}\n"));
        }
        _ => {}
    }
    qol_runtime::probe!(
        "VOICE_SESSION",
        "session={} sequence={} event=local_observation effects={}",
        update.snapshot.session_id.0,
        update.snapshot.last_sequence,
        update.effects.effects.len()
    );
}
