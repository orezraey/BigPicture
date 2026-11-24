//! Preferências do launcher guardadas em disco (`assets/config/settings.json`).

use std::path::Path;

use serde_json::{Value, json};

pub const SETTINGS_FILE: &str = "assets/config/settings.json";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Ler a biblioteca completa pelo cliente da Steam (liga a interface de depuração dela).
    /// `None`: ainda não perguntado.
    pub steam_full_library: Option<bool>,
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let Some(json) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            return Self::default();
        };
        Self {
            steam_full_library: json.get("steam_full_library").and_then(Value::as_bool),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = json!({ "steam_full_library": self.steam_full_library });
        std::fs::write(path, serde_json::to_vec_pretty(&json).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults_when_missing_or_broken() {
        let dir = std::env::temp_dir().join(format!("bp_settings_{}", std::process::id()));
        let path = dir.join("config").join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(&path), Settings::default());
        let set = Settings { steam_full_library: Some(false) };
        set.save(&path).unwrap();
        assert_eq!(Settings::load(&path), set);
        std::fs::write(&path, "{ quebrado").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
