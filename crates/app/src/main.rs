mod benchmark;
mod cli;
mod texture_lru;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bigpicture_core::{CatalogEntry, GridMetrics};
use bigpicture_media::{
    COVER_HEIGHT, COVER_WIDTH, CacheOutcome, CoverCache, CoverLoaded, CoverLoader,
    ensure_demo_covers,
};
use bigpicture_providers::{CatalogProvider as _, LocalFolderCatalogProvider};
use gpui_fps::fps_monitor;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{Theme, ThemeMode, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use image::{Frame, RgbaImage};

use crate::benchmark::ScrollBenchmarkRecorder;
use crate::cli::CliArgs;
use crate::texture_lru::CoverTextureLru;
use crate::window::{SelectedMonitorInfo, build_borderless_fullscreen_options};

const TARGET_CARD_COUNT: usize = 1_000;
const GPU_COVER_LRU_CAPACITY: usize = 192;
const KEY_CONTEXT: &str = "BigPictureRoot";

actions!(bigpicture, [Quit]);

struct LauncherApp {
    focus_handle: FocusHandle,
    monitor_info: SelectedMonitorInfo,
    entries: Vec<CatalogEntry>,
    grid_metrics: GridMetrics,
    scroll_handle: VirtualListScrollHandle,
    cached_row_sizes: (usize, Rc<Vec<Size<Pixels>>>),
    cover_loader: CoverLoader,
    textures: RefCell<CoverTextureLru>,
    cache_hits: usize,
    cache_misses: usize,
    benchmark_recorder: Option<ScrollBenchmarkRecorder>,
}

impl LauncherApp {
    fn new(
        monitor_info: SelectedMonitorInfo,
        entries: Vec<CatalogEntry>,
        cover_loader: CoverLoader,
        cover_rx: std::sync::mpsc::Receiver<CoverLoaded>,
        benchmark_scroll: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let grid_metrics = GridMetrics::default();
        let initial_cols = grid_metrics.columns_for_width(monitor_info.bounds.size.width.as_f32());
        let initial_rows = grid_metrics.row_count(entries.len(), initial_cols);
        let row_sizes = Rc::new(vec![
            size(px(0.), px(grid_metrics.row_stride()));
            initial_rows
        ]);

        let benchmark_recorder =
            benchmark_scroll.then(|| ScrollBenchmarkRecorder::new(window.window_handle().window_id()));

        let shared_rx = Arc::new(Mutex::new(cover_rx));
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let rx_wait = Arc::clone(&shared_rx);
                let first = cx
                    .background_spawn(async move { rx_wait.lock().ok()?.recv().ok() })
                    .await;
                let Some(first) = first else {
                    break;
                };

                let mut batch = vec![first];
                if let Ok(guard) = shared_rx.lock() {
                    while batch.len() < 24 {
                        match guard.try_recv() {
                            Ok(next) => batch.push(next),
                            Err(_) => break,
                        }
                    }
                }

                if this
                    .update_in(cx, |app, window, cx| {
                        app.ingest_loaded_covers(batch, window);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        if benchmark_scroll {
            cx.spawn_in(window, async move |this, cx| {
                // Allow initial viewport covers to settle briefly, then sweep through all rows.
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;

                let mut step = 0usize;
                let total_steps = 220usize;
                while step <= total_steps {
                    let keep_going = this
                        .update_in(cx, |app, _window, cx| {
                            let cols = app.cached_row_sizes.0.max(1);
                            let rows = app.grid_metrics.row_count(app.entries.len(), cols);
                            let target_row =
                                ((step as f32 / total_steps as f32) * rows.saturating_sub(1) as f32)
                                    .round() as usize;
                            app.scroll_handle
                                .scroll_to_item(target_row, ScrollStrategy::Top);
                            if let Some(rec) = &mut app.benchmark_recorder {
                                rec.poll();
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !keep_going {
                        return;
                    }
                    step += 1;
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                }

                // Settle final frames and emit benchmark report.
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let _ = this.update_in(cx, |app, _window, cx| {
                    let backend = if gpui_kit::GPUI_FAST {
                        "gpui-fast"
                    } else {
                        "gpui-pre"
                    };
                    if let Some(rec) = &mut app.benchmark_recorder {
                        rec.print_report(backend, app.cache_hits, app.cache_misses);
                    }
                    cx.quit();
                });
            })
            .detach();
        }

        Self {
            focus_handle,
            monitor_info,
            entries,
            grid_metrics,
            scroll_handle: VirtualListScrollHandle::new(),
            cached_row_sizes: (initial_cols, row_sizes),
            cover_loader,
            textures: RefCell::new(CoverTextureLru::new(GPU_COVER_LRU_CAPACITY)),
            cache_hits: 0,
            cache_misses: 0,
            benchmark_recorder,
        }
    }

    fn ingest_loaded_covers(&mut self, batch: Vec<CoverLoaded>, window: &mut Window) {
        let mut lru = self.textures.borrow_mut();
        for item in batch {
            match item.outcome {
                CacheOutcome::Hit => self.cache_hits += 1,
                CacheOutcome::Miss => self.cache_misses += 1,
            }
            let width = item.frame.width();
            let height = item.frame.height();
            let bgra = item.frame.into_bgra_bytes();
            if let Some(img_buf) = RgbaImage::from_raw(width, height, bgra) {
                let render_image = Arc::new(RenderImage::new([Frame::new(img_buf)]));
                for evicted in lru.insert(item.id, render_image) {
                    let _ = window.drop_image(evicted);
                }
            }
        }
    }

    fn row_sizes_for_columns(&mut self, columns: usize) -> Rc<Vec<Size<Pixels>>> {
        let row_count = self.grid_metrics.row_count(self.entries.len(), columns);
        if self.cached_row_sizes.0 != columns || self.cached_row_sizes.1.len() != row_count {
            self.cached_row_sizes = (
                columns,
                Rc::new(vec![
                    size(px(0.), px(self.grid_metrics.row_stride()));
                    row_count
                ]),
            );
        }
        self.cached_row_sizes.1.clone()
    }

    fn render_card(&self, ix: usize) -> impl IntoElement {
        let entry = &self.entries[ix];
        let card_id = entry.id().as_str();
        let maybe_image = self.textures.borrow_mut().get(card_id);
        if maybe_image.is_none()
            && let Some(path) = entry.cover_path()
        {
            self.cover_loader.request(card_id, path.clone());
        }

        let card_w = px(self.grid_metrics.card_width);
        let cover_h = px(self.grid_metrics.cover_height);
        let footer_h = px(self.grid_metrics.footer_height);

        let cover_surface = match maybe_image {
            Some(render_img) => div()
                .w(card_w)
                .h(cover_h)
                .overflow_hidden()
                .bg(rgb(0x141a24))
                .child(
                    img(render_img)
                        .w(card_w)
                        .h(cover_h)
                        .object_fit(ObjectFit::Cover),
                ),
            None => {
                let seed_hue = ((ix * 47) % 360) as f32 / 360.0;
                div()
                    .w(card_w)
                    .h(cover_h)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(hsla(seed_hue, 0.35, 0.14, 1.0))
                    .text_size(px(12.))
                    .text_color(rgb(0x6e7681))
                    .child(format!("{COVER_WIDTH}×{COVER_HEIGHT}"))
            }
        };

        div()
            .id(SharedString::from(format!("cover-card-{}", entry.id().as_str())))
            .w(card_w)
            .h(px(self.grid_metrics.card_height()))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(8.))
            .bg(rgb(0x131924))
            .border_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.09))
            .hover(|style| {
                style
                    .border_color(rgb(0x58a6ff))
                    .bg(rgb(0x1b2434))
            })
            .child(cover_surface)
            .child(
                div()
                    .w_full()
                    .h(footer_h)
                    .px_2p5()
                    .py_1()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xe6edf3))
                            .truncate()
                            .child(entry.title().to_string()),
                    )
                    .when_some(entry.subtitle(), |el, sub| {
                        el.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(0x8b949e))
                                .truncate()
                                .child(sub.to_string()),
                        )
                    }),
            )
    }

    fn render_header(&self, columns: usize) -> impl IntoElement {
        let backend_badge = if gpui_kit::GPUI_FAST {
            "GPUI-FAST"
        } else {
            "GPUI-PRE"
        };
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
            .w_full()
            .h(px(72.))
            .px(px(self.grid_metrics.horizontal_padding))
            .flex()
            .items_center()
            .justify_between()
            .bg(hsla(220.0 / 360.0, 0.30, 0.06, 0.92))
            .border_b_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.08))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xf0f6fc))
                            .child("BIG PICTURE"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(px(4.))
                            .bg(hsla(212.0 / 360.0, 0.85, 0.55, 0.18))
                            .border_1()
                            .border_color(hsla(212.0 / 360.0, 0.85, 0.60, 0.35))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0x79c0ff))
                            .child(backend_badge),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0x8b949e))
                            .child(format!(
                                "{} jogos · {} cols · VRAM LRU {}/{} · Cache disco: {} hits / {} miss",
                                self.entries.len(),
                                columns,
                                self.textures.borrow().len(),
                                GPU_COVER_LRU_CAPACITY,
                                self.cache_hits,
                                self.cache_misses,
                            )),
                    ),
            )
            .child(
                div()
                    .pr(px(190.))
                    .text_size(px(12.))
                    .text_color(rgb(0x8b949e))
                    .child(format!(
                        "Monitor {}/{} ({}) · {}×{} @ {}",
                        self.monitor_info.index,
                        self.monitor_info.total_monitors.max(1),
                        display_label,
                        width,
                        height,
                        refresh,
                    )),
            )
    }

    fn on_quit(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }
}

impl Render for LauncherApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(rec) = &mut self.benchmark_recorder {
            rec.poll();
        }

        let viewport = window.viewport_size();
        let vw = if viewport.width > px(100.) {
            viewport.width.as_f32()
        } else {
            self.monitor_info.bounds.size.width.as_f32()
        };
        let columns = self.grid_metrics.columns_for_width(vw);
        let row_sizes = self.row_sizes_for_columns(columns);
        let col_gap = px(self.grid_metrics.col_gap);
        let horiz_pad = px(self.grid_metrics.horizontal_padding);
        let row_stride = px(self.grid_metrics.row_stride());

        let grid = v_virtual_list(
            cx.entity(),
            "library-cover-grid",
            row_sizes,
            move |this, visible_rows, _window, _cx| {
                visible_rows
                    .map(|row_ix| {
                        let start = row_ix * columns;
                        let end = (start + columns).min(this.entries.len());
                        let cards = (start..end).map(|card_ix| this.render_card(card_ix));

                        div()
                            .id(("cover-grid-row", row_ix))
                            .w_full()
                            .h(row_stride)
                            .px(horiz_pad)
                            .flex()
                            .items_start()
                            .gap(col_gap)
                            .children(cards)
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll_handle)
        .pt(px(24.))
        .pb(px(48.));

        div()
            .id("launcher-root")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_quit))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0a0d14))
            .text_color(rgb(0xe6edf3))
            .child(self.render_header(columns))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(grid)
                    .child(Scrollbar::vertical(&self.scroll_handle)),
            )
            .child(fps_monitor(window, cx))
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

    let covers_dir = cli
        .covers_dir()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/covers"));
    let cache_dir = cli
        .cache_dir()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/cache/covers_600x900"));

    ensure_demo_covers(&covers_dir, TARGET_CARD_COUNT)?;
    let provider = LocalFolderCatalogProvider::from_directory(&covers_dir, TARGET_CARD_COUNT)?;
    let entries = provider.entries();

    let cover_cache = CoverCache::new(&cache_dir)?;
    let worker_threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(2, 8))
        .unwrap_or(4);
    let (cover_loader, cover_rx) = CoverLoader::new(cover_cache, worker_threads);

    let benchmark_scroll = cli.benchmark_scroll();

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
                cx.new(|cx| {
                    LauncherApp::new(
                        monitor_info,
                        entries,
                        cover_loader,
                        cover_rx,
                        benchmark_scroll,
                        window,
                        cx,
                    )
                })
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
