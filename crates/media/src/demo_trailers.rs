//! Trailers WebM de demonstração gerados com o próprio GStreamer (`videotestsrc` → VP8 →
//! WebM), para o spike rodar sem baixar vídeos de jogos.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gstreamer as gst;
use gstreamer::prelude::*;

use crate::trailer::init_gstreamer;

/// Padrões animados do `videotestsrc`, um por trailer demo.
const PATTERNS: [&str; 4] = ["ball", "pinwheel", "spokes", "smpte"];
const DEMO_WIDTH: i32 = 1280;
const DEMO_HEIGHT: i32 = 720;
const DEMO_FPS: i32 = 30;
const DEMO_SECONDS: u32 = 6;

/// Lista os `.webm` de `dir`, em ordem.
pub fn list_trailer_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir).with_context(|| format!("lendo {}", dir.display()))? {
            let path = entry?.path();
            if path.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("webm"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Garante ao menos `count` trailers em `dir`, gerando os que faltarem.
pub fn ensure_demo_trailers(dir: &Path, count: usize) -> Result<Vec<PathBuf>> {
    let existing = list_trailer_files(dir)?;
    if existing.len() >= count {
        return Ok(existing);
    }
    fs::create_dir_all(dir).with_context(|| format!("criando {}", dir.display()))?;
    init_gstreamer()?;
    for ix in 0..count {
        let dest = dir.join(format!("demo_trailer_{ix:02}.webm"));
        if dest.is_file() {
            continue;
        }
        let pattern = PATTERNS[ix % PATTERNS.len()];
        encode_demo(&dest, pattern)
            .with_context(|| format!("gerando trailer demo {}", dest.display()))?;
        tracing::info!(path = %dest.display(), pattern, "trailer demo gerado");
    }
    list_trailer_files(dir)
}

fn encode_demo(dest: &Path, pattern: &str) -> Result<()> {
    let tmp = dest.with_extension("tmp.webm");
    let pipeline = gst::Pipeline::default();
    let src = gst::ElementFactory::make("videotestsrc")
        .property("num-buffers", i32::try_from(DEMO_SECONDS)? * DEMO_FPS)
        .property_from_str("pattern", pattern)
        .build()?;
    if pattern == "smpte" {
        // Faz as barras andarem, para o loop ser visível.
        src.set_property("horizontal-speed", 4i32);
    }
    let caps = gst::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("width", DEMO_WIDTH)
                .field("height", DEMO_HEIGHT)
                .field("framerate", gst::Fraction::new(DEMO_FPS, 1))
                .build(),
        )
        .build()?;
    let convert = gst::ElementFactory::make("videoconvert").build()?;
    let enc = gst::ElementFactory::make("vp8enc")
        .property("deadline", 1i64)
        .property("cpu-used", 8i32)
        .build()?;
    let mux = gst::ElementFactory::make("webmmux").build()?;
    let sink = gst::ElementFactory::make("filesink")
        .property("location", tmp.to_str().context("caminho não UTF-8")?)
        .build()?;
    pipeline.add_many([&src, &caps, &convert, &enc, &mux, &sink])?;
    gst::Element::link_many([&src, &caps, &convert, &enc, &mux, &sink])?;

    pipeline.set_state(gst::State::Playing)?;
    let bus = pipeline.bus().context("pipeline sem barramento")?;
    let result = (|| {
        for msg in bus.iter_timed(gst::ClockTime::from_seconds(60)) {
            match msg.view() {
                gst::MessageView::Eos(..) => return Ok(()),
                gst::MessageView::Error(err) => bail!("{} ({:?})", err.error(), err.debug()),
                _ => {}
            }
        }
        bail!("tempo esgotado gerando o trailer")
    })();
    pipeline.set_state(gst::State::Null)?;
    result?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::trailer::first_frame;

    #[test]
    fn generates_demo_trailer_and_decodes_rgba_frames() {
        let dir = std::env::temp_dir().join(format!("bp_trailer_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let files = ensure_demo_trailers(&dir, 1).expect("gerar trailer demo");
        assert_eq!(files.len(), 1);

        let frame = first_frame(&files[0], 320, 180, Duration::from_secs(10)).expect("primeiro frame");
        assert_eq!((frame.width(), frame.height()), (320, 180));
        assert_eq!(frame.pixels().len(), 320 * 180 * 4);
        let _ = fs::remove_dir_all(&dir);
    }
}
