//! Single-owner GPU atlas lifecycle for `Arc<RenderImage>`.
//!
//! See ADR `docs/adr/ALTTAB-2-...md`. `MetalAtlas::remove` double-decrements its
//! texture refcount when called twice for the same key, so every cache that
//! holds an `Arc<RenderImage>` must route insertion/removal through this
//! registry. The registry guarantees `App::drop_image` runs exactly once per
//! `ImageId` when its last owner releases.

use gpui::{App, RenderImage, Window};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Process-wide single-owner registry. Every cache that holds an
/// `Arc<RenderImage>` (PickerState, SharedPreviewCache, SharedIconCache) routes
/// inserts through `REGISTRY.retain` and removals through `REGISTRY.release`.
/// `App::drop_image` runs exactly once per `ImageId` when its last owner releases.
pub static REGISTRY: LazyLock<ImageRegistry> = LazyLock::new(ImageRegistry::new);

#[derive(Default)]
struct Inner {
    refs: HashMap<gpui::ImageId, usize>,
}

#[derive(Clone, Default)]
pub struct ImageRegistry {
    inner: Arc<Mutex<Inner>>,
}

impl ImageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that one more owner now holds this image. Cheap; safe to call
    /// from any thread (including `BackgroundExecutor` paths).
    pub fn retain(&self, image: &Arc<RenderImage>) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        *inner.refs.entry(image.id).or_insert(0) += 1;
        #[cfg(debug_assertions)]
        {
            let outstanding = inner.refs.len();
            if outstanding > 256 {
                eprintln!(
                    "[image-registry] outstanding={} (Proposal C regression signal)",
                    outstanding
                );
            }
        }
    }

    /// Decrement the owner count. If this was the last owner, take ownership
    /// of the `Arc` and call `App::drop_image` exactly once. Must run on the
    /// foreground (`&mut App`) because `drop_image` walks platform windows.
    ///
    /// `current_window` MUST be `Some(window)` when called from inside a
    /// `WindowHandle::update` lease. The leased window is `None` in
    /// `App::windows` for the duration of the lease, so passing it explicitly
    /// is the only way for `drop_image` to reach its atlas. Pass `None` only
    /// from App-level contexts where no window is leased.
    pub fn release(
        &self,
        image: Arc<RenderImage>,
        app: &mut App,
        current_window: Option<&mut Window>,
    ) {
        let should_drop = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            let Some(count) = inner.refs.get_mut(&image.id) else {
                #[cfg(debug_assertions)]
                eprintln!("[image-registry] release without retain: id={:?}", image.id);
                return;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.refs.remove(&image.id);
                true
            } else {
                false
            }
        };
        if should_drop {
            app.drop_image(image, current_window);
        }
    }
}

pub fn replace_into<K, S>(
    cache: &mut HashMap<K, Arc<RenderImage>, S>,
    key: K,
    image: Arc<RenderImage>,
    app: &mut App,
    window: Option<&mut Window>,
) where
    K: std::hash::Hash + Eq,
    S: std::hash::BuildHasher,
{
    REGISTRY.retain(&image);
    if let Some(old) = cache.insert(key, image) {
        REGISTRY.release(old, app, window);
    }
}

pub fn retain_or_release<K, S>(
    cache: &mut HashMap<K, Arc<RenderImage>, S>,
    app: &mut App,
    mut window: Option<&mut Window>,
    mut keep: impl FnMut(&K) -> bool,
) where
    K: std::hash::Hash + Eq + Clone,
    S: std::hash::BuildHasher,
{
    let to_remove: Vec<K> = cache.keys().filter(|k| !keep(k)).cloned().collect();
    for key in to_remove {
        if let Some(arc) = cache.remove(&key) {
            REGISTRY.release(arc, app, window.as_deref_mut());
        }
    }
}

pub fn replace_map<K, S>(
    cache: &mut HashMap<K, Arc<RenderImage>, S>,
    new: HashMap<K, Arc<RenderImage>, S>,
    app: &mut App,
    mut window: Option<&mut Window>,
) where
    K: std::hash::Hash + Eq,
    S: std::hash::BuildHasher + Default,
{
    for (_, old) in cache.drain() {
        REGISTRY.release(old, app, window.as_deref_mut());
    }
    for (k, v) in new {
        REGISTRY.retain(&v);
        cache.insert(k, v);
    }
}

pub fn extend_with<K, S>(
    cache: &mut HashMap<K, Arc<RenderImage>, S>,
    new: HashMap<K, Arc<RenderImage>, S>,
    app: &mut App,
    mut window: Option<&mut Window>,
) where
    K: std::hash::Hash + Eq,
    S: std::hash::BuildHasher,
{
    for (k, v) in new {
        replace_into(cache, k, v, app, window.as_deref_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Frame;

    fn img() -> Arc<RenderImage> {
        let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_pixel(
            1,
            1,
            image::Rgba([0, 0, 0, 0]),
        );
        Arc::new(RenderImage::new(smallvec::smallvec![Frame::new(buf)]))
    }

    #[test]
    fn retain_increments_count() {
        let r = ImageRegistry::new();
        let a = img();
        r.retain(&a);
        assert_eq!(r.inner.lock().unwrap().refs[&a.id], 1);
    }

    #[test]
    fn second_retain_increments_again() {
        let r = ImageRegistry::new();
        let a = img();
        r.retain(&a);
        r.retain(&a);
        assert_eq!(r.inner.lock().unwrap().refs[&a.id], 2);
    }

    #[test]
    fn distinct_images_tracked_separately() {
        let r = ImageRegistry::new();
        let a = img();
        let b = img();
        r.retain(&a);
        r.retain(&b);
        assert_eq!(r.inner.lock().unwrap().refs.len(), 2);
    }
}
