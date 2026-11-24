//! Biblioteca completa da conta Steam (instalados ou não), lida do cliente Steam aberto
//! pela interface de depuração do navegador interno dele (Chrome DevTools Protocol).
//!
//! Não pede login nem chave: o cliente já está logado. A Steam só abre essa interface quando
//! existe o arquivo [`FLAG_FILE`] na pasta dela, e sempre só em `127.0.0.1`. A porta padrão no
//! Windows é 8080, que outros programas costumam ocupar; por isso o launcher abre a Steam com
//! `-devtools-port` ([`DEVTOOLS_PORT`]) quando ela está fechada.
//!
//! A leitura roda `appStore.allApps` no contexto compartilhado da Steam (`SharedJSContext`).
//! O resultado vai para um cache em disco, para a biblioteca aparecer mesmo com a Steam fechada.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Arquivo na pasta da Steam que faz o cliente abrir a interface de depuração ao iniciar.
pub const FLAG_FILE: &str = ".cef-enable-remote-debugging";
/// Porta pedida à Steam com `-devtools-port` quando o launcher a abre.
pub const DEVTOOLS_PORT: u16 = 8091;
/// Porta que a Steam usa no Windows sem `-devtools-port`.
pub const DEFAULT_PORT: u16 = 8080;
/// Títulos do contexto JavaScript compartilhado (o nome mudou entre versões do cliente).
const SHARED_CONTEXTS: [&str; 2] = ["SharedJSContext", "Steam Shared Context presented by Valve\u{2122}"];

/// Um jogo da conta (ou compartilhado pela Família Steam).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedGame {
    pub app_id: u32,
    pub name: String,
    pub installed: bool,
    /// Oculto na biblioteca da Steam.
    pub hidden: bool,
    pub playtime_minutes: u64,
    /// Última vez jogado (segundos Unix), 0 se nunca.
    pub last_played: u64,
    /// Conta de outro membro da Família Steam que compartilha o jogo; `None` se é da conta.
    pub family_owner: Option<u32>,
    /// Arquivo da capa vertical no CDN, relativo à pasta do app (`library_600x900.jpg` ou,
    /// em jogos recentes, `<hash>/library_capsule.jpg`).
    pub capsule: Option<String>,
}

/// Situação do cliente Steam para a leitura da biblioteca.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientStatus {
    /// A interface respondeu nesta porta.
    Ready(u16),
    /// A Steam está aberta, mas sem a interface (aberta antes do arquivo existir, ou a porta
    /// padrão está com outro programa): precisa ser reiniciada pelo launcher.
    NoDebugger,
    /// A Steam não está aberta.
    NotRunning,
}

/// Cria o [`FLAG_FILE`] em `steam_root`. `Ok(true)` se acabou de criar.
pub fn enable_debugger(steam_root: &Path) -> std::io::Result<bool> {
    let flag = steam_root.join(FLAG_FILE);
    if flag.exists() {
        return Ok(false);
    }
    std::fs::write(flag, b"")?;
    Ok(true)
}

/// Remove o [`FLAG_FILE`]: a Steam deixa de abrir a interface a partir da próxima abertura.
/// `Ok(true)` se havia arquivo para remover.
pub fn disable_debugger(steam_root: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(steam_root.join(FLAG_FILE)) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Procura a interface na porta do launcher e na padrão, confirmando que quem responde é o
/// cliente da Steam (outro programa na mesma porta é ignorado).
pub fn find_debugger(steam_running: bool) -> ClientStatus {
    for port in [DEVTOOLS_PORT, DEFAULT_PORT] {
        if is_steam_debugger(port) {
            return ClientStatus::Ready(port);
        }
    }
    if steam_running { ClientStatus::NoDebugger } else { ClientStatus::NotRunning }
}

fn is_steam_debugger(port: u16) -> bool {
    http_get_json(port, "/json/version")
        .ok()
        .and_then(|v| v.get("User-Agent").and_then(Value::as_str).map(|ua| ua.contains("Valve Steam Client")))
        .unwrap_or(false)
}

/// Lê a biblioteca do cliente na porta `port`, esperando até `wait` o cliente terminar de
/// carregá-la (logo depois de abrir, a lista ainda está vazia).
pub fn read_library(port: u16, wait: Duration) -> Result<Vec<OwnedGame>, String> {
    let started = Instant::now();
    // Logo depois de abrir, a interface já responde mas o contexto compartilhado ainda não
    // existe: tenta de novo até `wait`.
    let ws_url = loop {
        match shared_context_url(port) {
            Ok(url) => break url,
            Err(err) if started.elapsed() >= wait => return Err(err),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    };
    let (mut socket, _) = tungstenite::connect(&ws_url).map_err(|e| format!("WebSocket {ws_url}: {e}"))?;
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    }
    let mut next_id = 1u64;
    loop {
        let loaded = evaluate(&mut socket, &mut next_id, LOADED_JS)?;
        if loaded.as_bool() == Some(true) {
            break;
        }
        if started.elapsed() >= wait {
            return Err("a Steam não terminou de carregar a biblioteca (ela está logada?)".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let value = evaluate(&mut socket, &mut next_id, LIBRARY_JS)?;
    let text = value.as_str().ok_or("a Steam devolveu um resultado inesperado")?;
    let list: Value = serde_json::from_str(text).map_err(|e| format!("JSON da biblioteca: {e}"))?;
    let _ = socket.close(None);
    Ok(parse_library(&list))
}

/// Sem lançar exceção enquanto `appStore` ou `allApps` ainda não existem.
const LOADED_JS: &str = "(typeof appStore !== 'undefined') && !!appStore.allApps && \
    appStore.allApps.length > 0 && appStore.m_bIsInitialized !== false";

/// Só jogos (`app_type === 1`), sem atalhos de não-Steam nem demos.
const LIBRARY_JS: &str = r#"(() => {
  const hidden = (id) => { try { return collectionStore.BIsHidden(id) } catch (e) { return false } };
  const games = appStore.allApps.filter((a) => a.app_type === 1
    && !(a.BIsShortcut && a.BIsShortcut()) && !(a.BIsDemo && a.BIsDemo()));
  return JSON.stringify(games.map((a) => ({
    appid: a.appid,
    name: a.display_name,
    installed: Array.isArray(a.per_client_data) && a.per_client_data.some((c) => c.installed),
    hidden: hidden(a.appid),
    minutes: a.minutes_playtime_forever || 0,
    last_played: a.rt_last_time_played || 0,
    owner: a.owner_account_id || null,
    capsule: a.library_capsule_filename || null,
  })));
})()"#;

/// Converte a lista do cliente (ou do cache, que tem o mesmo formato).
pub fn parse_library(list: &Value) -> Vec<OwnedGame> {
    let mut games: Vec<OwnedGame> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let num = |k: &str| g.get(k).and_then(Value::as_u64);
            Some(OwnedGame {
                app_id: u32::try_from(num("appid")?).ok()?,
                name: g.get("name").and_then(Value::as_str)?.trim().to_string(),
                installed: g.get("installed").and_then(Value::as_bool).unwrap_or(false),
                hidden: g.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                playtime_minutes: num("minutes").unwrap_or(0),
                last_played: num("last_played").unwrap_or(0),
                family_owner: num("owner").and_then(|o| u32::try_from(o).ok()).filter(|&o| o != 0),
                capsule: g
                    .get("capsule")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty() && !c.contains(".."))
                    .map(str::to_string),
            })
        })
        .filter(|g| !g.name.is_empty())
        .collect();
    // `dedup` só junta vizinhos: agrupa por appid antes de ordenar pelo nome.
    games.sort_by_key(|g| g.app_id);
    games.dedup_by_key(|g| g.app_id);
    games.sort_by_key(|g| g.name.to_lowercase());
    games
}

/// Grava a lista no cache (mesmo formato que o cliente devolve).
pub fn save_cache(path: &Path, games: &[OwnedGame]) -> std::io::Result<()> {
    let list: Vec<Value> = games
        .iter()
        .map(|g| {
            json!({
                "appid": g.app_id, "name": g.name, "installed": g.installed, "hidden": g.hidden,
                "minutes": g.playtime_minutes, "last_played": g.last_played, "owner": g.family_owner,
                "capsule": g.capsule,
            })
        })
        .collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&list).unwrap_or_default())?;
    std::fs::rename(tmp, path)
}

pub fn load_cache(path: &Path) -> Option<Vec<OwnedGame>> {
    let text = std::fs::read_to_string(path).ok()?;
    let list: Value = serde_json::from_str(&text).ok()?;
    Some(parse_library(&list))
}

/// Endereço da capa vertical no CDN público da Steam (não pede login). `capsule` é o caminho
/// que o cliente informa; sem ele, vale o nome clássico `library_600x900.jpg` (jogos recentes
/// guardam a capa numa subpasta com hash, que só o cliente conhece).
pub fn cdn_cover_url(app_id: u32, capsule: Option<&str>) -> String {
    let file = capsule.unwrap_or("library_600x900.jpg");
    format!("https://shared.steamstatic.com/store_item_assets/steam/apps/{app_id}/{file}")
}

/// Onde guardar a capa baixada do CDN.
pub fn cdn_cover_path(cache_dir: &Path, app_id: u32) -> PathBuf {
    cache_dir.join(format!("{app_id}_library_600x900.jpg"))
}

fn shared_context_url(port: u16) -> Result<String, String> {
    let targets = http_get_json(port, "/json")?;
    targets
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t.get("title").and_then(Value::as_str).is_some_and(|title| SHARED_CONTEXTS.contains(&title)))
        .and_then(|t| t.get("webSocketDebuggerUrl").and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| "a Steam ainda não abriu o contexto compartilhado".to_string())
}

fn evaluate(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    next_id: &mut u64,
    expression: &str,
) -> Result<Value, String> {
    let id = *next_id;
    *next_id += 1;
    let request = json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression, "returnByValue": true, "awaitPromise": true },
    });
    socket
        .send(tungstenite::Message::text(request.to_string()))
        .map_err(|e| format!("enviando para a Steam: {e}"))?;
    loop {
        let message = socket.read().map_err(|e| format!("lendo da Steam: {e}"))?;
        let tungstenite::Message::Text(text) = message else { continue };
        let reply: Value = serde_json::from_str(text.as_str()).map_err(|e| format!("resposta da Steam: {e}"))?;
        if reply.get("id").and_then(Value::as_u64) != Some(id) {
            continue; // eventos e respostas de outras chamadas
        }
        if let Some(err) = reply.get("error") {
            return Err(format!("a Steam recusou o comando: {err}"));
        }
        let result = &reply["result"];
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(format!("erro no JavaScript da Steam: {exception}"));
        }
        return Ok(result["result"].get("value").cloned().unwrap_or(Value::Null));
    }
}

/// GET mínimo em `127.0.0.1:port` (a interface só existe localmente e sem TLS).
fn http_get_json(port: u16, path: &str) -> Result<Value, String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_millis(500)).map_err(|e| format!("porta {port}: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    // O servidor do Chromium pode manter a conexão aberta: lê até o tamanho que o cabeçalho
    // anuncia, ou até a conexão fechar ou o tempo acabar.
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if response_complete(&raw) {
                    break;
                }
            }
            Err(_) if !raw.is_empty() => break,
            Err(e) => return Err(format!("porta {port}: {e}")),
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).ok_or("resposta HTTP sem corpo")?;
    serde_json::from_str(body).map_err(|e| format!("porta {port} não devolveu JSON: {e}"))
}

/// Se `raw` já tem o cabeçalho e todo o corpo que o `Content-Length` anuncia.
fn response_complete(raw: &[u8]) -> bool {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else { return false };
    head.lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .is_some_and(|len| body.len() >= len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_complete_http_response_by_content_length() {
        assert!(!response_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n123"));
        assert!(response_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345"));
        assert!(!response_complete(b"HTTP/1.1 200 OK\r\n"));
    }

    #[test]
    fn parses_client_list_sorting_and_marking_family_games() {
        let list = json!([
            {"appid": 1903340, "name": "Clair Obscur: Expedition 33", "installed": true, "hidden": false, "minutes": 3000, "last_played": 1777000000, "owner": null},
            {"appid": 1454400, "name": "Cookie Clicker", "installed": true, "hidden": false, "minutes": 3702, "last_played": 1777549422, "owner": 449542984},
            {"appid": 620, "name": "Portal 2", "installed": false, "hidden": true, "minutes": 0, "last_played": 0, "capsule": "../evil.jpg"},
            {"appid": 620, "name": "Portal 2", "installed": false},
            {"appid": "x", "name": "Ruim"},
            {"appid": 1, "name": "  "}
        ]);
        let games = parse_library(&list);
        let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Clair Obscur: Expedition 33", "Cookie Clicker", "Portal 2"]);
        assert_eq!(games[1].family_owner, Some(449542984));
        assert_eq!(games[0].family_owner, None);
        assert!(games[2].hidden && !games[2].installed);
        assert_eq!(games[2].capsule, None, "caminho com .. é descartado");
    }

    #[test]
    fn cache_round_trips() {
        let path = std::env::temp_dir().join(format!("bp_steam_cache_{}.json", std::process::id()));
        let games = vec![OwnedGame {
            app_id: 620,
            name: "Portal 2".into(),
            installed: false,
            hidden: false,
            playtime_minutes: 12,
            last_played: 5,
            family_owner: Some(7),
            capsule: Some("abc/library_capsule.jpg".into()),
        }];
        save_cache(&path, &games).unwrap();
        assert_eq!(load_cache(&path), Some(games));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn creates_flag_file_once_and_finds_no_debugger_on_closed_ports() {
        let dir = std::env::temp_dir().join(format!("bp_steam_flag_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(enable_debugger(&dir).unwrap());
        assert!(!enable_debugger(&dir).unwrap());
        assert!(dir.join(FLAG_FILE).is_file());
        assert!(disable_debugger(&dir).unwrap());
        assert!(!disable_debugger(&dir).unwrap());
        assert!(!dir.join(FLAG_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!is_steam_debugger(9));
        assert_eq!(
            cdn_cover_url(620, None),
            "https://shared.steamstatic.com/store_item_assets/steam/apps/620/library_600x900.jpg"
        );
        assert_eq!(
            cdn_cover_url(3265700, Some("d2b2/library_capsule.jpg")),
            "https://shared.steamstatic.com/store_item_assets/steam/apps/3265700/d2b2/library_capsule.jpg"
        );
    }
}
