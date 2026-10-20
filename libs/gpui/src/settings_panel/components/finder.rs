use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId, RenderOnce, SharedString, Window,
};
use qol_theme::{translucent, Alpha, TextStyle};

use super::{
    finder_display_label, settings_description, settings_label_group, settings_value_text,
    ChoiceArt, RowGround, SettingsChoiceArt, SettingsRow, SettingsValueTone,
};
use crate::kit::Kit;
use crate::pictures::PictureContext;
use crate::search_mark::{MarkState, SearchMark};
use crate::text::TextStyled;

pub const FINDER_MARK_LEAD: f32 = qol_theme::HEIGHT_SETTING_ROW;
pub const FINDER_MARK_EMPTY: f32 = qol_theme::HEIGHT_BAND;
pub const FINDER_MARK_SMALL: f32 = qol_theme::SPACE_PAD;
const FINDER_EMPTY_HEIGHT: f32 =
    2.0 * qol_theme::HEIGHT_BAND + 2.0 * qol_theme::SPACE_CELL + qol_theme::SPACE_TIGHT;
const SIGNAL_BAR_WIDTH: f32 = 3.0;
const SIGNAL_BAR_HEIGHTS: [f32; 4] = [5.0, 8.0, 11.0, 14.0];

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub fn settings_search_mark(
    id: impl Into<ElementId>,
    size: f32,
    state: MarkState,
    row: RowGround,
    kit: Kit,
) -> SearchMark {
    let rest = match row {
        RowGround::Pane => kit.grounds.pane.soft,
        RowGround::Band => kit.grounds.band.ink,
    };
    SearchMark::new(id, size, state, kit.palette.success, rest)
}

pub fn settings_signal_bars(level: u8, row: RowGround, kit: Kit) -> gpui::Div {
    let ink = match row {
        RowGround::Pane => kit.grounds.pane.soft,
        RowGround::Band => kit.grounds.band.ink,
    };
    let unlit = rgba(translucent(ink, Alpha::Edge));
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(qol_theme::SPACE_STACK))
        .ml(px(qol_theme::SPACE_TIGHT))
        .children(SIGNAL_BAR_HEIGHTS.iter().enumerate().map(|(bar, height)| {
            let lit = bar < usize::from(level);
            div()
                .flex_none()
                .w(px(SIGNAL_BAR_WIDTH))
                .h(px(*height))
                .map(|bar| if lit { bar.bg(rgb(ink)) } else { bar.bg(unlit) })
        }))
}

pub fn settings_finder_empty(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    state: MarkState,
    kit: Kit,
) -> gpui::Div {
    let pane = kit.grounds.pane;
    div()
        .flex_none()
        .w_full()
        .h(px(FINDER_EMPTY_HEIGHT))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(qol_theme::SPACE_TIGHT))
        .px(px(qol_theme::SPACE_GUTTER))
        .child(
            div()
                .flex_none()
                .mb(px(qol_theme::SPACE_INSET))
                .child(settings_search_mark(
                    id,
                    FINDER_MARK_EMPTY,
                    state,
                    RowGround::Pane,
                    kit,
                )),
        )
        .child(
            div()
                .text(TextStyle::Name)
                .text_color(rgb(pane.soft))
                .child(title.into()),
        )
        .when_some(detail, |block, detail| {
            block.child(
                div()
                    .text(TextStyle::Detail)
                    .text_color(rgb(pane.faint))
                    .child(detail),
            )
        })
}

#[derive(IntoElement)]
pub struct SettingsFinderLead {
    id: ElementId,
    label: SharedString,
    subtitle: Option<SharedString>,
    state: MarkState,
    selected: bool,
    focused: bool,
    kit: Kit,
    on_click: Option<ClickHandler>,
}

impl SettingsFinderLead {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        state: MarkState,
        kit: Kit,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            subtitle: None,
            state,
            selected: false,
            focused: true,
            kit,
            on_click: None,
        }
    }

    pub fn subtitle(mut self, subtitle: Option<SharedString>) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn selected(mut self, selected: bool, focused: bool) -> Self {
        self.selected = selected;
        self.focused = focused;
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

impl RenderOnce for SettingsFinderLead {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let row = RowGround::of(self.selected, self.focused);
        let mark_id = ElementId::NamedChild(Box::new(self.id.clone()), "mark".into());
        let words = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_STACK))
            .child(finder_display_label(self.label, row, self.kit))
            .children(
                self.subtitle
                    .map(|subtitle| settings_description(subtitle, row, self.kit)),
            );
        let line = SettingsRow::lead(self.id, self.kit)
            .selected(self.selected, self.focused)
            .child(settings_search_mark(
                mark_id,
                FINDER_MARK_LEAD,
                self.state,
                row,
                self.kit,
            ))
            .child(words);
        match self.on_click {
            Some(handler) => line.on_click(handler),
            None => line,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinderPickEnd {
    None,
    Word {
        text: SharedString,
        tone: SettingsValueTone,
        working: bool,
    },
    Action {
        text: SharedString,
        working: bool,
    },
}

#[derive(IntoElement)]
pub struct SettingsFinderPick {
    id: ElementId,
    name: SharedString,
    subtitle: Option<SharedString>,
    art: ChoiceArt,
    context: PictureContext,
    end: FinderPickEnd,
    signal: Option<u8>,
    selected: bool,
    focused: bool,
    kit: Kit,
    above: Option<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl SettingsFinderPick {
    pub fn new(
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        art: ChoiceArt,
        context: PictureContext,
        kit: Kit,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            subtitle: None,
            art,
            context,
            end: FinderPickEnd::None,
            signal: None,
            selected: false,
            focused: true,
            kit,
            above: None,
            on_click: None,
        }
    }

    pub fn subtitle(mut self, subtitle: Option<SharedString>) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn end(mut self, end: FinderPickEnd) -> Self {
        self.end = end;
        self
    }

    pub fn signal(mut self, signal: Option<u8>) -> Self {
        self.signal = signal;
        self
    }

    pub fn selected(mut self, selected: bool, focused: bool) -> Self {
        self.selected = selected;
        self.focused = focused;
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

impl RenderOnce for SettingsFinderPick {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let row = RowGround::of(self.selected, self.focused);
        let kit = self.kit;
        let mark_id = ElementId::NamedChild(Box::new(self.id.clone()), "mark".into());
        let end = match self.end {
            FinderPickEnd::None => None,
            FinderPickEnd::Word {
                text,
                tone,
                working,
            } => Some(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(qol_theme::SPACE_SNUG))
                    .when(working, |word| {
                        word.child(settings_search_mark(
                            mark_id,
                            FINDER_MARK_SMALL,
                            MarkState::Live,
                            row,
                            kit,
                        ))
                    })
                    .child(settings_value_text(text, tone, row, kit)),
            ),
            FinderPickEnd::Action { text, working } => Some(
                kit.button_inked(row.rest(kit))
                    .when(working, |action| {
                        action.child(settings_search_mark(
                            mark_id,
                            FINDER_MARK_SMALL,
                            MarkState::Live,
                            row,
                            kit,
                        ))
                    })
                    .child(text),
            ),
        };
        let line = SettingsRow::setting(self.id, kit)
            .selected(self.selected, self.focused)
            .child(SettingsChoiceArt::new(self.art, row, self.context, kit))
            .child(settings_label_group(self.name, self.subtitle, row, kit))
            .children(end.map(IntoElement::into_any_element))
            .children(
                self.signal
                    .map(|level| settings_signal_bars(level, row, kit).into_any_element()),
            );
        let line = match self.on_click {
            Some(handler) => line.on_click(handler),
            None => line,
        };
        div().flex().flex_col().children(self.above).child(line)
    }
}

impl SettingsFinderPick {
    pub fn under(mut self, label: Option<SharedString>) -> Self {
        self.above =
            label.map(|label| super::settings_list_subheader(label, self.kit).into_any_element());
        self
    }
}
