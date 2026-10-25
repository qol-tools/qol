use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::SystemTime;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gpui::prelude::FluentBuilder;
use gpui::{
    div, img, px, rgb, AnyElement, IntoElement, ObjectFit, ParentElement, Styled, StyledImage,
};
use qol_gpui::kit::kit;
use qol_gpui::theme::{SPACE_STACK, SPACE_TIGHT};

use super::pictogram::play;
use crate::listing::Entry;
use crate::thumbs::{self, Flavor};
use crate::ui::kind::{kind_of, FileKind};

const WORKERS: usize = 2;
const QUEUE_LIMIT: usize = 256;
const PLAY_MARK: f32 = 20.0;
const PLAY_MARK_SMALL: f32 = 12.0;
const PLAY_GLYPH: f32 = 12.0;
const PLAY_GLYPH_SMALL: f32 = 8.0;
const SMALL_FRAME: f32 = 48.0;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key {
    path: PathBuf,
    flavor: Flavor,
    modified: Option<SystemTime>,
}

#[derive(Clone)]
pub(super) struct Shot {
    pub(super) image: PathBuf,
    pub(super) width: f32,
    pub(super) height: f32,
}

#[derive(Clone)]
enum Slot {
    Waiting,
    Ready(Shot),
    Missing,
}

#[derive(Default)]
struct Shared {
    slots: HashMap<Key, Slot>,
    queue: Vec<Key>,
    workers: usize,
}

pub(super) struct Previews {
    shared: Arc<Mutex<Shared>>,
    ready: UnboundedSender<()>,
}

impl Previews {
    pub(super) fn new() -> (Self, UnboundedReceiver<()>) {
        let (ready, readies) = unbounded();
        let previews = Self {
            shared: Arc::default(),
            ready,
        };
        (previews, readies)
    }

    pub(super) fn of(&self, entry: &Entry, edge: f32) -> Option<(Shot, bool)> {
        if entry.opens_as_folder {
            return None;
        }
        let video = match kind_of(&entry.display, entry.executable) {
            FileKind::Image => false,
            FileKind::Video => true,
            _ => return None,
        };
        let key = Key {
            path: entry.path.clone(),
            flavor: Flavor::for_edge(edge),
            modified: entry.modified,
        };
        self.shot(key).map(|shot| (shot, video))
    }

    fn shot(&self, key: Key) -> Option<Shot> {
        let mut shared = lock(&self.shared);
        match shared.slots.get(&key) {
            Some(Slot::Ready(shot)) => return Some(shot.clone()),
            Some(Slot::Waiting | Slot::Missing) => return None,
            None => {}
        }
        forget_older(&mut shared, &key);
        shared.slots.insert(key.clone(), Slot::Waiting);
        shared.queue.push(key);
        if shared.queue.len() > QUEUE_LIMIT {
            let dropped = shared.queue.remove(0);
            shared.slots.remove(&dropped);
        }
        if shared.workers < WORKERS {
            let spawned = thread::Builder::new()
                .name("qol-explorer-previews".into())
                .spawn({
                    let shared = Arc::clone(&self.shared);
                    let ready = self.ready.clone();
                    move || work(&shared, &ready)
                });
            match spawned {
                Ok(_) => shared.workers += 1,
                Err(error) if shared.workers == 0 => {
                    log::warn!("previews will not load: {error}");
                    let stranded = std::mem::take(&mut shared.queue);
                    for key in stranded {
                        shared.slots.insert(key, Slot::Missing);
                    }
                }
                Err(error) => log::warn!("previews load on fewer threads: {error}"),
            }
        }
        None
    }
}

fn work(shared: &Mutex<Shared>, ready: &UnboundedSender<()>) {
    loop {
        let key = {
            let mut shared = lock(shared);
            let Some(key) = shared.queue.pop() else {
                shared.workers -= 1;
                return;
            };
            key
        };
        let slot = panic::catch_unwind(AssertUnwindSafe(|| make(&key.path, key.flavor)))
            .unwrap_or_else(|_| {
                log::warn!("making the preview of {} failed", key.path.display());
                Slot::Missing
            });
        let shown = matches!(slot, Slot::Ready(_));
        if let Some(waiting) = lock(shared).slots.get_mut(&key) {
            *waiting = slot;
        }
        if shown {
            let _ = ready.unbounded_send(());
        }
    }
}

fn make(path: &Path, flavor: Flavor) -> Slot {
    match thumbs::thumbnail(path, flavor) {
        Ok(image) => match thumbs::image_size(&image) {
            Some((width, height)) => Slot::Ready(Shot {
                image,
                width: width as f32,
                height: height as f32,
            }),
            None => {
                log::debug!("unreadable preview {}", image.display());
                Slot::Missing
            }
        },
        Err(error) => {
            log::debug!("no preview for {}: {error}", path.display());
            Slot::Missing
        }
    }
}

fn forget_older(shared: &mut Shared, key: &Key) {
    shared
        .slots
        .retain(|known, _| known.path != key.path || known.flavor != key.flavor);
    shared
        .queue
        .retain(|queued| queued.path != key.path || queued.flavor != key.flavor);
}

pub(super) fn framed(shot: Shot, video: bool, room: (f32, f32)) -> AnyElement {
    let (width, height) = fit((shot.width, shot.height), room);
    let small = width.min(height) < SMALL_FRAME;
    let (mark, glyph, inset) = if small {
        (PLAY_MARK_SMALL, PLAY_GLYPH_SMALL, SPACE_STACK)
    } else {
        (PLAY_MARK, PLAY_GLYPH, SPACE_TIGHT)
    };
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .child(img(shot.image).size_full())
        .when(video, |frame| {
            frame.child(
                div()
                    .absolute()
                    .right(px(inset))
                    .bottom(px(inset))
                    .size(px(mark))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(kit().grounds.pane.bg))
                    .child(play(glyph)),
            )
        })
        .into_any_element()
}

pub(super) fn square(shot: Shot, size: f32) -> AnyElement {
    div()
        .flex_none()
        .size(px(size))
        .child(img(shot.image).size_full().object_fit(ObjectFit::Cover))
        .into_any_element()
}

fn fit((width, height): (f32, f32), (room_width, room_height): (f32, f32)) -> (f32, f32) {
    let scale = (room_width / width).min(room_height / height).min(1.0);
    (
        (width * scale).round().max(1.0),
        (height * scale).round().max(1.0),
    )
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn key(path: &str, flavor: Flavor, seconds: u64) -> Key {
        Key {
            path: PathBuf::from(path),
            flavor,
            modified: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
        }
    }

    #[test]
    fn a_changed_file_drops_its_old_preview_and_keeps_the_others() {
        let mut shared = Shared::default();
        let old = key("/v/a.webm", Flavor::Normal, 1);
        let large = key("/v/a.webm", Flavor::Large, 1);
        let other = key("/v/b.webm", Flavor::Normal, 1);
        for known in [&old, &large, &other] {
            shared.slots.insert(known.clone(), Slot::Missing);
            shared.queue.push(known.clone());
        }
        forget_older(&mut shared, &key("/v/a.webm", Flavor::Normal, 2));
        assert!(!shared.slots.contains_key(&old));
        assert!(shared.slots.contains_key(&large));
        assert!(shared.slots.contains_key(&other));
        assert_eq!(shared.queue, [large, other]);
    }

    #[test]
    fn a_preview_fits_its_room_keeping_its_shape_and_never_grows() {
        let cases = [
            ((128.0, 72.0), (136.0, 77.0), (128.0, 72.0)),
            ((256.0, 144.0), (136.0, 77.0), (136.0, 77.0)),
            ((96.0, 128.0), (136.0, 77.0), (58.0, 77.0)),
            ((128.0, 72.0), (104.0, 37.0), (66.0, 37.0)),
            ((16.0, 16.0), (136.0, 77.0), (16.0, 16.0)),
            ((4000.0, 1.0), (104.0, 37.0), (104.0, 1.0)),
        ];
        for (image, room, expected) in cases {
            assert_eq!(fit(image, room), expected, "{image:?} in {room:?}");
        }
    }
}
