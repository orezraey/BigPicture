use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui_kit::*;

/// Bounded LRU cache of GPU `RenderImage` handles.
///
/// Keeping ~180 600x900 covers resident (~388 MB raw BGRA) prevents the GPUI
/// sprite atlas from overflowing during full-catalog 1,000-card scrolling while
/// ensuring zero texture re-upload jitter for visible and recently visited rows.
pub struct CoverTextureLru {
    capacity: usize,
    map: HashMap<String, Arc<RenderImage>>,
    order: VecDeque<String>,
}

impl CoverTextureLru {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(16);
        Self {
            capacity: cap,
            map: HashMap::with_capacity(cap),
            order: VecDeque::with_capacity(cap),
        }
    }

    pub fn get(&mut self, id: &str) -> Option<Arc<RenderImage>> {
        let img = self.map.get(id)?.clone();
        if self.order.back().map(String::as_str) != Some(id) {
            self.order.retain(|k| k != id);
            self.order.push_back(id.to_string());
        }
        Some(img)
    }

    /// Esvazia o cache e devolve as imagens para o chamador liberar com `window.drop_image(...)`.
    pub fn clear(&mut self) -> Vec<Arc<RenderImage>> {
        self.order.clear();
        self.map.drain().map(|(_, image)| image).collect()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Inserts `image` and returns any evicted `RenderImage` handles that the caller
    /// should release from GPUI's `PlatformAtlas` via `window.drop_image(...)`.
    pub fn insert(&mut self, id: String, image: Arc<RenderImage>) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        if let Some(prev) = self.map.insert(id.clone(), image) {
            evicted.push(prev);
            self.order.retain(|k| k != &id);
        }
        self.order.push_back(id);

        while self.map.len() > self.capacity {
            if let Some(oldest_key) = self.order.pop_front()
                && let Some(old_img) = self.map.remove(&oldest_key)
            {
                evicted.push(old_img);
            }
        }
        evicted
    }
}
