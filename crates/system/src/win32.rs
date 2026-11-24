//! Backend Windows: GDI (`EnumDisplaySettingsW` / `ChangeDisplaySettingsExW`) para monitores e
//! User32 para janelas.
#![allow(unsafe_code)]

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{
    CDS_FULLSCREEN, CDS_TYPE, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
    DISPLAY_DEVICE_PRIMARY_DEVICE, DISPLAY_DEVICEW, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH,
    ChangeDisplaySettingsExW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplayDevicesW,
    EnumDisplaySettingsW,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SW_MINIMIZE, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, HSTRING, PCWSTR};

use crate::{
    DisplayId, DisplayInfo, DisplayMode, LaunchRequest, LaunchedApp, Result, SystemControl, SystemError,
    WindowId, closest_mode, normalize_modes, spawn_process,
};

#[derive(Debug, Default)]
pub struct WindowsSystem;

impl WindowsSystem {
    pub fn new() -> Self {
        Self
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn empty_devmode() -> DEVMODEW {
    DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    }
}

fn mode_of(dm: &DEVMODEW) -> DisplayMode {
    DisplayMode::new(dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency)
}

fn hwnd(window: WindowId) -> HWND {
    HWND(window.0 as usize as *mut core::ffi::c_void)
}

/// Monitores ligados à área de trabalho (`\\.\DISPLAYn`).
fn attached_devices() -> Vec<DISPLAY_DEVICEW> {
    let mut out = Vec::new();
    for index in 0.. {
        let mut device = DISPLAY_DEVICEW {
            cb: size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: `device` é um DISPLAY_DEVICEW válido com `cb` preenchido.
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
            break;
        }
        if device.StateFlags.contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP) {
            out.push(device);
        }
    }
    out
}

fn current_settings(name: &HSTRING) -> Option<DEVMODEW> {
    let mut dm = empty_devmode();
    // SAFETY: `dm` é um DEVMODEW válido com `dmSize` preenchido; `name` vive até o fim da chamada.
    unsafe { EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, &mut dm) }
        .as_bool()
        .then_some(dm)
}

fn offered_modes(name: &HSTRING) -> Vec<DisplayMode> {
    let mut modes = Vec::new();
    for index in 0.. {
        let mut dm = empty_devmode();
        // SAFETY: idem `current_settings`.
        if !unsafe { EnumDisplaySettingsW(name, ENUM_DISPLAY_SETTINGS_MODE(index), &mut dm) }.as_bool() {
            break;
        }
        if dm.dmDisplayFrequency > 1 {
            modes.push(mode_of(&dm));
        }
    }
    normalize_modes(modes)
}

fn change_settings(name: &HSTRING, devmode: Option<&DEVMODEW>, flags: CDS_TYPE) -> Result<()> {
    // SAFETY: ponteiros válidos durante a chamada; `None` pede o modo salvo no registro.
    let result = unsafe {
        ChangeDisplaySettingsExW(name, devmode.map(std::ptr::from_ref), None, flags, None)
    };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(SystemError::Failed(format!(
            "ChangeDisplaySettingsExW({name}) devolveu {}",
            result.0
        )))
    }
}

impl SystemControl for WindowsSystem {
    fn backend_name(&self) -> &'static str {
        "windows"
    }

    fn list_displays(&self) -> Result<Vec<DisplayInfo>> {
        let mut displays = Vec::new();
        for device in attached_devices() {
            let id = wide_to_string(&device.DeviceName);
            let name = HSTRING::from(id.as_str());
            let Some(current) = current_settings(&name) else { continue };
            // SAFETY: para monitores, o union de DEVMODEW guarda a posição em `Anonymous2`.
            let position = unsafe { current.Anonymous1.Anonymous2.dmPosition };
            displays.push(DisplayInfo {
                id: DisplayId(id),
                name: wide_to_string(&device.DeviceString),
                primary: device.StateFlags.contains(DISPLAY_DEVICE_PRIMARY_DEVICE),
                position: (position.x, position.y),
                current: mode_of(&current),
                modes: offered_modes(&name),
            });
        }
        Ok(displays)
    }

    fn set_display_mode(&self, display: &DisplayId, mode: DisplayMode) -> Result<()> {
        let name = HSTRING::from(display.0.as_str());
        let mut dm = current_settings(&name).ok_or_else(|| SystemError::DisplayNotFound(display.0.clone()))?;
        let offered = offered_modes(&name);
        let chosen = closest_mode(&offered, mode.width(), mode.height(), mode.refresh_hz())
            .filter(|m| *m == mode)
            .ok_or(SystemError::ModeNotAvailable(mode))?;
        dm.dmPelsWidth = chosen.width();
        dm.dmPelsHeight = chosen.height();
        dm.dmDisplayFrequency = chosen.refresh_hz();
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
        // CDS_FULLSCREEN: modo temporário, não vai para o registro; o Windows volta ao modo salvo
        // quando o processo termina.
        change_settings(&name, Some(&dm), CDS_FULLSCREEN)
    }

    fn restore_display_mode(&self, display: &DisplayId) -> Result<()> {
        let name = HSTRING::from(display.0.as_str());
        current_settings(&name).ok_or_else(|| SystemError::DisplayNotFound(display.0.clone()))?;
        change_settings(&name, None, CDS_TYPE(0))
    }

    fn launch_app(&self, request: &LaunchRequest) -> Result<LaunchedApp> {
        spawn_process(request)
    }

    fn windows_of_process(&self, pid: u32) -> Result<Vec<WindowId>> {
        struct Search {
            pid: u32,
            found: Vec<WindowId>,
        }
        unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` é o `&mut Search` passado abaixo, vivo durante `EnumWindows`.
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            let mut owner_pid = 0u32;
            // SAFETY: `window` vem do próprio EnumWindows.
            let _ = unsafe { GetWindowThreadProcessId(window, Some(&mut owner_pid)) };
            // Janelas principais: visíveis e sem dono (diálogos e tooltips têm dono).
            let top_level = unsafe { GetWindow(window, GW_OWNER) }.is_err();
            if owner_pid == search.pid && top_level && unsafe { IsWindowVisible(window) }.as_bool() {
                search.found.push(WindowId(window.0 as usize as u64));
            }
            BOOL(1)
        }
        let mut search = Search { pid, found: Vec::new() };
        // SAFETY: `visit` só usa `lparam` como `&mut Search` durante a chamada.
        unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) }
            .map_err(|e| SystemError::Failed(format!("EnumWindows: {e}")))?;
        Ok(search.found)
    }

    fn minimize_window(&self, window: WindowId) -> Result<()> {
        // SAFETY: HWND inválido só faz a chamada falhar.
        // ShowWindow devolve o estado anterior de visibilidade, não sucesso.
        let _ = unsafe { ShowWindow(hwnd(window), SW_MINIMIZE) };
        Ok(())
    }

    fn focus_window(&self, window: WindowId) -> Result<()> {
        let hwnd = hwnd(window);
        // SAFETY: idem.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd).as_bool() {
                Ok(())
            } else {
                Err(SystemError::Failed("SetForegroundWindow recusado".into()))
            }
        }
    }

    fn bring_launcher_to_front(&self, window: WindowId) -> Result<()> {
        if self.focus_window(window).is_ok() && is_foreground(window) {
            return Ok(());
        }
        let target = hwnd(window);
        // 1) Anexa a entrada à thread da janela da frente: o Windows então aceita a troca.
        // SAFETY: ids de thread vêm do próprio sistema; o anexo é desfeito logo depois.
        unsafe {
            let foreground = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(foreground, None);
            let me = GetCurrentThreadId();
            let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
            let _ = BringWindowToTop(target);
            let _ = SetForegroundWindow(target);
            if attached {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
        }
        if is_foreground(window) {
            return Ok(());
        }
        // 2) Um toque de Alt sintético conta como entrada do usuário e libera o foco.
        tap_alt();
        // SAFETY: idem.
        unsafe {
            let _ = SetForegroundWindow(target);
        }
        if is_foreground(window) {
            Ok(())
        } else {
            Err(SystemError::Failed("o Windows não deixou o launcher vir para a frente".into()))
        }
    }
}

fn is_foreground(window: WindowId) -> bool {
    // SAFETY: sem argumentos.
    let foreground = unsafe { GetForegroundWindow() };
    foreground == hwnd(window)
}

fn tap_alt() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_MENU,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    // SAFETY: `inputs` é um array válido de INPUT do tamanho informado.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_at_least_one_display_with_its_current_mode_offered() {
        let displays = WindowsSystem::new().list_displays().unwrap();
        assert!(!displays.is_empty(), "nenhum monitor ligado");
        let primary = displays.iter().find(|d| d.primary).unwrap_or(&displays[0]);
        assert!(primary.id.0.starts_with(r"\\.\DISPLAY"));
        assert!(primary.modes.contains(&primary.current), "o modo atual está entre os oferecidos");
    }

    #[test]
    fn rejects_mode_the_display_does_not_offer_without_touching_it() {
        let sys = WindowsSystem::new();
        let display = sys.list_displays().unwrap().remove(0);
        let err = sys.set_display_mode(&display.id, DisplayMode::new(123, 45, 67)).unwrap_err();
        assert_eq!(err, SystemError::ModeNotAvailable(DisplayMode::new(123, 45, 67)));
        assert_eq!(sys.list_displays().unwrap()[0].current, display.current);
    }

    #[test]
    fn finds_no_windows_for_a_process_that_does_not_exist() {
        assert!(WindowsSystem::new().windows_of_process(u32::MAX - 7).unwrap().is_empty());
    }
}
