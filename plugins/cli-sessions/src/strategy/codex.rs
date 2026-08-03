use crate::signal::screen::{codex_banner, codex_working, has_numbered_choice_prompt};
use crate::signal::title::title_working;
use crate::strategy::{Ctx, Strategy};

pub struct Codex;

impl Strategy for Codex {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn working(&self, ctx: &Ctx) -> bool {
        title_working(&ctx.pane.title) || ctx.screen.is_some_and(codex_working)
    }

    fn awaiting(&self, ctx: &Ctx) -> bool {
        ctx.screen.is_some_and(has_numbered_choice_prompt)
    }

    fn turn_taken(&self, ctx: &Ctx) -> bool {
        ctx.cli_session
            .has_activity
            .unwrap_or_else(|| !ctx.screen.is_some_and(codex_banner))
    }
}
