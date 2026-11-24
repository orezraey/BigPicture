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
}
