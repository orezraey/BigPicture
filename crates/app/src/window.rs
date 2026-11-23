use std::rc::Rc;

use gpui_kit::*;

/// Summary of the display chosen to host the borderless fullscreen launcher window.
#[derive(Clone, Copy, Debug)]
pub struct SelectedMonitorInfo {
    pub index: usize,
    pub total_monitors: usize,
    pub display_id: Option<DisplayId>,
    pub bounds: Bounds<Pixels>,
    pub refresh_hz: Option<u32>,
}

/// Resolves which monitor index to pick given the number of available displays,
/// an optional `--monitor N` override, and the optional index of the primary display.
pub fn resolve_monitor_index(
    total_monitors: usize,
    requested: Option<usize>,
    primary_index: Option<usize>,
) -> Option<usize> {
    if total_monitors == 0 {
        return None;
    }

    if let Some(req) = requested {
        if req < total_monitors {
            return Some(req);
        }
        tracing::warn!(
            requested_monitor = req,
            available_monitors = total_monitors,
            "requested --monitor index is out of range; falling back to primary/first display"
        );
    }

    primary_index
        .filter(|&ix| ix < total_monitors)
        .or(Some(0))
}

/// Builds borderless fullscreen `WindowOptions` for the requested monitor.
///
/// Fullscreen in GPUI (`WindowBounds::Fullscreen` + `titlebar: None` +
/// `WindowDecorations::Client`) strips window borders and covers the monitor's
/// exact bounds without ever entering exclusive GPU fullscreen mode.
pub fn build_borderless_fullscreen_options(
    requested_monitor: Option<usize>,
    cx: &App,
) -> (WindowOptions, SelectedMonitorInfo) {
    let displays: Vec<Rc<dyn PlatformDisplay>> = cx.displays();
    let primary_id = cx.primary_display().map(|d| d.id());
    let primary_index = primary_id.and_then(|pid| displays.iter().position(|d| d.id() == pid));

    let chosen_index = resolve_monitor_index(displays.len(), requested_monitor, primary_index);
    let chosen_display = chosen_index.and_then(|ix| displays.get(ix));

    let fallback_bounds = Bounds::new(point(px(0.), px(0.)), size(px(1920.), px(1080.)));
    let bounds = chosen_display
        .map(|d| d.bounds())
        .unwrap_or(fallback_bounds);
    let display_id = chosen_display.map(|d| d.id());
    let refresh_hz = chosen_display
        .and_then(|d| d.refresh_interval())
        .and_then(|interval| {
            let secs = interval.as_secs_f64();
            (secs > 0.0).then(|| (1.0 / secs).round() as u32)
        });

    let info = SelectedMonitorInfo {
        index: chosen_index.unwrap_or(0),
        total_monitors: displays.len(),
        display_id,
        bounds,
        refresh_hz,
    };

    tracing::info!(
        monitor_index = info.index,
        total_monitors = info.total_monitors,
        ?display_id,
        ?bounds,
        ?refresh_hz,
        "opening borderless fullscreen launcher window"
    );

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Fullscreen(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: false,
        is_resizable: false,
        is_minimizable: true,
        display_id,
        window_background: WindowBackgroundAppearance::Opaque,
        app_id: Some("io.bigpicture.launcher".to_string()),
        window_decorations: Some(WindowDecorations::Client),
        inactive_frame_interval: None,
        low_power_frame_interval: None,
        ..WindowOptions::default()
    };

    (options, info)
}

#[cfg(test)]
mod tests {
    use super::resolve_monitor_index;

    #[test]
    fn resolves_requested_monitor_when_valid() {
        assert_eq!(resolve_monitor_index(3, Some(2), Some(0)), Some(2));
        assert_eq!(resolve_monitor_index(2, Some(0), Some(1)), Some(0));
    }

    #[test]
    fn falls_back_to_primary_or_first_when_out_of_range_or_omitted() {
        assert_eq!(resolve_monitor_index(2, Some(5), Some(1)), Some(1));
        assert_eq!(resolve_monitor_index(2, None, Some(1)), Some(1));
        assert_eq!(resolve_monitor_index(2, None, None), Some(0));
        assert_eq!(resolve_monitor_index(0, Some(0), None), None);
    }
}
