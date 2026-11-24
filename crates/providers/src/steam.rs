//! Biblioteca Steam lida do disco, sem login nem API: bibliotecas em `libraryfolders.vdf`,
//! jogos em `appmanifest_*.acf`, capas em `appcache/librarycache/<appid>/` e conquistas do
//! cache de estatísticas em `appcache/stats/` (o mesmo que o cliente Steam mostra offline).

use std::fs;
use std::path::{Path, PathBuf};

use bigpicture_core::{Achievement, CatalogEntry, GameSource};

use crate::CatalogProvider;
use crate::keyvalues::{Kv, parse_binary, parse_text};

/// Diferença entre o SteamID64 e o "account id" que nomeia os arquivos de estatística.
const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

/// Apps que a Steam instala como dependência e que não são jogos.
const NOT_GAMES: &[u32] = &[
    228_980,   // Steamworks Common Redistributables
    1_070_560, // Steam Linux Runtime
    1_391_110, // Steam Linux Runtime - Soldier
    1_628_350, // Steam Linux Runtime - Sniper
    1_493_710, // Proton Experimental
];

/// Idiomas procurados nos textos das conquistas, na ordem de preferência.
const LANGUAGES: &[&str] = &["brazilian", "portuguese", "english"];

/// Um jogo instalado, como o manifesto descreve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamGame {
    pub app_id: u32,
    pub name: String,
    pub install_dir: PathBuf,
}

/// Instalação da Steam encontrada no computador.
#[derive(Clone, Debug)]
pub struct SteamLibrary {
    root: PathBuf,
}

impl SteamLibrary {
    /// Procura a Steam no registro (Windows) ou nas pastas padrão (Linux).
    pub fn detect() -> Option<Self> {
        Self::candidate_roots().into_iter().find(|p| p.join("steamapps").is_dir()).map(Self::at)
    }

    /// Usa a instalação em `root` (a pasta que contém `steamapps`).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn candidate_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        #[cfg(windows)]
        if let Some(path) = registry_steam_path() {
            roots.push(path);
        }
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(PathBuf::from(pf).join("Steam"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            roots.push(home.join(".local/share/Steam"));
            roots.push(home.join(".steam/steam"));
            roots.push(home.join(".var/app/com.valvesoftware.Steam/data/Steam"));
        }
        roots
    }

    /// Pastas `steamapps` de todas as bibliotecas (a principal primeiro).
    pub fn library_folders(&self) -> Vec<PathBuf> {
        let mut folders = vec![self.root.join("steamapps")];
        let file = self.root.join("steamapps").join("libraryfolders.vdf");
        if let Some(kv) = read_text_kv(&file) {
            let list = kv.get("libraryfolders").or_else(|| kv.get("LibraryFolders"));
            for (_, entry) in list.into_iter().flat_map(Kv::children) {
                // Formato novo: { "path" "..." }; antigo: "1" "D:\\SteamLibrary".
                let path = entry.get("path").and_then(Kv::as_str).or_else(|| entry.as_str());
                if let Some(path) = path {
                    let steamapps = PathBuf::from(path).join("steamapps");
                    if steamapps.is_dir() && !folders.iter().any(|f| same_path(f, &steamapps)) {
                        folders.push(steamapps);
                    }
                }
            }
        }
        folders
    }

    /// Jogos completamente instalados, em ordem alfabética.
    pub fn installed_games(&self) -> Vec<SteamGame> {
        let mut games = Vec::new();
        for steamapps in self.library_folders() {
            let Ok(dir) = fs::read_dir(&steamapps) else { continue };
            for entry in dir.flatten() {
                let path = entry.path();
                let is_manifest = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"));
                if let Some(game) = is_manifest.then(|| read_manifest(&path, &steamapps)).flatten() {
                    games.push(game);
                }
            }
        }
        games.sort_by_key(|g| g.name.to_lowercase());
        games.dedup_by_key(|g| g.app_id);
        games
    }

    /// Capa vertical 600×900 do cache local da Steam, se ela já foi baixada pelo cliente.
    pub fn cover(&self, app_id: u32) -> Option<PathBuf> {
        let cache = self.root.join("appcache").join("librarycache");
        // Layout antigo: arquivos soltos com o appid no nome.
        let flat = cache.join(format!("{app_id}_library_600x900.jpg"));
        if flat.is_file() {
            return Some(flat);
        }
        // Layout novo: pasta por appid, às vezes com subpasta de hash.
        find_named(&cache.join(app_id.to_string()), "library_600x900", 2)
    }

    /// Account id do usuário logado por último (o que nomeia `UserGameStats_<id>_<appid>.bin`).
    pub fn current_account_id(&self) -> Option<u32> {
        let users = read_text_kv(&self.root.join("config").join("loginusers.vdf"))?;
        let users = users.get("users")?;
        #[cfg(windows)]
        let auto_login = registry_auto_login_user();
        #[cfg(not(windows))]
        let auto_login: Option<String> = None;
        let pick = users
            .children()
            .find(|(_, u)| u.get("MostRecent").and_then(Kv::as_i64) == Some(1))
            .or_else(|| {
                let name = auto_login.as_deref()?;
                users.children().find(|(_, u)| {
                    u.get("AccountName").and_then(Kv::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
            })
            .or_else(|| users.children().next())?;
        let steam_id: u64 = pick.0.parse().ok()?;
        u32::try_from(steam_id.checked_sub(STEAMID64_BASE)?).ok()
    }

    /// Conquistas de `app_id` com o progresso do usuário atual, lidas do cache local. Vazio se
    /// o jogo não tem conquistas ou o cliente ainda não baixou o esquema.
    pub fn achievements(&self, app_id: u32) -> Vec<Achievement> {
        let stats = self.root.join("appcache").join("stats");
        let Some(schema) = read_binary_kv(&stats.join(format!("UserGameStatsSchema_{app_id}.bin"))) else {
            return Vec::new();
        };
        let user = self
            .current_account_id()
            .map(|account| stats.join(format!("UserGameStats_{account}_{app_id}.bin")))
            .and_then(|p| read_binary_kv(&p));
        achievements_from(&schema, user.as_ref(), app_id)
    }
}

impl CatalogProvider for SteamLibrary {
    fn provider_name(&self) -> &'static str {
        "steam"
    }

    fn entries(&self) -> Vec<CatalogEntry> {
        self.installed_games()
            .into_iter()
            .map(|game| {
                let mut entry = CatalogEntry::new(format!("steam-{}", game.app_id), &game.name)
                    .with_subtitle("Steam")
                    .with_source(GameSource::Steam { app_id: game.app_id })
                    .with_install_dir(game.install_dir.clone())
                    .with_launch_uri(format!("steam://rungameid/{}", game.app_id));
                if let Some(cover) = self.cover(game.app_id) {
                    entry = entry.with_cover_path(cover);
                }
                entry
            })
            .collect()
    }
}

fn read_manifest(path: &Path, steamapps: &Path) -> Option<SteamGame> {
    let kv = read_text_kv(path)?;
    let state = kv.get("AppState")?;
    let app_id = u32::try_from(state.get("appid")?.as_i64()?).ok()?;
    // StateFlags 4 = totalmente instalado; outros bits indicam download ou atualização.
    let flags = state.get("StateFlags").and_then(Kv::as_i64).unwrap_or(0);
    if flags & 4 == 0 || NOT_GAMES.contains(&app_id) {
        return None;
    }
    let name = state.get("name")?.as_str()?.to_string();
    let install_dir = steamapps.join("common").join(state.get("installdir")?.as_str()?);
    Some(SteamGame { app_id, name, install_dir })
}

/// Monta as conquistas a partir do esquema (`<appid>/stats/<id>/bits/<bit>`) e do progresso
/// do usuário (`cache/<id>/data` com um bit por conquista e `AchievementTimes/<bit>`).
pub fn achievements_from(schema: &Kv, user: Option<&Kv>, app_id: u32) -> Vec<Achievement> {
    let Some(game) = schema.get(&app_id.to_string()).or_else(|| schema.children().next().map(|(_, v)| v)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (stat_id, stat) in game.get("stats").into_iter().flat_map(Kv::children) {
        let Some(bits) = stat.get("bits") else { continue };
        let progress = user.and_then(|u| u.path(&["cache", stat_id]));
        let mask = progress.and_then(|p| p.get("data")).and_then(Kv::as_i64).unwrap_or(0);
        let times = progress.and_then(|p| p.get("AchievementTimes"));
        let mut ordered: Vec<(u32, &Kv)> = bits
            .children()
            .filter_map(|(bit, ach)| Some((bit.parse().ok()?, ach)))
            .collect();
        ordered.sort_by_key(|(bit, _)| *bit);
        for (bit, ach) in ordered {
            let display = ach.get("display");
            let text = |field: &str| {
                let node = display.and_then(|d| d.get(field));
                LANGUAGES
                    .iter()
                    .find_map(|lang| node.and_then(|n| n.get(lang)).and_then(Kv::as_str))
                    .unwrap_or_default()
                    .to_string()
            };
            let set = bit < 64 && (mask >> bit) & 1 == 1;
            let time = times
                .and_then(|t| t.get(&bit.to_string()))
                .and_then(Kv::as_i64)
                .filter(|&t| t > 0)
                .map(|t| t as u64);
            let unlocked_at = time.or(set.then_some(0));
            let icon = |field: &str| {
                display
                    .and_then(|d| d.get(field))
                    .and_then(Kv::as_str)
                    .filter(|f| !f.is_empty())
                    .map(|file| {
                        format!("https://cdn.steamstatic.com/steamcommunity/public/images/apps/{app_id}/{file}")
                    })
            };
            out.push(Achievement {
                api_name: ach.get("name").and_then(Kv::as_str).unwrap_or_default().to_string(),
                name: text("name"),
                description: text("desc"),
                hidden: display.and_then(|d| d.get("hidden")).and_then(Kv::as_i64).unwrap_or(0) != 0,
                unlocked_at,
                icon_url: icon("icon"),
                icon_locked_url: icon("icon_gray"),
            });
        }
    }
    out
}

fn read_text_kv(path: &Path) -> Option<Kv> {
    let text = fs::read_to_string(path).ok()?;
    parse_text(&text)
        .map_err(|err| tracing::warn!(path = %path.display(), error = err, "VDF inválido"))
        .ok()
}

fn read_binary_kv(path: &Path) -> Option<Kv> {
    let bytes = fs::read(path).ok()?;
    parse_binary(&bytes)
        .map_err(|err| tracing::warn!(path = %path.display(), error = err, "KeyValues binário inválido"))
        .ok()
}

/// Primeiro arquivo cujo nome (sem extensão) é `stem`, até `depth` níveis abaixo de `dir`.
fn find_named(dir: &Path, stem: &str, depth: usize) -> Option<PathBuf> {
    let entries: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    entries
        .iter()
        .find(|p| p.is_file() && p.file_stem().and_then(|s| s.to_str()) == Some(stem))
        .cloned()
        .or_else(|| {
            (depth > 0)
                .then(|| entries.iter().filter(|p| p.is_dir()).find_map(|d| find_named(d, stem, depth - 1)))
                .flatten()
        })
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    // O registro guarda o caminho com barras normais e em minúsculas ("c:/program files (x86)/steam").
    registry_value("SteamPath").map(|p| PathBuf::from(p.replace('/', "\\")))
}

#[cfg(windows)]
fn registry_auto_login_user() -> Option<String> {
    registry_value("AutoLoginUser")
}

/// Lê `HKCU\Software\Valve\Steam\<name>` com o `reg.exe` do sistema, sem dependência extra.
#[cfg(windows)]
fn registry_value(name: &str) -> Option<String> {
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", name])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|l| l.trim_start().starts_with(name))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn temp_steam(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bp_steam_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("steamapps")).unwrap();
        root
    }

    fn manifest(dir: &Path, app_id: u32, name: &str, flags: u32) {
        fs::write(
            dir.join(format!("appmanifest_{app_id}.acf")),
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{name}\"\n}}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn reads_games_from_all_libraries_skipping_redistributables_and_partial_installs() {
        let root = temp_steam("libs");
        let second = root.join("Second");
        fs::create_dir_all(second.join("steamapps")).unwrap();
        let second_text = second.to_string_lossy().replace('\\', "\\\\");
        fs::write(
            root.join("steamapps").join("libraryfolders.vdf"),
            format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{second_text}\" }} }}",
                root.to_string_lossy().replace('\\', "\\\\")),
        )
        .unwrap();
        manifest(&root.join("steamapps"), 1454400, "Cookie Clicker", 4);
        manifest(&root.join("steamapps"), 228980, "Steamworks Common Redistributables", 4);
        manifest(&second.join("steamapps"), 1903340, "Clair Obscur", 4);
        manifest(&second.join("steamapps"), 42, "Baixando", 1026);
        let cache = root.join("appcache").join("librarycache").join("1454400").join("abc");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("library_600x900.jpg"), b"jpg").unwrap();

        let steam = SteamLibrary::at(&root);
        assert_eq!(steam.library_folders().len(), 2);
        let games = steam.installed_games();
        let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Clair Obscur", "Cookie Clicker"]);
        assert!(games[1].install_dir.ends_with("steamapps/common/Cookie Clicker"));

        let entries = steam.entries();
        let cookie = entries.iter().find(|e| e.title() == "Cookie Clicker").unwrap();
        assert_eq!(cookie.launch_uri(), Some("steam://rungameid/1454400"));
        assert_eq!(cookie.source(), &GameSource::Steam { app_id: 1454400 });
        assert!(cookie.cover_path().unwrap().ends_with("library_600x900.jpg"));
        assert!(entries.iter().find(|e| e.title() == "Clair Obscur").unwrap().cover_path().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    fn block(pairs: Vec<(&str, Kv)>) -> Kv {
        Kv::Block(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<BTreeMap<_, _>>())
    }

    fn text(lang: &str, s: &str) -> Kv {
        block(vec![(lang, Kv::Str(s.into())), ("token", Kv::Str("T".into()))])
    }

    #[test]
    fn builds_achievements_from_schema_and_user_progress() {
        let ach = |api: &str, name: Kv, hidden: i64| {
            block(vec![
                ("name", Kv::Str(api.into())),
                (
                    "display",
                    block(vec![
                        ("name", name),
                        ("desc", text("english", "desc")),
                        ("hidden", Kv::Int(hidden)),
                        ("icon", Kv::Str("on.jpg".into())),
                        ("icon_gray", Kv::Str("off.jpg".into())),
                    ]),
                ),
            ])
        };
        let schema = block(vec![(
            "10",
            block(vec![(
                "stats",
                block(vec![(
                    "1",
                    block(vec![(
                        "bits",
                        block(vec![
                            ("0", ach("A", block(vec![("english", Kv::Str("First".into())), ("brazilian", Kv::Str("Primeira".into()))]), 0)),
                            ("1", ach("B", text("english", "Second"), 1)),
                            ("2", ach("C", text("english", "Third"), 0)),
                        ]),
                    )]),
                )]),
            )]),
        )]);
        let user = block(vec![(
            "cache",
            block(vec![(
                "1",
                block(vec![
                    ("data", Kv::Int(0b101)),
                    ("AchievementTimes", block(vec![("0", Kv::Int(1_700_000_000))])),
                ]),
            )]),
        )]);
        let list = achievements_from(&schema, Some(&user), 10);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "Primeira", "prefere português");
        assert_eq!(list[0].unlocked_at, Some(1_700_000_000));
        assert!(!list[1].unlocked() && list[1].hidden);
        assert_eq!(list[2].unlocked_at, Some(0), "bit marcado sem data conta como desbloqueada");
        assert_eq!(
            list[0].icon_url.as_deref(),
            Some("https://cdn.steamstatic.com/steamcommunity/public/images/apps/10/on.jpg")
        );
        assert!(achievements_from(&schema, None, 10).iter().all(|a| !a.unlocked()));
    }
}
