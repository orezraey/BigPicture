//! Sincroniza a biblioteca completa da Steam em segundo plano: garante a interface de
//! depuração do cliente, abre a Steam (minimizada) com a porta do launcher se ela estiver
//! fechada, lê a biblioteca e grava o cache. O resultado chega à janela por canal.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use bigpicture_providers::steam_client::{
    self, ClientStatus, DEVTOOLS_PORT, OwnedGame, enable_debugger, find_debugger, read_library,
};

/// Cache da última biblioteca lida (aparece já na abertura, mesmo com a Steam fechada).
pub const LIBRARY_CACHE: &str = "assets/cache/steam_library.json";
/// Capas baixadas do CDN da Steam.
pub const COVER_CACHE: &str = "assets/cache/steam_covers";
/// Quanto esperar a Steam abrir a interface e carregar a biblioteca.
const STARTUP_WAIT: Duration = Duration::from_secs(60);

pub fn load_cached() -> Option<Vec<OwnedGame>> {
    steam_client::load_cache(Path::new(LIBRARY_CACHE))
}

/// Roda [`sync`] numa thread e manda a lista lida por `tx`.
pub fn spawn(steam_root: PathBuf, tx: Sender<Vec<OwnedGame>>) {
    let _ = std::thread::Builder::new().name("bp-steam-sync".into()).spawn(move || match sync(&steam_root) {
        Ok(games) => {
            tracing::info!(games = games.len(), "biblioteca da Steam lida do cliente");
            if let Err(err) = steam_client::save_cache(Path::new(LIBRARY_CACHE), &games) {
                tracing::warn!(error = %err, "cache da biblioteca da Steam não gravado");
            }
            let _ = tx.send(games);
        }
        Err(err) => tracing::warn!(error = %err, "biblioteca completa da Steam indisponível; usando o cache"),
    });
}

fn sync(steam_root: &Path) -> Result<Vec<OwnedGame>, String> {
    match enable_debugger(steam_root) {
        Ok(true) => tracing::info!("interface de depuração da Steam habilitada (vale a partir da próxima abertura)"),
        Ok(false) => {}
        Err(err) => return Err(format!("não deu para criar o arquivo na pasta da Steam: {err}")),
    }
    let port = match find_debugger(steam_running()) {
        ClientStatus::Ready(port) => port,
        ClientStatus::NoDebugger => {
            return Err(
                "a Steam está aberta sem a interface de depuração; feche a Steam e abra o launcher de novo".into(),
            );
        }
        ClientStatus::NotRunning => {
            start_steam(steam_root)?;
            wait_for_debugger()?
        }
    };
    read_library(port, STARTUP_WAIT)
}

/// Abre a Steam minimizada na bandeja, com a interface na porta do launcher.
fn start_steam(steam_root: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let exe = steam_root.join("steam.exe");
    #[cfg(not(windows))]
    let exe = {
        let _ = steam_root;
        PathBuf::from("steam")
    };
    std::process::Command::new(&exe)
        .args(["-silent", "-devtools-port", &DEVTOOLS_PORT.to_string()])
        .spawn()
        .map(|_| tracing::info!("Steam aberta em segundo plano para ler a biblioteca"))
        .map_err(|err| format!("não deu para abrir a Steam ({}): {err}", exe.display()))
}

fn wait_for_debugger() -> Result<u16, String> {
    let started = Instant::now();
    while started.elapsed() < STARTUP_WAIT {
        if let ClientStatus::Ready(port) = find_debugger(true) {
            return Ok(port);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err("a Steam abriu, mas a interface não respondeu a tempo".into())
}

fn steam_running() -> bool {
    #[cfg(windows)]
    let output = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
        .output();
    #[cfg(not(windows))]
    let output = std::process::Command::new("pgrep").args(["-x", "steam"]).output();
    output
        .map(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
            if cfg!(windows) { text.contains("steam.exe") } else { o.status.success() }
        })
        .unwrap_or(false)
}
