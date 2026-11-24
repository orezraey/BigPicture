use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use anyhow::{Context as _, Result, bail};
use image::imageops::FilterType;
use image::{ImageFormat, RgbaImage};

use crate::{COVER_HEIGHT, COVER_WIDTH, RgbaFrame};

/// Whether a cover request was satisfied from the on-disk cache or decoded fresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheOutcome {
    Hit,
    Miss,
}

/// Disk-backed cache storing resized 600x900 cover images in QOI format.
#[derive(Clone, Debug)]
pub struct CoverCache {
    cache_dir: PathBuf,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

impl CoverCache {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let cache_dir = cache_dir.into();
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("failed to create cover cache dir {}", cache_dir.display()))?;
        Ok(Self { cache_dir })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Computes the deterministic cache file path for `source_path` taking its size and mtime
    /// into account so edits on disk invalidate stale cached covers automatically.
    pub fn cache_path_for(&self, source_path: &Path) -> Result<PathBuf> {
        let meta = fs::metadata(source_path)
            .with_context(|| format!("failed to stat cover source {}", source_path.display()))?;
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        let key = fnv1a_128_key(source_path, meta.len(), modified_ns);
        Ok(self.cache_dir.join(format!("{key:032x}_{COVER_WIDTH}x{COVER_HEIGHT}.qoi")))
    }

    /// Loads the 600x900 cover from disk cache if present, or decodes + resizes `source_path`
    /// to 600x900 and atomically persists it into `cache_dir`.
    pub fn load_or_decode(&self, source_path: &Path) -> Result<(RgbaFrame, CacheOutcome)> {
        let cached_file = self.cache_path_for(source_path)?;

        if cached_file.is_file() {
            match decode_cached_qoi(&cached_file) {
                Ok(frame) => return Ok((frame, CacheOutcome::Hit)),
                Err(err) => {
                    tracing::warn!(
                        cache_file = %cached_file.display(),
                        error = %err,
                        "corrupt cached cover detected; re-decoding from source"
                    );
                }
            }
        }

        // Pelo conteúdo, não pela extensão: a Steam grava PNG com nome `library_600x900.jpg`.
        let dyn_img = image::ImageReader::open(source_path)
            .and_then(|reader| reader.with_guessed_format())
            .with_context(|| format!("failed to open source cover {}", source_path.display()))?
            .decode()
            .with_context(|| format!("failed to decode source cover {}", source_path.display()))?;
        let resized = if dyn_img.width() == COVER_WIDTH && dyn_img.height() == COVER_HEIGHT {
            dyn_img.into_rgba8()
        } else {
            dyn_img
                .resize_to_fill(COVER_WIDTH, COVER_HEIGHT, FilterType::Triangle)
                .into_rgba8()
        };

        write_atomic_qoi(&cached_file, &resized)?;

        let frame = RgbaFrame::new(COVER_WIDTH, COVER_HEIGHT, resized.into_raw())
            .context("resized cover buffer size mismatch")?;
        Ok((frame, CacheOutcome::Miss))
    }
}

fn decode_cached_qoi(path: &Path) -> Result<RgbaFrame> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed reading cached cover {}", path.display()))?;
    let rgba = image::load_from_memory_with_format(&bytes, ImageFormat::Qoi)
        .context("invalid QOI data in cover cache")?
        .into_rgba8();
    if rgba.width() != COVER_WIDTH || rgba.height() != COVER_HEIGHT {
        bail!(
            "unexpected cached dimensions {}x{} (expected {}x{})",
            rgba.width(),
            rgba.height(),
            COVER_WIDTH,
            COVER_HEIGHT
        );
    }
    RgbaFrame::new(COVER_WIDTH, COVER_HEIGHT, rgba.into_raw())
        .context("unexpected RGBA byte count")
}

fn write_atomic_qoi(dest: &Path, img: &RgbaImage) -> Result<()> {
    let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tmp = dest.with_extension(format!("tmp.{pid}.{seq}.qoi"));
    img.save_with_format(&tmp, ImageFormat::Qoi)
        .with_context(|| format!("failed encoding cached QOI {}", tmp.display()))?;
    if let Err(err) = fs::rename(&tmp, dest) {
        // On Windows, if another worker raced to write the exact same file, clean up tmp.
        let _ = fs::remove_file(&tmp);
        if !dest.is_file() {
            return Err(err)
                .with_context(|| format!("failed renaming cached cover to {}", dest.display()));
        }
    }
    Ok(())
}

fn fnv1a_128_key(path: &Path, len: u64, modified_ns: u128) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;

    let mut hash = OFFSET;
    for &b in path.to_string_lossy().as_bytes() {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    for b in len.to_le_bytes() {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    for b in modified_ns.to_le_bytes() {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
