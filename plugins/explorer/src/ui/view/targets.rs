use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, px, AnyElement, AsyncApp, Context, Div, DragMoveEvent, ExternalPaths, InteractiveElement,
    IntoElement, ParentElement, Styled, WeakEntity, Window,
};
use qol_gpui::kit::{kit, Chip};
use qol_gpui::theme::{Ground, StatusTone, SPACE_GUTTER};

use super::ExplorerView;
use crate::ops::{self, Transfer, Verb};
use crate::ui::browse::{copy_only_note, short_note, Browser};

const PREFLIGHT_AFTER: Duration = Duration::from_millis(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Zone {
    Row(usize),
    Rail(usize),
    Crumb(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Fit {
    Checking,
    Fine,
    NoRoom(String),
    CopyOnly(String),
}

#[derive(Clone, Copy)]
struct Pointer {
    inside: bool,
    ctrl: bool,
    external: bool,
}

pub(super) struct Hover {
    zone: Zone,
    target: PathBuf,
    sources: Vec<PathBuf>,
    external: bool,
    ctrl: bool,
    moves: bool,
    fit: Fit,
}

fn takes(sources: &[PathBuf], target: &Path, moves: bool) -> bool {
    let inside = sources
        .iter()
        .any(|source| target.starts_with(source.as_path()));
    let stays = moves
        && !sources.is_empty()
        && sources.iter().all(|source| source.parent() == Some(target));
    !inside && !stays && !sources.is_empty()
}

pub(super) fn target<E: InteractiveElement>(
    element: E,
    zone: Zone,
    path: PathBuf,
    cx: &mut Context<ExplorerView>,
) -> E {
    let inner = path.clone();
    let outer = path.clone();
    let external = path.clone();
    let external_drop = path.clone();
    element
        .on_drag_move(
            cx.listener(move |view, event: &DragMoveEvent<PathBuf>, _window, cx| {
                let pointer = Pointer {
                    inside: event.bounds.contains(&event.event.position),
                    ctrl: event.event.modifiers.control,
                    external: false,
                };
                let from = event.drag(cx).clone();
                view.hover_move(zone, &inner, pointer, cx, |view| {
                    view.browser.drag_paths(&from)
                });
            }),
        )
        .on_drag_move(cx.listener(
            move |view, event: &DragMoveEvent<ExternalPaths>, _window, cx| {
                let pointer = Pointer {
                    inside: event.bounds.contains(&event.event.position),
                    ctrl: event.event.modifiers.control,
                    external: true,
                };
                let dropped = event.drag(cx).paths().to_vec();
                view.hover_move(zone, &external, pointer, cx, |_| dropped);
            },
        ))
        .on_drop(
            cx.listener(move |view, from: &PathBuf, window: &mut Window, cx| {
                let sources = view.browser.drag_paths(from);
                let ctrl = window.modifiers().control;
                view.drop_into(outer.clone(), sources, false, ctrl, cx);
            }),
        )
        .on_drop(cx.listener(
            move |view, dropped: &ExternalPaths, window: &mut Window, cx| {
                let ctrl = window.modifiers().control;
                view.drop_into(
                    external_drop.clone(),
                    dropped.paths().to_vec(),
                    true,
                    ctrl,
                    cx,
                );
            },
        ))
}

pub(super) struct Span {
    pub from: f32,
    pub to: f32,
    pub path: PathBuf,
}

fn span_at(spans: &[Span], x: f32) -> Option<usize> {
    spans.iter().position(|span| x >= span.from && x < span.to)
}

pub(super) fn trail<E: InteractiveElement>(
    element: E,
    spans: Rc<Vec<Span>>,
    cx: &mut Context<ExplorerView>,
) -> E {
    let paths = spans.clone();
    let others = spans;
    element
        .on_drag_move(
            cx.listener(move |view, event: &DragMoveEvent<PathBuf>, _window, cx| {
                let x = f32::from(event.event.position.x - event.bounds.left());
                let pointer = Pointer {
                    inside: event.bounds.contains(&event.event.position),
                    ctrl: event.event.modifiers.control,
                    external: false,
                };
                let from = event.drag(cx).clone();
                match span_at(&paths, x) {
                    Some(index) => view.hover_move(
                        Zone::Crumb(index),
                        &paths[index].path,
                        pointer,
                        cx,
                        |view| view.browser.drag_paths(&from),
                    ),
                    None => view.leave_trail(cx),
                }
            }),
        )
        .on_drag_move(cx.listener(
            move |view, event: &DragMoveEvent<ExternalPaths>, _window, cx| {
                let x = f32::from(event.event.position.x - event.bounds.left());
                let pointer = Pointer {
                    inside: event.bounds.contains(&event.event.position),
                    ctrl: event.event.modifiers.control,
                    external: true,
                };
                let dropped = event.drag(cx).paths().to_vec();
                match span_at(&others, x) {
                    Some(index) => view.hover_move(
                        Zone::Crumb(index),
                        &others[index].path,
                        pointer,
                        cx,
                        |_| dropped,
                    ),
                    None => view.leave_trail(cx),
                }
            },
        ))
        .on_drop(
            cx.listener(move |view, from: &PathBuf, window: &mut Window, cx| {
                let sources = view.browser.drag_paths(from);
                let ctrl = window.modifiers().control;
                view.drop_on_trail(sources, false, ctrl, cx);
            }),
        )
        .on_drop(cx.listener(
            move |view, dropped: &ExternalPaths, window: &mut Window, cx| {
                let ctrl = window.modifiers().control;
                view.drop_on_trail(dropped.paths().to_vec(), true, ctrl, cx);
            },
        ))
}

impl ExplorerView {
    fn leave_trail(&mut self, cx: &mut Context<Self>) {
        if self
            .hover
            .as_ref()
            .is_some_and(|hover| matches!(hover.zone, Zone::Crumb(_)))
        {
            self.hover = None;
            cx.notify();
        }
    }

    fn drop_on_trail(
        &mut self,
        sources: Vec<PathBuf>,
        external: bool,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .hover
            .as_ref()
            .filter(|hover| matches!(hover.zone, Zone::Crumb(_)))
            .map(|hover| hover.target.clone());
        if let Some(target) = target {
            self.drop_into(target, sources, external, ctrl, cx);
        }
    }

    fn hover_move(
        &mut self,
        zone: Zone,
        target: &Path,
        pointer: Pointer,
        cx: &mut Context<Self>,
        sources: impl FnOnce(&Self) -> Vec<PathBuf>,
    ) {
        let Pointer {
            inside,
            ctrl,
            external,
        } = pointer;
        if !inside {
            if self.hover.as_ref().is_some_and(|hover| hover.zone == zone) {
                self.hover = None;
                cx.notify();
            }
            return;
        }
        if let Some(hover) = &self.hover {
            if hover.zone == zone && hover.target == target && hover.ctrl == ctrl {
                return;
            }
        }
        let sources = match self.hover.take() {
            Some(hover) if hover.zone == zone && hover.target == target => hover.sources,
            _ => sources(self),
        };
        let copy = ctrl || external;
        let moves = !copy
            && matches!(
                self.browser
                    .drop_on(sources.clone(), target.to_path_buf(), false)
                    .verb,
                Verb::Move
            );
        if !takes(&sources, target, moves) {
            self.hover = None;
            cx.notify();
            return;
        }
        let verb = if moves { Verb::Move } else { Verb::Copy };
        self.hover = Some(Hover {
            zone,
            target: target.to_path_buf(),
            sources: sources.clone(),
            external,
            ctrl,
            moves,
            fit: Fit::Checking,
        });
        self.preflight(
            Transfer {
                verb,
                sources,
                into: target.to_path_buf(),
            },
            zone,
            cx,
        );
        cx.notify();
    }

    fn preflight(&mut self, transfer: Transfer, zone: Zone, cx: &mut Context<Self>) {
        let target = transfer.into.clone();
        let moves = transfer.verb == Verb::Move;
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(PREFLIGHT_AFTER).await;
            let current = view
                .update(cx, |view, _| view.hovering(zone, &target, moves))
                .unwrap_or(false);
            if !current {
                return;
            }
            let survey = cx
                .background_executor()
                .spawn(async move { ops::survey(transfer) })
                .await;
            let _ = view.update(cx, |view, cx| {
                let fit = match survey {
                    Ok(survey) => {
                        let volume = view.volume_label(&survey.transfer.into);
                        let short = short_note(&survey, &volume);
                        let kept = copy_only_note(&survey).filter(|_| moves);
                        match (short, kept) {
                            (Some(note), _) => Fit::NoRoom(note),
                            (None, Some(note)) => Fit::CopyOnly(note),
                            (None, None) => Fit::Fine,
                        }
                    }
                    Err(_) => Fit::Fine,
                };
                if view.hovering(zone, &target, moves) {
                    if let Some(hover) = &mut view.hover {
                        hover.fit = fit;
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn hovering(&self, zone: Zone, target: &Path, moves: bool) -> bool {
        self.hover.as_ref().is_some_and(|hover| {
            hover.zone == zone && hover.target == target && hover.moves == moves
        })
    }

    pub(super) fn drop_into(
        &mut self,
        into: PathBuf,
        sources: Vec<PathBuf>,
        external: bool,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) {
        let hover = self.hover.take();
        cx.notify();
        let fit = hover
            .filter(|hover| hover.target == into)
            .map_or(Fit::Fine, |hover| hover.fit);
        if let Fit::NoRoom(_) = fit {
            self.browser
                .fail("There is no room for that here".to_string());
            return;
        }
        let copy = ctrl || external || matches!(fit, Fit::CopyOnly(_));
        let moves = !copy
            && matches!(
                self.browser
                    .drop_on(sources.clone(), into.clone(), false)
                    .verb,
                Verb::Move
            );
        if !takes(&sources, &into, moves) {
            return;
        }
        let transfer = self.browser.drop_on(sources, into, copy);
        self.browser.begin(transfer);
    }

    pub(super) fn drop_here(&mut self, sources: Vec<PathBuf>, cx: &mut Context<Self>) {
        let into = self.browser.folder().to_path_buf();
        self.drop_into(into, sources, true, true, cx);
    }

    pub(super) fn drop_overlay(&self, zone: Zone, ground: Ground) -> Option<Div> {
        let tag = self.drop_tag(zone, ground)?;
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_row()
                .items_center()
                .justify_end()
                .pr(px(SPACE_GUTTER))
                .child(tag),
        )
    }

    fn drop_tag(&self, zone: Zone, ground: Ground) -> Option<AnyElement> {
        let hover = self.hover.as_ref().filter(|hover| hover.zone == zone)?;
        let text = match hover.fit {
            Fit::NoRoom(_) => "no room",
            Fit::CopyOnly(_) => "copy only",
            Fit::Checking | Fit::Fine if hover.moves => "move here",
            Fit::Checking | Fit::Fine => "copy here",
        };
        let kit = kit();
        let chip = match hover.fit {
            Fit::NoRoom(_) | Fit::CopyOnly(_) => Chip::Status {
                tone: StatusTone::Warning,
                text: text.into(),
            },
            Fit::Checking | Fit::Fine => Chip::Tag(text.into()),
        };
        Some(kit.chip(chip, ground).into_any_element())
    }

    pub(super) fn hovered(&self, zone: Zone) -> bool {
        self.hover.as_ref().is_some_and(|hover| hover.zone == zone)
    }

    pub(super) fn drag_note(&self) -> Option<String> {
        let hover = self.hover.as_ref()?;
        Some(match &hover.fit {
            Fit::NoRoom(note) | Fit::CopyOnly(note) => note.clone(),
            Fit::Checking | Fit::Fine if hover.external || (!hover.moves && !hover.ctrl) => {
                "Drop to copy.".to_string()
            }
            Fit::Checking | Fit::Fine => Browser::drop_hint(hover.ctrl).to_string(),
        })
    }

    pub(super) fn volume_label(&self, target: &Path) -> String {
        crate::places::device_label(&self.devices, target)
            .unwrap_or_else(|| "The destination".to_string())
    }

    pub(super) fn drop_modifiers_changed(&mut self, ctrl: bool, cx: &mut Context<Self>) {
        let Some(hover) = &self.hover else {
            return;
        };
        if hover.ctrl == ctrl || !cx.has_active_drag() {
            return;
        }
        let (zone, target, external) = (hover.zone, hover.target.clone(), hover.external);
        let pointer = Pointer {
            inside: true,
            ctrl,
            external,
        };
        self.hover_move(zone, &target, pointer, cx, |_| Vec::new());
    }
}
