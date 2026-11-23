mod cli;
mod model;
mod store;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io::Read;

use serde_json::{json, Value};

use model::Page;
pub(crate) use store::PageStore;

pub(crate) fn execute(args: &[OsString], stdin: &mut dyn Read) -> (String, u8) {
    let outcome = cli::execute(args, &PageStore::system(), stdin, super::mailbox::now_ms());
    (outcome.stdout, outcome.code)
}

pub(crate) fn tool(
    store: &PageStore,
    owner: &str,
    arguments: &Value,
    now_ms: u64,
) -> Result<String, String> {
    let text = |name: &str| arguments.get(name).and_then(Value::as_str);
    let id = || {
        text("id")
            .or_else(|| {
                arguments
                    .get("page")
                    .and_then(|page| page.get("id"))
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| "session_page needs id".to_owned())
    };
    let shown = match text("action") {
        Some("set") => {
            let mut page = arguments
                .get("page")
                .cloned()
                .ok_or_else(|| "session_page set needs page".to_owned())
                .and_then(|page| {
                    serde_json::from_value::<Page>(page)
                        .map_err(|error| format!("page is not a page: {error}"))
                })?;
            if !page.owner.is_empty() && page.owner != owner {
                return Err(format!(
                    "a page belongs to the fork that sets it, so its owner is {owner}; leave owner out"
                ));
            }
            page.owner = owner.to_owned();
            json!(store.set(page, now_ms).map_err(|error| error.to_string())?)
        }
        Some("read") => {
            let mine = store
                .list()
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|stored| stored.page.owner == owner)
                .collect::<Vec<_>>();
            json!({ "pages": mine })
        }
        Some("answers") => json!(store.show(id()?).map_err(|error| error.to_string())?),
        Some("clear") => {
            let cleared = store
                .clear(id()?, Some(owner))
                .map_err(|error| error.to_string())?;
            json!({ "cleared": [cleared.page.id] })
        }
        _ => return Err("session_page action is set, read, answers or clear".to_owned()),
    };
    serde_json::to_string(&shown).map_err(|error| format!("failed to encode the page: {error}"))
}
