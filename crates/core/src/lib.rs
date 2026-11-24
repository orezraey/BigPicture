//! Core domain models and pure layout/navigation geometry for BigPicture.

use std::path::PathBuf;

pub mod library;
pub mod spatial;

pub use library::{Achievement, GameSource, progress};
pub use spatial::{Direction, navigate_rects};

/// Unique identifier for a game or launchable entry in the library.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId(String);

impl AppId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Metadata for a game or application card in the launcher grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    id: AppId,
    title: String,
    subtitle: Option<String>,
    cover_path: Option<PathBuf>,
    /// De onde baixar a capa quando `cover_path` ainda não existe no disco (CDN da loja).
    cover_url: Option<String>,
    /// Vídeo WebM tocado em loop no fundo quando o card está focado.
    trailer_path: Option<PathBuf>,
    executable: Option<PathBuf>,
    args: Vec<String>,
    /// Loja de origem (Steam, Epic) ou local.
    source: GameSource,
    /// Pasta de instalação: serve para achar as janelas do jogo depois de aberto pela loja.
    install_dir: Option<PathBuf>,
    /// Endereço que a loja usa para abrir o jogo (`steam://rungameid/…`), no lugar de
    /// chamar o executável direto, para a loja cuidar de atualização, DRM e overlay.
    launch_uri: Option<String>,
}

impl CatalogEntry {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: AppId::new(id),
            title: title.into(),
            subtitle: None,
            cover_path: None,
            cover_url: None,
            trailer_path: None,
            executable: None,
            args: Vec::new(),
            source: GameSource::Local,
            install_dir: None,
            launch_uri: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn with_cover_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.cover_path = Some(path.into());
        self
    }

    pub fn with_trailer_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.trailer_path = Some(path.into());
        self
    }

    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Capa que ainda pode não estar no disco: `path` é onde ela fica depois de baixada de `url`.
    pub fn with_remote_cover(mut self, path: impl Into<PathBuf>, url: impl Into<String>) -> Self {
        self.cover_path = Some(path.into());
        self.cover_url = Some(url.into());
        self
    }

    pub fn cover_url(&self) -> Option<&str> {
        self.cover_url.as_deref()
    }

    pub fn with_source(mut self, source: GameSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_install_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.install_dir = Some(dir.into());
        self
    }

    pub fn with_launch_uri(mut self, uri: impl Into<String>) -> Self {
        self.launch_uri = Some(uri.into());
        self
    }

    pub fn source(&self) -> &GameSource {
        &self.source
    }

    pub fn install_dir(&self) -> Option<&PathBuf> {
        self.install_dir.as_ref()
    }

    pub fn launch_uri(&self) -> Option<&str> {
        self.launch_uri.as_deref()
    }

    pub fn id(&self) -> &AppId {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    pub fn cover_path(&self) -> Option<&PathBuf> {
        self.cover_path.as_ref()
    }

    pub fn trailer_path(&self) -> Option<&PathBuf> {
        self.trailer_path.as_ref()
    }

    pub fn executable(&self) -> Option<&PathBuf> {
        self.executable.as_ref()
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }
}

/// Axis-aligned rectangle in logical UI coordinates, used for pure-Rust
/// grid layout and spatial navigation calculations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LogicalRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn center_x(&self) -> f32 {
        self.x + self.width * 0.5
    }

    pub fn center_y(&self) -> f32 {
        self.y + self.height * 0.5
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

/// Pure-Rust responsive cover-grid layout calculator (2:3 aspect covers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridMetrics {
    pub card_width: f32,
    pub cover_height: f32,
    pub footer_height: f32,
    pub col_gap: f32,
    pub row_gap: f32,
    pub horizontal_padding: f32,
}

impl Default for GridMetrics {
    fn default() -> Self {
        Self {
            card_width: 200.0,
            cover_height: 300.0,
            footer_height: 46.0,
            col_gap: 24.0,
            row_gap: 28.0,
            horizontal_padding: 56.0,
        }
    }
}

impl GridMetrics {
    pub fn card_height(&self) -> f32 {
        self.cover_height + self.footer_height
    }

    pub fn row_stride(&self) -> f32 {
        self.card_height() + self.row_gap
    }

    /// Computes how many columns fit comfortably in `viewport_width`.
    pub fn columns_for_width(&self, viewport_width: f32) -> usize {
        let usable = (viewport_width - self.horizontal_padding * 2.0).max(self.card_width);
        let cols = ((usable + self.col_gap) / (self.card_width + self.col_gap)).floor() as usize;
        cols.max(1)
    }

    pub fn row_count(&self, item_count: usize, columns: usize) -> usize {
        let cols = columns.max(1);
        item_count.div_ceil(cols)
    }

    /// Computes the logical bounds of card `index` in grid content space.
    pub fn card_bounds(&self, index: usize, columns: usize) -> LogicalRect {
        let cols = columns.max(1);
        let row = index / cols;
        let col = index % cols;
        let x = self.horizontal_padding + col as f32 * (self.card_width + self.col_gap);
        let y = row as f32 * self.row_stride();
        LogicalRect::new(x, y, self.card_width, self.card_height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_catalog_entry_cleanly() {
        let entry = CatalogEntry::new("game-001", "Hades")
            .with_subtitle("Action Roguelike")
            .with_cover_path("covers/001.png")
            .with_executable("/usr/bin/hades")
            .with_args(["--fullscreen"]);

        assert_eq!(entry.id().as_str(), "game-001");
        assert_eq!(entry.title(), "Hades");
        assert_eq!(entry.subtitle(), Some("Action Roguelike"));
        assert_eq!(
            entry.cover_path().and_then(|p| p.to_str()),
            Some("covers/001.png")
        );
        assert_eq!(entry.args(), &["--fullscreen"]);
    }

    #[test]
    fn computes_responsive_columns_and_bounds() {
        let metrics = GridMetrics::default();
        assert_eq!(metrics.columns_for_width(1920.0), 8);
        assert_eq!(metrics.columns_for_width(3440.0), 14);
        assert_eq!(metrics.row_count(1000, 8), 125);

        let b0 = metrics.card_bounds(0, 8);
        let b9 = metrics.card_bounds(9, 8);
        assert_eq!(b0.x, 56.0);
        assert_eq!(b0.y, 0.0);
        assert_eq!(b9.x, 56.0 + 224.0);
        assert_eq!(b9.y, metrics.row_stride());
    }
}
