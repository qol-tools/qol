use qol_config::contract::NumberConstraints;

use super::numbers::{
    format_number, number_preview, parsed_number, stepped_number, NumberSpec, COARSE_STEP,
};
use crate::text_edit::{Span, TextField};

#[derive(Clone, Debug)]
pub struct NumberEdit {
    field: TextField,
    capped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumberKey {
    Step { direction: f64, coarse: bool },
    Lowest,
    Highest,
    Type(String),
    Erase,
    Apply,
    Restore,
}

pub fn number_key(key: &str, key_char: Option<&str>, coarse: bool) -> Option<NumberKey> {
    if let Some(direction) = horizontal_step_direction(key) {
        return Some(NumberKey::Step { direction, coarse });
    }
    match key {
        "home" => Some(NumberKey::Lowest),
        "end" => Some(NumberKey::Highest),
        "enter" | "return" => Some(NumberKey::Apply),
        "escape" | "esc" => Some(NumberKey::Restore),
        "backspace" => Some(NumberKey::Erase),
        _ => key_char
            .filter(|typed| !typed.is_empty())
            .map(|typed| NumberKey::Type(typed.to_string())),
    }
}

pub(in crate::settings_panel) fn horizontal_step_direction(key: &str) -> Option<f64> {
    match key {
        "left" => Some(-1.0),
        "right" => Some(1.0),
        _ => None,
    }
}

impl NumberEdit {
    pub fn begin(value: f64) -> Self {
        Self::with_text(format_number(value))
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        let mut field = TextField::with_text(text);
        field.select_all();
        Self {
            field,
            capped: false,
        }
    }

    pub fn text(&self) -> &str {
        self.field.text()
    }

    pub fn into_text(self) -> String {
        self.field.into_text()
    }

    pub fn capped(&self) -> bool {
        self.capped
    }

    pub fn selects_all(&self) -> bool {
        self.field.selected_range().is_some()
    }

    pub fn select_all(&mut self) {
        self.field.select_all();
    }

    pub fn display(&self) -> String {
        edit_display(&self.field)
    }

    pub fn insert(&mut self, typed: &str) {
        self.field.insert_str(typed);
    }

    pub fn erase(&mut self) {
        erase_typed(&mut self.field, &mut self.capped);
    }

    pub fn preview(&self, number: &NumberSpec) -> f64 {
        number_preview(
            Some(self.text()),
            number.value,
            number.min,
            number.max,
            number.step,
        )
    }

    pub fn value(&self, number: &NumberSpec) -> Option<f64> {
        parsed_number(self.text(), number.min, number.max, number.step)
    }

    pub fn press(&mut self, key: &NumberKey, number: &NumberSpec) -> bool {
        match key {
            NumberKey::Step { direction, coarse } => {
                let scale = if *coarse { COARSE_STEP } else { 1.0 };
                let stepped = stepped_number(
                    self.text(),
                    number.value,
                    number.min,
                    number.max,
                    number.step.unwrap_or(1.0),
                    *direction * scale,
                );
                self.replace(stepped)
            }
            NumberKey::Lowest => self.reach(number.min, number),
            NumberKey::Highest => self.reach(number.max, number),
            NumberKey::Type(typed) => {
                let before = self.preview(number);
                type_number(
                    &mut self.field,
                    &mut self.capped,
                    typed,
                    &number.constraints(),
                );
                self.preview(number) != before
            }
            NumberKey::Erase => {
                let before = self.preview(number);
                self.erase();
                self.preview(number) != before
            }
            NumberKey::Apply | NumberKey::Restore => false,
        }
    }

    fn reach(&mut self, bound: Option<f64>, number: &NumberSpec) -> bool {
        let Some(bound) = bound else {
            return false;
        };
        let target = parsed_number(&format_number(bound), number.min, number.max, number.step)
            .unwrap_or(bound);
        self.replace(format_number(target))
    }

    fn replace(&mut self, text: String) -> bool {
        let changed = self.field.text() != text;
        self.field.set_text(text);
        self.capped = false;
        changed
    }
}

pub(in crate::settings_panel) fn type_number(
    edit: &mut TextField,
    capped: &mut bool,
    typed: &str,
    bounds: &NumberConstraints,
) {
    let signed = bounds.lowest() < 0.0;
    if !typed
        .chars()
        .all(|ch| ch.is_ascii_digit() || ch == '.' || (signed && ch == '-'))
    {
        return;
    }
    edit.insert_str(typed);
    *capped = edit
        .text()
        .trim()
        .parse::<f64>()
        .is_ok_and(|value| value > bounds.highest() && value.is_sign_positive());
    if *capped {
        edit.set_text(format_number(bounds.highest()));
    }
}

pub(in crate::settings_panel) fn erase_typed(edit: &mut TextField, capped: &mut bool) {
    edit.backspace(Span::Char);
    *capped = false;
}

pub(in crate::settings_panel) fn edit_display(edit: &TextField) -> String {
    if edit.selected_range().is_some() {
        return edit.text().to_string();
    }
    format!("{}_", edit.text())
}
