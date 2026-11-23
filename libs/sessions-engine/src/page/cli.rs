use std::ffi::OsString;
use std::io::Read;

use serde_json::json;

use super::model::{Page, Verdict};
use super::store::PageStore;
use crate::verb::{Failure, Flags, Outcome};

pub(crate) const HELP: &str = include_str!("help.txt");

const STDIN: &str = "-";
const READ_MAX: u64 = 256 * 1024;

pub(crate) fn execute(
    args: &[OsString],
    store: &PageStore,
    stdin: &mut dyn Read,
    now_ms: u64,
) -> Outcome {
    crate::verb::execute(args, "page", HELP, |verb, flags| {
        run_verb(verb, flags, store, stdin, now_ms)
    })
}

fn run_verb(
    verb: &str,
    flags: &Flags,
    store: &PageStore,
    stdin: &mut dyn Read,
    now_ms: u64,
) -> Result<Outcome, Failure> {
    match verb {
        "set" => {
            flags.only(&["file"])?;
            let text = page_text(flags.get("file").unwrap_or(STDIN), stdin)?;
            let page = serde_json::from_str::<Page>(&text)
                .map_err(|error| Failure::Usage(format!("the page JSON is not a page: {error}")))?;
            Ok(Outcome::ok(json!(store.set(page, now_ms)?)))
        }
        "read" => {
            flags.only(&[])?;
            Ok(Outcome::ok(json!({ "pages": store.list()? })))
        }
        "answers" => {
            flags.only(&["id"])?;
            Ok(Outcome::ok(json!(store.show(flags.required("id")?)?)))
        }
        "answer" => {
            flags.only(&["id", "item", "verdict", "note"])?;
            let verdict = Verdict::parse(flags.required("verdict")?)
                .ok_or_else(|| Failure::Usage("--verdict is pass, fail or skip".to_owned()))?;
            Ok(Outcome::ok(json!(store.answer(
                flags.required("id")?,
                flags.required("item")?,
                verdict,
                flags.get("note"),
                now_ms,
            )?)))
        }
        "clear" => {
            flags.only(&["id", "owner"])?;
            match (flags.get("id"), flags.get("owner")) {
                (Some(id), owner) => {
                    let cleared = store.clear(id, owner)?;
                    Ok(Outcome::ok(json!({ "cleared": [cleared.page.id] })))
                }
                (None, Some(owner)) => {
                    Ok(Outcome::ok(json!({ "cleared": store.clear_owner(owner)? })))
                }
                (None, None) => Err(Failure::Usage(
                    "clear takes --id, --owner, or both".to_owned(),
                )),
            }
        }
        other => Err(Failure::Usage(format!("unknown page verb `{other}`"))),
    }
}

fn page_text(file: &str, stdin: &mut dyn Read) -> Result<String, Failure> {
    let cannot = |error: std::io::Error| {
        Failure::Usage(format!("cannot read the page from {file}: {error}"))
    };
    let mut text = String::new();
    if file == STDIN {
        stdin
            .take(READ_MAX)
            .read_to_string(&mut text)
            .map_err(cannot)?;
    } else {
        std::fs::File::open(file)
            .map_err(cannot)?
            .take(READ_MAX)
            .read_to_string(&mut text)
            .map_err(cannot)?;
    }
    Ok(text)
}
