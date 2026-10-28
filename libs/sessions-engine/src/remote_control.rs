use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::{
    CliRemoteControlToggle, CliSessionInterpreter, ComposerGuard, Typing,
};
use qol_terminal_sessions::{ScreenReader, SessionBinding, TerminalSessionService, TextInput};
use serde::Serialize;

const SWITCH_WAIT: Duration = Duration::from_secs(10);
const MENU_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(250);
const MENU_STEPS: usize = 4;

#[derive(Debug, Serialize)]
pub(super) struct RemoteControlOutcome {
    pub(super) session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) key: Option<String>,
    pub(super) remote_control: bool,
    pub(super) changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) remote_control_url: Option<String>,
}

pub(super) fn execute(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    on: bool,
    caller: &str,
) -> Result<RemoteControlOutcome> {
    if binding.token() == caller {
        bail!("`{binding}` is the calling session, which cannot type into its own prompt mid-turn; run /remote-control in it instead");
    }
    let facts = super::bridge::resolve_target(terminals, binding)?;
    let tool = interpreter.describe(&facts).tool.id;
    let toggle = interpreter
        .remote_control_toggle(&facts)
        .ok_or_else(|| anyhow!("`{binding}` runs `{tool}`, which has no Remote Control"))?;
    let key = facts
        .spawn_identity
        .as_ref()
        .map(|identity| identity.key.as_str().to_owned());
    let current = interpreter.remote_control_url(&facts);
    if current.is_some() == on {
        return Ok(RemoteControlOutcome {
            session: binding.token(),
            key,
            remote_control: on,
            changed: false,
            remote_control_url: current,
        });
    }
    let command = match on {
        true => toggle.enable_command(key.as_deref()),
        false => toggle.command.to_owned(),
    };
    super::typing::type_into(
        terminals,
        interpreter,
        &mut ComposerGuard::default(),
        Typing::submit(binding, &command),
    )?;
    if !on {
        pick_disconnect(terminals, binding, &toggle)?;
    }
    let url = await_switch(terminals, interpreter, binding, on);
    if url.is_some() != on {
        bail!(
            "`{command}` was submitted in `{binding}`, but Remote Control did not turn {} within {}s",
            if on { "on" } else { "off" },
            SWITCH_WAIT.as_secs()
        );
    }
    Ok(RemoteControlOutcome {
        session: binding.token(),
        key,
        remote_control: on,
        changed: true,
        remote_control_url: url,
    })
}

fn pick_disconnect(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
    toggle: &CliRemoteControlToggle,
) -> Result<()> {
    let read = || {
        terminals
            .read_screen(binding)
            .context("reading the Remote Control menu failed")
    };
    let deadline = Instant::now() + MENU_WAIT;
    let mut screen = read()?;
    while !toggle.disconnect_menu_open(&screen) {
        if Instant::now() >= deadline {
            bail!(
                "the Remote Control menu did not open in `{binding}` within {}s",
                MENU_WAIT.as_secs()
            );
        }
        std::thread::sleep(POLL);
        screen = read()?;
    }
    for _ in 0..MENU_STEPS {
        if toggle.disconnect_selected(&screen) {
            return terminals
                .send_key(binding, toggle.menu_select_key)
                .context("choosing the disconnect entry failed");
        }
        terminals
            .send_key(binding, toggle.menu_up_key)
            .context("moving through the Remote Control menu failed")?;
        std::thread::sleep(POLL);
        screen = read()?;
    }
    let _ = terminals.send_key(binding, toggle.menu_cancel_key);
    bail!(
        "the Remote Control menu in `{binding}` never put its cursor on `{}`, so it was dismissed",
        toggle.disconnect_choice
    )
}

fn await_switch(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    on: bool,
) -> Option<String> {
    let deadline = Instant::now() + SWITCH_WAIT;
    loop {
        let url = super::bridge::resolve_target(terminals, binding)
            .ok()
            .and_then(|facts| interpreter.remote_control_url(&facts));
        if url.is_some() == on || Instant::now() >= deadline {
            return url;
        }
        std::thread::sleep(POLL);
    }
}
