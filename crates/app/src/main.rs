mod backdrop;
mod benchmark;
mod cli;
mod display_menu;
mod game_menu;
mod steam_sync;
mod texture_lru;
mod window;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bigpicture_core::{CatalogEntry, Direction, GridMetrics};
use bigpicture_input::NavCommand;
use bigpicture_system::{DisplayInfo, LaunchRequest, SystemControl, WindowId};
use bigpicture_media::{
    COVER_HEIGHT, COVER_WIDTH, CacheOutcome, CoverCache, CoverLoaded, CoverLoader,
    ensure_demo_covers, ensure_demo_trailers, ensure_telegram_cover,
};
use bigpicture_providers::epic::EpicLibrary;
use bigpicture_providers::steam::SteamLibrary;
use bigpicture_providers::steam_client::OwnedGame;
use bigpicture_providers::{CatalogProvider as _, InstalledAppsProvider, LocalFolderCatalogProvider};
use gpui_fps::fps_monitor;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{Theme, ThemeMode, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use image::{Frame, RgbaImage};

use crate::backdrop::TrailerBackdrop;
use crate::benchmark::ScrollBenchmarkRecorder;
use crate::cli::CliArgs;
use crate::display_menu::{DisplayMenu, RateOption};
use crate::game_menu::{GameAction, GameMenu, RunningGame};
use crate::texture_lru::CoverTextureLru;
use crate::window::{SelectedMonitorInfo, build_borderless_fullscreen_options};

const TARGET_CARD_COUNT: usize = 1_000;
const GPU_COVER_LRU_CAPACITY: usize = 192;
const KEY_CONTEXT: &str = "BigPictureRoot";
/// Quanto o foco precisa ficar parado num card para o trailer dele começar.
const TRAILER_FOCUS_DELAY: Duration = Duration::from_millis(450);
/// Quantos trailers demo gerar em `assets/trailers` quando a pasta estiver vazia.
const DEMO_TRAILER_COUNT: usize = 4;
/// Opacidade do trailer de fundo, para os cards continuarem legíveis.
const TRAILER_OPACITY: f32 = 0.35;

actions!(
    bigpicture,
    [Quit, NavUp, NavDown, NavLeft, NavRight, NavConfirm, NavBack, NavMenu, NavGuide]
);

/// Canais que alimentam a janela a partir de threads de fundo.
struct InputChannels {
    covers: std::sync::mpsc::Receiver<CoverLoaded>,
    /// `None` quando a thread do controle não pôde ser criada (só teclado).
    nav: Option<std::sync::mpsc::Receiver<NavCommand>>,
    /// Biblioteca completa da Steam lida do cliente em segundo plano.
    steam_library: Option<std::sync::mpsc::Receiver<Vec<OwnedGame>>>,
}

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
    /// Índice do card com o foco de navegação (controle e teclado).
    focused: usize,
    /// Steam instalada, para remontar os cards quando a biblioteca completa chegar.
    steam: Option<SteamLibrary>,
    /// Cards que vêm depois dos da Steam (Epic, apps instalados, demo).
    other_entries: Vec<CatalogEntry>,
    /// Quantos cards demo estão no fim de `other_entries` só porque não havia jogo na abertura;
    /// saem quando a biblioteca completa da Steam chega com jogos.
    fallback_demo_len: usize,
    /// Trailer do card focado tocando no fundo.
    backdrop: TrailerBackdrop,
    /// Desligado no `--benchmark-scroll`, para não mexer nos números de frame.
    trailers_enabled: bool,
    /// Muda a cada movimento de foco; o trailer só troca quando o foco para.
    focus_generation: u64,
    /// Monitores e janelas do sistema (backend da plataforma).
    system: Arc<dyn SystemControl>,
    /// Janela principal do launcher no sistema, achada na primeira vez que precisa voltar à frente.
    launcher_window: Option<WindowId>,
    /// `--demo-launch`: o que o Confirmar abre nos cards sem executável.
    demo_launch: Option<PathBuf>,
    /// Último jogo aberto pelo launcher, enquanto ainda tiver janela.
    running_game: Option<RunningGame>,
    /// Menu do jogo (Guide com um jogo aberto); enquanto aberto, a navegação vai para ele.
    game_menu: Option<GameMenu>,
    /// Tela de taxa de atualização (botão Menu).
    display_menu: Option<DisplayMenu>,
}

/// Opções de inicialização da janela vindas da linha de comando.
struct LaunchOptions {
    benchmark_scroll: bool,
    demo_launch: Option<PathBuf>,
    steam: Option<SteamLibrary>,
    other_entries: Vec<CatalogEntry>,
    fallback_demo_len: usize,
}

impl LauncherApp {
    fn new(
        monitor_info: SelectedMonitorInfo,
        entries: Vec<CatalogEntry>,
        cover_loader: CoverLoader,
        channels: InputChannels,
        options: LaunchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let LaunchOptions {
            benchmark_scroll,
            demo_launch,
            steam,
            other_entries,
            fallback_demo_len,
        } = options;
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

        if let Some(nav_rx) = channels.nav {
            let nav_rx = Arc::new(Mutex::new(nav_rx));
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    let rx = Arc::clone(&nav_rx);
                    let cmd = cx
                        .background_spawn(async move { rx.lock().ok()?.recv().ok() })
                        .await;
                    let Some(cmd) = cmd else {
                        break;
                    };
                    if this
                        .update_in(cx, |app, window, cx| app.handle_nav(cmd, window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        let (backdrop, trailer_rx) = TrailerBackdrop::new();
        let trailer_rx = Arc::new(Mutex::new(trailer_rx));
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let rx = Arc::clone(&trailer_rx);
                let frame = cx
                    .background_spawn(async move { rx.lock().ok()?.recv().ok() })
                    .await;
                let Some(frame) = frame else {
                    break;
                };
                if this
                    .update_in(cx, |app, window, cx| {
                        if app.backdrop.accept(frame, window) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        if let Some(library_rx) = channels.steam_library {
            let library_rx = Arc::new(Mutex::new(library_rx));
            cx.spawn_in(window, async move |this, cx| {
                let rx = Arc::clone(&library_rx);
                let owned = cx
                    .background_spawn(async move { rx.lock().ok()?.recv().ok() })
                    .await;
                if let Some(owned) = owned {
                    let _ = this.update_in(cx, |app, _window, cx| {
                        app.apply_steam_library(&owned, cx);
                        cx.notify();
                    });
                }
            })
            .detach();
        }

        let shared_rx = Arc::new(Mutex::new(channels.covers));
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

        let mut app = Self {
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
            focused: 0,
            steam,
            other_entries,
            fallback_demo_len,
            backdrop,
            trailers_enabled: !benchmark_scroll,
            focus_generation: 0,
            system: Arc::from(bigpicture_system::platform()),
            launcher_window: None,
            demo_launch,
            running_game: None,
            game_menu: None,
            display_menu: None,
        };
        app.play_focused_trailer();
        app
    }

    /// Troca os cards da Steam pela biblioteca completa lida do cliente, mantendo o foco no
    /// mesmo jogo quando ele continua na grade.
    fn apply_steam_library(&mut self, owned: &[OwnedGame], cx: &mut Context<Self>) {
        let Some(steam) = &self.steam else {
            return;
        };
        let focused_id = self.entries.get(self.focused).map(|e| e.id().clone());
        let mut entries = steam.entries_with_owned(owned, Path::new(steam_sync::COVER_CACHE));
        // As capas demo só estavam na grade por falta de jogos; agora há jogos.
        if !entries.is_empty() && self.fallback_demo_len > 0 {
            let keep = self.other_entries.len().saturating_sub(self.fallback_demo_len);
            self.other_entries.truncate(keep);
            self.fallback_demo_len = 0;
        }
        entries.extend(self.other_entries.iter().cloned());
        let previous = self.focused;
        self.focused = focused_id
            .and_then(|id| entries.iter().position(|e| *e.id() == id))
            .unwrap_or(0);
        tracing::info!(cards = entries.len(), "grade atualizada com a biblioteca completa da Steam");
        self.entries = entries;
        // O card focado pode ter mudado de posição (ou virado outro): leva a grade e o trailer junto.
        if self.focused != previous {
            let columns = self.cached_row_sizes.0.max(1);
            self.scroll_handle
                .scroll_to_item(self.focused / columns, ScrollStrategy::Center);
        }
        self.schedule_trailer(cx);
    }

    /// Toca o trailer do card focado (ou para o fundo, se ele não tiver trailer).
    fn play_focused_trailer(&mut self) {
        if !self.trailers_enabled {
            return;
        }
        let path = self
            .entries
            .get(self.focused)
            .and_then(CatalogEntry::trailer_path)
            .cloned();
        self.backdrop.play(path.as_deref());
    }

    /// Troca o trailer só depois que o foco fica parado por `TRAILER_FOCUS_DELAY`, para não
    /// abrir um pipeline a cada card ao segurar o direcional.
    fn schedule_trailer(&mut self, cx: &mut Context<Self>) {
        if !self.trailers_enabled {
            return;
        }
        self.focus_generation += 1;
        let generation = self.focus_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TRAILER_FOCUS_DELAY).await;
            let _ = this.update(cx, |app, _cx| {
                if app.focus_generation == generation {
                    app.play_focused_trailer();
                }
            });
        })
        .detach();
    }

    /// Aplica um comando de navegação vindo do controle (thread `bp-gamepad`) ou do teclado.
    fn handle_nav(&mut self, cmd: NavCommand, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.game_menu {
            match cmd {
                NavCommand::Up | NavCommand::Down | NavCommand::Left | NavCommand::Right => {
                    if let Some(dir) = cmd.direction() {
                        menu.step(dir);
                    }
                }
                NavCommand::Confirm => {
                    let action = menu.selected();
                    self.run_game_action(action);
                }
                NavCommand::Guide => self.run_game_action(GameAction::Resume),
                NavCommand::Back => self.game_menu = None,
                _ => {}
            }
            cx.notify();
            return;
        }
        if self.display_menu.is_some() && cmd != NavCommand::Guide {
            self.handle_display_menu(cmd, cx);
            cx.notify();
            return;
        }
        if let Some(dir) = cmd.direction() {
            self.move_focus(dir, cx);
            return;
        }
        match cmd {
            NavCommand::Menu => {
                self.open_display_menu();
                cx.notify();
            }
            NavCommand::Confirm => self.launch_focused(),
            NavCommand::Guide => {
                self.bring_launcher_back();
                if self.running_game.is_some() && !self.game_windows().is_empty() {
                    self.game_menu = Some(GameMenu::default());
                } else {
                    self.running_game = None;
                }
                cx.notify();
            }
            NavCommand::Back => {
                tracing::info!("voltar: nada para fechar na grade");
            }
            _ => {}
        }
    }

    /// Monitor em que o launcher está (pela origem da janela), senão o principal.
    fn launcher_display(&self) -> Option<DisplayInfo> {
        let displays = self
            .system
            .list_displays()
            .map_err(|err| tracing::warn!(error = %err, "não deu para listar os monitores"))
            .ok()?;
        let origin = self.monitor_info.bounds.origin;
        let at = (origin.x.as_f32() as i32, origin.y.as_f32() as i32);
        displays
            .iter()
            .find(|d| d.position == at)
            .or_else(|| displays.iter().find(|d| d.primary))
            .or(displays.first())
            .cloned()
    }

    fn open_display_menu(&mut self) {
        self.display_menu = self.launcher_display().map(DisplayMenu::new);
    }

    fn handle_display_menu(&mut self, cmd: NavCommand, cx: &mut Context<Self>) {
        let Some(menu) = &mut self.display_menu else {
            return;
        };
        if let Some(dir) = cmd.direction() {
            menu.step(dir);
            return;
        }
        let pending = menu.pending;
        match (cmd, pending) {
            (NavCommand::Confirm, Some(p)) => self.finish_rate_change(p.keep_selected),
            (NavCommand::Back, Some(_)) => self.finish_rate_change(false),
            (NavCommand::Back | NavCommand::Menu, None) => self.display_menu = None,
            (NavCommand::Confirm, None) => self.apply_selected_rate(cx),
            _ => {}
        }
    }

    fn apply_selected_rate(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = &mut self.display_menu else {
            return;
        };
        let id = menu.display.id.clone();
        match menu.selected_option() {
            RateOption::Mode(mode) if mode == menu.display.current => {}
            RateOption::Mode(mode) => match self.system.set_display_mode(&id, mode) {
                Ok(()) => {
                    tracing::info!(display = %id, %mode, "taxa aplicada; aguardando confirmação");
                    menu.start_confirmation(mode, Instant::now());
                    self.watch_rate_confirmation(cx);
                }
                Err(err) => tracing::warn!(display = %id, %mode, error = %err, "taxa não aplicada"),
            },
            RateOption::SystemDefault => {
                if let Err(err) = self.system.restore_display_mode(&id) {
                    tracing::warn!(display = %id, error = %err, "não deu para voltar ao padrão");
                }
                // Relê o modo atual depois da volta ao padrão.
                self.open_display_menu();
            }
        }
    }

    /// Fecha a confirmação; sem `keep`, volta à taxa anterior.
    fn finish_rate_change(&mut self, keep: bool) {
        let Some(menu) = &mut self.display_menu else {
            return;
        };
        let id = menu.display.id.clone();
        if let Some(previous) = menu.finish_confirmation(keep) {
            match self.system.set_display_mode(&id, previous) {
                Ok(()) => tracing::info!(display = %id, mode = %previous, "taxa anterior restaurada"),
                Err(err) => {
                    tracing::warn!(display = %id, error = %err, "falha ao voltar; usando o padrão do sistema");
                    let _ = self.system.restore_display_mode(&id);
                }
            }
        } else {
            tracing::info!(display = %id, "taxa mantida");
        }
    }

    /// Atualiza a contagem regressiva e volta sozinho quando o tempo acaba.
    fn watch_rate_confirmation(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let still_pending = this
                    .update(cx, |app, cx| {
                        let Some(pending) = app.display_menu.as_ref().and_then(|m| m.pending) else {
                            return false;
                        };
                        if pending.expired(Instant::now()) {
                            app.finish_rate_change(false);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !still_pending {
                    break;
                }
            }
        })
        .detach();
    }

    fn render_display_menu(&self) -> Option<impl IntoElement + use<>> {
        let menu = self.display_menu.as_ref()?;
        let button = |label: String, selected: bool| {
            div()
                .px(px(24.))
                .py(px(12.))
                .rounded(px(10.))
                .text_size(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .border_2()
                .border_color(if selected { rgb(0x58a6ff) } else { rgb(0x30363d) })
                .bg(if selected { rgb(0x1f3a5f) } else { rgb(0x161b22) })
                .text_color(rgb(0xf0f6fc))
                .child(label)
        };
        let current = menu.display.current;
        let (heading, body, hint) = match menu.pending {
            Some(p) => (
                format!("Manter {} Hz?", p.applied.refresh_hz()),
                div().flex().gap(px(16.)).children([
                    button("Manter".into(), p.keep_selected),
                    button(format!("Voltar para {} Hz", p.previous.refresh_hz()), !p.keep_selected),
                ]),
                format!(
                    "Voltando sozinho em {} s · A confirma · B volta",
                    p.seconds_left(Instant::now())
                ),
            ),
            None => (
                format!("Taxa de atualização · {}×{}", current.width(), current.height()),
                div().flex().gap(px(12.)).children(
                    menu.options
                        .iter()
                        .enumerate()
                        .map(|(ix, option)| {
                            let active = *option == RateOption::Mode(current);
                            let label = if active {
                                format!("{} (atual)", option.label())
                            } else {
                                option.label()
                            };
                            button(label, ix == menu.selected)
                        })
                        .collect::<Vec<_>>(),
                ),
                "A aplica · B ou Menu fecha · vale até o launcher fechar".to_string(),
            ),
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0.0, 0.0, 0.0, 0.6))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(24.))
                        .p(px(40.))
                        .rounded(px(16.))
                        .bg(rgb(0x0d1117))
                        .border_1()
                        .border_color(rgb(0x30363d))
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(rgb(0x8b949e))
                                .child(format!("{} · {}", menu.display.id, menu.display.name)),
                        )
                        .child(
                            div()
                                .text_size(px(28.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf0f6fc))
                                .child(heading),
                        )
                        .child(body)
                        .child(div().text_size(px(13.)).text_color(rgb(0x8b949e)).child(hint)),
                ),
        )
    }

    /// Janelas do jogo aberto (vazio se ele já fechou).
    fn game_windows(&self) -> Vec<WindowId> {
        let Some(game) = &self.running_game else {
            return Vec::new();
        };
        if game.pid != 0 {
            let by_pid = self.system.windows_of_process(game.pid).unwrap_or_default();
            if !by_pid.is_empty() {
                return by_pid;
            }
        }
        // Apps de instância única (Telegram): o processo aberto saiu e a janela é da instância
        // que já existia.
        if let Some(exe) = &game.executable {
            let by_exe = self.system.windows_of_executable(exe).unwrap_or_default();
            if !by_exe.is_empty() {
                return by_exe;
            }
        }
        // Jogos abertos pela loja: qualquer processo rodando de dentro da pasta de instalação.
        game.install_dir
            .as_ref()
            .and_then(|dir| self.system.windows_in_directory(dir).ok())
            .unwrap_or_default()
    }

    fn run_game_action(&mut self, action: GameAction) {
        self.game_menu = None;
        let windows = self.game_windows();
        if windows.is_empty() {
            self.running_game = None;
            return;
        }
        let result = match action {
            GameAction::Resume => self.system.bring_launcher_to_front(windows[0]),
            GameAction::Minimize => windows
                .iter()
                .try_for_each(|w| self.system.minimize_window(*w)),
            GameAction::Close => {
                let closed = windows.iter().try_for_each(|w| self.system.close_window(*w));
                if closed.is_ok() {
                    self.running_game = None;
                }
                closed
            }
        };
        match result {
            Ok(()) => tracing::info!(action = action.label(), "ação no jogo"),
            Err(err) => tracing::warn!(action = action.label(), error = %err, "ação no jogo falhou"),
        }
    }

    fn render_game_menu(&self) -> Option<impl IntoElement + use<>> {
        let menu = self.game_menu?;
        let title = self.running_game.as_ref()?.title.clone();
        let buttons = GameAction::ALL.into_iter().map(move |action| {
            let selected = menu.selected() == action;
            div()
                .px(px(28.))
                .py(px(14.))
                .rounded(px(10.))
                .text_size(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .border_2()
                .border_color(if selected {
                    rgb(0x58a6ff)
                } else {
                    rgb(0x30363d)
                })
                .bg(if selected { rgb(0x1f3a5f) } else { rgb(0x161b22) })
                .text_color(rgb(0xf0f6fc))
                .child(action.label())
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0.0, 0.0, 0.0, 0.6))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(24.))
                        .p(px(40.))
                        .rounded(px(16.))
                        .bg(rgb(0x0d1117))
                        .border_1()
                        .border_color(rgb(0x30363d))
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(rgb(0x8b949e))
                                .child("EM EXECUÇÃO"),
                        )
                        .child(
                            div()
                                .text_size(px(28.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf0f6fc))
                                .child(title),
                        )
                        .child(div().flex().gap(px(16.)).children(buttons))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(0x8b949e))
                                .child("A confirma · B fecha o menu · Guide volta ao jogo"),
                        ),
                ),
        )
    }

    /// Abre o jogo do card focado (ou o `--demo-launch`, nos cards demo).
    fn launch_focused(&mut self) {
        let Some(entry) = self.entries.get(self.focused) else {
            return;
        };
        // Jogos de loja abrem pela própria loja (atualizações, DRM, overlay da Steam).
        if let Some(uri) = entry.launch_uri() {
            match bigpicture_system::open_uri(uri) {
                Ok(()) => {
                    tracing::info!(title = entry.title(), uri, "jogo aberto pela loja");
                    self.running_game = Some(RunningGame {
                        pid: 0,
                        title: entry.title().to_string(),
                        executable: entry.executable().cloned(),
                        install_dir: entry.install_dir().cloned(),
                    });
                }
                Err(err) => tracing::warn!(title = entry.title(), error = %err, "falha ao abrir pela loja"),
            }
            return;
        }
        let Some(executable) = entry.executable().or(self.demo_launch.as_ref()).cloned() else {
            tracing::info!(
                title = entry.title(),
                "card sem executável (use --demo-launch EXE para testar)"
            );
            return;
        };
        let request = LaunchRequest {
            executable,
            args: entry.args().to_vec(),
            working_dir: None,
        };
        match self.system.launch_app(&request) {
            Ok(app) => {
                tracing::info!(title = entry.title(), pid = app.pid, "jogo aberto");
                self.running_game = Some(RunningGame {
                    pid: app.pid,
                    title: entry.title().to_string(),
                    executable: Some(request.executable.clone()),
                    install_dir: entry.install_dir().cloned(),
                });
            }
            Err(err) => tracing::warn!(title = entry.title(), error = %err, "falha ao abrir jogo"),
        }
    }

    /// Guide: traz o launcher de volta para a frente, mesmo com um jogo em primeiro plano.
    fn bring_launcher_back(&mut self) {
        let window = match self.launcher_window {
            Some(window) => window,
            None => match self.system.windows_of_process(std::process::id()) {
                Ok(windows) if !windows.is_empty() => {
                    self.launcher_window = Some(windows[0]);
                    windows[0]
                }
                Ok(_) => {
                    tracing::warn!("janela do launcher não encontrada");
                    return;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "não deu para achar a janela do launcher");
                    return;
                }
            },
        };
        match self.system.bring_launcher_to_front(window) {
            Ok(()) => tracing::info!("launcher trazido para a frente"),
            Err(err) => tracing::warn!(error = %err, "launcher não veio para a frente"),
        }
    }

    /// Move o foco na grade e rola para deixar a linha focada no centro da área visível.
    fn move_focus(&mut self, dir: Direction, cx: &mut Context<Self>) {
        let columns = self.cached_row_sizes.0.max(1);
        let Some(next) = self
            .grid_metrics
            .navigate(self.focused, columns, self.entries.len(), dir)
        else {
            return;
        };
        self.focused = next;
        self.scroll_handle
            .scroll_to_item(next / columns, ScrollStrategy::Center);
        self.schedule_trailer(cx);
        cx.notify();
    }

    fn on_nav_up(&mut self, _: &NavUp, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Up, window, cx);
    }

    fn on_nav_down(&mut self, _: &NavDown, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Down, window, cx);
    }

    fn on_nav_left(&mut self, _: &NavLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Left, window, cx);
    }

    fn on_nav_right(&mut self, _: &NavRight, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Right, window, cx);
    }

    fn on_nav_confirm(&mut self, _: &NavConfirm, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Confirm, window, cx);
    }

    fn on_nav_back(&mut self, _: &NavBack, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Back, window, cx);
    }

    fn on_nav_menu(&mut self, _: &NavMenu, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Menu, window, cx);
    }

    fn on_nav_guide(&mut self, _: &NavGuide, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_nav(NavCommand::Guide, window, cx);
    }

    fn ingest_loaded_covers(&mut self, batch: Vec<CoverLoaded>, window: &mut Window) {
        let ingest_start = std::time::Instant::now();
        let covers = batch.len();
        self.ingest_batch(batch, window);
        // Roda fora de `Window::draw`; medido à parte para comparar com o Rune, cujo `poll`
        // (onde as capas chegam) entra no `update_ms`.
        if let Some(rec) = &mut self.benchmark_recorder {
            rec.record_ingest(ingest_start.elapsed(), covers);
        }
    }

    fn ingest_batch(&mut self, batch: Vec<CoverLoaded>, window: &mut Window) {
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
            match entry.cover_url() {
                Some(url) => self.cover_loader.request_remote(card_id, path.clone(), url),
                None => self.cover_loader.request(card_id, path.clone()),
            }
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
            .when(ix == self.focused, |card| {
                card.border_2()
                    .border_color(rgb(0x58a6ff))
                    .bg(rgb(0x1b2434))
            })
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
            .on_action(cx.listener(Self::on_nav_up))
            .on_action(cx.listener(Self::on_nav_down))
            .on_action(cx.listener(Self::on_nav_left))
            .on_action(cx.listener(Self::on_nav_right))
            .on_action(cx.listener(Self::on_nav_confirm))
            .on_action(cx.listener(Self::on_nav_back))
            .on_action(cx.listener(Self::on_nav_menu))
            .on_action(cx.listener(Self::on_nav_guide))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0a0d14))
            .text_color(rgb(0xe6edf3))
            // Trailer do card focado atrás de tudo (filho absoluto desenhado primeiro).
            .when_some(self.backdrop.image(), |root, frame| {
                root.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(TRAILER_OPACITY)
                        .child(img(frame).size_full().object_fit(ObjectFit::Cover)),
                )
            })
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
            .children(self.render_display_menu())
            .children(self.render_game_menu())
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
    // Sem GStreamer (ou no benchmark), o launcher segue sem trailers.
    let trailer_files = if cli.benchmark_scroll() {
        Vec::new()
    } else {
        ensure_demo_trailers(Path::new("assets/trailers"), DEMO_TRAILER_COUNT).unwrap_or_else(
            |err| {
                tracing::warn!(error = %err, "trailers desligados");
                Vec::new()
            },
        )
    };
    let provider = LocalFolderCatalogProvider::from_directory(&covers_dir, TARGET_CARD_COUNT)?
        .with_trailer_files(trailer_files);
    // Biblioteca real (Steam, Epic) e apps instalados (Telegram). O benchmark usa só as 1.000
    // capas demo; fora dele, as capas demo só entram quando não há nenhum jogo instalado (ou com
    // --demo-catalog).
    let mut entries = Vec::new();
    let mut steam_entries_len = 0;
    let mut fallback_demo_len = 0;
    let steam = (!cli.benchmark_scroll()).then(SteamLibrary::detect).flatten();
    let (library_tx, library_rx) = std::sync::mpsc::channel();
    if !cli.benchmark_scroll() {
        if let Some(steam) = &steam {
            // A última biblioteca completa (cache) aparece na hora; a leitura do cliente atualiza
            // a grade quando termina.
            match steam_sync::load_cached() {
                Some(owned) => entries.extend(steam.entries_with_owned(&owned, Path::new(steam_sync::COVER_CACHE))),
                None => entries.extend(steam.entries()),
            }
            steam_entries_len = entries.len();
            steam_sync::spawn(steam.root().to_path_buf(), library_tx);
        }
        if let Some(epic) = EpicLibrary::detect() {
            entries.extend(epic.entries());
        }
        tracing::info!(games = entries.len(), "biblioteca real carregada");
        let games_found = !entries.is_empty();
        let cover = ensure_telegram_cover(Path::new("assets/apps/telegram_cover.png"))
            .map_err(|err| tracing::warn!(error = %err, "capa do Telegram não gerada"))
            .ok();
        for app in InstalledAppsProvider::detect().entries() {
            entries.push(match &cover {
                Some(path) => app.with_cover_path(path.clone()),
                None => app,
            });
        }
        if !games_found || cli.demo_catalog() {
            let before = entries.len();
            entries.extend(provider.entries());
            if !games_found && !cli.demo_catalog() {
                fallback_demo_len = entries.len() - before;
            }
        }
    } else {
        entries.extend(provider.entries());
    }

    let cover_cache = CoverCache::new(&cache_dir)?;
    let worker_threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(2, 8))
        .unwrap_or(4);
    let (cover_loader, cover_rx) = CoverLoader::new(cover_cache, worker_threads);

    let options = LaunchOptions {
        benchmark_scroll: cli.benchmark_scroll(),
        demo_launch: cli.demo_launch().map(Path::to_path_buf),
        other_entries: entries[steam_entries_len..].to_vec(),
        fallback_demo_len,
        steam,
    };

    let (nav_tx, nav_rx) = std::sync::mpsc::channel();
    let nav_rx = match bigpicture_input::gamepad::spawn(nav_tx) {
        Ok(_) => Some(nav_rx),
        Err(err) => {
            tracing::warn!(error = %err, "falha ao criar a thread do controle; só teclado");
            None
        }
    };
    let channels = InputChannels {
        covers: cover_rx,
        nav: nav_rx,
        steam_library: options.steam.is_some().then_some(library_rx),
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);

            cx.bind_keys([
                KeyBinding::new("alt-f4", Quit, None),
                KeyBinding::new("escape", Quit, Some(KEY_CONTEXT)),
                KeyBinding::new("up", NavUp, Some(KEY_CONTEXT)),
                KeyBinding::new("down", NavDown, Some(KEY_CONTEXT)),
                KeyBinding::new("left", NavLeft, Some(KEY_CONTEXT)),
                KeyBinding::new("right", NavRight, Some(KEY_CONTEXT)),
                KeyBinding::new("enter", NavConfirm, Some(KEY_CONTEXT)),
                KeyBinding::new("backspace", NavBack, Some(KEY_CONTEXT)),
                // Equivalente de teclado ao Guide (só chega quando o launcher tem o foco).
                KeyBinding::new("home", NavGuide, Some(KEY_CONTEXT)),
                KeyBinding::new("m", NavMenu, Some(KEY_CONTEXT)),
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
                        channels,
                        options,
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
