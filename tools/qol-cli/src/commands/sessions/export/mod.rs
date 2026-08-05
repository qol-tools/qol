use std::ffi::OsString;

use anyhow::{bail, Result};

mod pi;

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    match args.first().and_then(|argument| argument.to_str()) {
        Some("pi") => print!("{}", pi::pi_extension()?),
        Some("help") | Some("-h") | Some("--help") => print!("{}", help_text()),
        Some(other) => bail!("unknown export client `{other}`\n\n{}", help_text()),
        None => print!("{}", help_text()),
    }
    Ok(())
}

pub(crate) fn help_text() -> &'static str {
    "qol sessions export\n\nGenerate a per-client agent-tool surface from the shared sessions tool contract\nin tools/qol-cli/src/commands/sessions/contract.rs.\n\nUsage:\n  qol sessions export\n  qol sessions export pi\n  qol sessions export help\n\nClients:\n  pi   pi extension source registering the session tools; the qol-skills\n       manifest sync writes it to plugins/qol-sessions/extensions/hooks.ts\n\nOutput:\n  The client surface goes to stdout. Every client is rendered from the same\n  contract, so tool names, descriptions, and input schemas never drift.\n\nExit:\n  Exits non-zero when a contract tool has no adapter for the requested client.\n"
}
