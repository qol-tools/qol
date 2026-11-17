use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use gpui::{
    hash, App, AppContext, Asset, AssetLogger, Entity, ImageAssetLoader, ImageCache,
    ImageCacheError, ImageCacheItem, RenderImage, Resource, Window,
};

const KEPT: usize = 1024;

pub(super) struct PreviewImages {
    items: HashMap<u64, ImageCacheItem>,
    recency: Recency,
}

impl PreviewImages {
    pub(super) fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            items: HashMap::new(),
            recency: Recency::new(KEPT),
        })
    }
}

impl ImageCache for PreviewImages {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);
        for stale in self.recency.touch(key) {
            if let Some(mut item) = self.items.remove(&stale) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, Some(&mut *window));
                }
            }
        }
        if let Some(item) = self.items.get_mut(&key) {
            return item.get();
        }
        let task = cx
            .background_executor()
            .spawn(AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx))
            .shared();
        self.items
            .insert(key, ImageCacheItem::Loading(task.clone()));
        let view = window.current_view();
        window
            .spawn(cx, async move |cx| {
                let _ = task.await;
                cx.on_next_frame(move |_, cx| cx.notify(view));
            })
            .detach();
        None
    }
}

struct Recency {
    kept: usize,
    clock: u64,
    used: HashMap<u64, u64>,
}

impl Recency {
    fn new(kept: usize) -> Self {
        Self {
            kept,
            clock: 0,
            used: HashMap::new(),
        }
    }

    fn touch(&mut self, key: u64) -> Vec<u64> {
        self.clock += 1;
        self.used.insert(key, self.clock);
        let mut dropped = Vec::new();
        while self.used.len() > self.kept {
            let Some(oldest) = self
                .used
                .iter()
                .min_by_key(|(_, used)| **used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.used.remove(&oldest);
            dropped.push(oldest);
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::Recency;

    #[test]
    fn previews_beyond_the_limit_drop_the_least_recently_shown() {
        let mut recency = Recency::new(3);
        assert!(recency.touch(1).is_empty());
        assert!(recency.touch(2).is_empty());
        assert!(recency.touch(3).is_empty());
        assert!(recency.touch(1).is_empty());
        assert_eq!(recency.touch(4), [2]);
        assert_eq!(recency.touch(5), [3]);
        assert!(recency.touch(1).is_empty());
        assert_eq!(recency.touch(6), [4]);
    }

    #[test]
    fn showing_a_kept_preview_again_drops_nothing() {
        let mut recency = Recency::new(2);
        recency.touch(1);
        recency.touch(2);
        for _ in 0..10 {
            assert!(recency.touch(1).is_empty());
            assert!(recency.touch(2).is_empty());
        }
    }
}
