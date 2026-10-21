use std::time::Duration;

use anyhow::{bail, Result};
use qol_terminal_sessions::cli::{CliSessionInterpreter, ComposerGuard, Typed, Typing};
use qol_terminal_sessions::TerminalSessionService;

pub(crate) const TYPING_WAIT: Duration = Duration::from_secs(60);

pub(crate) fn type_into(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    guard: &mut ComposerGuard,
    typing: Typing<'_>,
) -> Result<()> {
    let binding = typing.binding;
    match guard.type_text_within(
        terminals,
        interpreter,
        typing,
        TYPING_WAIT,
        &mut std::thread::sleep,
    )? {
        Typed::Submitted | Typed::Inserted => Ok(()),
        Typed::Unsubmitted => bail!(
            "the text was typed into `{binding}` but not submitted, because its input box changed while it was typed"
        ),
        Typed::Held(hold) => bail!(
            "nothing was typed into `{binding}`: {hold} for {}s",
            TYPING_WAIT.as_secs()
        ),
    }
}
