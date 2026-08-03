use crate::signal::screen::{has_numbered_choice_prompt, kimi_working};
use crate::strategy::{Ctx, Phase, Reading, Strategy};

pub struct Kimi;

impl Strategy for Kimi {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let screen = ctx.screen.unwrap_or("");
        let phase = if kimi_working(screen) {
            Phase::Busy
        } else if has_numbered_choice_prompt(screen) {
            Phase::Blocked
        } else if turn_taken(ctx) {
            Phase::Done
        } else {
            Phase::Idle
        };
        Reading {
            phase,
            label: self.label(ctx),
        }
    }
}

fn turn_taken(ctx: &Ctx) -> bool {
    ctx.cli_session.has_activity.unwrap_or(true)
}
