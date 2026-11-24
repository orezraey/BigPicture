//! Asynchronous media decoding, disk cover cache, and video playback for BigPicture.

pub mod cover_cache;
pub mod cover_loader;
pub mod demo_covers;
pub mod demo_trailers;
pub mod trailer;

pub use cover_cache::{CacheOutcome, CoverCache};
pub use cover_loader::{CoverLoaded, CoverLoader};
pub use demo_covers::{ensure_demo_covers, ensure_telegram_cover, list_image_files};
pub use demo_trailers::{ensure_demo_trailers, list_trailer_files};
pub use trailer::{TrailerPlayer, init_gstreamer};

/// Target dimensions for game cover artwork in the launcher grid.
pub const COVER_WIDTH: u32 = 600;
pub const COVER_HEIGHT: u32 = 900;

/// Decoded RGBA8 image frame ready for upload to the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaFrame {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if pixels.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Consumes the RGBA8 frame and converts channels in-place to BGRA8
    /// (the pixel layout expected by GPUI's `RenderImage`).
    pub fn into_bgra_bytes(mut self) -> Vec<u8> {
        for px in self.pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        self.pixels
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bigpicture_media_test_{}_{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validates_rgba_and_converts_to_bgra() {
        let frame = RgbaFrame::new(1, 2, vec![10, 20, 30, 255, 1, 2, 3, 128]).unwrap();
        assert_eq!(frame.into_bgra_bytes(), vec![30, 20, 10, 255, 3, 2, 1, 128]);
        assert!(RgbaFrame::new(2, 2, vec![0; 15]).is_none());
    }

    #[test]
    fn decodes_cover_by_content_even_with_wrong_extension() {
        let root = temp_test_dir("wrong_ext");
        let src = root.join("library_600x900.jpg");
        image::RgbaImage::from_pixel(30, 45, image::Rgba([1, 2, 3, 255]))
            .save_with_format(&src, image::ImageFormat::Png)
            .unwrap();
        let cache = CoverCache::new(root.join("cache")).unwrap();
        let (frame, outcome) = cache.load_or_decode(&src).unwrap();
        assert_eq!((frame.width(), frame.height(), outcome), (600, 900, CacheOutcome::Miss));
    }

    #[test]
    fn cover_cache_resizes_to_600x900_and_serves_disk_hits() {
        let root = temp_test_dir("cache_hit_miss");
        let src_dir = root.join("src");
        let cache_dir = root.join("cache");

        let paths = ensure_demo_covers(&src_dir, 2).unwrap();
        assert_eq!(paths.len(), 2);

        let cache = CoverCache::new(&cache_dir).unwrap();
        let (first_frame, first_outcome) = cache.load_or_decode(&paths[0]).unwrap();
        assert_eq!(first_outcome, CacheOutcome::Miss);
        assert_eq!(first_frame.width(), COVER_WIDTH);
        assert_eq!(first_frame.height(), COVER_HEIGHT);

        let (second_frame, second_outcome) = cache.load_or_decode(&paths[0]).unwrap();
        assert_eq!(second_outcome, CacheOutcome::Hit);
        assert_eq!(first_frame, second_frame);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cover_loader_decodes_asynchronously_on_worker_threads() {
        let root = temp_test_dir("async_loader");
        let src_dir = root.join("src");
        let cache_dir = root.join("cache");

        let paths = ensure_demo_covers(&src_dir, 4).unwrap();
        let cache = CoverCache::new(&cache_dir).unwrap();
        let (loader, rx) = CoverLoader::new(cache, 2);

        for (ix, path) in paths.iter().enumerate() {
            loader.request(format!("card-{ix}"), path);
        }

        let mut received = Vec::new();
        for _ in 0..4 {
            let msg = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(msg.frame.width(), COVER_WIDTH);
            assert_eq!(msg.frame.height(), COVER_HEIGHT);
            received.push(msg.id);
        }
        received.sort();
        assert_eq!(received, vec!["card-0", "card-1", "card-2", "card-3"]);

        drop(loader);
        let _ = fs::remove_dir_all(&root);
    }
}
