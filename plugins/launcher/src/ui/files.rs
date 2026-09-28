use std::path::Path;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::icon::{icon, Icon};
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{
    css_rgba_milli, TextStyle, LINE, RADIUS_CARD, RADIUS_TIGHT, SPACE_CELL, SPACE_INSET, SPACE_PAD,
    SPACE_SNUG, SPACE_STACK, SPACE_TIGHT, TEXT_CAPTION,
};
use qol_gpui::Key;

use super::feedback::Feedback;
use super::layout::PANEL_WIDTH;
use super::sizing::FileCardSize;
use super::state::PanelItem;
use super::tags;
use super::view::{card_cast, card_ground, growing, name_text, tile};
use super::LauncherView;
use crate::discovery::details::{FactKind, FactText, FileDetails};

const PATH_CHARS: usize = 50;
const PATH_HEIGHT: f32 = 22.0;
const TAG_ALPHA: u16 = 260;
const ICON_TILE: f32 = 26.0;
const ICON_SIZE: f32 = 20.0;
const PANEL_ROW: f32 = 32.0;
const PANEL_GLYPH: f32 = 16.0;
const PANEL_ICON: f32 = 18.0;
const WELL_SHADE: u32 = 0x000000;
const WELL_ALPHA: u16 = 60;
const WELL_LINE_ALPHA: u16 = 70;
const PANEL_CURRENT_ALPHA: u16 = 160;
const PANEL_RESTING_ALPHA: u16 = 50;
const PANEL_DONE_ALPHA: u16 = 300;
const FILE_GROW: f32 = 6.0;

pub struct FileCard<'a> {
    pub index: Option<usize>,
    pub name: &'a str,
    pub path: &'a Path,
    pub details: Option<&'a FileDetails>,
    pub lit: f32,
    pub size: FileCardSize,
    pub feedback: Option<&'a Feedback>,
}

pub struct Panel<'a> {
    pub name: &'a str,
    pub icon: Option<&'a Path>,
    pub choice: PanelItem,
    pub done: Option<PanelItem>,
    pub active: bool,
    pub fill: u32,
}

impl PanelItem {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::OpenFolder => "Open folder",
            Self::CopyPath => "Copy path",
            Self::CopyName => "Copy name",
        }
    }

    fn done_label(self) -> &'static str {
        match self {
            Self::CopyPath => "Path copied",
            Self::CopyName => "Name copied",
            _ => self.label(),
        }
    }

    fn glyph(self) -> Icon {
        match self {
            Self::Open => Icon::Reveal,
            Self::OpenFolder => Icon::Folder,
            Self::CopyPath => Icon::Copy,
            Self::CopyName => Icon::Edit,
        }
    }

    fn key(self) -> Option<Key> {
        match self {
            Self::Open => Some(Key::ENTER),
            Self::OpenFolder => Some(Key::ENTER.shift()),
            Self::CopyPath => Some(Key::letter('c').secondary()),
            Self::CopyName => None,
        }
    }
}

pub fn file_card(
    card: FileCard<'_>,
    home: Option<&Path>,
    cx: &mut Context<LauncherView>,
) -> Stateful<Div> {
    let kit = qol_gpui::kit::kit();
    let pane = kit.grounds.pane;
    let index = card.index;
    let label = path_label(
        card.path,
        home,
        card.details.and_then(|d| d.link.as_deref()),
    );
    let facts = card
        .details
        .map(|details| details.facts.as_slice())
        .unwrap_or_default();
    div()
        .id(card_id("launcher-file", index, card.name))
        .flex_none()
        .w_full()
        .h(px(card.size.height))
        .px(px(SPACE_PAD))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(SPACE_SNUG))
        .rounded(px(RADIUS_CARD))
        .bg(rgb(card_ground(card.lit)))
        .shadow(card_cast(card.lit))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(SPACE_INSET))
                .child({
                    let name = card.name.to_owned();
                    let art = card.details.and_then(|details| details.icon.clone());
                    growing(
                        card.feedback.filter(|feedback| {
                            feedback.cue.grows() && feedback.is_about(card.name)
                        }),
                        FILE_GROW,
                        move |extra| {
                            tile(
                                &name,
                                art.as_deref(),
                                ICON_TILE + extra,
                                ICON_SIZE + extra,
                                RADIUS_TIGHT,
                                css_rgba_milli(WELL_SHADE, WELL_ALPHA),
                                pane.soft,
                            )
                            .border_b(px(LINE))
                            .border_color(rgba(css_rgba_milli(pane.ink, WELL_LINE_ALPHA).packed()))
                        },
                    )
                })
                .child(
                    name_text(
                        card.name,
                        card.size.name,
                        card.size.display,
                        FontWeight::NORMAL,
                        FontWeight::NORMAL,
                        pane.ink,
                    )
                    .min_w(px(0.0)),
                ),
        )
        .child(
            div()
                .id(card_id("launcher-file-path", index, card.name))
                .flex_none()
                .w_full()
                .h(px(PATH_HEIGHT))
                .px(px(SPACE_INSET))
                .flex()
                .items_center()
                .gap(px(SPACE_INSET))
                .rounded(px(RADIUS_TIGHT))
                .bg(rgba(css_rgba_milli(WELL_SHADE, WELL_ALPHA).packed()))
                .border_b(px(LINE))
                .border_color(rgba(css_rgba_milli(pane.ink, WELL_LINE_ALPHA).packed()))
                .text(TextStyle::Detail)
                .text_color(rgb(pane.faint))
                .cursor_pointer()
                .hover(|well| well.bg(rgba(kit.washes.fill_hover.packed())))
                .child(div().flex_grow().min_w_0().line_clamp(1).child(label))
                .child(icon(
                    Icon::Reveal,
                    TextStyle::Detail.spec().size,
                    pane.faint,
                ))
                .when_some(index, |well, index| {
                    well.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.state.scroll_list.selected = index;
                        this.open_selected_folder(cx);
                    }))
                }),
        )
        .child(
            div()
                .flex_none()
                .h(px(PATH_HEIGHT))
                .flex()
                .items_center()
                .gap(px(SPACE_SNUG))
                .children(facts.iter().map(|fact| {
                    let text = match &fact.text {
                        FactText::Words(words) => words.clone(),
                        FactText::Bytes(bytes) => qol_gpui::format_bytes(*bytes),
                    };
                    tags::tag(text, tag_hue(fact.kind), TAG_ALPHA, pane.ink)
                })),
        )
}

fn card_id(part: &'static str, index: Option<usize>, name: &str) -> ElementId {
    match index {
        Some(index) => (part, index).into(),
        None => ElementId::Name(format!("{part}-leaving-{name}").into()),
    }
}

pub fn panel(panel: Panel<'_>) -> Div {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    div()
        .flex_none()
        .w(px(PANEL_WIDTH))
        .h_full()
        .p(px(SPACE_SNUG))
        .flex()
        .flex_col()
        .gap(px(SPACE_STACK))
        .rounded(px(RADIUS_CARD))
        .bg(rgb(panel.fill))
        .child(
            div()
                .flex_none()
                .h(px(PANEL_ROW))
                .px(px(SPACE_INSET))
                .flex()
                .items_center()
                .gap(px(SPACE_INSET))
                .child(tile(
                    panel.name,
                    panel.icon,
                    PANEL_ICON,
                    PANEL_ICON,
                    0.0,
                    css_rgba_milli(ground.bg, 0),
                    ground.soft,
                ))
                .child(
                    div()
                        .flex_grow()
                        .min_w_0()
                        .line_clamp(1)
                        .font_family(qol_gpui::theme::font_ui())
                        .text_size(px(TEXT_CAPTION))
                        .text_color(rgb(ground.ink))
                        .child(panel.name.to_owned()),
                ),
        )
        .children(PanelItem::ALL.into_iter().map(|item| {
            panel_row(
                item,
                item == panel.choice,
                panel.done == Some(item),
                panel.active,
            )
        }))
        .child(div().flex_1())
        .when(panel.active, |active| active.child(panel_hints()))
}

fn panel_row(item: PanelItem, current: bool, done: bool, active: bool) -> Div {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    let fill = if done {
        Some(PANEL_DONE_ALPHA)
    } else if current && active {
        Some(PANEL_CURRENT_ALPHA)
    } else if current {
        Some(PANEL_RESTING_ALPHA)
    } else {
        None
    };
    div()
        .flex_none()
        .h(px(PANEL_ROW))
        .px(px(SPACE_INSET))
        .flex()
        .items_center()
        .gap(px(SPACE_INSET))
        .rounded(px(RADIUS_TIGHT))
        .when_some(fill, |row, alpha| {
            row.bg(rgba(css_rgba_milli(ground.ink, alpha).packed()))
        })
        .child(
            div()
                .flex_none()
                .w(px(PANEL_GLYPH))
                .flex()
                .justify_center()
                .child(icon(
                    if done { Icon::Tick } else { item.glyph() },
                    TextStyle::Detail.spec().size,
                    if (current && active) || done {
                        ground.ink
                    } else {
                        ground.faint
                    },
                )),
        )
        .child(
            div()
                .flex_grow()
                .min_w_0()
                .font_family(qol_gpui::theme::font_ui())
                .text_size(px(TEXT_CAPTION))
                .text_color(rgb(ground.ink))
                .child(if done {
                    item.done_label()
                } else {
                    item.label()
                }),
        )
        .when(!done, |row| row.children(item.key().map(panel_key)))
}

fn panel_key(key: Key) -> Div {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    kit.keycap_inked(key, ground.soft)
        .border_color(rgba(ground.edge.packed()))
}

fn panel_hints() -> Div {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    let hint = |key: Key, label: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(SPACE_TIGHT))
            .child(panel_key(key))
            .child(label)
    };
    div()
        .flex_none()
        .flex()
        .justify_center()
        .items_center()
        .gap(px(SPACE_CELL))
        .pt(px(SPACE_INSET))
        .pb(px(SPACE_STACK))
        .border_t(px(LINE))
        .border_color(rgba(ground.edge.packed()))
        .text(TextStyle::Hint)
        .text_color(rgb(ground.faint))
        .child(hint(Key::TAB, "next"))
        .child(hint(Key::TAB.shift(), "back"))
}

fn tag_hue(kind: FactKind) -> u32 {
    match kind {
        FactKind::Content => tags::LAW,
        FactKind::Size => tags::FACT,
        FactKind::Time => tags::USE,
        FactKind::Empty | FactKind::Hidden => tags::DIM,
        FactKind::NoAccess => tags::NEG,
    }
}

pub fn path_label(path: &Path, home: Option<&Path>, link: Option<&Path>) -> String {
    let mut label = spaced_path(path.parent().unwrap_or(path), home, PATH_CHARS);
    if let Some(link) = link {
        let target = home
            .and_then(|home| link.strip_prefix(home).ok())
            .map_or_else(
                || link.display().to_string(),
                |rest| format!("~/{}", rest.display()),
            );
        label.push_str("  \u{2192} ");
        label.push_str(&target);
    }
    label
}

pub fn spaced_path(path: &Path, home: Option<&Path>, max: usize) -> String {
    let (first, rest) = match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => ("~", rest),
        None => ("/", path.strip_prefix("/").unwrap_or(path)),
    };
    let segments: Vec<String> = rest
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let joined = shorten(first, segments, max).join(" / ");
    if joined.is_empty() {
        first.to_owned()
    } else if first == "~" {
        format!("~ / {joined}")
    } else {
        format!("/ {joined}")
    }
}

fn shorten(first: &str, segments: Vec<String>, max: usize) -> Vec<String> {
    let width = |parts: &[String]| {
        parts
            .iter()
            .map(|part| part.chars().count() + 1)
            .sum::<usize>()
    };
    if first.chars().count() + width(&segments) <= max {
        return segments;
    }
    let mut kept: Vec<String> = Vec::new();
    for segment in segments.iter().rev() {
        let grown = first.chars().count() + 2 + width(&kept) + segment.chars().count() + 1;
        if !kept.is_empty() && grown > max {
            break;
        }
        kept.insert(0, segment.clone());
    }
    std::iter::once("\u{2026}".to_owned()).chain(kept).collect()
}

#[cfg(test)]
mod tests {
    use super::{path_label, spaced_path, PanelItem};
    use std::path::{Path, PathBuf};

    #[test]
    fn path_labels_shorten_from_the_front_like_the_board() {
        let home = PathBuf::from("/home/qol");
        let deep = home.join(
            "Projects/qol-monorepo/plugins/launcher/src/ui/components/results/rows/render.rs",
        );
        assert_eq!(
            path_label(&deep, Some(&home), None),
            "~ / \u{2026} / launcher / src / ui / components / results / rows"
        );
        assert_eq!(
            path_label(&home.join("Downloads/recovery.iso"), Some(&home), None),
            "~ / Downloads"
        );
        assert_eq!(path_label(&home.join("todo.txt"), Some(&home), None), "~");
        assert_eq!(
            path_label(Path::new("/etc/hosts"), Some(&home), None),
            "/ etc"
        );
        assert_eq!(path_label(Path::new("/hosts"), Some(&home), None), "/");
    }

    #[test]
    fn spaced_paths_keep_the_file_name() {
        let home = PathBuf::from("/home/qol");
        assert_eq!(
            spaced_path(Path::new("/usr/bin/xed"), Some(&home), 42),
            "/ usr / bin / xed"
        );
        assert_eq!(
            spaced_path(&home.join(".local/bin/tool"), Some(&home), 42),
            "~ / .local / bin / tool"
        );
    }

    #[test]
    fn links_name_their_target() {
        let home = PathBuf::from("/home/qol");
        assert_eq!(
            path_label(
                &home.join("Desktop/recipes.md"),
                Some(&home),
                Some(&home.join("Documents/cooking/recipes.md"))
            ),
            "~ / Desktop  \u{2192} ~/Documents/cooking/recipes.md"
        );
    }

    #[test]
    fn panel_keys_match_the_board() {
        assert_eq!(
            PanelItem::ALL.map(PanelItem::label),
            ["Open", "Open folder", "Copy path", "Copy name"]
        );
        assert!(PanelItem::CopyName.key().is_none());
    }
}
