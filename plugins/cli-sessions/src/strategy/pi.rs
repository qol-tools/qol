use crate::signal::screen::{
    has_numbered_choice_prompt, pi_awaiting_choice, pi_banner, pi_working,
};
use crate::strategy::{Ctx, Strategy};

pub struct Pi;

impl Strategy for Pi {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn working(&self, ctx: &Ctx) -> bool {
        ctx.screen.is_some_and(pi_working)
    }

    fn awaiting(&self, ctx: &Ctx) -> bool {
        ctx.screen
            .is_some_and(|screen| pi_awaiting_choice(screen) || has_numbered_choice_prompt(screen))
    }

    fn turn_taken(&self, ctx: &Ctx) -> bool {
        ctx.cli_session
            .has_activity
            .unwrap_or_else(|| !ctx.screen.is_some_and(pi_banner))
    }

    fn stable_screen_hash<'a>(&self, text: &'a str) -> &'a str {
        let lines: Vec<&str> = text.lines().collect();
        let is_rule = |l: &str| !l.trim().is_empty() && l.trim().chars().all(|c| c == '\u{2500}');
        let Some(border) = lines.iter().rposition(|l| is_rule(l)) else {
            return text;
        };
        if lines.len().saturating_sub(border) > 5 {
            return text;
        }
        let paired = lines[..border]
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| is_rule(l));
        if !paired {
            return text;
        }
        let end = lines[..border].iter().map(|l| l.len() + 1).sum();
        text.get(..end).unwrap_or(text)
    }
}
