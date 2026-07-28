use crate::signal::screen::{
    claude_awaiting_choice, claude_done, claude_working, has_numbered_choice_prompt,
};
use crate::signal::title::title_working;
use crate::strategy::{Ctx, Phase, Reading, Strategy};

pub struct Claude;

impl Strategy for Claude {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let screen = ctx.screen.unwrap_or("");
        let phase = if claude_working(screen) || title_working(&ctx.pane.title) {
            Phase::Busy
        } else if claude_awaiting_choice(screen) || has_numbered_choice_prompt(screen) {
            Phase::Blocked
        } else if claude_done(screen) {
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
