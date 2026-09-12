use gpui::prelude::*;
use gpui::{div, px, rgba, AnyElement, App, ClickEvent, ElementId, RenderOnce, Window};

use crate::kit::kit;
use crate::theme::SettingsPanelPalette;

use super::{paint_settings_attention, paint_settings_selection, DIMMED_OPACITY};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Setting,
    Rule,
    Add,
}

#[derive(IntoElement)]
pub struct SettingsRow {
    id: ElementId,
    palette: SettingsPanelPalette,
    kind: RowKind,
    selected: bool,
    focused: bool,
    dimmed: bool,
    attention: bool,
    children: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl SettingsRow {
    pub fn setting(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Self {
        Self::new(id, palette, RowKind::Setting)
    }

    pub fn rule(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Self {
        Self::new(id, palette, RowKind::Rule)
    }

    pub fn add(id: impl Into<ElementId>, palette: SettingsPanelPalette) -> Self {
        Self::new(id, palette, RowKind::Add)
    }

    fn new(id: impl Into<ElementId>, palette: SettingsPanelPalette, kind: RowKind) -> Self {
        Self {
            id: id.into(),
            palette,
            kind,
            selected: false,
            focused: true,
            dimmed: false,
            attention: false,
            children: Vec::new(),
            on_click: None,
        }
    }

    pub fn selected(mut self, selected: bool, focused: bool) -> Self {
        self.selected = selected;
        self.focused = focused;
        self
    }

    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub fn attention(mut self, attention: bool) -> Self {
        self.attention = attention;
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        let height = match self.kind {
            RowKind::Setting => qol_theme::HEIGHT_SETTING_ROW,
            RowKind::Rule | RowKind::Add => qol_theme::HEIGHT_RULE_ROW,
        };
        let mut row = div()
            .id(self.id)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(qol_theme::SPACE_CELL))
            .w_full()
            .h(px(height))
            .px(px(qol_theme::SPACE_INSET))
            .py(px(qol_theme::SPACE_TIGHT))
            .rounded_none()
            .children(self.children);
        if self.kind == RowKind::Rule {
            row = row.rounded(px(qol_theme::RADIUS_CONTROL));
        }
        if self.dimmed {
            row = row.opacity(DIMMED_OPACITY);
        }
        if self.selected && self.focused {
            row = paint_settings_selection(row, self.palette);
        } else if self.attention {
            row = paint_settings_attention(row, self.palette);
        }
        if let Some(on_click) = self.on_click {
            row = row
                .cursor(gpui::CursorStyle::PointingHand)
                .hover(|style| style.bg(rgba(shared.washes.fill_hover.packed())))
                .on_click(move |event, window, cx| on_click(event, window, cx));
        }
        row
    }
}
