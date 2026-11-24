//! Fundo com o trailer do card focado: um `TrailerPlayer` por vez, frames RGBA chegando por
//! canal e virando `RenderImage` na thread de UI.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use bigpicture_media::{RgbaFrame, TrailerPlayer};
use gpui_kit::*;
use image::{Frame, RgbaImage};

/// Resolução em que o trailer é decodificado; a GPU escala para a tela toda.
pub const TRAILER_WIDTH: u32 = 1280;
pub const TRAILER_HEIGHT: u32 = 720;

/// Frame de um trailer, marcado com a geração do player que o produziu.
pub type TrailerFrame = (u64, RgbaFrame);

pub struct TrailerBackdrop {
    player: Option<TrailerPlayer>,
    image: Option<Arc<RenderImage>>,
    /// Muda a cada troca de trailer; frames de players antigos ainda na fila são descartados.
    generation: u64,
    frames_tx: SyncSender<TrailerFrame>,
}

impl TrailerBackdrop {
    pub fn new() -> (Self, Receiver<TrailerFrame>) {
        // Só dois frames em voo: se a UI atrasar, o player descarta em vez de acumular.
        let (frames_tx, frames_rx) = sync_channel(2);
        let backdrop = Self {
            player: None,
            image: None,
            generation: 0,
            frames_tx,
        };
        (backdrop, frames_rx)
    }

    /// Toca `path` em loop (ou para, com `None`). Não reinicia se já estiver tocando o mesmo.
    pub fn play(&mut self, path: Option<&Path>) {
        if self.player.as_ref().map(TrailerPlayer::path) == path {
            return;
        }
        self.generation += 1;
        self.player = None;
        let Some(path) = path else { return };
        let generation = self.generation;
        let tx = self.frames_tx.clone();
        match TrailerPlayer::start(path, TRAILER_WIDTH, TRAILER_HEIGHT, move |frame| {
            let _ = tx.try_send((generation, frame));
        }) {
            Ok(player) => self.player = Some(player),
            Err(err) => tracing::warn!(path = %path.display(), error = %err, "trailer indisponível"),
        }
    }

    /// Recebe um frame. Retorna `true` quando a imagem de fundo mudou e a view deve redesenhar.
    pub fn accept(&mut self, (generation, frame): TrailerFrame, window: &mut Window) -> bool {
        if generation != self.generation {
            return false;
        }
        let (width, height) = (frame.width(), frame.height());
        let Some(buffer) = RgbaImage::from_raw(width, height, frame.into_bgra_bytes()) else {
            return false;
        };
        let image = Arc::new(RenderImage::new([Frame::new(buffer)]));
        if let Some(previous) = self.image.replace(image) {
            let _ = window.drop_image(previous);
        }
        true
    }

    pub fn image(&self) -> Option<Arc<RenderImage>> {
        self.image.clone()
    }
}
