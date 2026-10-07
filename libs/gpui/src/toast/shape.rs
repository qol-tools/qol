use gpui::*;

use super::POINTER_POLL;
use crate::popup_window::{InputEvent, InputWatch, WindowGeometrySession};

pub(super) type Held<V> = fn(&mut V) -> &mut InputShape;
pub(super) type Named<V> = fn(&V) -> String;
pub(super) type Sensed<V> = fn(&mut V, Option<InputEvent>, &mut Context<V>);

#[derive(Default)]
pub(super) struct InputShape {
    session: Option<WindowGeometrySession>,
    reach: [f32; 4],
    sensing: bool,
    listening: bool,
}

impl InputShape {
    pub(super) fn run<V: 'static, R: Send + 'static>(
        &self,
        title: String,
        held: Held<V>,
        work: impl FnOnce(&WindowGeometrySession) -> R + Send + 'static,
        done: impl FnOnce(&mut V, Option<R>, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) {
        let session = self.session.clone();
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
                let shape = held(view);
                if shape.session.is_none() {
                    shape.session = session;
                }
                done(view, result, cx);
            });
        })
        .detach();
    }

    pub(super) fn reach<V: 'static>(
        &mut self,
        title: String,
        held: Held<V>,
        reach: [f32; 4],
        cx: &mut Context<V>,
    ) {
        if reach == self.reach {
            return;
        }
        self.reach = reach;
        let (x, y) = (reach[0].max(0.0) as i16, reach[1].max(0.0) as i16);
        let (width, height) = (reach[2].ceil() as u16, reach[3].ceil() as u16);
        self.run(
            title,
            held,
            move |session| session.set_input_region(x, y, width, height),
            move |view, applied, _| {
                let shape = held(view);
                if applied != Some(true) && shape.reach == reach {
                    shape.reach = [f32::NAN; 4];
                }
            },
            cx,
        );
    }

    pub(super) fn sense<V: 'static>(
        &mut self,
        title: String,
        held: Held<V>,
        named: Named<V>,
        on: Sensed<V>,
        cx: &mut Context<V>,
    ) {
        if std::mem::replace(&mut self.sensing, true) {
            return;
        }
        match crate::popup_window::watch_input(&title) {
            Some(watch) => {
                self.listening = true;
                listen(watch, held, on, cx);
                confirm(held, named, on, cx);
            }
            None => poll(held, named, on, cx),
        }
    }

    pub(super) fn rest(&mut self) {
        self.sensing = false;
    }

    pub(super) fn listening(&self) -> bool {
        self.listening
    }
}

fn listen<V: 'static>(mut watch: InputWatch, held: Held<V>, on: Sensed<V>, cx: &mut Context<V>) {
    cx.spawn(async move |this, cx| {
        while let Some(event) = watch.next().await {
            let live = this.update(cx, |view, cx| {
                on(view, Some(event), cx);
                held(view).sensing
            });
            if !live.unwrap_or(false) {
                break;
            }
        }
        let _ = this.update(cx, |view, _| {
            let shape = held(view);
            shape.listening = false;
            shape.sensing = false;
        });
    })
    .detach();
}

fn confirm<V: 'static>(held: Held<V>, named: Named<V>, on: Sensed<V>, cx: &mut Context<V>) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(POINTER_POLL).await;
        let _ = this.update(cx, |view, cx| {
            if !held(view).sensing {
                return;
            }
            let pointer = crate::popup_window::pointer_on_window_by_title(&named(view));
            on(
                view,
                Some(InputEvent::Pointer(pointer.unwrap_or_default())),
                cx,
            );
            if held(view).sensing {
                confirm(held, named, on, cx);
            }
        });
    })
    .detach();
}

fn poll<V: 'static>(held: Held<V>, named: Named<V>, on: Sensed<V>, cx: &mut Context<V>) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(POINTER_POLL).await;
        let _ = this.update(cx, |view, cx| {
            let title = named(view);
            held(view).run(
                title,
                held,
                WindowGeometrySession::pointer_on,
                move |view, pointer, cx| {
                    on(view, pointer.flatten().map(InputEvent::Pointer), cx);
                    if held(view).sensing {
                        poll(held, named, on, cx);
                    }
                },
                cx,
            )
        });
    })
    .detach();
}
