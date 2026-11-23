//! Local catalog providers for BigPicture (store integrations are Milestone 2).

use bigpicture_core::CatalogEntry;

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
}
