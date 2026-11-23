//! Core domain models and pure algorithms for BigPicture.

use std::path::PathBuf;

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
    cover_path: Option<PathBuf>,
    executable: Option<PathBuf>,
    args: Vec<String>,
}

impl CatalogEntry {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: AppId::new(id),
            title: title.into(),
            cover_path: None,
            executable: None,
            args: Vec::new(),
        }
    }

    pub fn with_cover_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.cover_path = Some(path.into());
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

    pub fn id(&self) -> &AppId {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn cover_path(&self) -> Option<&PathBuf> {
        self.cover_path.as_ref()
    }

    pub fn executable(&self) -> Option<&PathBuf> {
        self.executable.as_ref()
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_catalog_entry_cleanly() {
        let entry = CatalogEntry::new("game-001", "Hades")
            .with_cover_path("covers/001.png")
            .with_executable("/usr/bin/hades")
            .with_args(["--fullscreen"]);

        assert_eq!(entry.id().as_str(), "game-001");
        assert_eq!(entry.title(), "Hades");
        assert_eq!(
            entry.cover_path().and_then(|p| p.to_str()),
            Some("covers/001.png")
        );
        assert_eq!(entry.args(), &["--fullscreen"]);
    }
}
