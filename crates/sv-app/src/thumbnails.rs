//! Thumbnails for the browser, made on background threads. Each frame the grid asks for
//! the thumbnails it is showing; only those are worked on, so scrolling through a huge
//! folder never queues up work for pictures that already left the screen.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::SystemTime;
use sv_core::folder::Entry;
use sv_core::thumbs::{self, CacheStore};

/// All cache files together stay below this; the least recently written go first.
const CACHE_CAP: u64 = 4 * 1024 * 1024 * 1024;

/// Thumbnails kept as textures; the least recently shown go first.
const MAX_IN_MEMORY: usize = 4000;

pub enum Thumb {
    Pending,
    Ready(TextureHandle),
    Failed,
}

#[derive(Clone, PartialEq)]
struct Job {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    in_flight: HashSet<PathBuf>,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    cache: Option<CacheStore>,
    /// Folders large enough to get a cache file.
    cached_folders: Mutex<HashSet<PathBuf>>,
}

struct Slot {
    texture: Option<TextureHandle>,
    used: u64,
}

pub struct ThumbLoader {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<(PathBuf, Option<TextureHandle>)>,
    slots: HashMap<PathBuf, Slot>,
    wanted: Vec<Job>,
    frame: u64,
}

impl ThumbLoader {
    pub fn new(ctx: &egui::Context) -> ThumbLoader {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            cache: CacheStore::default_location(),
            cached_folders: Mutex::new(HashSet::new()),
        });
        {
            // Housekeeping, off the UI thread: old-style thumbnail files, caches of
            // folders that are gone, and the overall size limit.
            let shared = shared.clone();
            std::thread::spawn(move || {
                thumbs::remove_legacy_cache();
                if let Some(cache) = &shared.cache {
                    cache.tidy(CACHE_CAP);
                }
            });
        }
        let (tx, rx) = mpsc::channel();
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1)).clamp(2, 6);
        for _ in 0..workers {
            let (shared, tx, ctx) = (shared.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || worker(&shared, &tx, &ctx));
        }
        ThumbLoader { shared, rx, slots: HashMap::new(), wanted: Vec::new(), frame: 0 }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.wanted.clear();
        while let Ok((path, texture)) = self.rx.try_recv() {
            self.slots.insert(path, Slot { texture, used: self.frame });
        }
    }

    /// The thumbnail for an entry. Asking is what gets it made.
    pub fn get(&mut self, entry: &Entry) -> Thumb {
        if let Some(slot) = self.slots.get_mut(&entry.path) {
            slot.used = self.frame;
            return match &slot.texture {
                Some(t) => Thumb::Ready(t.clone()),
                None => Thumb::Failed,
            };
        }
        self.wanted.push(Job { path: entry.path.clone(), size: entry.size, modified: entry.modified });
        Thumb::Pending
    }

    pub fn end_frame(&mut self) {
        {
            let mut queue = self.shared.queue.lock().unwrap();
            let jobs: Vec<Job> = self.wanted.iter().filter(|j| !queue.in_flight.contains(&j.path)).cloned().collect();
            if jobs != queue.jobs {
                queue.jobs = jobs;
                self.shared.wake.notify_all();
            }
        }
        if self.slots.len() > MAX_IN_MEMORY {
            let mut ages: Vec<u64> = self.slots.values().map(|s| s.used).collect();
            ages.sort_unstable();
            let cutoff = ages[self.slots.len() - MAX_IN_MEMORY * 3 / 4];
            self.slots.retain(|_, s| s.used > cutoff);
        }
    }

    /// Throw away the thumbnail of a picture whose contents changed.
    pub fn invalidate(&mut self, path: &Path) {
        if let Some(cache) = &self.shared.cache {
            cache.forget(path);
        }
        self.slots.remove(path);
    }

    /// Which folders are big enough to keep their thumbnails on disk.
    pub fn set_cached_folders(&self, folders: HashSet<PathBuf>) {
        *self.shared.cached_folders.lock().unwrap() = folders;
    }

    pub fn cache_size(&self) -> u64 {
        self.shared.cache.as_ref().map_or(0, CacheStore::total_size)
    }

    pub fn clear_cache(&self) {
        if let Some(cache) = &self.shared.cache {
            cache.clear();
        }
    }

    /// Remove the cache files of folders below `threshold` bytes. Returns how many.
    pub fn remove_small_caches(&self, threshold: u64) -> usize {
        self.shared.cache.as_ref().map_or(0, |c| c.remove_small(threshold))
    }

    /// Drop every thumbnail held in memory, so each is read again when next shown. Those
    /// kept on disk are reused when their picture has not changed.
    pub fn forget_all(&mut self) {
        self.slots.clear();
    }

    pub fn forget(&mut self, path: &Path) {
        self.slots.remove(path);
    }
}

impl Drop for ThumbLoader {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap().closed = true;
        self.shared.wake.notify_all();
    }
}

fn worker(shared: &Shared, tx: &mpsc::Sender<(PathBuf, Option<TextureHandle>)>, ctx: &egui::Context) {
    loop {
        let job = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if queue.closed {
                    return;
                }
                if !queue.jobs.is_empty() {
                    let job = queue.jobs.remove(0);
                    queue.in_flight.insert(job.path.clone());
                    break job;
                }
                queue = shared.wake.wait(queue).unwrap();
            }
        };
        let on_disk = job.path.parent().is_some_and(|dir| shared.cached_folders.lock().unwrap().contains(dir));
        let store = shared.cache.as_ref().filter(|_| on_disk);
        let texture = thumbs::load_or_make(store, &job.path, job.size, job.modified).ok().map(|b| {
            let image = ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba);
            ctx.load_texture("thumb", image, TextureOptions::LINEAR)
        });
        let sent = tx.send((job.path.clone(), texture)).is_ok();
        shared.queue.lock().unwrap().in_flight.remove(&job.path);
        if !sent {
            return;
        }
        ctx.request_repaint();
    }
}
