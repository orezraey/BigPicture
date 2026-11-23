use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
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
}

struct QueueState {
    pending: VecDeque<CoverJob>,
    in_flight: HashSet<String>,
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
}

impl CoverLoader {
    pub fn new(cache: CoverCache, worker_count: usize) -> (Self, Receiver<CoverLoaded>) {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new((
            Mutex::new(QueueState {
                pending: VecDeque::new(),
                in_flight: HashSet::new(),
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let threads = worker_count.max(1);
        let mut workers = Vec::with_capacity(threads);

        for ix in 0..threads {
            let shared_ref = Arc::clone(&shared);
            let cache_ref = cache.clone();
            let tx_ref: Sender<CoverLoaded> = tx.clone();

            let handle = thread::Builder::new()
                .name(format!("bp-cover-worker-{ix}"))
                .spawn(move || worker_loop(shared_ref, cache_ref, tx_ref))
                .expect("failed to spawn cover worker thread");
            workers.push(handle);
        }

        (Self { shared, workers }, rx)
    }

    /// Enqueues a cover for off-thread decoding/caching if it is not already in flight.
    pub fn request(&self, id: impl Into<String>, source_path: impl Into<PathBuf>) {
        let id = id.into();
        let source_path = source_path.into();

        let (lock, cvar) = &*self.shared;
        let Ok(mut state) = lock.lock() else {
            return;
        };
        if state.shutdown || state.in_flight.contains(&id) {
            return;
        }
        state.in_flight.insert(id.clone());
        // Push back & pop back = LIFO priority so freshly scrolled-into-view cards decode first.
        state.pending.push_back(CoverJob { id, source_path });
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

fn worker_loop(
    shared: Arc<(Mutex<QueueState>, Condvar)>,
    cache: CoverCache,
    tx: Sender<CoverLoaded>,
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

        let result = cache.load_or_decode(&job.source_path);

        if let Ok(mut state) = lock.lock() {
            state.in_flight.remove(&job.id);
        }

        match result {
            Ok((frame, outcome)) => {
                if tx
                    .send(CoverLoaded {
                        id: job.id,
                        frame,
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
