use gpui_kit::*;

struct LauncherPlaceholder;

impl Render for LauncherPlaceholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x0b0e14))
            .text_color(rgb(0xe6edf3))
            .child("BigPicture")
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_window, cx| {
                cx.new(|_cx| LauncherPlaceholder)
            })
            .expect("failed to open BigPicture window");
        });
}
