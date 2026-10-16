use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::ReadResult;
use qol_runtime::protocol::DaemonRequest;
use serde_json::{json, Value};

use crate::app::rows::{self, RowContext, ROW_LIMIT};
use crate::app::Daemon;
use crate::capture::{Clipboard, Payload};
use crate::history::{self, now_ms, Clip};

pub fn handle(state: &mut Daemon, request: &DaemonRequest) -> ReadResult<()> {
    let result = match request.action.as_str() {
        "ping" | "kill" => return ReadResult::Handled,
        "rows" => rows(state, &request.input),
        "detail" => detail(state, &request.input),
        "copy" => copy(state, &request.input),
        "pin" => pin(state, &request.input),
        "forget" => forget(state, &request.input),
        "open" => crate::app::host::open_in_launcher().map(|()| json!({ "opened": true })),
        "settings" => crate::app::host::open_settings().map(|()| json!({ "opened": true })),
        _ => return ReadResult::Fallback,
    };
    match result {
        Ok(payload) => ReadResult::HandledWithData(payload),
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn rows(state: &Daemon, input: &Value) -> Result<Value> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let now = now_ms();
    let retention = crate::config::load().retention();
    let live = state.history().live(now, retention)?;
    let total = live.len();
    let shown = history::matching(live, query, ROW_LIMIT);
    let context = RowContext {
        now,
        secret_ttl: retention.secret_ttl,
        store: state.history().store(),
    };
    let flow_rows = rows::build(&shown, state.paused_reason().is_some(), &context);
    qol_runtime::probe!(
        "QOL_CLIPBOARD",
        "event=rows query_chars={} shown={} total={}",
        query.chars().count(),
        shown.len(),
        total
    );
    serde_json::to_value(flow_rows).context("failed to encode the clipboard rows")
}

fn detail(state: &Daemon, input: &Value) -> Result<Value> {
    let clip = live_clip(state, input)?;
    serde_json::to_value(crate::app::detail::build(&clip))
        .context("failed to encode the clip detail")
}

fn copy(state: &Daemon, input: &Value) -> Result<Value> {
    let clip = live_clip(state, input)?;
    let payload = payload_of(state, &clip)?;
    let secret_until = match &payload {
        Payload::Secret { until, .. } => Some(*until),
        _ => None,
    };
    let clipboard = state.clipboard()?;
    clipboard.write(payload)?;
    if let Some(until) = secret_until {
        clear_secret_at(Arc::clone(clipboard), until)?;
    }
    Ok(json!({ "copied": clip.id }))
}

fn clear_secret_at(clipboard: Arc<dyn Clipboard>, until: Instant) -> Result<()> {
    std::thread::Builder::new()
        .name("qol-clipboard-secret".to_owned())
        .spawn(move || {
            std::thread::sleep(until.saturating_duration_since(Instant::now()));
            if let Err(error) = clipboard.clear_expired_secret() {
                log::warn!("failed to clear an expired secret from the clipboard: {error}");
            }
        })
        .context("failed to start the secret timer")?;
    Ok(())
}

fn pin(state: &Daemon, input: &Value) -> Result<Value> {
    let id = id_field(input)?;
    let retention = crate::config::load().retention();
    let pinned = state.history().toggle_pin(id, now_ms(), retention)?;
    Ok(json!({ "id": id, "pinned": pinned }))
}

fn forget(state: &Daemon, input: &Value) -> Result<Value> {
    let id = id_field(input)?;
    state.history().forget(id)?;
    Ok(json!({ "forgot": id }))
}

fn live_clip(state: &Daemon, input: &Value) -> Result<Clip> {
    let id = id_field(input)?;
    let retention = crate::config::load().retention();
    state
        .history()
        .get(id, now_ms(), retention)?
        .with_context(|| format!("no clip with id {id}"))
}

fn payload_of(state: &Daemon, clip: &Clip) -> Result<Payload> {
    if let Some(image) = &clip.image {
        let path = state.history().store().blob_path(&image.blob);
        let png =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        return Ok(Payload::Png(png));
    }
    let text = clip
        .text
        .clone()
        .with_context(|| format!("clip {} has no content", clip.id))?;
    if !clip.is_secret() {
        return Ok(Payload::Text(text));
    }
    let ttl = crate::config::load().retention().secret_ttl;
    let left = crate::secrets::remaining(clip.copied_at_ms, now_ms(), ttl)
        .with_context(|| format!("secret {} has already cleared", clip.id))?;
    Ok(Payload::Secret {
        text,
        until: Instant::now() + left,
    })
}

fn id_field(input: &Value) -> Result<&str> {
    input
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .context("input.id must be a clip id")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use qol_runtime::local_ipc::{encode_secret_json, MAX_MESSAGE_BYTES};
    use qol_runtime::protocol::DaemonResponse;

    use super::*;
    use crate::history::{ClipKind, Image, Store};

    const MEGABYTE: usize = 1024 * 1024;
    const KINDS: [ClipKind; 8] = [
        ClipKind::Text,
        ClipKind::Code,
        ClipKind::Image,
        ClipKind::Link,
        ClipKind::Command,
        ClipKind::Secret,
        ClipKind::Color,
        ClipKind::File,
    ];

    fn answer_bytes(payload: Value) -> usize {
        let response = DaemonResponse::Handled {
            data: Some(payload),
        };
        match encode_secret_json(&response) {
            Ok(line) => line.len(),
            Err(_) => usize::MAX,
        }
    }

    fn worst_text(line: usize) -> String {
        let line = format!("\u{1}\"{}", "\u{1}".repeat(line));
        let mut text = String::with_capacity(MEGABYTE + line.len());
        while text.len() < MEGABYTE {
            text.push_str(&line);
            text.push('\n');
        }
        text
    }

    fn worst_clip(at: usize, text: &str) -> Clip {
        let kind = KINDS[at % KINDS.len()];
        Clip {
            id: format!("c_{at:016x}"),
            kind,
            text: (kind != ClipKind::Image).then(|| text.to_string()),
            image: (kind == ClipKind::Image).then(|| Image {
                blob: format!("c_{at:016x}.png"),
                width: u32::MAX,
                height: u32::MAX,
            }),
            bytes: MEGABYTE as u64,
            app: Some("\u{1}".repeat(4_096)),
            copied_at_ms: 1_800_000_000_000,
            pinned: at.is_multiple_of(2),
        }
    }

    #[test]
    fn every_answer_stays_under_the_message_limit_whatever_the_history_holds() {
        let store = Store::at(
            "/home/someone-with-a-long-user-name/.local/share/qol-tray/plugin-data/qol-clipboard",
        );
        let context = RowContext {
            now: 1_800_000_000_000,
            secret_ttl: Duration::from_secs(30),
            store: &store,
        };
        for text in [worst_text(MEGABYTE), worst_text(64), worst_text(1)] {
            let clips: Vec<Clip> = (0..ROW_LIMIT).map(|at| worst_clip(at, &text)).collect();
            assert!(clips
                .iter()
                .all(|clip| clip.text.as_ref().is_none_or(|text| text.len() >= MEGABYTE)));

            let rows = serde_json::to_value(rows::build(&clips, true, &context)).unwrap();
            let size = answer_bytes(rows);
            assert!(size < MAX_MESSAGE_BYTES, "rows answer is {size} bytes");

            for clip in &clips {
                let detail = serde_json::to_value(crate::app::detail::build(clip)).unwrap();
                let size = answer_bytes(detail);
                assert!(
                    size < MAX_MESSAGE_BYTES,
                    "{:?} detail answer is {size} bytes",
                    clip.kind
                );
            }
        }
    }
}
