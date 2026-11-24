//! Background decoding. The UI says which pictures it wants (the current one first, then
//! its neighbours); worker threads decode them and prepare GPU textures.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use sv_core::decode::{self, Bitmap, Format, Frame, Loaded};

/// File path and page number.
pub type Key = (PathBuf, usize);

/// Decoded pictures kept in memory beyond this are dropped, neighbours first.
const MEMORY_BUDGET: usize = 1024 * 1024 * 1024;

pub struct Ready {
    pub loaded: Loaded,
    /// One texture per frame.
    pub textures: Vec<TextureHandle>,
    /// Some pixel is not fully opaque, so a backdrop shows through.
    pub transparent: bool,
}

fn has_transparency(loaded: &Loaded) -> bool {
    loaded.frames.iter().any(|f| f.bitmap.rgba.chunks_exact(4).any(|p| p[3] < 255))
}

pub enum State<'a> {
    Loading,
    Ready(&'a Arc<Ready>),
    Failed(&'a str),
}

enum Slot {
    Ready(Arc<Ready>),
    Failed(String),
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Key>,
    in_flight: HashSet<Key>,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Largest texture the GPU takes; 0 until the first frame has told us.
    max_texture_side: AtomicUsize,
}

pub struct Loader {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<(Key, Result<Ready, String>)>,
    slots: HashMap<Key, Slot>,
    wanted: Vec<Key>,
    evicted: HashSet<Key>,
    /// The neighbours have been handed to the workers. Until the picture on screen is
    /// decoded they wait, so they do not slow it down.
    neighbours_queued: bool,
}

fn texture_options() -> TextureOptions {
    TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    }
}

fn upload(ctx: &egui::Context, name: &str, bitmap: &Bitmap, max_side: usize) -> TextureHandle {
    let scaled = bitmap.downscaled(max_side as u32);
    let b = scaled.as_ref().unwrap_or(bitmap);
    let image = ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba);
    ctx.load_texture(name, image, texture_options())
}

impl Ready {
    fn new(ctx: &egui::Context, name: &str, loaded: Loaded, max_side: usize) -> Ready {
        let textures = loaded.frames.iter().map(|f| upload(ctx, name, &f.bitmap, max_side)).collect();
        let transparent = has_transparency(&loaded);
        Ready { loaded, textures, transparent }
    }

    /// A picture made in memory, e.g. the result of an edit.
    pub fn from_bitmap(ctx: &egui::Context, bitmap: Bitmap, format: Format) -> Ready {
        let max_side = ctx.input(|i| i.max_texture_side);
        let texture = upload(ctx, "edit", &bitmap, max_side);
        let loaded = Loaded { format, frames: vec![Frame { bitmap, delay_ms: 0 }], page: 0, page_count: 1 };
        let transparent = has_transparency(&loaded);
        Ready { loaded, textures: vec![texture], transparent }
    }

    /// A picture that did not come from a file, e.g. pasted from the clipboard.
    pub fn from_color_image(ctx: &egui::Context, image: &ColorImage) -> Ready {
        let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
        for p in &image.pixels {
            rgba.extend_from_slice(&p.to_srgba_unmultiplied());
        }
        let bitmap = Bitmap { width: image.size[0] as u32, height: image.size[1] as u32, rgba };
        let max_side = ctx.input(|i| i.max_texture_side);
        let texture = upload(ctx, "clipboard", &bitmap, max_side);
        let loaded = Loaded {
            format: Format::Png,
            frames: vec![Frame { bitmap, delay_ms: 0 }],
            page: 0,
            page_count: 1,
        };
        let transparent = has_transparency(&loaded);
        Ready { loaded, textures: vec![texture], transparent }
    }
}

impl Loader {
    pub fn new(ctx: &egui::Context) -> Loader {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            max_texture_side: AtomicUsize::new(0),
        });
        let (tx, rx) = mpsc::channel();
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 3);
        for _ in 0..workers {
            let (shared, tx, ctx) = (shared.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || worker(&shared, &tx, &ctx));
        }
        Loader { shared, rx, slots: HashMap::new(), wanted: Vec::new(), evicted: HashSet::new(), neighbours_queued: false }
    }

    pub fn set_max_texture_side(&self, side: usize) {
        self.shared.max_texture_side.store(side, Ordering::Relaxed);
    }

    /// Collect finished work. Returns true when something arrived.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((key, result)) = self.rx.try_recv() {
            any = true;
            if self.wanted.contains(&key) {
                let slot = match result {
                    Ok(ready) => Slot::Ready(Arc::new(ready)),
                    Err(e) => Slot::Failed(e),
                };
                self.slots.insert(key, slot);
            }
        }
        if any {
            self.enforce_budget();
            if !self.neighbours_queued {
                self.queue_jobs();
            }
        }
        any
    }

    /// Hand the workers what is still missing. The first picture (the one on screen) goes
    /// alone until it is decoded: neighbours decoding at the same time would share the
    /// processor and the disk with it and make it appear later.
    fn queue_jobs(&mut self) {
        let first_done = self.wanted.first().is_none_or(|k| self.slots.contains_key(k));
        let take = if first_done { self.wanted.len() } else { 1 };
        self.neighbours_queued = first_done;
        let mut queue = self.shared.queue.lock().unwrap();
        queue.jobs = self
            .wanted
            .iter()
            .take(take)
            .filter(|k| !self.slots.contains_key(*k) && !queue.in_flight.contains(*k) && !self.evicted.contains(*k))
            .cloned()
            .collect();
        drop(queue);
        self.shared.wake.notify_all();
    }

    fn enforce_budget(&mut self) {
        let size = |slots: &HashMap<Key, Slot>| -> usize {
            slots.values().map(|s| if let Slot::Ready(r) = s { r.loaded.bytes() } else { 0 }).sum()
        };
        for key in self.wanted.iter().skip(1).rev() {
            if size(&self.slots) <= MEMORY_BUDGET {
                break;
            }
            if self.slots.remove(key).is_some() {
                self.evicted.insert(key.clone());
            }
        }
    }

    /// Declare what should be in memory, most important first. Everything else is dropped.
    pub fn want(&mut self, keys: Vec<Key>) {
        if keys == self.wanted {
            return;
        }
        self.poll();
        self.slots.retain(|k, _| keys.contains(k));
        self.evicted.retain(|k| keys.contains(k));
        if let Some(first) = keys.first() {
            self.evicted.remove(first);
        }
        self.wanted = keys;
        self.neighbours_queued = false;
        self.queue_jobs();
    }

    pub fn state(&self, key: &Key) -> State<'_> {
        match self.slots.get(key) {
            Some(Slot::Ready(r)) => State::Ready(r),
            Some(Slot::Failed(e)) => State::Failed(e),
            None => State::Loading,
        }
    }

    /// Drop everything known about a file, e.g. after it was deleted or renamed.
    pub fn forget(&mut self, path: &std::path::Path) {
        self.slots.retain(|k, _| k.0 != path);
        self.wanted.clear();
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap().closed = true;
        self.shared.wake.notify_all();
    }
}

fn worker(shared: &Shared, tx: &mpsc::Sender<(Key, Result<Ready, String>)>, ctx: &egui::Context) {
    loop {
        let key = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if queue.closed {
                    return;
                }
                if let Some(key) = queue.jobs.pop_front() {
                    queue.in_flight.insert(key.clone());
                    break key;
                }
                queue = shared.wake.wait(queue).unwrap();
            }
        };

        let result = decode::load(&key.0, key.1).map_err(|e| e.to_string()).map(|loaded| {
            // Decoding can finish before the window's first frame; the texture size
            // limit is only known after it.
            let mut waited = 0;
            let mut max_side = shared.max_texture_side.load(Ordering::Relaxed);
            while max_side == 0 && waited < 2000 {
                std::thread::sleep(std::time::Duration::from_millis(2));
                waited += 2;
                max_side = shared.max_texture_side.load(Ordering::Relaxed);
            }
            if max_side == 0 {
                max_side = 8192;
            }
            let name = key.0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ready::new(ctx, &name, loaded, max_side)
        });

        let sent = tx.send((key.clone(), result)).is_ok();
        shared.queue.lock().unwrap().in_flight.remove(&key);
        if !sent {
            return;
        }
        ctx.request_repaint();
    }
}
