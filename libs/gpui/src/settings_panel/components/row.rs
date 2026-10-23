use gpui::prelude::*;
use gpui::{div, px, rgb, AnyElement, App, ClickEvent, ElementId, RenderOnce, Window};

use crate::kit::kit;
use crate::kit::Kit;

use super::{
    attention_dot, paint_settings_attention, paint_settings_selection, DIMMED_OPACITY,
    SETTINGS_ROW_GROUP,
};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Lead,
    Setting,
    Rule,
    Add,
}

#[derive(IntoElement)]
pub struct SettingsRow {
    id: ElementId,
    kit: Kit,
    kind: RowKind,
    selected: bool,
    focused: bool,
    dimmed: bool,
    attention: bool,
    lift: f32,
    children: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl SettingsRow {
    pub fn setting(id: impl Into<ElementId>, kit: Kit) -> Self {
        Self::new(id, kit, RowKind::Setting)
    }

    pub fn lead(id: impl Into<ElementId>, kit: Kit) -> Self {
        Self::new(id, kit, RowKind::Lead)
    }

    pub fn rule(id: impl Into<ElementId>, kit: Kit) -> Self {
        Self::new(id, kit, RowKind::Rule)
    }

    pub fn add(id: impl Into<ElementId>, kit: Kit) -> Self {
        Self::new(id, kit, RowKind::Add)
    }

    fn new(id: impl Into<ElementId>, kit: Kit, kind: RowKind) -> Self {
        Self {
            id: id.into(),
            kit,
            kind,
            selected: false,
            focused: true,
            dimmed: false,
            attention: false,
            lift: 0.0,
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

    pub fn lifted(mut self, lift: f32) -> Self {
        self.lift = lift.clamp(0.0, 1.0);
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

pub fn settings_row_shade(lift: f32, kit: Kit) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-qol_theme::SPACE_PAD))
        .right(px(-(qol_theme::SPACE_PAD - qol_theme::SPACE_INSET)))
        .bg(gpui::rgba(kit.grounds.pane.well.packed()))
        .opacity(lift.clamp(0.0, 1.0))
}

impl RenderOnce for SettingsRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        let attention = self.attention && !(self.selected && self.focused);
        let height = match self.kind {
            RowKind::Lead => qol_theme::HEIGHT_SETTING_ROW + 2.0 * qol_theme::SPACE_GUTTER,
            RowKind::Setting => qol_theme::HEIGHT_SETTING_ROW,
            RowKind::Rule | RowKind::Add => qol_theme::HEIGHT_RULE_ROW,
        };
        let mut row = div()
            .id(self.id)
            .group(SETTINGS_ROW_GROUP)
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
            .when(attention, |row| row.child(attention_dot(self.kit)))
            .children(self.children);
        if self.kind == RowKind::Rule {
            row = row.rounded(px(qol_theme::RADIUS_CONTROL));
        }
        if self.dimmed {
            row = row.opacity(DIMMED_OPACITY);
        }
        if self.selected && self.focused {
            row = paint_settings_selection(row, self.kit);
        } else if attention {
            row = paint_settings_attention(row, self.kit);
        }
        if self.lift > 0.0 {
            let mut shadow = crate::kit::float_shadow(self.kit.grounds.pane.ink);
            for layer in &mut shadow {
                layer.color.a *= self.lift;
            }
            row = row.shadow(shadow);
        }
        if let Some(on_click) = self.on_click {
            let ground = if self.selected && self.focused {
                self.kit.grounds.band
            } else {
                self.kit.grounds.pane
            };
            row = shared
                .pointable(
                    row.cursor(gpui::CursorStyle::PointingHand),
                    rgb(ground.lift),
                )
                .on_click(move |event, window, cx| on_click(event, window, cx));
        }
        row
    }
}
