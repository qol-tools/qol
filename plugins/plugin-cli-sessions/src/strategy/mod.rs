pub mod claude;
pub mod codex;

use crate::host::Pane;
use crate::signal::screen::{has_input_request, has_prompt_markers};
use crate::status::Status;
use crate::tool::Tool;

const DONE_THRESHOLD_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Busy,
    Blocked,
    Done,
    Idle,
}

#[derive(Debug, Clone, Copy)]
pub struct Prev {
    pub status: Status,
    pub running_since: Option<u64>,
}

pub struct Ctx<'a> {
    pub pane: &'a Pane,
    pub screen: Option<&'a str>,
    pub screen_changed: bool,
    pub prev: Option<Prev>,
    pub now: u64,
}

pub struct Reading {
    pub phase: Phase,
    pub label: Option<String>,
    pub running_since: Option<u64>,
}

pub trait Strategy {
    fn wants_screen(&self, pane: &Pane) -> bool {
        !pane.at_prompt
    }

    fn label(&self, ctx: &Ctx) -> Option<String> {
        ctx.pane.reported_cmd.clone().filter(|c| !c.is_empty())
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let pane = ctx.pane;
        let prev_running = ctx.prev.and_then(|p| p.running_since);
        let (phase, running_since) = if pane.at_prompt {
            let finished_long = prev_running
                .map(|start| ctx.now.saturating_sub(start) > DONE_THRESHOLD_SECS)
                .unwrap_or(false);
            let phase = if finished_long {
                Phase::Done
            } else {
                Phase::Idle
            };
            (phase, None)
        } else {
            let phase = if blocked(ctx) {
                Phase::Blocked
            } else {
                Phase::Busy
            };
            (phase, Some(prev_running.unwrap_or(ctx.now)))
        };
        Reading {
            phase,
            label: self.label(ctx),
            running_since,
        }
    }
}

fn blocked(ctx: &Ctx) -> bool {
    let Some(screen) = ctx.screen else {
        return false;
    };
    !ctx.screen_changed && (has_prompt_markers(screen) || has_input_request(screen))
}

pub fn status_for(prev: Status, phase: Phase) -> Status {
    match phase {
        Phase::Blocked => Status::NeedsYou,
        Phase::Busy => Status::Working,
        Phase::Done => {
            if prev == Status::Acknowledged {
                Status::Acknowledged
            } else {
                Status::YourTurn
            }
        }
        Phase::Idle => {
            if prev == Status::Acknowledged {
                Status::Acknowledged
            } else {
                Status::Unknown
            }
        }
    }
}

pub struct Cli;
impl Strategy for Cli {}

pub fn for_tool<'a>(tool: Tool, codex_store: &'a dyn codex::CodexStore) -> Box<dyn Strategy + 'a> {
    match tool {
        Tool::Claude => Box::new(claude::Claude),
        Tool::Codex => Box::new(codex::Codex::new(codex_store)),
        Tool::Generic => Box::new(Cli),
    }
}
