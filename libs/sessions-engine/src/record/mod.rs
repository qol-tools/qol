mod cli;
mod key;
mod search;
mod store;
#[cfg(test)]
mod tests;

use std::ffi::OsString;

use anyhow::Result;

pub(crate) use search::rows;
pub(crate) use store::RecordStore;

pub fn execute(args: &[OsString]) -> (String, u8) {
    match RecordStore::system() {
        Ok(store) => {
            let outcome = cli::execute(args, &store, super::mailbox::now_ms());
            (outcome.stdout, outcome.code)
        }
        Err(error) => (
            serde_json::json!({ "error": format!("{error:#}") }).to_string(),
            1,
        ),
    }
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let (stdout, code) = execute(args);
    crate::cli::line(stdout);
    if code != 0 {
        std::process::exit(i32::from(code));
    }
    Ok(())
}
