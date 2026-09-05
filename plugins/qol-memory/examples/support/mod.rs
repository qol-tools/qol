use anyhow::{bail, Context, Result};
use qol_memory::app::{request, warm::WarmState};
use qol_memory::store::Store;
use qol_plugin_daemon::daemon::ReadResult;
use qol_runtime::protocol::DaemonRequest;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FixtureMemory {
    QuestionAnswer {
        id: String,
        question: String,
        answer: String,
    },
    Text {
        id: String,
        text: String,
    },
}

impl FixtureMemory {
    pub fn id(&self) -> &str {
        match self {
            Self::QuestionAnswer { id, .. } | Self::Text { id, .. } => id,
        }
    }

    pub fn capture_text(&self) -> String {
        match self {
            Self::QuestionAnswer {
                question, answer, ..
            } => format!("Q: {question} A: {answer}"),
            Self::Text { text, .. } => text.clone(),
        }
    }
}

pub fn fixture_store(root: &Path, memories: &[FixtureMemory], caller: &str) -> Result<Store> {
    std::fs::create_dir(root).context("fixture store must not already exist")?;
    let mut body = String::new();
    for memory in memories {
        let unit = json!({"key":memory.id(),"kind":"capture","agent_home":caller,"cwd":"/fixture/comparison","session":"fixture","text":memory.capture_text()});
        body.push_str(&serde_json::to_string(&unit)?);
        body.push('\n');
    }
    qol_fs::atomic_write(&root.join("units.jsonl"), body.as_bytes())?;
    Store::resolve(Some(root))
}

pub fn call(
    state: &mut Arc<Mutex<WarmState>>,
    action: &str,
    query: &str,
    caller: &str,
) -> Result<Value> {
    match request::handle(
        state,
        &DaemonRequest {
            action: action.into(),
            input: json!({"query":query,"agent_home":caller,"no_log":true}),
        },
    ) {
        ReadResult::HandledWithData(value) => Ok(value),
        ReadResult::Error(error) => bail!("{action}: {error}"),
        _ => bail!("{action} returned no data"),
    }
}
