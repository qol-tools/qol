use crate::signal::screen::{
    claude_awaiting_choice, claude_done, claude_working, has_numbered_choice_prompt,
};
use crate::signal::title::title_working;
use crate::strategy::{Ctx, Strategy};

pub struct Claude;

impl Strategy for Claude {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn working(&self, ctx: &Ctx) -> bool {
        ctx.screen.is_some_and(claude_working) || title_working(&ctx.pane.title)
    }

    fn awaiting(&self, ctx: &Ctx) -> bool {
        ctx.screen.is_some_and(|screen| {
            claude_awaiting_choice(screen) || has_numbered_choice_prompt(screen)
        })
    }

    fn turn_taken(&self, ctx: &Ctx) -> bool {
        ctx.screen.is_some_and(claude_done)
    }
}
