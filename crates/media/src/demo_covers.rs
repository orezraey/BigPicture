use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context as _, Result};
use image::{ImageFormat, RgbaImage};

const SUPPORTED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "qoi"];

/// Ensures `dir` has at least `target_count` local cover images on disk, generating
/// stylized procedural artwork for any missing slots so 1,000-card spike runs work
/// out of the box while exercising real file I/O, resize-to-600x900, and disk caching.
pub fn ensure_demo_covers(dir: &Path, target_count: usize) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create covers directory {}", dir.display()))?;

    let mut existing = list_image_files(dir)?;
    if existing.len() >= target_count {
        existing.truncate(target_count);
        return Ok(existing);
    }

    let missing_indices: Vec<usize> = (existing.len()..target_count).collect();
    let worker_count = thread::available_parallelism()
        .map(|n| n.get().min(16))
        .unwrap_or(4);
    let chunk_size = missing_indices.len().div_ceil(worker_count).max(1);

    tracing::info!(
        dir = %dir.display(),
        existing = existing.len(),
        generating = missing_indices.len(),
        "generating local cover images for 1000-card spike catalog"
    );

    thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for chunk in missing_indices.chunks(chunk_size) {
            handles.push(scope.spawn(move || -> Result<()> {
                for &ix in chunk {
                    let path = dir.join(format!("cover_{ix:04}.qoi"));
                    if !path.is_file() {
                        let img = render_procedural_poster(ix as u32);
                        img.save_with_format(&path, ImageFormat::Qoi)
                            .with_context(|| format!("failed writing {}", path.display()))?;
                    }
                }
                Ok(())
            }));
        }
        for h in handles {
            h.join().expect("cover generator thread panicked")?;
        }
        Ok(())
    })?;

    let mut all = list_image_files(dir)?;
    all.truncate(target_count);
    Ok(all)
}

/// Gera (uma vez) a capa do card do Telegram: degradê azul com um círculo branco e um avião
/// de papel, no tamanho final 600×900. Fica fora da pasta de capas demo para não entrar no ciclo
/// das 1.000 capas.
pub fn ensure_telegram_cover(dest: &Path) -> Result<PathBuf> {
    if !dest.is_file() {
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).with_context(|| format!("criando {}", dir.display()))?;
        }
        render_telegram_cover()
            .save_with_format(dest, ImageFormat::Png)
            .with_context(|| format!("gravando {}", dest.display()))?;
    }
    Ok(dest.to_path_buf())
}

fn render_telegram_cover() -> RgbaImage {
    const W: u32 = 600;
    const H: u32 = 900;
    let top = [0x37u8, 0xae, 0xe2];
    let bottom = [0x1e, 0x6f, 0xa8];
    let (cx, cy, radius) = (300.0f32, 420.0f32, 170.0f32);
    // Avião de papel: triângulo apontando para a direita, dentro do círculo.
    let tri = [(205.0f32, 415.0f32), (395.0, 340.0), (350.0, 505.0)];
    let fold = [(395.0f32, 340.0f32), (285.0, 450.0), (300.0, 500.0)];
    let inside = |p: (f32, f32), t: [(f32, f32); 3]| {
        let sign = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| (a.0 - c.0) * (b.1 - c.1) - (b.0 - c.0) * (a.1 - c.1);
        let (d1, d2, d3) = (sign(p, t[0], t[1]), sign(p, t[1], t[2]), sign(p, t[2], t[0]));
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    };
    let mut img = RgbaImage::new(W, H);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let t = y as f32 / (H - 1) as f32;
        let mut rgb = [lerp_u8(top[0], bottom[0], t), lerp_u8(top[1], bottom[1], t), lerp_u8(top[2], bottom[2], t)];
        let p = (x as f32 + 0.5, y as f32 + 0.5);
        let dist = ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt();
        if dist <= radius {
            rgb = [0xff, 0xff, 0xff];
            if inside(p, tri) {
                rgb = [0x2a, 0xab, 0xee];
            }
            if inside(p, fold) {
                rgb = [0x1c, 0x8c, 0xc9];
            }
        }
        *px = image::Rgba([rgb[0], rgb[1], rgb[2], 255]);
    }
    img
}

/// Lists all supported image files in `dir` in deterministic lexicographical order.
pub fn list_image_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read covers directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && let Some(ext) = path.extension().and_then(|e| e.to_str())
            && SUPPORTED_EXTENSIONS.iter().any(|s| s.eq_ignore_ascii_case(ext))
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Generates a 300x450 procedural poster (2:3 aspect, half target size so `CoverCache`
/// genuinely performs off-thread decoding + 600x900 scaling + disk caching).
fn render_procedural_poster(seed: u32) -> RgbaImage {
    const W: u32 = 300;
    const H: u32 = 450;

    let hue_a = ((seed.wrapping_mul(47)) % 360) as f32;
    let hue_b = ((hue_a as u32 + 55 + (seed % 90)) % 360) as f32;
    let accent_hue = ((hue_a as u32 + 190) % 360) as f32;

    let top_rgb = hsl_to_rgb(hue_a, 0.72, 0.24);
    let bottom_rgb = hsl_to_rgb(hue_b, 0.80, 0.10);
    let accent_rgb = hsl_to_rgb(accent_hue, 0.90, 0.58);

    let cx = 70.0 + ((seed * 37) % 160) as f32;
    let cy = 95.0 + ((seed * 53) % 180) as f32;
    let ring_r = 46.0 + ((seed * 19) % 48) as f32;
    let stripe_offset = ((seed * 29) % 180) as i32;

    let mut raw = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        let ty = y as f32 / (H - 1) as f32;
        let base_r = lerp_u8(top_rgb[0], bottom_rgb[0], ty);
        let base_g = lerp_u8(top_rgb[1], bottom_rgb[1], ty);
        let base_b = lerp_u8(top_rgb[2], bottom_rgb[2], ty);

        for x in 0..W {
            let mut r = base_r;
            let mut g = base_g;
            let mut b = base_b;

            // Subtle diagonal geometric band
            let diag = (x as i32 + y as i32 / 2 - stripe_offset).rem_euclid(140);
            if diag < 18 {
                r = blend_u8(r, accent_rgb[0], 0.24);
                g = blend_u8(g, accent_rgb[1], 0.24);
                b = blend_u8(b, accent_rgb[2], 0.24);
            }

            // Emblem ring
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if (dist - ring_r).abs() < 5.5 {
                r = blend_u8(r, accent_rgb[0], 0.82);
                g = blend_u8(g, accent_rgb[1], 0.82);
                b = blend_u8(b, accent_rgb[2], 0.82);
            } else if dist < ring_r * 0.48 {
                r = blend_u8(r, accent_rgb[0], 0.30);
                g = blend_u8(g, accent_rgb[1], 0.30);
                b = blend_u8(b, accent_rgb[2], 0.30);
            }

            // Bottom vignette for title legibility
            if ty > 0.72 {
                let dark = ((ty - 0.72) / 0.28).clamp(0.0, 1.0) * 0.68;
                r = blend_u8(r, 8, dark);
                g = blend_u8(g, 10, dark);
                b = blend_u8(b, 16, dark);
            }

            let idx = ((y * W + x) * 4) as usize;
            raw[idx] = r;
            raw[idx + 1] = g;
            raw[idx + 2] = b;
            raw[idx + 3] = 255;
        }
    }

    RgbaImage::from_raw(W, H, raw).expect("valid 300x450 RGBA buffer")
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

fn blend_u8(base: u8, top: u8, alpha: f32) -> u8 {
    lerp_u8(base, top, alpha.clamp(0.0, 1.0))
}

fn hsl_to_rgb(h_deg: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = (h_deg / 60.0) % 6.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c * 0.5;
    [
        ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}
