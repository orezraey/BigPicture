//! Local catalog providers for BigPicture (store integrations are Milestone 2).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use bigpicture_core::CatalogEntry;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "qoi"];

const SAMPLE_PREFIXES: &[&str] = &[
    "Starfall",
    "Chrono",
    "Neon",
    "Shadow",
    "Aether",
    "Iron",
    "Crimson",
    "Valkyrie",
    "Obsidian",
    "Solaris",
    "Frost",
    "Astral",
    "Ember",
    "Phantom",
    "Hyperion",
    "Eclipse",
];

const SAMPLE_SUFFIXES: &[&str] = &[
    "Odyssey",
    "Protocol",
    "Tactics",
    "Horizons",
    "Frontier",
    "Vanguard",
    "Chronicles",
    "Requiem",
    "Legacy",
    "Drifter",
    "Ascension",
    "Siege",
];

const SAMPLE_GENRES: &[&str] = &[
    "Action RPG",
    "Sci-Fi Adventure",
    "Roguelike",
    "Tactical Strategy",
    "Metroidvania",
    "Arcade Racing",
    "Immersive Sim",
    "Co-op Shooter",
];

/// Trait implemented by catalog sources (local folders, shortcuts, etc.).
pub trait CatalogProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn entries(&self) -> Vec<CatalogEntry>;
}

/// Static in-memory provider useful for testing and spike demos.
#[derive(Clone, Debug, Default)]
pub struct StaticCatalogProvider {
    items: Vec<CatalogEntry>,
}

impl StaticCatalogProvider {
    pub fn new(items: Vec<CatalogEntry>) -> Self {
        Self { items }
    }
}

impl CatalogProvider for StaticCatalogProvider {
    fn provider_name(&self) -> &'static str {
        "static"
    }

    fn entries(&self) -> Vec<CatalogEntry> {
        self.items.clone()
    }
}

/// Provider that builds a catalog of `target_count` cards backed by cover images
/// discovered in a local directory on disk.
#[derive(Clone, Debug)]
pub struct LocalFolderCatalogProvider {
    covers_dir: PathBuf,
    cover_files: Vec<PathBuf>,
    trailer_files: Vec<PathBuf>,
    target_count: usize,
}

impl LocalFolderCatalogProvider {
    pub fn from_directory(covers_dir: impl Into<PathBuf>, target_count: usize) -> Result<Self> {
        let covers_dir = covers_dir.into();
        let mut cover_files = Vec::new();

        if covers_dir.is_dir() {
            for entry in fs::read_dir(&covers_dir).with_context(|| {
                format!(
                    "failed to scan local covers folder {}",
                    covers_dir.display()
                )
            })? {
                let path = entry?.path();
                if path.is_file()
                    && let Some(ext) = path.extension().and_then(|e| e.to_str())
                    && IMAGE_EXTENSIONS.iter().any(|s| s.eq_ignore_ascii_case(ext))
                {
                    cover_files.push(path);
                }
            }
            cover_files.sort();
        }

        tracing::info!(
            dir = %covers_dir.display(),
            discovered_files = cover_files.len(),
            target_count,
            "initialized LocalFolderCatalogProvider"
        );

        Ok(Self {
            covers_dir,
            cover_files,
            trailer_files: Vec::new(),
            target_count,
        })
    }

    /// Trailers distribuídos entre os cards em ciclo, como as capas.
    pub fn with_trailer_files(mut self, trailer_files: Vec<PathBuf>) -> Self {
        self.trailer_files = trailer_files;
        self
    }

    pub fn covers_dir(&self) -> &Path {
        &self.covers_dir
    }
}

impl CatalogProvider for LocalFolderCatalogProvider {
    fn provider_name(&self) -> &'static str {
        "local-folder"
    }

    fn entries(&self) -> Vec<CatalogEntry> {
        (0..self.target_count)
            .map(|ix| {
                let prefix = SAMPLE_PREFIXES[ix % SAMPLE_PREFIXES.len()];
                let suffix = SAMPLE_SUFFIXES[(ix / SAMPLE_PREFIXES.len()) % SAMPLE_SUFFIXES.len()];
                let edition = ix / (SAMPLE_PREFIXES.len() * SAMPLE_SUFFIXES.len());
                let title = if edition == 0 {
                    format!("{prefix} {suffix}")
                } else {
                    format!("{prefix} {suffix} {}", edition + 1)
                };
                let genre = SAMPLE_GENRES[ix % SAMPLE_GENRES.len()];

                let mut entry =
                    CatalogEntry::new(format!("card-{ix:04}"), title).with_subtitle(genre);

                if !self.cover_files.is_empty() {
                    let cover = &self.cover_files[ix % self.cover_files.len()];
                    entry = entry.with_cover_path(cover.clone());
                }

                if !self.trailer_files.is_empty() {
                    let trailer = &self.trailer_files[ix % self.trailer_files.len()];
                    entry = entry.with_trailer_path(trailer.clone());
                }

                entry
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_provider_returns_configured_entries() {
        let provider = StaticCatalogProvider::new(vec![
            CatalogEntry::new("app-1", "Demo One"),
            CatalogEntry::new("app-2", "Demo Two"),
        ]);
        assert_eq!(provider.provider_name(), "static");
        assert_eq!(provider.entries().len(), 2);
    }

    #[test]
    fn local_folder_provider_builds_thousand_cards_from_local_images() {
        let dir = std::env::temp_dir().join(format!(
            "bigpicture_provider_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), b"placeholder").unwrap();
        fs::write(dir.join("b.qoi"), b"placeholder").unwrap();

        let provider = LocalFolderCatalogProvider::from_directory(&dir, 1_000).unwrap();
        let entries = provider.entries();
        assert_eq!(entries.len(), 1_000);
        assert!(entries[0].cover_path().unwrap().ends_with("a.png"));
        assert!(entries[1].cover_path().unwrap().ends_with("b.qoi"));
        assert!(entries[0].trailer_path().is_none());

        let with_trailers = provider
            .with_trailer_files(vec![PathBuf::from("t0.webm"), PathBuf::from("t1.webm")]);
        let entries = with_trailers.entries();
        assert!(entries[0].trailer_path().unwrap().ends_with("t0.webm"));
        assert!(entries[3].trailer_path().unwrap().ends_with("t1.webm"));

        let _ = fs::remove_dir_all(&dir);
    }
}
