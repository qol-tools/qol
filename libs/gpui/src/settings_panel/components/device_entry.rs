use crate::text::{cased, TextStyled};
use gpui::prelude::*;
use gpui::{div, img, px, rgb, App, RenderOnce, SharedString, Window};
use qol_theme::{translucent, Alpha, TextStyle};

use super::paint_rail_selection;
use crate::kit::Kit;
use crate::pictures::{self, PictureContext, Tone};

pub const DEVICE_LIST_WIDTH: f32 = 240.0;
pub const DEVICE_KIND_HEIGHT: f32 = 32.0;
const DEVICE_PICTURE_WIDTH: f32 = 48.0;
const DEVICE_PICTURE_HEIGHT: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceLight {
    Live,
    Away,
    Unlit,
}

#[derive(IntoElement)]
pub struct SettingsDeviceEntry {
    name: SharedString,
    status: Option<SharedString>,
    picture: Option<String>,
    light: DeviceLight,
    chosen: bool,
    focused: bool,
    context: PictureContext,
    kit: Kit,
}

impl SettingsDeviceEntry {
    pub fn new(
        name: impl Into<SharedString>,
        picture: Option<String>,
        context: PictureContext,
        kit: Kit,
    ) -> Self {
        Self {
            name: name.into(),
            status: None,
            picture,
            light: DeviceLight::Unlit,
            chosen: false,
            focused: false,
            context,
            kit,
        }
    }

    pub fn status(mut self, status: Option<SharedString>, light: DeviceLight) -> Self {
        self.status = status;
        self.light = light;
        self
    }

    pub fn chosen(mut self, chosen: bool, focused: bool) -> Self {
        self.chosen = chosen;
        self.focused = focused;
        self
    }
}

impl RenderOnce for SettingsDeviceEntry {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let kit = self.kit;
        let on_band = self.chosen && self.focused;
        let ground = if on_band {
            kit.grounds.band
        } else {
            kit.grounds.rail
        };
        let tone = if on_band { Tone::Awake } else { Tone::Rest };
        let line = if on_band { ground.ink } else { ground.soft };
        let art = self.picture.as_deref().and_then(|spec| {
            pictures::fitted_image(
                spec,
                line,
                tone,
                DEVICE_PICTURE_WIDTH,
                DEVICE_PICTURE_HEIGHT,
                window.scale_factor(),
                &self.context,
            )
        });
        let dot = match self.light {
            DeviceLight::Live => Some(kit.palette.success),
            DeviceLight::Away => Some(ground.faint),
            DeviceLight::Unlit => None,
        }
        .map(|tone| {
            kit.status_dot(tone, translucent(tone, Alpha::Halo))
                .ml(px(qol_theme::STATUS_DOT_HALO))
        });
        let status = self.status.map(|status| {
            div()
                .flex()
                .items_center()
                .gap(px(qol_theme::SPACE_SNUG))
                .children(dot)
                .child(
                    div()
                        .min_w_0()
                        .text(TextStyle::Detail)
                        .text_color(rgb(ground.soft))
                        .child(status),
                )
        });
        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(qol_theme::HEIGHT_RULE_ROW))
            .px(px(qol_theme::SPACE_PAD))
            .gap(px(qol_theme::SPACE_CELL))
            .when(self.chosen, |entry| {
                paint_rail_selection(entry, kit, self.focused)
            })
            .child(
                div()
                    .flex_none()
                    .when(self.light == DeviceLight::Away && !self.chosen, |art| {
                        art.opacity(qol_theme::OPACITY_REST)
                    })
                    .w(px(DEVICE_PICTURE_WIDTH))
                    .h(px(DEVICE_PICTURE_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(art.map(|art| {
                        img(art)
                            .w(px(DEVICE_PICTURE_WIDTH))
                            .h(px(DEVICE_PICTURE_HEIGHT))
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(qol_theme::SPACE_STACK))
                    .when(self.light == DeviceLight::Away && !self.chosen, |text| {
                        text.opacity(qol_theme::OPACITY_REST)
                    })
                    .child(
                        div()
                            .text(TextStyle::ListName)
                            .text_color(rgb(ground.ink))
                            .child(self.name),
                    )
                    .children(status),
            )
    }
}

pub fn settings_device_kind(label: &str, kit: Kit) -> gpui::Div {
    div()
        .flex()
        .items_end()
        .w_full()
        .h(px(DEVICE_KIND_HEIGHT))
        .px(px(qol_theme::SPACE_PAD))
        .pb(px(qol_theme::SPACE_TIGHT))
        .text(TextStyle::Label)
        .text_color(rgb(kit.grounds.rail.faint))
        .child(cased(TextStyle::Label, label))
}
