use std::path::{Path, PathBuf};

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::icon::{icon, Icon};
use qol_gpui::text::{shaped_width, wrapped_line_count, TextStyled};
use qol_gpui::theme::{
    css_rgba_milli, TextStyle, LINE, RADIUS_CARD, RADIUS_CONTROL, RADIUS_TIGHT, SPACE_INSET,
    SPACE_SNUG, SPACE_STACK, SPACE_TIGHT, TEXT_MICRO,
};

use super::files::spaced_path;
use super::layout::APP_PANEL_WIDTH;
use super::tags;
use super::view::tile;
use super::LauncherView;
use crate::discovery::details::{date_label, AppAbout};

const BAND_SHADE_ALPHA: u16 = 180;
const WELL_SHADE_ALPHA: u16 = 120;
const WELL_LINE_ALPHA: u16 = 70;
const DONE_ALPHA: u16 = 300;
const TAG_ALPHA: u16 = 300;
const HEAD_HEIGHT: f32 = 60.0;
const HEAD_TILE: f32 = 44.0;
const HEAD_ICON: f32 = 36.0;
const HEAD_NAME: f32 = 17.0;
const HEAD_NAME_LINE: f32 = 1.15;
const SUMMARY_LINE: f32 = 16.0;
const BAND_AIR: f32 = 6.0;
const BAND_INSET: f32 = SPACE_SNUG + SPACE_INSET;
const WELL_BLOCK: f32 = 24.0;
const WELL_HEIGHT: f32 = 22.0;
const WELL_CHARS: usize = 42;
const TAGS_TOP: f32 = 2.0;
const TAGS_BOTTOM: f32 = 4.0;
const SECTION_AIR: f32 = 8.0;
const DESCRIPTION_LINE: f32 = 18.0;
const DESCRIPTION_LINES: usize = 8;
const DESCRIPTION_AIR: f32 = 4.0;
const RULE_BLOCK: f32 = 9.0;
const LINE_BLOCK: f32 = 22.0;
const INNER_WIDTH: f32 = APP_PANEL_WIDTH - 2.0 * SPACE_SNUG - 2.0 * SPACE_INSET;

pub struct About<'a> {
    pub name: &'a str,
    pub icon: Option<&'a Path>,
    pub about: Option<&'a AppAbout>,
    pub home: Option<&'a Path>,
    pub copied: bool,
    pub fill: u32,
}

pub struct AboutPanel {
    pub element: Div,
    pub height: f32,
}

struct Blocks {
    children: Vec<AnyElement>,
    height: f32,
}

impl Blocks {
    fn push(&mut self, height: f32, element: impl IntoElement) {
        if !self.children.is_empty() {
            self.height += SPACE_STACK;
        }
        self.height += height;
        self.children.push(element.into_any_element());
    }

    fn space(&mut self, height: f32) {
        self.push(height, div().flex_none().h(px(height)));
    }
}

pub fn panel(about: About<'_>, window: &mut Window, cx: &mut Context<LauncherView>) -> AboutPanel {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    let facts = about.about;
    let mut blocks = Blocks {
        children: Vec::new(),
        height: 0.0,
    };
    let (head, head_height) = head(&about);
    blocks.push(head_height - SPACE_SNUG, head);
    if let Some(facts) = facts {
        blocks.space(BAND_AIR);
        let mut first = true;
        let mut section = |blocks: &mut Blocks| {
            if !first {
                blocks.space(SECTION_AIR);
            }
            first = false;
        };
        if facts.binary.is_some() || facts.command.is_some() {
            section(&mut blocks);
            if let Some(binary) = &facts.binary {
                blocks.push(
                    WELL_BLOCK,
                    path_well(binary.clone(), about.home, about.copied, cx),
                );
            }
            if let Some(command) = &facts.command {
                blocks.push(WELL_BLOCK, command_well(command));
            }
        }
        let list = tag_list(facts);
        if !list.is_empty() {
            section(&mut blocks);
            let rows = tag_rows(&list, window);
            blocks.push(
                TAGS_TOP + tag_block_height(rows) + TAGS_BOTTOM,
                div()
                    .flex_none()
                    .px(px(SPACE_INSET))
                    .pt(px(TAGS_TOP))
                    .pb(px(TAGS_BOTTOM))
                    .flex()
                    .flex_wrap()
                    .gap(px(SPACE_TIGHT))
                    .children(
                        list.into_iter()
                            .map(|(text, hue)| tags::tag(text, hue, TAG_ALPHA, ground.ink)),
                    ),
            );
        }
        if let Some(long) = &facts.long {
            section(&mut blocks);
            let lines = wrapped_line_count(
                window,
                long,
                font(qol_gpui::theme::font_ui()),
                TEXT_MICRO,
                INNER_WIDTH,
            )
            .clamp(1, DESCRIPTION_LINES);
            blocks.push(
                lines as f32 * DESCRIPTION_LINE + DESCRIPTION_AIR,
                div()
                    .flex_none()
                    .px(px(SPACE_INSET))
                    .text(TextStyle::Detail)
                    .line_height(px(DESCRIPTION_LINE))
                    .text_color(rgb(ground.soft))
                    .wraps()
                    .line_clamp(DESCRIPTION_LINES)
                    .child(long.clone()),
            );
        }
        if facts.developer.is_some() || facts.website.is_some() {
            blocks.space(0.0);
            blocks.push(
                RULE_BLOCK,
                div()
                    .flex_none()
                    .h(px(RULE_BLOCK))
                    .px(px(SPACE_INSET))
                    .flex()
                    .items_center()
                    .child(div().w_full().h(px(LINE)).bg(rgba(ground.edge.packed()))),
            );
            blocks.space(0.0);
            if let Some(developer) = &facts.developer {
                blocks.push(
                    LINE_BLOCK,
                    line_block()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(ground.ink))
                        .child(div().line_clamp(1).child(developer.clone())),
                );
            }
            if let Some(website) = &facts.website {
                blocks.push(LINE_BLOCK, link(website.clone(), cx));
            }
        }
    }
    let height = 2.0 * SPACE_SNUG + blocks.height;
    let element = div()
        .flex_none()
        .w(px(APP_PANEL_WIDTH))
        .h_full()
        .p(px(SPACE_SNUG))
        .flex()
        .flex_col()
        .gap(px(SPACE_STACK))
        .overflow_hidden()
        .rounded(px(RADIUS_CARD))
        .bg(rgb(about.fill))
        .children(blocks.children);
    AboutPanel { element, height }
}

fn head(about: &About<'_>) -> (Div, f32) {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    let asks = about.about.is_some_and(|facts| facts.asks_password);
    let summary = about.about.and_then(|facts| facts.summary.clone());
    let height = BAND_AIR
        + SPACE_STACK
        + HEAD_HEIGHT
        + if asks {
            SPACE_STACK + TAGS_TOP + tags::HEIGHT + TAGS_BOTTOM
        } else {
            0.0
        }
        + SPACE_STACK
        + BAND_AIR;
    let element = div()
        .flex_none()
        .mt(px(-SPACE_SNUG))
        .mx(px(-SPACE_SNUG))
        .px(px(BAND_INSET))
        .pt(px(BAND_AIR + SPACE_STACK))
        .pb(px(BAND_AIR + SPACE_STACK))
        .flex()
        .flex_col()
        .gap(px(SPACE_STACK))
        .bg(rgba(css_rgba_milli(0x000000, BAND_SHADE_ALPHA).packed()))
        .child(
            div()
                .flex_none()
                .h(px(HEAD_HEIGHT))
                .flex()
                .items_center()
                .gap(px(SPACE_INSET))
                .child(tile(
                    about.name,
                    about.icon,
                    HEAD_TILE,
                    HEAD_ICON,
                    RADIUS_CONTROL,
                    ground.well,
                    ground.soft,
                ))
                .child(
                    div()
                        .flex_grow()
                        .flex()
                        .flex_col()
                        .gap(px(SPACE_STACK))
                        .child(
                            div()
                                .line_clamp(1)
                                .font_family(qol_gpui::theme::font_display())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(HEAD_NAME))
                                .line_height(px((HEAD_NAME * HEAD_NAME_LINE).round()))
                                .text_color(rgb(ground.ink))
                                .child(about.name.to_owned()),
                        )
                        .when_some(summary, |text, summary| {
                            text.child(
                                div()
                                    .text(TextStyle::Detail)
                                    .line_height(px(SUMMARY_LINE))
                                    .text_color(rgb(ground.soft))
                                    .child(summary),
                            )
                        }),
                ),
        )
        .when(asks, |band_block| {
            band_block.child(
                div()
                    .flex_none()
                    .pt(px(TAGS_TOP))
                    .pb(px(TAGS_BOTTOM))
                    .flex()
                    .child(tags::tag(
                        "Asks for your password",
                        kit.palette.warning,
                        TAG_ALPHA,
                        ground.ink,
                    )),
            )
        });
    (element, height)
}

fn well_frame(done: bool) -> Div {
    let ground = qol_gpui::kit::kit().grounds.menu;
    div()
        .flex_grow()
        .min_w_0()
        .h(px(WELL_HEIGHT))
        .px(px(SPACE_INSET))
        .flex()
        .items_center()
        .gap(px(SPACE_INSET))
        .rounded(px(RADIUS_TIGHT))
        .bg(rgba(if done {
            css_rgba_milli(ground.ink, DONE_ALPHA).packed()
        } else {
            css_rgba_milli(0x000000, WELL_SHADE_ALPHA).packed()
        }))
        .border_b(px(LINE))
        .border_color(rgba(css_rgba_milli(ground.ink, WELL_LINE_ALPHA).packed()))
        .text(TextStyle::Detail)
        .text_color(rgb(ground.soft))
}

fn well_block() -> Div {
    div()
        .flex_none()
        .h(px(WELL_BLOCK))
        .px(px(SPACE_INSET))
        .flex()
        .items_center()
}

fn path_well(
    binary: PathBuf,
    home: Option<&Path>,
    copied: bool,
    cx: &mut Context<LauncherView>,
) -> Stateful<Div> {
    let ground = qol_gpui::kit::kit().grounds.menu;
    let label = spaced_path(&binary, home, WELL_CHARS);
    well_block()
        .id("launcher-app-path")
        .cursor_pointer()
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.reveal_path(&binary, cx);
        }))
        .child(
            well_frame(copied)
                .when(copied, |well| {
                    well.child(icon(Icon::Tick, TextStyle::Detail.spec().size, ground.ink))
                        .child(
                            div()
                                .flex_grow()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(ground.ink))
                                .child("Copied path"),
                        )
                })
                .when(!copied, |well| {
                    well.child(div().flex_grow().min_w_0().line_clamp(1).child(label))
                })
                .child(icon(
                    Icon::Reveal,
                    TextStyle::Detail.spec().size,
                    ground.soft,
                )),
        )
}

fn command_well(command: &str) -> Div {
    well_block().child(
        well_frame(false).child(
            div()
                .flex_grow()
                .min_w_0()
                .line_clamp(1)
                .font_family(qol_gpui::theme::font_mono())
                .child(command.to_owned()),
        ),
    )
}

fn line_block() -> Div {
    div()
        .flex_none()
        .h(px(LINE_BLOCK))
        .px(px(SPACE_INSET))
        .flex()
        .items_center()
        .gap(px(SPACE_SNUG))
        .text(TextStyle::Detail)
}

fn link(url: String, cx: &mut Context<LauncherView>) -> Stateful<Div> {
    let kit = qol_gpui::kit::kit();
    let ink = kit.grounds.menu.soft;
    line_block()
        .id("launcher-app-website")
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(ink))
        .cursor_pointer()
        .child(div().line_clamp(1).child(website_label(&url)))
        .child(icon(Icon::Reveal, TextStyle::Detail.spec().size, ink))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.open_website(&url, window, cx);
        }))
}

fn tag_list(facts: &AppAbout) -> Vec<(String, u32)> {
    let package = facts.package.as_ref();
    [
        facts.kind.clone().map(|kind| (kind, tags::WHAT)),
        package.map(|package| (package.name.clone(), tags::WHAT)),
        facts
            .source
            .map(|source| (format!("from {source}"), tags::WHAT)),
        package
            .and_then(|package| package.version.as_ref())
            .map(|version| (format!("version {version}"), tags::FACT)),
        package
            .and_then(|package| package.size)
            .map(|size| (qol_gpui::format_bytes(size), tags::FACT)),
        facts.licence.clone().map(|licence| (licence, tags::LAW)),
        package
            .and_then(|package| package.installed)
            .map(|installed| (format!("installed {}", date_label(installed)), tags::USE)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn tag_rows(list: &[(String, u32)], window: &mut Window) -> usize {
    let mut rows = 1;
    let mut run = 0.0;
    for (text, _) in list {
        let width = shaped_width(window, text, font(qol_gpui::theme::font_ui()), TEXT_MICRO)
            + 2.0 * tags::PAD;
        if run > 0.0 && run + SPACE_TIGHT + width > INNER_WIDTH {
            rows += 1;
            run = width;
        } else {
            run += if run > 0.0 { SPACE_TIGHT } else { 0.0 } + width;
        }
    }
    rows
}

fn tag_block_height(rows: usize) -> f32 {
    rows as f32 * tags::HEIGHT + rows.saturating_sub(1) as f32 * SPACE_TIGHT
}

pub fn website_label(url: &str) -> String {
    let rest = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .trim_end_matches('/');
    rest.strip_prefix("www.").unwrap_or(rest).to_owned()
}

#[cfg(test)]
mod tests {
    use super::{tag_list, tags, website_label};
    use crate::discovery::details::{AppAbout, Package};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn website_labels_drop_the_scheme_and_www() {
        assert_eq!(
            website_label("http://www.github.com/linuxmint/xed"),
            "github.com/linuxmint/xed"
        );
        assert_eq!(
            website_label("https://wiki.gnome.org/Apps/Terminal/"),
            "wiki.gnome.org/Apps/Terminal"
        );
        assert_eq!(website_label("example.org"), "example.org");
    }

    #[test]
    fn tags_are_sorted_by_colour_like_the_board() {
        let facts = AppAbout {
            kind: Some("Text editor".to_owned()),
            source: Some("apt"),
            package: Some(Package {
                name: "xed".to_owned(),
                version: Some("3.8.9".to_owned()),
                size: Some(1_248_000),
                installed: Some(UNIX_EPOCH + Duration::from_secs(20_500 * 86_400)),
            }),
            licence: Some("GPL-2.0+".to_owned()),
            ..AppAbout::default()
        };
        assert_eq!(
            tag_list(&facts),
            vec![
                ("Text editor".to_owned(), tags::WHAT),
                ("xed".to_owned(), tags::WHAT),
                ("from apt".to_owned(), tags::WHAT),
                ("version 3.8.9".to_owned(), tags::FACT),
                ("1.2 MB".to_owned(), tags::FACT),
                ("GPL-2.0+".to_owned(), tags::LAW),
                ("installed 16 Feb 2026".to_owned(), tags::USE),
            ]
        );
        assert!(tag_list(&AppAbout::default()).is_empty());
    }
}
