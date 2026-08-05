pub mod claude;
pub mod codex;
pub mod kimi;
pub mod pi;

use crate::host::Pane;
use crate::session::status::Status;
use crate::session::tool::Tool;
use crate::signal::screen::{has_input_request, has_prompt_markers};
use qol_terminal_sessions::cli::CliSessionDescriptor;

const DONE_THRESHOLD_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Busy,
    Service,
    Blocked,
    Done,
    Idle,
    Hold,
}

#[derive(Debug, Clone, Copy)]
pub struct Prev {
    pub status: Status,
    pub running_since: Option<u64>,
}

pub struct Ctx<'a> {
    pub pane: &'a Pane,
    pub cli_session: CliSessionDescriptor,
    pub screen: Option<&'a str>,
    pub screen_changed: bool,
    pub prev: Option<Prev>,
    pub now: u64,
    pub is_service: bool,
}

pub struct Reading {
    pub phase: Phase,
    pub label: Option<String>,
}

pub trait Strategy {
    fn wants_screen(&self, pane: &Pane) -> bool {
        !pane.at_prompt
    }

    fn label(&self, ctx: &Ctx) -> Option<String> {
        ctx.cli_session.display_name.clone()
    }

    fn working(&self, _ctx: &Ctx) -> bool {
        false
    }

    fn awaiting(&self, _ctx: &Ctx) -> bool {
        false
    }

    fn turn_taken(&self, _ctx: &Ctx) -> bool {
        false
    }

    fn stable_screen_hash<'a>(&self, text: &'a str) -> &'a str {
        text
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let phase = phase_for(
            self.working(ctx),
            self.awaiting(ctx),
            self.turn_taken(ctx),
            ctx.screen_changed,
            ctx.prev.map(|p| p.status),
        );
        Reading {
            phase,
            label: self.label(ctx),
        }
    }
}

pub fn phase_for(
    working: bool,
    awaiting: bool,
    turn_taken: bool,
    screen_changed: bool,
    prev: Option<Status>,
) -> Phase {
    if working {
        return Phase::Busy;
    }
    if screen_changed && moving_screen_is_busy(prev, awaiting) {
        return Phase::Busy;
    }
    if awaiting {
        return Phase::Blocked;
    }
    if turn_taken {
        return Phase::Done;
    }
    if screen_changed {
        return Phase::Busy;
    }
    Phase::Idle
}

fn moving_screen_is_busy(prev: Option<Status>, awaiting: bool) -> bool {
    matches!(prev, None | Some(Status::Working)) || (prev == Some(Status::NeedsYou) && !awaiting)
}

pub fn running_since_for(prev_running: Option<u64>, phase: Phase, now: u64) -> Option<u64> {
    match phase {
        Phase::Busy | Phase::Service => Some(prev_running.unwrap_or(now)),
        Phase::Hold => prev_running,
        Phase::Blocked | Phase::Done | Phase::Idle => None,
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
        Phase::Service => Status::Service,
        Phase::Hold => prev,
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
impl Strategy for Cli {
    fn read(&self, ctx: &Ctx) -> Reading {
        let pane = ctx.pane;
        let phase = if pane.at_prompt {
            let prev_running = ctx.prev.and_then(|p| p.running_since);
            let finished_long = prev_running
                .map(|start| ctx.now.saturating_sub(start) > DONE_THRESHOLD_SECS)
                .unwrap_or(false);
            if finished_long {
                Phase::Done
            } else {
                Phase::Idle
            }
        } else if blocked(ctx) {
            Phase::Blocked
        } else if ctx.is_service {
            Phase::Service
        } else {
            Phase::Busy
        };
        Reading {
            phase,
            label: self.label(ctx),
        }
    }
}

pub fn for_tool(tool: Tool) -> Box<dyn Strategy> {
    match tool {
        Tool::Claude => Box::new(claude::Claude),
        Tool::Codex => Box::new(codex::Codex),
        Tool::Kimi => Box::new(kimi::Kimi),
        Tool::Pi => Box::new(pi::Pi),
        Tool::Generic => Box::new(Cli),
    }
}
