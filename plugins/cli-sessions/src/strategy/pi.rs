use crate::signal::screen::{
    has_numbered_choice_prompt, pi_awaiting_choice, pi_banner, pi_working,
};
use crate::strategy::{Ctx, Phase, Reading, Strategy};

pub struct Pi;

impl Strategy for Pi {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let screen = ctx.screen.unwrap_or("");
        let phase = if pi_working(screen) {
            Phase::Busy
        } else if pi_awaiting_choice(screen) || has_numbered_choice_prompt(screen) {
            Phase::Blocked
        } else if turn_taken(ctx, screen) {
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

fn turn_taken(ctx: &Ctx, screen: &str) -> bool {
    ctx.cli_session
        .has_activity
        .unwrap_or_else(|| !pi_banner(screen))
}
