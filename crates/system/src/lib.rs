//! Controle do sistema operacional: monitores (resolução e taxa de atualização) e janelas/processos
//! (abrir apps, minimizar, focar, trazer o launcher de volta).
//!
//! [`platform`] escolhe o backend da plataforma atual. Windows é o backend completo; no Linux há
//! adaptadores para X11 (RandR + EWMH) e KDE Wayland, que por ora respondem
//! [`SystemError::Unsupported`] para o que ainda não foi implementado. Os tipos e a lógica de
//! escolha de modo são puros e testáveis sem monitor.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use thiserror::Error;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win32;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SystemError {
    #[error("operation not supported on this platform/compositor: {0}")]
    Unsupported(&'static str),
    #[error("system operation failed: {0}")]
    Failed(String),
    #[error("display not found: {0}")]
    DisplayNotFound(String),
    #[error("display mode {0} is not offered by the display")]
    ModeNotAvailable(DisplayMode),
}

pub type Result<T> = std::result::Result<T, SystemError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DisplayMode {
    width: u32,
    height: u32,
    refresh_hz: u32,
}

impl DisplayMode {
    pub fn new(width: u32, height: u32, refresh_hz: u32) -> Self {
        Self {
            width,
            height,
            refresh_hz,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn refresh_hz(&self) -> u32 {
        self.refresh_hz
    }
}

impl fmt::Display for DisplayMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}@{}", self.width, self.height, self.refresh_hz)
    }
}

impl FromStr for DisplayMode {
    type Err = String;

    /// `LARGURAxALTURA@HZ`, por exemplo `2560x1440@144`.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (size, hz) = s
            .split_once('@')
            .ok_or_else(|| format!("esperado LARGURAxALTURA@HZ, veio {s:?}"))?;
        let (w, h) = size
            .split_once(['x', 'X'])
            .ok_or_else(|| format!("esperado LARGURAxALTURA, veio {size:?}"))?;
        let num = |v: &str| v.trim().parse::<u32>().map_err(|e| format!("{v:?}: {e}"));
        Ok(Self::new(num(w)?, num(h)?, num(hz)?))
    }
}

/// Identificador estável de um monitor no backend (no Windows, `\\.\DISPLAY1`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DisplayId(pub String);

impl fmt::Display for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub id: DisplayId,
    /// Nome amigável do adaptador/monitor, quando o sistema informa.
    pub name: String,
    pub primary: bool,
    /// Canto superior esquerdo na área de trabalho, em pixels físicos.
    pub position: (i32, i32),
    pub current: DisplayMode,
    /// Modos oferecidos, sem repetição, do maior para o menor.
    pub modes: Vec<DisplayMode>,
}

/// Janela de nível superior, opaca (HWND no Windows, XID no X11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchRequest {
    pub executable: PathBuf,
    pub args: Vec<String>,
    /// Pasta de trabalho; por padrão, a pasta do executável (jogos costumam depender disso).
    pub working_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchedApp {
    pub pid: u32,
}

/// Operações de sistema que o launcher precisa. Todas são síncronas e baratas o bastante para
/// rodar fora da thread de UI sob demanda.
pub trait SystemControl: Send + Sync {
    fn backend_name(&self) -> &'static str;

    fn list_displays(&self) -> Result<Vec<DisplayInfo>>;

    /// Troca resolução e taxa de atualização. A troca é temporária: no Windows ela some quando
    /// o processo termina, e [`SystemControl::restore_display_mode`] volta antes disso.
    fn set_display_mode(&self, display: &DisplayId, mode: DisplayMode) -> Result<()>;

    /// Volta o monitor ao modo salvo nas configurações do sistema.
    fn restore_display_mode(&self, display: &DisplayId) -> Result<()>;

    fn launch_app(&self, request: &LaunchRequest) -> Result<LaunchedApp>;

    /// Janelas visíveis de nível superior do processo `pid`.
    fn windows_of_process(&self, pid: u32) -> Result<Vec<WindowId>>;

    fn minimize_window(&self, window: WindowId) -> Result<()>;

    /// Restaura (se minimizada) e põe a janela em primeiro plano.
    fn focus_window(&self, window: WindowId) -> Result<()>;

    /// Traz o launcher de volta com mais insistência que [`SystemControl::focus_window`]: o
    /// Windows recusa `SetForegroundWindow` a processos que não estão em primeiro plano, e é
    /// exatamente o caso quando o botão Guide chega com um jogo na frente.
    fn bring_launcher_to_front(&self, window: WindowId) -> Result<()>;
}

/// Escolhe o modo pedido entre os oferecidos: tamanho exato e a taxa mais próxima de `refresh_hz`
/// (empate fica com a maior). `None` se o tamanho não existir.
pub fn closest_mode(modes: &[DisplayMode], width: u32, height: u32, refresh_hz: u32) -> Option<DisplayMode> {
    modes
        .iter()
        .copied()
        .filter(|m| m.width == width && m.height == height)
        .min_by_key(|m| (m.refresh_hz.abs_diff(refresh_hz), std::cmp::Reverse(m.refresh_hz)))
}

/// Ordena do maior para o menor (área, depois taxa) e remove repetidos.
pub fn normalize_modes(mut modes: Vec<DisplayMode>) -> Vec<DisplayMode> {
    modes.sort_by_key(|m| std::cmp::Reverse((u64::from(m.width) * u64::from(m.height), m.width, m.refresh_hz)));
    modes.dedup();
    modes
}

/// Backend para a plataforma atual.
pub fn platform() -> Box<dyn SystemControl> {
    #[cfg(windows)]
    {
        Box::new(win32::WindowsSystem::new())
    }
    #[cfg(target_os = "linux")]
    {
        linux::detect()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(UnsupportedSystem)
    }
}

/// Backend que responde "não suportado" para tudo, menos abrir apps (que é portátil).
#[derive(Debug, Default)]
pub struct UnsupportedSystem;

impl SystemControl for UnsupportedSystem {
    fn backend_name(&self) -> &'static str {
        "unsupported"
    }

    fn list_displays(&self) -> Result<Vec<DisplayInfo>> {
        Err(SystemError::Unsupported("list_displays"))
    }

    fn set_display_mode(&self, _: &DisplayId, _: DisplayMode) -> Result<()> {
        Err(SystemError::Unsupported("set_display_mode"))
    }

    fn restore_display_mode(&self, _: &DisplayId) -> Result<()> {
        Err(SystemError::Unsupported("restore_display_mode"))
    }

    fn launch_app(&self, request: &LaunchRequest) -> Result<LaunchedApp> {
        spawn_process(request)
    }

    fn windows_of_process(&self, _: u32) -> Result<Vec<WindowId>> {
        Err(SystemError::Unsupported("windows_of_process"))
    }

    fn minimize_window(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("minimize_window"))
    }

    fn focus_window(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("focus_window"))
    }

    fn bring_launcher_to_front(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("bring_launcher_to_front"))
    }
}

/// Abre um processo de forma portátil, sem esperar por ele.
pub fn spawn_process(request: &LaunchRequest) -> Result<LaunchedApp> {
    let mut cmd = std::process::Command::new(&request.executable);
    cmd.args(&request.args);
    let dir = request
        .working_dir
        .clone()
        .or_else(|| request.executable.parent().filter(|p| !p.as_os_str().is_empty()).map(Into::into));
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let child = cmd
        .spawn()
        .map_err(|e| SystemError::Failed(format!("{}: {e}", request.executable.display())))?;
    Ok(LaunchedApp { pid: child.id() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mode_accessors_and_text_round_trip() {
        let mode = DisplayMode::new(1920, 1080, 120);
        assert_eq!(mode.width(), 1920);
        assert_eq!(mode.height(), 1080);
        assert_eq!(mode.refresh_hz(), 120);
        assert_eq!(mode.to_string(), "1920x1080@120");
        assert_eq!("1920x1080@120".parse::<DisplayMode>(), Ok(mode));
        assert_eq!("2560X1440@ 144".parse::<DisplayMode>(), Ok(DisplayMode::new(2560, 1440, 144)));
        assert!("1920x1080".parse::<DisplayMode>().is_err());
        assert!("axb@c".parse::<DisplayMode>().is_err());
    }

    #[test]
    fn closest_mode_prefers_exact_size_and_nearest_refresh() {
        let modes = [
            DisplayMode::new(3440, 1440, 240),
            DisplayMode::new(3440, 1440, 144),
            DisplayMode::new(3440, 1440, 60),
            DisplayMode::new(1920, 1080, 60),
        ];
        assert_eq!(closest_mode(&modes, 3440, 1440, 165), Some(DisplayMode::new(3440, 1440, 144)));
        assert_eq!(closest_mode(&modes, 3440, 1440, 192), Some(DisplayMode::new(3440, 1440, 240)));
        assert_eq!(closest_mode(&modes, 1920, 1080, 144), Some(DisplayMode::new(1920, 1080, 60)));
        assert_eq!(closest_mode(&modes, 1280, 720, 60), None);
    }

    #[test]
    fn normalize_sorts_largest_first_and_dedups() {
        let modes = normalize_modes(vec![
            DisplayMode::new(1920, 1080, 60),
            DisplayMode::new(3440, 1440, 60),
            DisplayMode::new(1920, 1080, 60),
            DisplayMode::new(3440, 1440, 240),
        ]);
        assert_eq!(
            modes,
            vec![
                DisplayMode::new(3440, 1440, 240),
                DisplayMode::new(3440, 1440, 60),
                DisplayMode::new(1920, 1080, 60),
            ]
        );
    }

    #[test]
    fn spawn_reports_missing_executable() {
        let err = spawn_process(&LaunchRequest {
            executable: PathBuf::from("definitely-not-a-real-binary-bp"),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(err, SystemError::Failed(_)));
    }

    #[test]
    fn platform_backend_has_a_name() {
        assert!(!platform().backend_name().is_empty());
    }
}
