use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use super::super::card::{self, Ring};
use super::super::{pile, RowId, SlabSnapshotRow, Tick, AGE_TICK, POINTER_POLL, RING_TICK};
use super::SlabPresenter;
use crate::popup_window::{EscapeGrab, PointerOnWindow, WindowGeometrySession};
use crate::surface::SurfaceDismisser;
use crate::text::TextStyled as _;

struct Shown {
    row: SlabSnapshotRow,
    pose: pile::Pose,
    index: usize,
    leaving: bool,
}

struct Glide {
    start: Instant,
    from: Vec<Shown>,
    strip: Option<(pile::Pose, usize)>,
}

pub(super) struct SlabToastView {
    host: SlabPresenter,
    card: card::Host,
    dismisser: SurfaceDismisser,
    session: Option<WindowGeometrySession>,
    inside: bool,
    hovered: Option<RowId>,
    lit: bool,
    grow: pile::Tween,
    open: pile::Tween,
    focus: Vec<(RowId, pile::Tween)>,
    reach: [f32; 4],
    shown: Vec<Shown>,
    strip_shown: Option<(pile::Pose, usize)>,
    glide: Option<Glide>,
    closing: bool,
    scroll: pile::Tween,
    scroll_max: f32,
    arriving: bool,
    arrival: Option<Instant>,
    escape: Option<EscapeGrab>,
    escape_armed: bool,
    polling: bool,
    ring: Tick,
    age: Tick,
}

impl SlabToastView {
    pub(super) fn new(
        host: SlabPresenter,
        dismisser: SurfaceDismisser,
        scroll: f32,
        arriving: bool,
    ) -> Self {
        let open = if host.state.borrow().expanded {
            1.0
        } else {
            0.0
        };
        Self {
            card: Rc::new(host.clone()),
            host,
            dismisser,
            session: None,
            inside: false,
            hovered: None,
            lit: false,
            grow: pile::Tween::at(0.0),
            open: pile::Tween::at(open),
            focus: Vec::new(),
            reach: [0.0; 4],
            shown: Vec::new(),
            strip_shown: None,
            glide: None,
            closing: false,
            scroll: pile::Tween::at(scroll),
            scroll_max: 0.0,
            arriving,
            arrival: None,
            escape: None,
            escape_armed: false,
            polling: false,
            ring: Tick::default(),
            age: Tick::default(),
        }
    }

    pub(super) fn moving_away(&mut self) -> f32 {
        self.escape = None;
        self.scroll.target()
    }

    fn enter(&mut self, cx: &mut Context<Self>) {
        if self.inside {
            return;
        }
        self.inside = true;
        self.host.set_hovering(true);
        self.ensure_polling(cx);
        cx.notify();
    }

    fn point(&mut self, hovered: Option<RowId>, lit: bool, cx: &mut Context<Self>) {
        if self.hovered != hovered || self.lit != lit {
            self.hovered = hovered;
            self.lit = lit;
            cx.notify();
        }
        self.enter(cx);
    }

    fn scroll_by(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = f32::from(event.delta.pixel_delta(px(pile::CARD_HEIGHT / 2.0)).y);
        self.scroll_step(delta, cx);
    }

    fn scroll_step(&mut self, delta: f32, cx: &mut Context<Self>) {
        let target = (self.scroll.target() + delta).clamp(0.0, self.scroll_max);
        if target != self.scroll.target() {
            self.scroll.toward(target, Instant::now());
            cx.notify();
        }
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        if !self.inside {
            return;
        }
        self.inside = false;
        self.hovered = None;
        self.lit = false;
        self.host.set_hovering(false);
        self.host.restart_timers(cx);
        cx.notify();
    }

    fn on_session<R: Send + 'static>(
        &mut self,
        work: impl FnOnce(&WindowGeometrySession) -> R + Send + 'static,
        done: impl FnOnce(&mut Self, Option<R>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.clone();
        let title = self.dismisser.current_title();
        cx.spawn(async move |this, cx| {
            let (session, result) = cx
                .background_spawn(async move {
                    let session =
                        session.or_else(|| crate::popup_window::window_geometry_session(&title));
                    let result = session.as_ref().map(work);
                    (session, result)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.session.is_none() {
                    view.session = session;
                }
                done(view, result, cx);
            });
        })
        .detach();
    }

    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        self.polling = true;
        self.poll_pointer(cx);
    }

    fn poll_pointer(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POINTER_POLL).await;
            let _ = this.update(cx, |view, cx| {
                view.on_session(
                    WindowGeometrySession::pointer_on,
                    |view, pointer, cx| view.on_pointer(pointer.flatten(), cx),
                    cx,
                )
            });
        })
        .detach();
    }

    fn on_pointer(&mut self, pointer: Option<PointerOnWindow>, cx: &mut Context<Self>) {
        let Some(pointer) = pointer else {
            self.polling = false;
            self.leave(cx);
            return;
        };
        if !pointer.inside {
            self.leave(cx);
        }
        let expanded = self.host.state.borrow().expanded;
        if !self.inside && !expanded {
            self.polling = false;
            return;
        }
        let escaped = self.escape.as_ref().is_some_and(EscapeGrab::take_pressed);
        if expanded && (escaped || (pointer.pressed && !pointer.inside)) {
            let host = self.host.clone();
            cx.defer(move |cx| host.set_expanded(false, cx));
        }
        self.poll_pointer(cx);
    }

    fn reach_input(&mut self, reach: [f32; 4], cx: &mut Context<Self>) {
        if reach == self.reach {
            return;
        }
        self.reach = reach;
        let (x, y) = (reach[0].max(0.0) as i16, reach[1].max(0.0) as i16);
        let (width, height) = (reach[2].ceil() as u16, reach[3].ceil() as u16);
        self.on_session(
            move |session| session.set_input_region(x, y, width, height),
            move |view, applied, _| {
                if applied != Some(true) && view.reach == reach {
                    view.reach = [f32::NAN; 4];
                }
            },
            cx,
        );
    }

    fn settle_focus(&mut self, rows: &[&SlabSnapshotRow], open: bool, now: Instant) {
        self.focus
            .retain(|(id, _)| rows.iter().any(|row| row.id == *id));
        for row in rows {
            let target = if open && self.hovered == Some(row.id) {
                1.0
            } else {
                0.0
            };
            match self.focus.iter_mut().find(|(id, _)| *id == row.id) {
                Some((_, tween)) => tween.toward(target, now),
                None => {
                    let mut tween = pile::Tween::at(0.0);
                    tween.toward(target, now);
                    self.focus.push((row.id, tween));
                }
            }
        }
    }

    fn focus_of(&self, id: RowId) -> Option<&pile::Tween> {
        self.focus
            .iter()
            .find(|(row, _)| *row == id)
            .map(|(_, tween)| tween)
    }
}

impl Render for SlabToastView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = crate::kit::kit();
        let now = Instant::now();
        let (snapshot, expanded) = self.host.slab_snapshot();
        let rows: Vec<&SlabSnapshotRow> = snapshot.iter().rev().collect();
        let count = rows.len();
        let open = expanded && count >= 2;

        self.grow.toward(if self.inside { 1.0 } else { 0.0 }, now);
        self.open.toward(if open { 1.0 } else { 0.0 }, now);
        self.settle_focus(&rows, open, now);
        let focus_now: Vec<f32> = rows
            .iter()
            .map(|row| self.focus_of(row.id).map_or(0.0, |tween| tween.value(now)))
            .collect();
        let focus_end: Vec<f32> = rows
            .iter()
            .map(|row| self.focus_of(row.id).map_or(0.0, pile::Tween::target))
            .collect();
        let shown = window.bounds().size;
        let width = f32::from(shown.width);
        let height = f32::from(shown.height);
        self.scroll_max = if open {
            pile::overflow(count, height)
        } else {
            0.0
        };
        let escapable = open && self.inside;
        if escapable != self.escape_armed {
            self.escape_armed = escapable;
            self.escape = escapable.then(crate::popup_window::grab_escape).flatten();
            if escapable && self.escape.is_none() {
                log::warn!("[toast] Escape cannot fold the open stack: the key grab failed");
            }
        }
        if open {
            self.ensure_polling(cx);
            if self.scroll.target() > self.scroll_max {
                self.scroll.toward(self.scroll_max, now);
            }
        } else if self.open.value(now) <= 0.0 {
            self.scroll = pile::Tween::at(0.0);
        }
        let scroll = self.scroll.value(now);
        let current = pile::layout(
            count,
            self.grow.value(now),
            self.open.value(now),
            &focus_now,
            scroll,
        );
        let settled = pile::layout(
            count,
            self.grow.target(),
            self.open.target(),
            &focus_end,
            self.scroll.target(),
        );
        let departed = self
            .shown
            .iter()
            .any(|seen| !seen.leaving && rows.iter().all(|row| row.id != seen.row.id));
        if departed {
            self.glide = Some(Glide {
                start: now,
                from: std::mem::take(&mut self.shown),
                strip: self.strip_shown.take(),
            });
        }
        let glide_t = self.glide.as_ref().map_or(1.0, |glide| {
            pile::GLIDE.progress(now.saturating_duration_since(glide.start))
        });
        if self
            .glide
            .as_ref()
            .is_some_and(|glide| now.saturating_duration_since(glide.start) >= pile::GLIDE.duration)
        {
            self.glide = None;
        }
        let poses: Vec<pile::Pose> = (0..count)
            .map(|index| {
                let target = pile::Pose::of(current.cards[index], current.width, current.height);
                self.glide
                    .as_ref()
                    .and_then(|glide| glide.from.iter().find(|seen| seen.row.id == rows[index].id))
                    .map_or(target, |seen| seen.pose.toward(target, glide_t))
            })
            .collect();
        let ghosts: Vec<Shown> = self.glide.as_ref().map_or(Vec::new(), |glide| {
            glide
                .from
                .iter()
                .filter(|seen| rows.iter().all(|row| row.id != seen.row.id))
                .map(|seen| Shown {
                    row: seen.row.clone(),
                    pose: seen.pose.leaving(glide_t),
                    index: seen.index,
                    leaving: true,
                })
                .collect()
        });
        let strip_ghost = self
            .glide
            .as_ref()
            .and_then(|glide| glide.strip)
            .filter(|_| current.strip.is_none())
            .map(|(pose, shown_count)| (pose.leaving(glide_t), shown_count));
        let cards: Vec<pile::CardFrame> = poses
            .iter()
            .map(|pose| pose.card(current.width, current.height))
            .collect();
        if std::mem::take(&mut self.arriving) {
            self.arrival = Some(now);
        }
        let arrival = self.arrival.map_or(1.0, |start| {
            qol_theme::Motion::TRAVEL.progress(now.saturating_duration_since(start))
        });
        let moving = self.grow.moving(now)
            || self.open.moving(now)
            || arrival < 1.0
            || self.focus.iter().any(|(_, tween)| tween.moving(now))
            || self.scroll.moving(now)
            || self.glide.is_some();

        let (reach_width, reach_height) = if moving {
            (
                settled.width.max(self.reach[2]),
                settled.height.max(self.reach[3]),
            )
        } else {
            (settled.width, settled.height)
        };
        let reach_height = reach_height.min(height);
        self.reach_input(
            [
                width - reach_width,
                height - reach_height,
                reach_width,
                reach_height,
            ],
            cx,
        );
        if moving {
            window.request_animation_frame();
        }
        let frame = window.bounds().size;
        let dx = f32::from(frame.width) - current.width;
        let dy = f32::from(frame.height) - current.height;
        let place = |element: Stateful<Div>, at: pile::Frame| {
            element
                .absolute()
                .left(px(at.left + dx))
                .top(px(at.top + dy))
                .w(px(at.width))
                .h(px(at.height))
        };

        let mut ring_running = false;
        let ring_of = |row: &SlabSnapshotRow, view: &Self, ring_running: &mut bool| {
            row.toast.effective_timeout().map(|timeout| {
                if view.inside || open {
                    Ring {
                        remaining: 1.0,
                        ink: kit.grounds.pane.faint,
                    }
                } else {
                    *ring_running = true;
                    let left = row.deadline.map_or(Duration::ZERO, |deadline| {
                        deadline.saturating_duration_since(now)
                    });
                    Ring {
                        remaining: left.as_secs_f32() / timeout.as_secs_f32(),
                        ink: row.toast.tone.color(kit),
                    }
                }
            })
        };
        let clipped = self.scroll_max > 0.0 || scroll > 0.0;
        let deck_height = if clipped {
            f32::from(frame.height) - current.strip.map_or(0.0, |strip| strip.frame.height)
        } else {
            f32::from(frame.height)
        };
        let (above, below) = if clipped {
            (self.scroll_max - scroll, scroll)
        } else {
            (0.0, 0.0)
        };
        let edge_fade = |at: pile::Frame| {
            pile::edge_fade(at.top + dy + at.height / 2.0, deck_height, above, below)
        };
        let ghost_view = |ghost: &Shown, view: &Self| -> AnyElement {
            let frame = ghost.pose.card(current.width, current.height);
            let row = &ghost.row;
            place(div().id(("toast-leaving", row.id.0)), frame.frame)
                .opacity(frame.opacity * edge_fade(frame.frame))
                .child(
                    kit.window()
                        .bg(rgb(card::row_ground(row, kit)))
                        .child(card::content(
                            row,
                            card::CardParts {
                                scale: frame.scale,
                                content: frame.content,
                                interactive: false,
                                ring: ring_of(row, view, &mut false),
                                age: card::age_label(now.saturating_duration_since(row.created)),
                            },
                            kit,
                            None,
                        )),
                )
                .into_any_element()
        };
        let card_view = |index: usize, view: &Self, ring_running: &mut bool| -> AnyElement {
            let row = rows[index];
            let frame = cards[index];
            let ring = ring_of(row, view, ring_running);
            let ground = if index > 0 && view.lit && !open {
                card::row_lift(row, kit)
            } else {
                rgb(card::row_ground(row, kit))
            };
            let id = row.id;
            let mut element = place(div().id(card::card_id(id)), frame.frame)
                .opacity(frame.opacity * edge_fade(frame.frame))
                .occlude()
                .on_mouse_move(cx.listener(move |view, _: &MouseMoveEvent, _, cx| {
                    view.point(Some(id), false, cx)
                }))
                .on_scroll_wheel(
                    cx.listener(|view, event: &ScrollWheelEvent, _, cx| view.scroll_by(event, cx)),
                );
            if index > 0 && !open {
                let opener = view.host.clone();
                element = element
                    .cursor_pointer()
                    .on_click(move |_, _, cx| opener.set_expanded(true, cx));
            }
            element
                .child(kit.window().bg(ground).child(card::content(
                    row,
                    card::CardParts {
                        scale: frame.scale,
                        content: frame.content,
                        interactive: index == 0 || open,
                        ring,
                        age: card::age_label(now.saturating_duration_since(row.created)),
                    },
                    kit,
                    Some(view.card.clone()),
                )))
                .into_any_element()
        };
        let strip_of = |shown_count: usize, scale: f32, view: &Self| {
            let host = view.host.clone();
            kit.window()
                .shadow(Vec::new())
                .child(card::strip(shown_count, scale, kit, move |cx| {
                    host.clear_all(cx)
                }))
        };

        let mut layers: Vec<AnyElement> = Vec::new();
        if open {
            let host = self.host.clone();
            layers.push(
                div()
                    .id("toast-fold")
                    .absolute()
                    .size_full()
                    .on_click(move |_, _, cx| host.set_expanded(false, cx))
                    .into_any_element(),
            );
        }
        let mut deck: Vec<AnyElement> = Vec::new();
        let mut strips: Vec<AnyElement> = Vec::new();
        let mut behind: Vec<&Shown> = ghosts.iter().filter(|ghost| ghost.index > 0).collect();
        behind.sort_by_key(|ghost| std::cmp::Reverse(ghost.index));
        for ghost in behind {
            deck.push(ghost_view(ghost, self));
        }
        let focused = rows
            .iter()
            .position(|row| open && self.hovered == Some(row.id));
        for index in (1..count).rev() {
            if focused != Some(index) && cards[index].opacity > 0.01 {
                deck.push(card_view(index, self, &mut ring_running));
            }
        }
        if let Some((pose, shown_count)) = strip_ghost {
            let strip = pose.card(current.width, current.height);
            strips.push(
                place(div().id("toast-strip-leaving"), strip.frame)
                    .opacity(strip.opacity)
                    .child(strip_of(shown_count, strip.scale, self))
                    .into_any_element(),
            );
        }
        if let Some(strip) = current.strip {
            let newest = rows[0].id;
            strips.push(
                place(div().id("toast-strip"), strip.frame)
                    .occlude()
                    .on_mouse_move(cx.listener(move |view, _: &MouseMoveEvent, _, cx| {
                        view.point(Some(newest), false, cx)
                    }))
                    .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _, cx| {
                        view.scroll_by(event, cx)
                    }))
                    .child(strip_of(count, strip.scale, self))
                    .into_any_element(),
            );
        }
        if !clipped {
            deck.append(&mut strips);
        }
        if count > 0 {
            deck.push(card_view(0, self, &mut ring_running));
        }
        if let Some(index) = focused.filter(|index| *index > 0) {
            deck.push(card_view(index, self, &mut ring_running));
        }
        for ghost in ghosts.iter().filter(|ghost| ghost.index == 0) {
            deck.push(ghost_view(ghost, self));
        }
        layers.push(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(deck_height))
                .overflow_hidden()
                .children(deck)
                .when(clipped, |deck| {
                    let ground = kit.grounds.pane;
                    let veil = qol_theme::translucent(ground.bg, qol_theme::Alpha::Strong);
                    let bar = |edge: crate::scrollbar::OverflowEdge,
                               id: &'static str,
                               hidden: f32,
                               step: f32| {
                        let up = matches!(edge, crate::scrollbar::OverflowEdge::Top);
                        let count = (hidden / pile::LIST_STEP).ceil();
                        let words = if up { "older" } else { "newer" };
                        div()
                            .id(id)
                            .absolute()
                            .left_0()
                            .w_full()
                            .h(px(qol_theme::HEIGHT_HINT_BAR))
                            .when(up, |bar| bar.top_0())
                            .when(!up, |bar| bar.bottom_0())
                            .occlude()
                            .cursor_pointer()
                            .bg(crate::scrollbar::overflow_fade(
                                edge,
                                veil,
                                qol_theme::clear(ground.bg),
                            ))
                            .on_click(cx.listener(move |view, _, _, cx| view.scroll_step(step, cx)))
                            .child(
                                kit.pointable(
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap(px(qol_theme::SPACE_SNUG)),
                                    rgba(qol_theme::translucent(
                                        kit.palette.accent,
                                        qol_theme::Alpha::Halo,
                                    )),
                                )
                                .text(qol_theme::TextStyle::Detail)
                                .text_color(rgb(kit.palette.accent))
                                .child(crate::scrollbar::chevron(edge, kit.palette.accent))
                                .child(SharedString::from(format!("{count} {words}"))),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .w_full()
                                    .h(px(crate::scrollbar::OVERFLOW_CHEVRON_STROKE))
                                    .bg(rgb(kit.palette.accent))
                                    .when(up, |line| line.top_0())
                                    .when(!up, |line| line.bottom_0()),
                            )
                    };
                    deck.child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .w(px(pile::CARD_WIDTH))
                            .h_full()
                            .when(above >= 1.0, |cue| {
                                cue.child(bar(
                                    crate::scrollbar::OverflowEdge::Top,
                                    "toast-scroll-up",
                                    above,
                                    pile::LIST_STEP,
                                ))
                            })
                            .when(below >= 1.0, |cue| {
                                cue.child(bar(
                                    crate::scrollbar::OverflowEdge::Bottom,
                                    "toast-scroll-down",
                                    below,
                                    -pile::LIST_STEP,
                                ))
                            }),
                    )
                })
                .into_any_element(),
        );
        layers.append(&mut strips);
        if let Some(band) = current.band.filter(|_| current.words > 0.01) {
            let host = self.host.clone();
            layers.push(
                place(div().id("toast-show-all"), band)
                    .occlude()
                    .cursor_pointer()
                    .on_mouse_move(
                        cx.listener(|view, _: &MouseMoveEvent, _, cx| view.point(None, true, cx)),
                    )
                    .on_click(move |_, _, cx| host.set_expanded(true, cx))
                    .child(card::show_all(current.words, kit))
                    .into_any_element(),
            );
        }

        if ring_running {
            self.ring.after(RING_TICK, cx);
        }
        if count > 0 {
            self.age.after(AGE_TICK, cx);
        }
        let gliding = self.glide.is_some();
        self.strip_shown = current
            .strip
            .map(|strip| (pile::Pose::of(strip, current.width, current.height), count))
            .or(strip_ghost.filter(|_| gliding));
        self.shown = rows
            .iter()
            .zip(poses)
            .enumerate()
            .map(|(index, (row, pose))| Shown {
                row: (*row).clone(),
                pose,
                index,
                leaving: false,
            })
            .chain(ghosts.into_iter().filter(|_| gliding))
            .collect();
        if count > 0 {
            self.closing = false;
        } else if !gliding && !self.closing {
            self.closing = true;
            let host = self.host.clone();
            cx.defer(move |cx| host.close_if_empty(cx));
        }

        div()
            .id("toast-pile")
            .size_full()
            .relative()
            .top(px((1.0 - arrival) * qol_theme::SPACE_INSET))
            .opacity(arrival)
            .on_mouse_move(
                cx.listener(|view, _: &MouseMoveEvent, _, cx| view.point(None, false, cx)),
            )
            .on_scroll_wheel(
                cx.listener(|view, event: &ScrollWheelEvent, _, cx| view.scroll_by(event, cx)),
            )
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if *hovered {
                    view.enter(cx);
                } else {
                    view.leave(cx);
                }
            }))
            .children(layers)
    }
}
