use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::RgbaFrame;
use crate::cover_cache::{CacheOutcome, CoverCache};

/// Result delivered by `CoverLoader` background worker threads.
#[derive(Debug)]
pub struct CoverLoaded {
    pub id: String,
    pub frame: RgbaFrame,
    pub outcome: CacheOutcome,
}

struct CoverJob {
    id: String,
    source_path: PathBuf,
    /// Baixa daqui para `source_path` quando o arquivo ainda não existe.
    url: Option<String>,
}

struct QueueState {
    pending: VecDeque<CoverJob>,
    in_flight: HashSet<String>,
    /// Capas que falharam (arquivo ruim, download 404): não tenta de novo nesta execução.
    failed: HashSet<String>,
    shutdown: bool,
}

/// Multi-threaded asynchronous cover loader that decodes, resizes to 600x900,
/// and caches covers on disk strictly off the UI thread.
///
/// Uses a LIFO stack with deduplication so the cards currently visible in the
/// user's viewport are always prioritized ahead of off-screen scroll history.
pub struct CoverLoader {
    shared: Arc<(Mutex<QueueState>, Condvar)>,
    workers: Vec<JoinHandle<()>>,
    /// Tamanho entregue à UI (largura << 32 | altura); 0 = 600×900 do cache.
    output_size: Arc<AtomicU64>,
}

impl CoverLoader {
    pub fn new(cache: CoverCache, worker_count: usize) -> (Self, Receiver<CoverLoaded>) {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new((
            Mutex::new(QueueState {
                pending: VecDeque::new(),
                in_flight: HashSet::new(),
                failed: HashSet::new(),
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let output_size = Arc::new(AtomicU64::new(0));
        let threads = worker_count.max(1);
        let mut workers = Vec::with_capacity(threads);

        for ix in 0..threads {
            let shared_ref = Arc::clone(&shared);
            let cache_ref = cache.clone();
            let tx_ref: Sender<CoverLoaded> = tx.clone();
            let size_ref = Arc::clone(&output_size);

            let handle = thread::Builder::new()
                .name(format!("bp-cover-worker-{ix}"))
                .spawn(move || worker_loop(shared_ref, cache_ref, tx_ref, size_ref))
                .expect("failed to spawn cover worker thread");
            workers.push(handle);
        }

        (
            Self {
                shared,
                workers,
                output_size,
            },
            rx,
        )
    }

    /// Entrega as capas já reduzidas para `width`×`height` (o tamanho em que o card as mostra,
    /// em pixels físicos). O cache em disco continua em 600×900; só a cópia em memória encolhe,
    /// e cada capa ocupa uma fração dos 2,1 MB do tamanho cheio.
    pub fn set_output_size(&self, width: u32, height: u32) {
        let packed = if width == 0 || height == 0 {
            0
        } else {
            (u64::from(width) << 32) | u64::from(height)
        };
        self.output_size.store(packed, Ordering::Relaxed);
    }

    /// Enqueues a cover for off-thread decoding/caching if it is not already in flight.
    pub fn request(&self, id: impl Into<String>, source_path: impl Into<PathBuf>) {
        self.enqueue(id.into(), source_path.into(), None);
    }

    /// Como [`CoverLoader::request`], mas baixa a imagem de `url` para `source_path` antes, se
    /// ela ainda não estiver no disco (capas do CDN da loja).
    pub fn request_remote(&self, id: impl Into<String>, source_path: impl Into<PathBuf>, url: impl Into<String>) {
        self.enqueue(id.into(), source_path.into(), Some(url.into()));
    }

    fn enqueue(&self, id: String, source_path: PathBuf, url: Option<String>) {

        let (lock, cvar) = &*self.shared;
        let Ok(mut state) = lock.lock() else {
            return;
        };
        if state.shutdown || state.in_flight.contains(&id) || state.failed.contains(&id) {
            return;
        }
        state.in_flight.insert(id.clone());
        // Push back & pop back = LIFO priority so freshly scrolled-into-view cards decode first.
        state.pending.push_back(CoverJob {
            id,
            source_path,
            url,
        });
        // Bound queue backlog so rapid scrolling across 1,000 cards drops stale offscreen requests.
        while state.pending.len() > 160 {
            if let Some(dropped) = state.pending.pop_front() {
                state.in_flight.remove(&dropped.id);
            }
        }
        cvar.notify_one();
    }
}

impl Drop for CoverLoader {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.shutdown = true;
            state.pending.clear();
            cvar.notify_all();
        }
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Baixa `url` para `dest` (gravando num temporário e renomeando no fim).
fn download(url: &str, dest: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    // Sem prazo o ureq espera para sempre, e uma conexão parada prenderia a thread de capas.
    let bytes = ureq::get(url)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .build()
        .call()
        .with_context(|| format!("baixando {url}"))?
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("lendo {url}"))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension("download");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Reduz o frame para o tamanho combinado (`w << 32 | h`), sem nunca ampliar: se o alvo não é
/// menor nas duas dimensões, o frame segue como está.
fn fit_output(frame: RgbaFrame, packed: u64) -> RgbaFrame {
    let (w, h) = ((packed >> 32) as u32, packed as u32);
    if packed == 0 || w >= frame.width() || h >= frame.height() {
        return frame;
    }
    frame.resized(w, h).unwrap_or(frame)
}

fn worker_loop(
    shared: Arc<(Mutex<QueueState>, Condvar)>,
    cache: CoverCache,
    tx: Sender<CoverLoaded>,
    output_size: Arc<AtomicU64>,
) {
    let (lock, cvar) = &*shared;
    loop {
        let job = {
            let Ok(mut state) = lock.lock() else {
                return;
            };
            while state.pending.is_empty() && !state.shutdown {
                state = match cvar.wait(state) {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
            }
            if state.shutdown {
                return;
            }
            state.pending.pop_back()
        };

        let Some(job) = job else {
            continue;
        };

        let result = match &job.url {
            Some(url) if !job.source_path.is_file() => download(url, &job.source_path),
            _ => Ok(()),
        }
        .and_then(|()| cache.load_or_decode(&job.source_path));
        if result.is_err() && job.url.is_some() {
            // Arquivo baixado que não decodifica: apaga para baixar de novo na próxima execução,
            // em vez de ficar sem capa para sempre.
            let _ = std::fs::remove_file(&job.source_path);
        }

        if let Ok(mut state) = lock.lock() {
            state.in_flight.remove(&job.id);
            if result.is_err() {
                state.failed.insert(job.id.clone());
            }
        }

        match result {
            Ok((frame, outcome)) => {
                if tx
                    .send(CoverLoaded {
                        id: job.id,
                        frame: fit_output(frame, output_size.load(Ordering::Relaxed)),
                        outcome,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(err) => {
                tracing::warn!(
                    card_id = %job.id,
                    path = %job.source_path.display(),
                    error = %err,
                    "failed to decode cover artwork"
                );
            }
        }
    }
}
