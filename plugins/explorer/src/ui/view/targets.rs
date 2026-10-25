use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, px, AnyElement, AsyncApp, Bounds, Context, Div, DragMoveEvent, ExternalPaths,
    InteractiveElement, IntoElement, ParentElement, Pixels, Point, Styled, WeakEntity, Window,
};
use qol_gpui::kit::{kit, Chip};
use qol_gpui::theme::{Ground, StatusTone, HEIGHT_CONTROL, SPACE_GUTTER};

use super::ExplorerView;
use crate::ops::{self, Transfer, Verb};
use crate::ui::browse::{copy_only_note, short_note, Browser};

const PREFLIGHT_AFTER: Duration = Duration::from_millis(120);
const PIN_EDGE: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Zone {
    Row(usize),
    Rail(usize),
    Crumb(usize),
    Pin(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PinSpot {
    slot: Option<usize>,
    row: Option<usize>,
}

pub(super) struct PointerTag {
    pub(super) text: String,
    pub(super) landing: bool,
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

pub(super) fn pin_spot(y: f32, open: Option<usize>, count: usize) -> PinSpot {
    let visual = (y / HEIGHT_CONTROL).floor().max(0.0) as usize;
    if open == Some(visual) {
        return PinSpot {
            slot: open,
            row: None,
        };
    }
    let row = match open {
        Some(open) if visual > open => visual - 1,
        _ => visual,
    };
    if row >= count {
        return PinSpot {
            slot: Some(count),
            row: None,
        };
    }
    let within = (y / HEIGHT_CONTROL).fract();
    let slot = if within < PIN_EDGE {
        Some(row)
    } else if within > 1.0 - PIN_EDGE {
        Some(row + 1)
    } else {
        None
    };
    PinSpot {
        slot,
        row: Some(row),
    }
}

fn ordinal(place: usize) -> String {
    let suffix = match (place % 10, place % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{place}{suffix}")
}

fn pin_words(count: usize, slot: usize) -> String {
    let place = ordinal(slot + 1);
    if count == 1 {
        format!("pin {place}")
    } else {
        format!("pin {count} from {place}")
    }
}

fn pin_y(bounds: Bounds<Pixels>, at: Point<Pixels>) -> Option<f32> {
    bounds.contains(&at).then(|| f32::from(at.y - bounds.top()))
}

pub(super) fn pins<E: InteractiveElement>(element: E, cx: &mut Context<ExplorerView>) -> E {
    element
        .on_drag_move(
            cx.listener(move |view, event: &DragMoveEvent<PathBuf>, _window, cx| {
                let pointer = Pointer {
                    inside: event.bounds.contains(&event.event.position),
                    ctrl: event.event.modifiers.control,
                    external: false,
                };
                let y = pin_y(event.bounds, event.event.position);
                let from = event.drag(cx).clone();
                view.pins_move(y, pointer, cx, |view| view.browser.drag_paths(&from));
            }),
        )
        .on_drag_move(cx.listener(
            move |view, event: &DragMoveEvent<ExternalPaths>, _window, cx| {
                let pointer = Pointer {
                    inside: event.bounds.contains(&event.event.position),
                    ctrl: event.event.modifiers.control,
                    external: true,
                };
                let y = pin_y(event.bounds, event.event.position);
                let dropped = event.drag(cx).paths().to_vec();
                view.pins_move(y, pointer, cx, |_| dropped);
            },
        ))
        .on_drop(
            cx.listener(move |view, from: &PathBuf, window: &mut Window, cx| {
                let sources = view.browser.drag_paths(from);
                let ctrl = window.modifiers().control;
                view.drop_on_pins(sources, false, ctrl, cx);
            }),
        )
        .on_drop(cx.listener(
            move |view, dropped: &ExternalPaths, window: &mut Window, cx| {
                let ctrl = window.modifiers().control;
                view.drop_on_pins(dropped.paths().to_vec(), true, ctrl, cx);
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
    fn on_pins(&self, zone: Zone) -> bool {
        match zone {
            Zone::Pin(_) => true,
            Zone::Rail(row) => row < self.places.len(),
            Zone::Row(_) | Zone::Crumb(_) => false,
        }
    }

    fn leave_pins(&mut self, cx: &mut Context<Self>) {
        if self
            .hover
            .as_ref()
            .is_some_and(|hover| self.on_pins(hover.zone))
        {
            self.hover = None;
            cx.notify();
        }
    }

    fn pins_move(
        &mut self,
        y: Option<f32>,
        pointer: Pointer,
        cx: &mut Context<Self>,
        sources: impl FnOnce(&Self) -> Vec<PathBuf>,
    ) {
        let Some(y) = y else {
            self.leave_pins(cx);
            return;
        };
        let open = self.hover.as_ref().and_then(|hover| match hover.zone {
            Zone::Pin(slot) => Some(slot),
            _ => None,
        });
        let spot = pin_spot(y, open, self.places.len());
        if spot.slot.is_some() && spot.slot == open {
            return;
        }
        let settled = self.hover.as_ref().is_some_and(|hover| {
            spot.slot.is_none()
                && spot.row.map(Zone::Rail) == Some(hover.zone)
                && hover.ctrl == pointer.ctrl
        });
        if settled {
            return;
        }
        let sources = sources(self);
        if let Some(slot) = spot.slot {
            let folders = self.pinnable(&sources);
            if !folders.is_empty() {
                self.hover = Some(Hover {
                    zone: Zone::Pin(slot),
                    target: PathBuf::new(),
                    sources: folders,
                    external: pointer.external,
                    ctrl: pointer.ctrl,
                    moves: false,
                    fit: Fit::Fine,
                });
                cx.notify();
                return;
            }
        }
        let row = spot
            .row
            .and_then(|row| Some((row, self.places.get(row)?)))
            .filter(|(_, place)| place.connected)
            .map(|(row, place)| (row, place.path.clone()));
        match row {
            Some((row, target)) => {
                self.hover_move(Zone::Rail(row), &target, pointer, cx, |_| sources)
            }
            None => self.leave_pins(cx),
        }
    }

    fn pinnable(&self, sources: &[PathBuf]) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = Vec::new();
        for source in sources {
            let pinned = self.places.iter().any(|place| place.path == *source);
            if !pinned && !folders.contains(source) && source.is_dir() {
                folders.push(source.clone());
            }
        }
        folders
    }

    fn drop_on_pins(
        &mut self,
        sources: Vec<PathBuf>,
        external: bool,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(hover) = self.hover.as_ref() else {
            return;
        };
        match hover.zone {
            Zone::Pin(slot) => {
                let folders = hover.sources.clone();
                self.hover = None;
                cx.notify();
                self.pin_folders(slot, folders, cx);
            }
            Zone::Rail(row) if row < self.places.len() => {
                let target = hover.target.clone();
                self.drop_into(target, sources, external, ctrl, cx);
            }
            Zone::Rail(_) | Zone::Row(_) | Zone::Crumb(_) => {}
        }
    }

    fn pin_folders(&mut self, slot: usize, folders: Vec<PathBuf>, cx: &mut Context<Self>) {
        if !self.places_ready() {
            return;
        }
        let home = self.browser.home();
        let added = folders
            .iter()
            .map(|folder| crate::places::pin_for(folder, home))
            .collect();
        if let Some(pins) = crate::places::pin_at(&self.pins, &self.places, slot, added) {
            self.change_pins(pins, cx);
        }
    }

    pub(super) fn pin_landing(&self) -> Option<(usize, String)> {
        let hover = self.hover.as_ref()?;
        let Zone::Pin(slot) = hover.zone else {
            return None;
        };
        let first = hover.sources.first()?;
        let name = first.file_name().map_or_else(
            || first.to_string_lossy().into_owned(),
            crate::listing::display_name,
        );
        Some((slot, name))
    }

    pub(super) fn pointer_tag(&self) -> Option<PointerTag> {
        let hover = self.hover.as_ref()?;
        match hover.zone {
            Zone::Pin(slot) => Some(PointerTag {
                text: pin_words(hover.sources.len(), slot),
                landing: true,
            }),
            Zone::Rail(row) if !hover.external => {
                let place = self.places.get(row)?;
                let verb = match hover.fit {
                    Fit::Checking | Fit::Fine if hover.moves => "move",
                    Fit::Checking | Fit::Fine => "copy",
                    Fit::NoRoom(_) | Fit::CopyOnly(_) => return None,
                };
                Some(PointerTag {
                    text: format!("{verb} into {}", place.name),
                    landing: false,
                })
            }
            Zone::Rail(_) | Zone::Row(_) | Zone::Crumb(_) => None,
        }
    }

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
        let overlay = div().absolute().inset_0();
        if self.pointer_tag().is_some() {
            return Some(overlay);
        }
        Some(
            overlay
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
        if let Zone::Pin(_) = hover.zone {
            let them = if hover.sources.len() == 1 {
                "it"
            } else {
                "them"
            };
            return Some(format!("Drop to pin {them} in the sidebar."));
        }
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
        if hover.ctrl == ctrl || !cx.has_active_drag() || matches!(hover.zone, Zone::Pin(_)) {
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

#[cfg(test)]
mod tests {
    use super::{ordinal, pin_spot, pin_words, PinSpot};

    fn spot(slot: Option<usize>, row: Option<usize>) -> PinSpot {
        PinSpot { slot, row }
    }

    #[test]
    fn the_pin_spot_is_a_slot_at_a_row_edge_and_the_row_in_its_middle() {
        let cases = [
            (2.0, None, 3, spot(Some(0), Some(0))),
            (18.0, None, 3, spot(None, Some(0))),
            (34.0, None, 3, spot(Some(1), Some(0))),
            (40.0, None, 3, spot(Some(1), Some(1))),
            (126.0, None, 3, spot(Some(3), None)),
            (40.0, Some(1), 3, spot(Some(1), None)),
            (76.0, Some(1), 3, spot(Some(1), Some(1))),
            (90.0, Some(1), 3, spot(None, Some(1))),
            (34.0, Some(1), 3, spot(Some(1), Some(0))),
            (110.0, Some(3), 3, spot(Some(3), None)),
            (-4.0, None, 3, spot(Some(0), Some(0))),
            (4.0, None, 0, spot(Some(0), None)),
        ];
        for (y, open, count, expected) in cases {
            assert_eq!(pin_spot(y, open, count), expected, "{y} {open:?} {count}");
        }
    }

    #[test]
    fn the_pin_words_name_the_place_the_folders_land() {
        let cases = [
            (1, 0, "pin 1st"),
            (1, 1, "pin 2nd"),
            (1, 2, "pin 3rd"),
            (1, 4, "pin 5th"),
            (1, 10, "pin 11th"),
            (1, 20, "pin 21st"),
            (3, 4, "pin 3 from 5th"),
        ];
        for (count, slot, words) in cases {
            assert_eq!(pin_words(count, slot), words, "{count} {slot}");
        }
        assert_eq!(ordinal(112), "112th");
        assert_eq!(ordinal(102), "102nd");
    }
}
