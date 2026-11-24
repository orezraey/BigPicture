//! Biblioteca da Epic Games lida dos manifestos do Epic Games Launcher
//! (`%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item`, JSON).
//!
//! A Epic não guarda capas nem conquistas no disco: as capas ficam no CDN do catálogo e as
//! conquistas exigem login na conta Epic (Epic Online Services). Por enquanto os jogos da Epic
//! entram sem capa e sem conquistas.

use std::fs;
use std::path::{Path, PathBuf};

use bigpicture_core::{CatalogEntry, GameSource};
use serde_json::Value;

use crate::CatalogProvider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpicGame {
    pub app_name: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub executable: Option<PathBuf>,
    pub launch_uri: String,
}

#[derive(Clone, Debug)]
pub struct EpicLibrary {
    manifests_dir: PathBuf,
}

impl EpicLibrary {
    /// Pasta de manifestos padrão do Epic Games Launcher no Windows.
    pub fn detect() -> Option<Self> {
        let base = std::env::var_os("ProgramData")?;
        let dir = PathBuf::from(base).join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests");
        dir.is_dir().then(|| Self::at(dir))
    }

    pub fn at(manifests_dir: impl Into<PathBuf>) -> Self {
        Self { manifests_dir: manifests_dir.into() }
    }

    /// Jogos completamente instalados (sem DLCs nem instalações pela metade), em ordem alfabética.
    pub fn installed_games(&self) -> Vec<EpicGame> {
        let Ok(dir) = fs::read_dir(&self.manifests_dir) else {
            return Vec::new();
        };
        let mut games: Vec<EpicGame> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("item")))
            .filter_map(|p| read_manifest(&p))
            .collect();
        games.sort_by_key(|g| g.name.to_lowercase());
        games.dedup_by(|a, b| a.app_name == b.app_name);
        games
    }
}

impl CatalogProvider for EpicLibrary {
    fn provider_name(&self) -> &'static str {
        "epic"
    }

    fn entries(&self) -> Vec<CatalogEntry> {
        self.installed_games()
            .into_iter()
            .map(|game| {
                let mut entry = CatalogEntry::new(format!("epic-{}", game.app_name), &game.name)
                    .with_subtitle("Epic Games")
                    .with_source(GameSource::Epic { app_name: game.app_name.clone() })
                    .with_install_dir(game.install_dir.clone())
                    .with_launch_uri(game.launch_uri.clone());
                if let Some(exe) = game.executable {
                    entry = entry.with_executable(exe);
                }
                entry
            })
            .collect()
    }
}

fn read_manifest(path: &Path) -> Option<EpicGame> {
    let text = fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|err| tracing::warn!(path = %path.display(), error = %err, "manifesto da Epic inválido"))
        .ok()?;
    parse_manifest(&json)
}

/// Converte um manifesto `.item` em jogo. `None` para DLCs, instalações incompletas e itens que
/// não são jogos.
pub fn parse_manifest(json: &Value) -> Option<EpicGame> {
    let field = |name: &str| json.get(name).and_then(Value::as_str).filter(|s| !s.is_empty());
    if json.get("bIsIncompleteInstall").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let app_name = field("AppName")?;
    // DLCs apontam para o jogo principal com um MainGameAppName diferente.
    if field("MainGameAppName").is_some_and(|main| main != app_name) {
        return None;
    }
    if let Some(categories) = json.get("AppCategories").and_then(Value::as_array)
        && !categories.iter().any(|c| c.as_str() == Some("games"))
    {
        return None;
    }
    let install_dir = PathBuf::from(field("InstallLocation")?);
    let executable = field("LaunchExecutable").map(|exe| install_dir.join(exe));
    let launch_uri = match (field("CatalogNamespace"), field("CatalogItemId")) {
        (Some(ns), Some(item)) => {
            format!("com.epicgames.launcher://apps/{ns}%3A{item}%3A{app_name}?action=launch&silent=true")
        }
        _ => format!("com.epicgames.launcher://apps/{app_name}?action=launch&silent=true"),
    };
    Some(EpicGame {
        app_name: app_name.to_string(),
        name: field("DisplayName").unwrap_or(app_name).to_string(),
        install_dir,
        executable,
        launch_uri,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn game_manifest(app: &str, name: &str) -> Value {
        json!({
            "FormatVersion": 0,
            "bIsIncompleteInstall": false,
            "DisplayName": name,
            "InstallLocation": format!("C:\\Games\\{name}"),
            "LaunchExecutable": "Bin/Game.exe",
            "AppName": app,
            "MainGameAppName": app,
            "CatalogNamespace": "ns1",
            "CatalogItemId": "item1",
            "AppCategories": ["public", "games", "applications"]
        })
    }

    #[test]
    fn parses_game_and_builds_launcher_uri() {
        let game = parse_manifest(&game_manifest("Fortnite", "Fortnite")).unwrap();
        assert_eq!(game.name, "Fortnite");
        assert_eq!(
            game.launch_uri,
            "com.epicgames.launcher://apps/ns1%3Aitem1%3AFortnite?action=launch&silent=true"
        );
        assert!(game.executable.unwrap().ends_with("Bin/Game.exe"));
    }

    #[test]
    fn skips_dlc_incomplete_and_non_games() {
        let mut dlc = game_manifest("DlcPack", "DLC");
        dlc["MainGameAppName"] = json!("Fortnite");
        assert!(parse_manifest(&dlc).is_none());
        let mut partial = game_manifest("A", "A");
        partial["bIsIncompleteInstall"] = json!(true);
        assert!(parse_manifest(&partial).is_none());
        let mut tool = game_manifest("UE", "Unreal Engine");
        tool["AppCategories"] = json!(["engines"]);
        assert!(parse_manifest(&tool).is_none());
    }

    #[test]
    fn reads_manifest_folder_into_catalog_entries() {
        let dir = std::env::temp_dir().join(format!("bp_epic_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.item"), game_manifest("Zeta", "Zeta Game").to_string()).unwrap();
        fs::write(dir.join("a.item"), game_manifest("Alpha", "Alpha Game").to_string()).unwrap();
        fs::write(dir.join("broken.item"), "{ não é json").unwrap();
        fs::write(dir.join("notes.txt"), "ignorado").unwrap();
        let entries = EpicLibrary::at(&dir).entries();
        let titles: Vec<&str> = entries.iter().map(CatalogEntry::title).collect();
        assert_eq!(titles, ["Alpha Game", "Zeta Game"]);
        assert_eq!(entries[0].source(), &GameSource::Epic { app_name: "Alpha".into() });
        let _ = fs::remove_dir_all(&dir);
    }
}
