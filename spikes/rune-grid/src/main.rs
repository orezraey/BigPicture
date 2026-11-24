//! Spike: a mesma grade virtualizada de 1.000 capas 600×900 do `bigpicture-app`, em Rune.
//!
//! Reaproveita `bigpicture-core` (GridMetrics), `bigpicture-providers` (catálogo local) e
//! `bigpicture-media` (CoverLoader + cache QOI em disco), então decodificação, cache e catálogo
//! são exatamente os mesmos do app GPUI. Só a camada de UI muda.
//!
//! `cargo run --release -- --benchmark-scroll [--mode stepped|stepped-commit] [--monitor N]
//!   [--covers-dir DIR] [--cache-dir DIR]`

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use bigpicture_core::{CatalogEntry, GridMetrics};
use bigpicture_media::{
    COVER_HEIGHT, COVER_WIDTH, CacheOutcome, CoverCache, CoverLoaded, CoverLoader,
    ensure_demo_covers,
};
use bigpicture_providers::{CatalogProvider, LocalFolderCatalogProvider};
use rune::tok::TextStyle;
use rune::{
    AppSpec, Color, Cx, El, FrameEncoded, FrameOutcome, GrowthSpec, Pixels, ScrollSpec,
    SurfaceFit, Tx, Waker, WindowOptions, WindowSpec, col, el, row, text,
};

const TARGET_CARD_COUNT: usize = 1_000;
/// Mesmo limite do `CoverTextureLru` do app GPUI.
const GPU_COVER_LRU_CAPACITY: usize = 192;
/// Igual ao `WARMUP_DRAWS` do `crates/app/src/benchmark.rs`.
const WARMUP_DRAWS: usize = 8;
/// Igual ao driver do GPUI: 250 ms de espera, 221 passos a cada 16 ms, 200 ms de assentamento.
const SETTLE_START: f64 = 0.250;
const STEP_INTERVAL: f64 = 0.016;
const TOTAL_STEPS: usize = 220;
const SETTLE_END: f64 = 0.200;
const HEADER_H: f32 = 72.0;
const LIST_PAD_TOP: f32 = 24.0;
const LIST_PAD_BOTTOM: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Como se escreve naturalmente em Rune: o scroll instantâneo não reexecuta a view; a view só
    /// roda de novo quando chegam capas.
    Stepped,
    /// Força a view a rodar de novo a cada passo, como o `cx.notify()` do driver do GPUI.
    SteppedCommit,
}

struct Cli {
    monitor: Option<usize>,
    covers_dir: PathBuf,
    cache_dir: PathBuf,
    benchmark: bool,
    mode: Mode,
}

fn parse_cli() -> Result<Cli> {
    let mut cli = Cli {
        monitor: None,
        covers_dir: PathBuf::from("assets/covers"),
        cache_dir: PathBuf::from("assets/cache/covers_600x900"),
        benchmark: false,
        mode: Mode::Stepped,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--monitor" => cli.monitor = Some(args.next().context("--monitor N")?.parse()?),
            "--covers-dir" => cli.covers_dir = args.next().context("--covers-dir DIR")?.into(),
            "--cache-dir" => cli.cache_dir = args.next().context("--cache-dir DIR")?.into(),
            "--benchmark-scroll" => cli.benchmark = true,
            "--mode" => {
                cli.mode = match args.next().as_deref() {
                    Some("stepped") => Mode::Stepped,
                    Some("stepped-commit") => Mode::SteppedCommit,
                    other => bail!("--mode stepped|stepped-commit, got {other:?}"),
                }
            }
            other => bail!("unrecognized argument: {other}"),
        }
    }
    Ok(cli)
}

/// LRU de handles `Pixels`: soltar o último handle libera a página de imagem do Rune, como o
/// `window.drop_image` do GPUI.
struct PixelsLru {
    map: HashMap<String, Pixels>,
    order: VecDeque<String>,
}

impl PixelsLru {
    fn get(&mut self, id: &str) -> Option<Pixels> {
        let px = self.map.get(id)?.clone();
        if self.order.back().map(String::as_str) != Some(id) {
            self.order.retain(|k| k != id);
            self.order.push_back(id.to_string());
        }
        Some(px)
    }

    fn insert(&mut self, id: String, px: Pixels) {
        if self.map.insert(id.clone(), px).is_some() {
            self.order.retain(|k| k != &id);
        }
        self.order.push_back(id);
        while self.map.len() > GPU_COVER_LRU_CAPACITY {
            let Some(old) = self.order.pop_front() else { break };
            self.map.remove(&old);
        }
    }
}

struct Bench {
    mode: Mode,
    next_step_at: f64,
    step: usize,
    done_at: Option<f64>,
}

struct Grid {
    entries: Vec<CatalogEntry>,
    metrics: GridMetrics,
    columns: usize,
    loader: CoverLoader,
    loader_rx: Option<Receiver<CoverLoaded>>,
    covers_rx: Option<Receiver<CoverLoaded>>,
    lru: RefCell<PixelsLru>,
    hits: usize,
    misses: usize,
    bench: Option<Bench>,
}

#[derive(Clone, Copy)]
struct Sample {
    update_ms: f64,
    encode_ms: f64,
    started: std::time::Instant,
    /// Parte do `update_ms` gasta no `poll` recebendo capas (cópia + `Pixels::set` + LRU).
    ingest_ms: f64,
    covers: usize,
    stepped: bool,
}

/// O que o `poll` deste frame fez, lido pelo `on_frame_encoded` do mesmo frame.
#[derive(Clone, Copy, Default)]
struct FrameWork {
    ingest_ms: f64,
    covers: usize,
    stepped: bool,
}

thread_local! {
    static SAMPLES: RefCell<(usize, Vec<Sample>)> = const { RefCell::new((0, Vec::new())) };
    static PENDING: std::cell::Cell<FrameWork> = std::cell::Cell::new(FrameWork::default());
}

fn note_work(f: impl FnOnce(&mut FrameWork)) {
    PENDING.with(|p| {
        let mut w = p.get();
        f(&mut w);
        p.set(w);
    });
}

fn record_frame(f: &FrameEncoded) {
    if f.outcome() != FrameOutcome::Drawn {
        return;
    }
    let (Some(update_ms), Some(encode_ms), Some(started)) = (f.update_ms(), f.encode_ms(), f.started())
    else {
        return;
    };
    let work = PENDING.with(|p| p.take());
    SAMPLES.with_borrow_mut(|(seen, v)| {
        *seen += 1;
        if *seen > WARMUP_DRAWS {
            v.push(Sample {
                update_ms,
                encode_ms,
                started,
                ingest_ms: work.ingest_ms,
                covers: work.covers,
                stepped: work.stepped,
            });
        }
    });
}

fn hex(s: &str) -> Color {
    rune::color::hex(s).expect("valid hex color")
}

fn setup(s: &mut Grid, waker: Waker) {
    // Encaminha os resultados das worker threads e acorda a janela, como o `cx.spawn_in` do GPUI.
    let Some(rx) = s.loader_rx.take() else { return };
    let (tx, covers_rx) = mpsc::channel();
    s.covers_rx = Some(covers_rx);
    std::thread::Builder::new()
        .name("rune-grid-cover-forward".into())
        .spawn(move || {
            while let Ok(item) = rx.recv() {
                if tx.send(item).is_err() {
                    break;
                }
                waker();
            }
        })
        .expect("spawn forwarder");
}

fn poll(s: &mut Grid, t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(rx) = &s.covers_rx {
        let ingest_start = std::time::Instant::now();
        let mut covers = 0;
        let mut lru = s.lru.borrow_mut();
        while let Ok(item) = rx.try_recv() {
            match item.outcome {
                CacheOutcome::Hit => s.hits += 1,
                CacheOutcome::Miss => s.misses += 1,
            }
            let (w, h) = (item.frame.width(), item.frame.height());
            let px = Pixels::new();
            // RGBA straight-alpha sRGB, sem mipmaps (paridade com `RenderImage` do GPUI).
            px.set(w, h, item.frame.pixels().to_vec());
            lru.insert(item.id, px);
            changed = true;
            covers += 1;
        }
        if covers > 0 {
            let ms = ingest_start.elapsed().as_secs_f64() * 1000.0;
            note_work(|w| {
                w.ingest_ms += ms;
                w.covers += covers;
            });
        }
    }

    let rows = s.metrics.row_count(s.entries.len(), s.columns);
    if let Some(b) = &mut s.bench {
        if let Some(done_at) = b.done_at {
            if t >= done_at {
                print_report(b.mode, s.hits, s.misses);
                tx.quit();
            } else {
                tx.wake_after(Duration::from_secs_f64(done_at - t));
            }
        } else if t >= b.next_step_at {
            let target_row = ((b.step as f32 / TOTAL_STEPS as f32) * rows.saturating_sub(1) as f32)
                .round() as usize;
            // Equivalente a `scroll_to_item(row, ScrollStrategy::Top)` do GPUI.
            tx.scroll_key_to(format!("row-{target_row}"), 0.0);
            note_work(|w| w.stepped = true);
            if b.mode == Mode::SteppedCommit {
                changed = true;
            }
            b.step += 1;
            if b.step > TOTAL_STEPS {
                b.done_at = Some(t + SETTLE_END);
                tx.wake_after(Duration::from_secs_f64(SETTLE_END));
            } else {
                b.next_step_at = t + STEP_INTERVAL;
                tx.wake_after(Duration::from_secs_f64(STEP_INTERVAL));
            }
        } else {
            tx.wake_after(Duration::from_secs_f64(b.next_step_at - t));
        }
    }
    changed
}

fn print_report(mode: Mode, hits: usize, misses: usize) {
    SAMPLES.with_borrow(|(_, v)| {
        let mode = match mode {
            Mode::Stepped => "stepped",
            Mode::SteppedCommit => "stepped-commit",
        };
        if v.is_empty() {
            println!("BENCHMARK_RESULT backend=rune mode={mode} samples=0");
            return;
        }
        let stats = |mut xs: Vec<f64>| {
            xs.sort_by(f64::total_cmp);
            let n = xs.len();
            let pct = |p: f64| xs[((p * (n - 1) as f64).round() as usize).min(n - 1)];
            let mean = xs.iter().sum::<f64>() / n as f64;
            format!(
                "mean_ms={mean:.3} min_ms={:.3} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3}",
                xs[0],
                pct(0.50),
                pct(0.95),
                pct(0.99),
                xs[n - 1]
            )
        };
        let update: Vec<f64> = v.iter().map(|s| s.update_ms).collect();
        let encode: Vec<f64> = v.iter().map(|s| s.encode_ms).collect();
        let total: Vec<f64> = v.iter().map(|s| s.update_ms + s.encode_ms).collect();
        // Mesmo critério do relatório do GPUI: só a fase equivalente a `Window::draw`.
        let over_240 = update.iter().filter(|&&ms| ms > 1000.0 / 240.0).count();
        let span = v.last().unwrap().started.duration_since(v[0].started).as_secs_f64();
        println!(
            "BENCHMARK_RESULT backend=rune mode={mode} samples={} span_s={span:.2} cache_hits={hits} cache_misses={misses} over_4_17ms={over_240}",
            v.len()
        );
        println!("  update (view+layout+paint ~ GPUI draw): {}", stats(update));
        println!("  encode (upload+prepare+submit ~ GPUI present): {}", stats(encode));
        println!("  update+encode: {}", stats(total));
        let ex: Vec<f64> = v.iter().map(|s| s.update_ms - s.ingest_ms).collect();
        println!("  update sem ingestao de capas: {}", stats(ex));
        let ingest_total: f64 = v.iter().map(|s| s.ingest_ms).sum();
        let covers: usize = v.iter().map(|s| s.covers).sum();
        let with_covers = v.iter().filter(|s| s.covers > 0).count();
        let with_step = v.iter().filter(|s| s.stepped).count();
        let neither = v.iter().filter(|s| s.covers == 0 && !s.stepped).count();
        println!(
            "  ingest: total_ms={ingest_total:.1} covers={covers} frames_com_capas={with_covers} frames_com_passo={with_step} frames_sem_nenhum={neither}"
        );
        let slow: Vec<&Sample> = v.iter().filter(|s| s.update_ms > 1000.0 / 240.0).collect();
        println!(
            "  frames >4,17ms: total={} com_capas={} com_passo={} sem_nenhum={} capas_por_frame_lento={:.1}",
            slow.len(),
            slow.iter().filter(|s| s.covers > 0).count(),
            slow.iter().filter(|s| s.stepped).count(),
            slow.iter().filter(|s| s.covers == 0 && !s.stepped).count(),
            slow.iter().map(|s| s.covers).sum::<usize>() as f64 / slow.len().max(1) as f64,
        );
    });
}

fn card(s: &Grid, ix: usize) -> El {
    let entry = &s.entries[ix];
    let id = entry.id().as_str();
    // Pedidos de decodificação saem de `on_visible_rows` (só linhas visíveis ±1): o Rune monta
    // ~2 viewports de overscan por lado, mais cards do que cabem no LRU de 192.
    let maybe_px = s.lru.borrow_mut().get(id);
    let m = &s.metrics;
    let cover = match maybe_px {
        Some(px) => el()
            .size(m.card_width, m.cover_height)
            .fill(hex("#141a24"))
            .pixels_fit(&px, SurfaceFit::Fill),
        None => {
            let hue = ((ix * 47) % 360) as f32;
            el().size(m.card_width, m.cover_height)
                .center()
                .fill(rune::oklch(0.25, 0.04, hue))
                .child(
                    text(format!("{COVER_WIDTH}×{COVER_HEIGHT}"))
                        .font(TextStyle::new(12.0, 400.0))
                        .color(hex("#6e7681")),
                )
        }
    };
    let mut footer = col()
        .w_full()
        .h(m.footer_height)
        .px(10.0)
        .py(4.0)
        .center_y()
        .child(
            text(entry.title().to_string())
                .font(TextStyle::new(13.0, 600.0))
                .color(hex("#e6edf3"))
                .truncate(),
        );
    if let Some(sub) = entry.subtitle() {
        footer = footer.child(
            text(sub.to_string())
                .font(TextStyle::new(11.0, 400.0))
                .color(hex("#8b949e"))
                .truncate(),
        );
    }
    col()
        .key(format!("card-{id}"))
        .size(m.card_width, m.card_height())
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(hex("#131924"))
        .border(1.0, Color::WHITE.alpha(0.09))
        .hover(hex("#58a6ff").alpha(0.10))
        .child(cover)
        .child(footer)
}

fn request_visible(s: &mut Grid, rows: std::ops::Range<usize>) -> bool {
    let first = rows.start.saturating_sub(1) * s.columns;
    let last = ((rows.end + 1) * s.columns).min(s.entries.len());
    let mut lru = s.lru.borrow_mut();
    for entry in &s.entries[first.min(last)..last] {
        if lru.get(entry.id().as_str()).is_none()
            && let Some(path) = entry.cover_path()
        {
            s.loader.request(entry.id().as_str(), path.clone());
        }
    }
    false
}

fn grid_row(s: &Grid, row_ix: usize) -> El {
    let start = row_ix * s.columns;
    let end = (start + s.columns).min(s.entries.len());
    (start..end).fold(
        row()
            .w_full()
            .h(s.metrics.row_stride())
            .px(s.metrics.horizontal_padding)
            .gap(s.metrics.col_gap),
        |r, ix| r.child(card(s, ix)),
    )
}

fn header(s: &Grid) -> El {
    row()
        .w_full()
        .h(HEADER_H)
        .px(s.metrics.horizontal_padding)
        .center_y()
        .gap(12.0)
        .fill(hex("#0b0f17").alpha(0.92))
        .child(text("BIG PICTURE").font(TextStyle::new(22.0, 700.0)).color(hex("#f0f6fc")))
        .child(
            el().px(8.0)
                .py(2.0)
                .radius(4.0)
                .fill(hex("#2f81f7").alpha(0.18))
                .border(1.0, hex("#388bfd").alpha(0.35))
                .child(text("RUNE").font(TextStyle::new(11.0, 600.0)).color(hex("#79c0ff"))),
        )
        .child(
            text(format!(
                "{} jogos · {} cols · VRAM LRU {}/{} · Cache disco: {} hits / {} miss",
                s.entries.len(),
                s.columns,
                s.lru.borrow().map.len(),
                GPU_COVER_LRU_CAPACITY,
                s.hits,
                s.misses
            ))
            .font(TextStyle::new(12.0, 400.0))
            .color(hex("#8b949e")),
        )
}

fn view(s: &Grid, cx: &mut Cx<Grid>) -> El {
    let _ = cx;
    let rows = s.metrics.row_count(s.entries.len(), s.columns);
    let list = col()
        .key("grid-rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            rows,
            s.metrics.row_stride(),
            |_: &Grid, i| format!("row-{i}").into(),
            grid_row,
        )
        .on_visible_rows(request_visible);
    col()
        .w_full()
        .h_full()
        .fill(hex("#0a0d14"))
        .child(header(s))
        .child(
            col()
                .key("grid")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(
                    col()
                        .w_full()
                        .pt(LIST_PAD_TOP)
                        .pb(LIST_PAD_BOTTOM)
                        .child(list),
                ),
        )
}

fn main() -> Result<()> {
    let cli = parse_cli()?;
    ensure_demo_covers(&cli.covers_dir, TARGET_CARD_COUNT)?;
    let provider = LocalFolderCatalogProvider::from_directory(&cli.covers_dir, TARGET_CARD_COUNT)?;
    let entries = provider.entries();
    let cache = CoverCache::new(&cli.cache_dir)?;
    // Mesmo número de workers do app GPUI.
    let workers = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(2, 8))
        .unwrap_or(4);
    let (loader, loader_rx) = CoverLoader::new(cache, workers);
    let (benchmark, mode, monitor) = (cli.benchmark, cli.mode, cli.monitor);

    rune::run_windows(move |displays| {
        let display = monitor
            .and_then(|ix| displays.get(ix))
            .or_else(|| displays.iter().find(|d| d.primary))
            .or_else(|| displays.first())
            .cloned();
        let metrics = GridMetrics::default();
        let logical_w = display
            .as_ref()
            .map(|d| d.size.0 as f32 / d.scale as f32)
            .unwrap_or(1920.0);
        let columns = metrics.columns_for_width(logical_w);
        if let Some(d) = &display {
            eprintln!(
                "rune-grid: display {:?} {}x{} @ scale {} -> {columns} colunas",
                d.name, d.size.0, d.size.1, d.scale
            );
        }
        let state = Grid {
            entries,
            metrics,
            columns,
            loader,
            loader_rx: Some(loader_rx),
            covers_rx: None,
            lru: RefCell::new(PixelsLru { map: HashMap::new(), order: VecDeque::new() }),
            hits: 0,
            misses: 0,
            bench: benchmark.then_some(Bench {
                mode,
                next_step_at: SETTLE_START,
                step: 0,
                done_at: None,
            }),
        };
        let spec = AppSpec {
            setup: Some(setup),
            poll: Some(poll),
            ..AppSpec::new("BigPicture (Rune)", (1280.0, 800.0), || unreachable!(), view)
        };
        // Borderless fullscreen no monitor escolhido: posição e tamanho físicos do display, sem
        // borda. `App::full_screen` sozinho usa `Fullscreen::Borderless(None)` (monitor atual).
        let options = WindowOptions {
            borderless: true,
            // App::full_screen só é aplicado a janelas redimensionáveis (host/create.rs).
            resizable: true,
            position: display.as_ref().map(|d| d.position),
            physical_size: display.as_ref().map(|d| d.size),
            min_size: Some((0.0, 0.0)),
            ..Default::default()
        };
        vec![
            WindowSpec::new("main", spec)
                .state(state)
                .options(options)
                .full_screen(true)
                .on_frame_encoded(record_frame),
        ]
    });
    Ok(())
}
