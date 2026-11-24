//! Adaptadores Linux. A detecção já escolhe o backend certo; as operações de monitor e janela
//! ainda respondem `Unsupported` e serão preenchidas com `x11rb` (RandR + EWMH) no X11 e com
//! `kscreen-doctor` / `org.kde.KWin` no KDE Wayland. Outros compositores Wayland não permitem
//! mudar modo nem focar janelas de outro app e continuam `Unsupported`.

use crate::{
    DisplayId, DisplayInfo, DisplayMode, LaunchRequest, LaunchedApp, Result, SystemControl, SystemError,
    UnsupportedSystem, WindowId, spawn_process,
};

/// Sessão gráfica detectada pelas variáveis de ambiente.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    X11,
    KdeWayland,
    OtherWayland,
    None,
}

pub fn detect_session(wayland_display: Option<&str>, x_display: Option<&str>, desktop: Option<&str>) -> Session {
    let is_kde = desktop.is_some_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("KDE")));
    match (wayland_display.filter(|v| !v.is_empty()), x_display.filter(|v| !v.is_empty())) {
        (Some(_), _) if is_kde => Session::KdeWayland,
        (Some(_), _) => Session::OtherWayland,
        (None, Some(_)) => Session::X11,
        (None, None) => Session::None,
    }
}

pub fn detect() -> Box<dyn SystemControl> {
    let var = |k| std::env::var(k).ok();
    match detect_session(
        var("WAYLAND_DISPLAY").as_deref(),
        var("DISPLAY").as_deref(),
        var("XDG_CURRENT_DESKTOP").as_deref(),
    ) {
        Session::X11 => Box::new(LinuxSystem { name: "x11" }),
        Session::KdeWayland => Box::new(LinuxSystem { name: "kde-wayland" }),
        Session::OtherWayland | Session::None => Box::new(UnsupportedSystem),
    }
}

/// Backend Linux (X11 ou KDE Wayland) com a interface pronta e a implementação pendente.
#[derive(Debug)]
pub struct LinuxSystem {
    name: &'static str,
}

impl SystemControl for LinuxSystem {
    fn backend_name(&self) -> &'static str {
        self.name
    }

    fn list_displays(&self) -> Result<Vec<DisplayInfo>> {
        Err(SystemError::Unsupported("list_displays (pendente: RandR / kscreen)"))
    }

    fn set_display_mode(&self, _: &DisplayId, _: DisplayMode) -> Result<()> {
        Err(SystemError::Unsupported("set_display_mode (pendente: RandR / kscreen)"))
    }

    fn restore_display_mode(&self, _: &DisplayId) -> Result<()> {
        Err(SystemError::Unsupported("restore_display_mode (pendente: RandR / kscreen)"))
    }

    fn launch_app(&self, request: &LaunchRequest) -> Result<LaunchedApp> {
        spawn_process(request)
    }

    fn windows_of_process(&self, _: u32) -> Result<Vec<WindowId>> {
        Err(SystemError::Unsupported("windows_of_process (pendente: _NET_WM_PID / KWin)"))
    }

    fn minimize_window(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("minimize_window (pendente: EWMH / KWin)"))
    }

    fn close_window(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("close_window (pendente: _NET_CLOSE_WINDOW / KWin)"))
    }

    fn focus_window(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("focus_window (pendente: _NET_ACTIVE_WINDOW / KWin)"))
    }

    fn bring_launcher_to_front(&self, _: WindowId) -> Result<()> {
        Err(SystemError::Unsupported("bring_launcher_to_front (pendente: _NET_ACTIVE_WINDOW / KWin)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_session_from_environment() {
        assert_eq!(detect_session(None, Some(":0"), Some("GNOME")), Session::X11);
        assert_eq!(detect_session(Some("wayland-0"), Some(":0"), Some("KDE")), Session::KdeWayland);
        assert_eq!(detect_session(Some("wayland-0"), None, Some("ubuntu:GNOME")), Session::OtherWayland);
        assert_eq!(detect_session(Some(""), Some(":1"), None), Session::X11);
        assert_eq!(detect_session(None, None, None), Session::None);
    }
}
