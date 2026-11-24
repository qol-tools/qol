use gpui::prelude::*;
use gpui::{div, px, App, ClickEvent, Div, SharedString, Window};

use crate::key::Key;
use crate::kit::Kit;
use crate::settings_panel::{intent, CustomHints, Intent};

use super::{
    inked_label_group, settings_action_spinner, settings_label_group, settings_tag,
    settings_value_group, HintTone, RowGround, SettingsGroupHeader, SettingsHint, SettingsRow,
    SettingsValueTone,
};

const SPINNER_SIZE: f32 = 12.0;

#[derive(Clone, Debug)]
pub struct SettingsAnswer<A> {
    pub action: A,
    pub label: SharedString,
    pub detail: SharedString,
    pub verb: SharedString,
    pub tone: HintTone,
    pub busy_detail: SharedString,
    pub busy_hint: SharedString,
}

impl<A> SettingsAnswer<A> {
    pub fn new(
        action: A,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        verb: impl Into<SharedString>,
    ) -> Self {
        let verb = verb.into();
        Self {
            action,
            label: label.into(),
            detail: detail.into(),
            busy_detail: SharedString::default(),
            busy_hint: verb.clone(),
            verb,
            tone: HintTone::Plain,
        }
    }

    pub fn tone(mut self, tone: HintTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn busy(mut self, detail: impl Into<SharedString>, hint: impl Into<SharedString>) -> Self {
        self.busy_detail = detail.into();
        self.busy_hint = hint.into();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionKey<A> {
    Answer(A),
    Back,
    Moved,
    Held,
}

#[derive(Clone, Debug)]
pub struct SettingsQuestion<A> {
    title: SharedString,
    colophon: SharedString,
    answers: Vec<SettingsAnswer<A>>,
    selected: usize,
    busy: Option<usize>,
}

impl<A: Copy + PartialEq> SettingsQuestion<A> {
    pub fn new(title: impl Into<SharedString>, colophon: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            colophon: colophon.into(),
            answers: Vec::new(),
            selected: 0,
            busy: None,
        }
    }

    pub fn answer(mut self, answer: SettingsAnswer<A>) -> Self {
        self.answers.push(answer);
        self
    }

    pub fn detail(mut self, action: A, detail: impl Into<SharedString>) -> Self {
        let detail = detail.into();
        if let Some(answer) = self
            .answers
            .iter_mut()
            .find(|answer| answer.action == action)
        {
            answer.detail = detail;
        }
        self
    }

    pub fn save_changes(name: Option<&str>, item: &str, save: A, discard: A) -> Self {
        let (colophon, save_label, save_detail, busy_detail) = match name {
            Some(name) => (
                format!("you changed {}.", name.to_lowercase()),
                "Save changes".to_string(),
                format!("{} keeps your changes.", sentence(name)),
                format!("Saving {name}."),
            ),
            None => (
                format!("you are adding a new {item}."),
                format!("Save the new {item}"),
                format!("The new {item} is added."),
                format!("Saving the new {item}."),
            ),
        };
        Self::new("save changes", colophon)
            .answer(
                SettingsAnswer::new(save, save_label, save_detail, "save")
                    .tone(HintTone::Save)
                    .busy(busy_detail, "saving"),
            )
            .answer(discard_answer(discard, name, item))
    }

    pub fn blocked(field: &str, name: Option<&str>, item: &str, fill: A, discard: A) -> Self {
        let field = field.to_lowercase();
        Self::new(
            format!("{field} is empty"),
            format!("the {item} cannot be saved without it."),
        )
        .answer(SettingsAnswer::new(
            fill,
            format!("Fill in the {field}"),
            format!("Goes back to the {field} field."),
            "fill in",
        ))
        .answer(discard_answer(discard, name, item))
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn colophon(&self) -> &SharedString {
        &self.colophon
    }

    pub fn answers(&self) -> &[SettingsAnswer<A>] {
        &self.answers
    }

    pub fn selected(&self) -> Option<A> {
        self.answers.get(self.selected).map(|answer| answer.action)
    }

    pub fn select(&mut self, action: A) {
        if let Some(index) = self
            .answers
            .iter()
            .position(|answer| answer.action == action)
        {
            self.selected = index;
        }
    }

    pub fn busy(&self) -> bool {
        self.busy.is_some()
    }

    pub fn set_busy(&mut self, action: Option<A>) {
        self.busy = action.and_then(|action| {
            self.answers
                .iter()
                .position(|answer| answer.action == action)
        });
    }

    pub fn on_key(&mut self, key: &str, key_char: Option<&str>) -> QuestionKey<A> {
        if self.busy.is_some() {
            return QuestionKey::Held;
        }
        match intent(key, key_char, false) {
            Some(Intent::Up) if self.selected > 0 => {
                self.selected -= 1;
                QuestionKey::Moved
            }
            Some(Intent::Down) if self.selected + 1 < self.answers.len() => {
                self.selected += 1;
                QuestionKey::Moved
            }
            Some(Intent::Activate) => self
                .selected()
                .map_or(QuestionKey::Held, QuestionKey::Answer),
            Some(Intent::Close) => QuestionKey::Back,
            _ => QuestionKey::Held,
        }
    }

    pub fn hints(&self) -> CustomHints {
        if let Some(answer) = self.busy.and_then(|index| self.answers.get(index)) {
            return CustomHints {
                left: vec![SettingsHint::busy(answer.busy_hint.clone())],
                right: Vec::new(),
            };
        }
        let mut left = Vec::new();
        if let Some(answer) = self.answers.get(self.selected) {
            left.push(SettingsHint::new(Key::ENTER, answer.verb.clone()).tone(answer.tone));
        }
        if self.answers.len() > 1 {
            left.push(SettingsHint::new(Key::UP_DOWN, "move"));
        }
        CustomHints {
            left,
            right: vec![SettingsHint::new(Key::ESC, "back")],
        }
    }

    pub fn render<H>(
        &self,
        id: &'static str,
        focused: bool,
        kit: Kit,
        click: impl Fn(A) -> H,
    ) -> Div
    where
        H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(
                SettingsGroupHeader::new(self.title.clone(), Some(self.colophon.clone()), kit)
                    .current(focused),
            );
        for (index, answer) in self.answers.iter().enumerate() {
            body = body.child(self.render_answer(id, index, answer, focused, kit, &click));
        }
        body
    }

    fn render_answer<H>(
        &self,
        id: &'static str,
        index: usize,
        answer: &SettingsAnswer<A>,
        focused: bool,
        kit: Kit,
        click: &impl Fn(A) -> H,
    ) -> SettingsRow
    where
        H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        let selected = index == self.selected;
        let row = RowGround::of(selected, focused);
        let running = self.busy == Some(index);
        let detail = if running {
            answer.busy_detail.clone()
        } else {
            answer.detail.clone()
        };
        let label = match answer.tone {
            HintTone::Plain => settings_label_group(answer.label.clone(), Some(detail), row, kit),
            HintTone::Save => inked_label_group(
                answer.label.clone(),
                Some(detail),
                kit.palette.success,
                row,
                kit,
            ),
            HintTone::Discard => inked_label_group(
                answer.label.clone(),
                Some(detail),
                kit.palette.danger,
                row,
                kit,
            ),
        };
        let end = if running {
            settings_value_group()
                .child(settings_action_spinner((id, index), kit).size(px(SPINNER_SIZE)))
        } else if self.busy.is_some() {
            settings_value_group()
        } else {
            settings_value_group().child(settings_tag(
                answer.verb.clone(),
                answer_tag_tone(answer.tone),
                row,
                kit,
            ))
        };
        let row = SettingsRow::setting((id, index), kit)
            .selected(selected, focused)
            .destroys(answer.tone == HintTone::Discard)
            .child(label)
            .child(end);
        if self.busy.is_some() {
            row
        } else {
            row.on_click(click(answer.action))
        }
    }
}

fn discard_answer<A>(action: A, name: Option<&str>, item: &str) -> SettingsAnswer<A> {
    let (label, detail) = match name {
        Some(name) => (
            "Discard changes".to_string(),
            format!("{} goes back the way it was.", sentence(name)),
        ),
        None => (
            format!("Discard the new {item}"),
            "Nothing is added.".to_string(),
        ),
    };
    SettingsAnswer::new(action, label, detail, "discard").tone(HintTone::Discard)
}

fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn answer_tag_tone(tone: HintTone) -> SettingsValueTone {
    match tone {
        HintTone::Plain => SettingsValueTone::Normal,
        HintTone::Save => SettingsValueTone::Success,
        HintTone::Discard => SettingsValueTone::Danger,
    }
}

#[cfg(test)]
mod tests {
    use super::{answer_tag_tone, sentence, QuestionKey, SettingsAnswer, SettingsQuestion};
    use crate::key::Key;
    use crate::settings_panel::components::{HintTone, SettingsValueTone};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Answer {
        Keep,
        Restart,
    }

    fn restart() -> SettingsQuestion<Answer> {
        SettingsQuestion::new("restart qol-tray", "#631's build carries qol-tray.")
            .answer(SettingsAnswer::new(
                Answer::Keep,
                "Keep the installed version",
                "Nothing is installed.",
                "keep",
            ))
            .answer(
                SettingsAnswer::new(
                    Answer::Restart,
                    "Restart on #631's build",
                    "qol-tray restarts.",
                    "restart",
                )
                .tone(HintTone::Discard)
                .busy(
                    "Restarting qol-tray on #631's build.",
                    "restarting qol-tray",
                ),
            )
    }

    #[test]
    fn the_cursor_starts_on_the_first_answer_and_moves_within_the_rows() {
        let mut question = restart();
        assert_eq!(question.selected(), Some(Answer::Keep));
        assert_eq!(question.on_key("up", None), QuestionKey::Held);
        assert_eq!(question.on_key("down", None), QuestionKey::Moved);
        assert_eq!(question.selected(), Some(Answer::Restart));
        assert_eq!(question.on_key("down", None), QuestionKey::Held);
        assert_eq!(
            question.on_key("enter", None),
            QuestionKey::Answer(Answer::Restart)
        );
        assert_eq!(question.on_key("escape", None), QuestionKey::Back);
        assert_eq!(question.on_key("x", Some("x")), QuestionKey::Held);
    }

    #[test]
    fn a_busy_card_holds_every_key_and_shows_only_the_busy_hint() {
        let mut question = restart();
        question.set_busy(Some(Answer::Restart));
        assert!(question.busy());
        for key in ["enter", "escape", "up", "down"] {
            assert_eq!(question.on_key(key, None), QuestionKey::Held);
        }
        let hints = question.hints();
        assert_eq!(hints.left.len(), 1);
        assert!(hints.left[0].busy);
        assert_eq!(hints.left[0].label, "restarting qol-tray");
        assert!(hints.right.is_empty());
        question.set_busy(None);
        assert!(!question.busy());
    }

    #[test]
    fn the_hints_name_the_selected_answer_in_its_tone() {
        let mut question = restart();
        let hints = question.hints();
        assert_eq!(hints.left[0].key, Some(Key::ENTER));
        assert_eq!(hints.left[0].label, "keep");
        assert_eq!(hints.left[0].tone, HintTone::Plain);
        assert_eq!(hints.left[1].label, "move");
        assert_eq!(hints.right[0].key, Some(Key::ESC));
        assert_eq!(hints.right[0].label, "back");
        question.select(Answer::Restart);
        let hints = question.hints();
        assert_eq!(hints.left[0].label, "restart");
        assert_eq!(hints.left[0].tone, HintTone::Discard);
    }

    #[test]
    fn save_changes_names_the_item_and_puts_the_cursor_on_save() {
        let question = SettingsQuestion::save_changes(Some("Docs"), "shortcut", 1, 2);
        assert_eq!(question.title(), "save changes");
        assert_eq!(question.colophon, "you changed docs.");
        assert_eq!(question.selected(), Some(1));
        assert_eq!(question.answers[0].label, "Save changes");
        assert_eq!(question.answers[0].tone, HintTone::Save);
        assert_eq!(question.answers[0].busy_detail, "Saving Docs.");
        assert_eq!(question.answers[0].busy_hint, "saving");
        assert_eq!(question.answers[1].label, "Discard changes");
        assert_eq!(question.answers[1].detail, "Docs goes back the way it was.");
        assert_eq!(question.answers[1].tone, HintTone::Discard);

        let new = SettingsQuestion::save_changes(None, "hotkey", 1, 2).detail(1, "Runs Shot.");
        assert_eq!(new.colophon, "you are adding a new hotkey.");
        assert_eq!(new.answers[0].label, "Save the new hotkey");
        assert_eq!(new.answers[0].detail, "Runs Shot.");
        assert_eq!(new.answers[1].label, "Discard the new hotkey");
    }

    #[test]
    fn blocked_names_the_empty_field_and_offers_to_fill_it_in() {
        let question = SettingsQuestion::blocked("Name", None, "shortcut", 1, 2);
        assert_eq!(question.title(), "name is empty");
        assert_eq!(
            question.colophon,
            "the shortcut cannot be saved without it."
        );
        assert_eq!(question.selected(), Some(1));
        assert_eq!(question.answers[0].label, "Fill in the name");
        assert_eq!(question.answers[0].verb, "fill in");
        assert_eq!(question.answers[0].tone, HintTone::Plain);
        assert_eq!(question.answers[1].verb, "discard");
    }

    #[test]
    fn details_start_a_sentence_with_a_capital() {
        assert_eq!(sentence("zed"), "Zed");
        assert_eq!(sentence("this path"), "This path");
        assert_eq!(sentence(""), "");
        let question = SettingsQuestion::save_changes(Some("zed"), "application", 1, 2);
        assert_eq!(question.answers()[0].detail, "Zed keeps your changes.");
        assert_eq!(
            question.answers()[1].detail,
            "Zed goes back the way it was."
        );
        assert_eq!(question.colophon(), "you changed zed.");
    }

    #[test]
    fn answer_tags_take_the_answer_tone() {
        assert_eq!(answer_tag_tone(HintTone::Plain), SettingsValueTone::Normal);
        assert_eq!(answer_tag_tone(HintTone::Save), SettingsValueTone::Success);
        assert_eq!(
            answer_tag_tone(HintTone::Discard),
            SettingsValueTone::Danger
        );
    }
}
