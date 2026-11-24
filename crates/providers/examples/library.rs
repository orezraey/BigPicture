//! Lista a biblioteca local (Steam e Epic) com capas e conquistas:
//! `cargo run -p bigpicture-providers --example library`.
use bigpicture_core::{GameSource, progress};
use bigpicture_providers::CatalogProvider;
use bigpicture_providers::steam::SteamLibrary;

fn main() {
    match SteamLibrary::detect() {
        Some(steam) => {
            println!("Steam em {} (conta {:?})", steam.root().display(), steam.current_account_id());
            for folder in steam.library_folders() {
                println!("  biblioteca: {}", folder.display());
            }
            for entry in steam.entries() {
                let GameSource::Steam { app_id } = entry.source() else { continue };
                let achievements = steam.achievements(*app_id);
                let (done, total) = progress(&achievements);
                println!(
                    "  [{app_id}] {} | capa: {} | conquistas: {done}/{total}",
                    entry.title(),
                    entry.cover_path().map_or("não".into(), |p| p.display().to_string()),
                );
                for a in achievements.iter().filter(|a| a.unlocked()).take(2) {
                    println!("      ✓ {}: {}", a.name, a.description);
                }
            }
        }
        None => println!("Steam não encontrada"),
    }
    let running = std::process::Command::new("tasklist").arg("/FI").arg("IMAGENAME eq steam.exe").output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("steam.exe")).unwrap_or(false);
    match bigpicture_providers::steam_client::find_debugger(running) {
        bigpicture_providers::steam_client::ClientStatus::Ready(port) => {
            match bigpicture_providers::steam_client::read_library(port, std::time::Duration::from_secs(30)) {
                Ok(games) => {
                    let installed = games.iter().filter(|g| g.installed).count();
                    let family = games.iter().filter(|g| g.family_owner.is_some()).count();
                    let hidden = games.iter().filter(|g| g.hidden).count();
                    println!("Biblioteca pelo cliente (porta {port}): {} jogos, {installed} instalados, {family} da família, {hidden} ocultos", games.len());
                    for g in games.iter().filter(|g| !g.installed).take(5) {
                        println!("  não instalado: [{}] {} ({} min)", g.app_id, g.name, g.playtime_minutes);
                    }
                }
                Err(err) => println!("leitura falhou: {err}"),
            }
        }
        other => println!("cliente Steam: {other:?}"),
    }
}
