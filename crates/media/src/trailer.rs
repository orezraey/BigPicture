//! Trailer WebM tocado em loop via GStreamer, entregando frames RGBA fora da thread de UI.
//!
//! Pipeline: `filesrc ! decodebin ! videoconvert ! videoscale ! appsink (RGBA WxH)`.
//! O áudio é descartado num `fakesink` (trailer de fundo é mudo). No fim do arquivo, uma
//! thread de barramento volta ao início com um seek, o que dá o loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::RgbaFrame;

/// Inicializa o GStreamer uma vez. Falha quando o runtime não está instalado; o app segue sem
/// trailers nesse caso.
pub fn init_gstreamer() -> Result<()> {
    gst::init().context("GStreamer indisponível (runtime não instalado?)")
}

/// Um trailer em reprodução. Soltar o valor para o pipeline e encerra a thread do barramento.
pub struct TrailerPlayer {
    path: PathBuf,
    pipeline: gst::Pipeline,
    stop: Arc<AtomicBool>,
    bus_thread: Option<JoinHandle<()>>,
}

impl TrailerPlayer {
    /// Começa a tocar `path` em loop, escalado para `width`×`height`. `on_frame` é chamado na
    /// thread de streaming do GStreamer a cada frame decodificado; deve só repassar o frame.
    pub fn start(
        path: &Path,
        width: u32,
        height: u32,
        on_frame: impl Fn(RgbaFrame) + Send + Sync + 'static,
    ) -> Result<Self> {
        init_gstreamer()?;
        let location = path
            .to_str()
            .ok_or_else(|| anyhow!("caminho de trailer não é UTF-8: {}", path.display()))?;

        let pipeline = gst::Pipeline::default();
        let src = gst::ElementFactory::make("filesrc")
            .property("location", location)
            .build()?;
        let decode = gst::ElementFactory::make("decodebin").build()?;
        let convert = gst::ElementFactory::make("videoconvert").build()?;
        let scale = gst::ElementFactory::make("videoscale").build()?;
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", i32::try_from(width)?)
            .field("height", i32::try_from(height)?)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build();
        // `sync` mantém a velocidade real do vídeo; poucos buffers e `drop` evitam acumular
        // frames atrasados se a UI demorar.
        let sink = gst_app::AppSink::builder()
            .caps(&caps)
            .max_buffers(2)
            .drop(true)
            .sync(true)
            .build();

        pipeline.add_many([&src, &decode, &convert, &scale, sink.upcast_ref()])?;
        src.link(&decode)?;
        gst::Element::link_many([&convert, &scale, sink.upcast_ref()])?;

        // O vídeo vai para o `videoconvert`; qualquer outro fluxo (áudio) vai para um `fakesink`,
        // para o demuxer não parar com "not-linked". (Restringir `decodebin` com `caps` impede o
        // autoplug do decodificador VP8.)
        let convert_weak = convert.downgrade();
        let pipeline_weak = pipeline.downgrade();
        decode.connect_pad_added(move |_, pad| {
            let (Some(convert), Some(pipeline)) = (convert_weak.upgrade(), pipeline_weak.upgrade())
            else {
                return;
            };
            let is_video = pad
                .current_caps()
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
                .unwrap_or(false);
            let linked = if is_video {
                convert
                    .static_pad("sink")
                    .filter(|sink| !sink.is_linked())
                    .is_some_and(|sink| pad.link(&sink).is_ok())
            } else {
                discard_pad(&pipeline, pad)
            };
            if !linked {
                tracing::warn!(video = is_video, "fluxo do trailer não pôde ser ligado");
            }
        });

        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(frame) = sample_to_frame(&sample) {
                        on_frame(frame);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        pipeline
            .set_state(gst::State::Playing)
            .with_context(|| format!("trailer não pôde tocar: {}", path.display()))?;

        let stop = Arc::new(AtomicBool::new(false));
        let bus = pipeline.bus().context("pipeline sem barramento")?;
        let bus_thread = {
            let stop = Arc::clone(&stop);
            let pipeline = pipeline.downgrade();
            let path = path.to_path_buf();
            thread::Builder::new()
                .name("bp-trailer-bus".into())
                .spawn(move || bus_loop(&bus, &pipeline, &stop, &path))?
        };

        Ok(Self { path: path.to_path_buf(), pipeline, stop, bus_thread: Some(bus_thread) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TrailerPlayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(handle) = self.bus_thread.take() {
            let _ = handle.join();
        }
    }
}

fn bus_loop(bus: &gst::Bus, pipeline: &gst::glib::WeakRef<gst::Pipeline>, stop: &AtomicBool, path: &Path) {
    while !stop.load(Ordering::Relaxed) {
        let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) else { continue };
        match msg.view() {
            gst::MessageView::Eos(..) => {
                let Some(pipeline) = pipeline.upgrade() else { return };
                if pipeline
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, gst::ClockTime::ZERO)
                    .is_err()
                {
                    tracing::warn!(path = %path.display(), "trailer não voltou ao início");
                    return;
                }
            }
            gst::MessageView::Error(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err.error(),
                    debug = ?err.debug(),
                    "erro ao tocar trailer"
                );
                return;
            }
            _ => {}
        }
    }
}

fn discard_pad(pipeline: &gst::Pipeline, pad: &gst::Pad) -> bool {
    let Ok(sink) = gst::ElementFactory::make("fakesink").property("sync", false).build() else {
        return false;
    };
    if pipeline.add(&sink).is_err() || sink.sync_state_with_parent().is_err() {
        return false;
    }
    sink.static_pad("sink").is_some_and(|sink_pad| pad.link(&sink_pad).is_ok())
}

fn sample_to_frame(sample: &gst::Sample) -> Option<RgbaFrame> {
    let s = sample.caps()?.structure(0)?;
    let width = u32::try_from(s.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(s.get::<i32>("height").ok()?).ok()?;
    let map = sample.buffer()?.map_readable().ok()?;
    let row = width as usize * 4;
    let data = map.as_slice();
    // O RGBA do videoconvert tem linhas de exatamente width*4 bytes, mas aceita stride maior.
    let stride = data.len().checked_div(height as usize)?;
    if stride < row {
        return None;
    }
    let pixels = if stride == row {
        data[..row * height as usize].to_vec()
    } else {
        data.chunks(stride).take(height as usize).flat_map(|line| &line[..row]).copied().collect()
    };
    RgbaFrame::new(width, height, pixels)
}

/// Espera até `timeout` pelo primeiro frame de um trailer; útil em testes e diagnóstico.
pub fn first_frame(path: &Path, width: u32, height: u32, timeout: Duration) -> Result<RgbaFrame> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let _player = TrailerPlayer::start(path, width, height, move |frame| {
        let _ = tx.try_send(frame);
    })?;
    rx.recv_timeout(timeout)
        .with_context(|| format!("nenhum frame em {timeout:?}: {}", path.display()))
}
