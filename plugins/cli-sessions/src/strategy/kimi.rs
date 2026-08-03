use crate::signal::screen::{has_numbered_choice_prompt, kimi_questionnaire, kimi_working};
use crate::strategy::{phase_for, Ctx, Phase, Reading, Strategy};

pub struct Kimi;

impl Strategy for Kimi {
    fn wants_screen(&self, _pane: &crate::host::Pane) -> bool {
        true
    }

    fn working(&self, ctx: &Ctx) -> bool {
        ctx.screen.is_some_and(kimi_working)
    }

    fn awaiting(&self, ctx: &Ctx) -> bool {
        ctx.screen
            .is_some_and(|screen| has_numbered_choice_prompt(screen) || kimi_questionnaire(screen))
    }

    fn turn_taken(&self, ctx: &Ctx) -> bool {
        ctx.cli_session.has_activity.unwrap_or(true)
    }

    fn stable_screen_hash<'a>(&self, text: &'a str) -> &'a str {
        let lines: Vec<&str> = text.lines().collect();
        let Some(box_top) = lines
            .iter()
            .rposition(|l| l.trim_start().starts_with('\u{256D}'))
        else {
            return text;
        };
        if lines.len().saturating_sub(box_top) > 6 {
            return text;
        }
        let end = lines[..box_top].iter().map(|l| l.len() + 1).sum();
        text.get(..end).unwrap_or(text)
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let working = self.working(ctx);
        let awaiting = self.awaiting(ctx);
        if awaiting && !working {
            return Reading {
                phase: Phase::Blocked,
                label: self.label(ctx),
            };
        }
        Reading {
            phase: phase_for(
                working,
                awaiting,
                self.turn_taken(ctx),
                ctx.screen_changed,
                ctx.prev.map(|prev| prev.status),
            ),
            label: self.label(ctx),
        }
    }
}
