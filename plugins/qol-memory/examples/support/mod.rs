use anyhow::{bail, Context, Result};
use qol_memory::app::{request, warm::WarmState};
use qol_memory::store::Store;
use qol_memory::verification::Fact;
use qol_plugin_daemon::daemon::ReadResult;
use qol_runtime::protocol::DaemonRequest;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub fn fixture_store(root: &Path, facts: &[Fact], caller: &str) -> Result<Store> {
    std::fs::create_dir(root).context("fixture store must not already exist")?;
    let mut body = String::new();
    for fact in facts {
        let unit = json!({"key":fact.id,"kind":"capture","agent_home":caller,"cwd":"/fixture/comparison","session":"fixture","text":format!("Q: {} A: {}",fact.question,fact.answer)});
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
