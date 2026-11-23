mod cli;
mod window;

use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::cli::CliArgs;
use crate::window::{SelectedMonitorInfo, build_borderless_fullscreen_options};

actions!(bigpicture, [Quit]);

const KEY_CONTEXT: &str = "BigPictureRoot";

struct LauncherApp {
    focus_handle: FocusHandle,
    monitor_info: SelectedMonitorInfo,
}

impl LauncherApp {
    fn new(monitor_info: SelectedMonitorInfo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            focus_handle,
            monitor_info,
        }
    }

    fn on_quit(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }
}

impl Render for LauncherApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.monitor_info.bounds.size.width.as_f32() as u32;
        let height = self.monitor_info.bounds.size.height.as_f32() as u32;
        let refresh = self
            .monitor_info
            .refresh_hz
            .map(|hz| format!("{hz} Hz"))
            .unwrap_or_else(|| "auto Hz".to_string());

        let display_label = self
            .monitor_info
            .display_id
            .map(|id| format!("{id:?}"))
            .unwrap_or_else(|| "default".to_string());

        div()
            .id("launcher-root")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_quit))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(rgb(0x0a0d14))
            .text_color(rgb(0xe6edf3))
            .child(
                div()
                    .text_size(px(32.))
                    .font_weight(FontWeight::BOLD)
                    .child("BIG PICTURE"),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(rgb(0x8b949e))
                    .child(format!(
                        "Monitor {}/{} ({}) · {}×{} · {} · Borderless Fullscreen",
                        self.monitor_info.index,
                        self.monitor_info.total_monitors.max(1),
                        display_label,
                        width,
                        height,
                        refresh,
                    )),
            )
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = CliArgs::from_env()?;

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);

            cx.bind_keys([
                KeyBinding::new("alt-f4", Quit, None),
                KeyBinding::new("escape", Quit, Some(KEY_CONTEXT)),
            ]);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            cx.activate(true);

            let (window_options, monitor_info) =
                build_borderless_fullscreen_options(cli.monitor(), cx);

            gpui_kit::open_window(window_options, cx, move |window, cx| {
                window.activate_window();
                window.set_window_title("BigPicture");
                cx.new(|cx| LauncherApp::new(monitor_info, window, cx))
            })
            .expect("failed to open BigPicture borderless fullscreen window");

            if let Some(exit_ms) = cli.exit_after_ms() {
                cx.spawn(async move |cx: &mut AsyncApp| {
                    cx.background_executor()
                        .timer(Duration::from_millis(exit_ms))
                        .await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });

    Ok(())
}
