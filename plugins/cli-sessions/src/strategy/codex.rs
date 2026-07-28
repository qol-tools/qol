use crate::signal::screen::{codex_banner, codex_working, has_numbered_choice_prompt};
use crate::signal::title::title_working;
use crate::strategy::{Ctx, Phase, Reading, Strategy};

pub struct Codex;

impl Strategy for Codex {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let screen = ctx.screen.unwrap_or("");
        let phase = if title_working(&ctx.pane.title) || codex_working(screen) {
            Phase::Busy
        } else if has_numbered_choice_prompt(screen) {
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
        .unwrap_or_else(|| !codex_banner(screen))
}
