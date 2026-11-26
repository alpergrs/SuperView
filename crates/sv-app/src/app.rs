//! The SuperView window: a viewer and a thumbnail browser sharing one picture list.

mod batch_ui;
mod browser;
mod editor;
mod icons;
mod library;
mod settings_ui;

use crate::input::{self, Binding, Command};
use crate::loader::{Key, Loader, Ready, State};
use crate::thumbnails::ThumbLoader;
use crate::viewer::View;
use crate::OpenRequest;
use eframe::egui::{self, vec2, Align2, Color32, FontId, Rect, Sense, Vec2};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use sv_core::folder::{self, Entry, SortKey};
use sv_core::settings::{Backdrop, FitMode, Settings, Theme, WheelAction};
use sv_core::decode::Format;
use sv_core::fileops;
use sv_core::save::{self, Turn};

const ZOOM_STEP: f32 = 1.2;

enum ZoomRequest {
    Factor(f32),
    Fit,
    Actual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Viewer,
    Browser,
}

/// How long the user must keep pushing past the last (or first) picture before the
/// list wraps around.
const WRAP_DELAY: f64 = 0.5;

/// The user tried to go past an end of the list.
struct EdgeHit {
    /// +1 at the last picture, -1 at the first.
    direction: i64,
    /// When the end was first pushed against.
    since: f64,
}

struct Shown {
    /// `None` for a picture pasted from the clipboard.
    key: Option<Key>,
    ready: Arc<Ready>,
}

struct RenameDialog {
    name: String,
    error: Option<String>,
    focus: bool,
}

pub struct App {
    settings: Settings,
    bindings: Vec<Binding>,
    loader: Loader,
    requests: Receiver<Option<OpenRequest>>,

    mode: Mode,
    /// The viewer was entered from the browser, so Esc goes back there.
    from_browser: bool,
    thumbs: ThumbLoader,
    tree: browser::Tree,
    grid: browser::GridState,
    settings_ui: settings_ui::State,
    editor: editor::State,
    batch: batch_ui::State,
    search: library::Search,
    /// The search currently narrowing the list.
    query: folder::Query,
    duplicates: library::Duplicates,
    marked: HashSet<PathBuf>,
    /// Subfolders of the current folder, shown before the pictures in the browser.
    subdirs: Vec<PathBuf>,
    /// Files copied or cut inside SuperView (the second part is "cut"). Used where the
    /// system clipboard cannot hold files.
    file_clip: Option<(Vec<PathBuf>, bool)>,
    /// Overrides the "include subfolders" setting for the next slideshow request.
    force_recursive: Option<bool>,
    /// Folders visited before this one, and those gone back from.
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// The folder change in progress is a step through that history.
    history_step: bool,
    /// Show hidden and system files and folders. On when SuperView was started with one.
    show_hidden: bool,
    hidden_formats: HashSet<Format>,
    /// Everything in the folder; `entries` is this minus the hidden file types.
    all_entries: Vec<Entry>,
    last_zoom: Option<f32>,
    edge: Option<EdgeHit>,
    /// Size of the picture area in points, and pixels per point, from the last frame.
    last_canvas: (Vec2, f32),
    /// Tab presses taken out of the input before the GUI library sees them (it would move
    /// keyboard focus to a button, which Space or Enter would then press).
    held_keys: Vec<egui::Event>,
    /// A two-by-two pattern tile for the checkerboard backdrop; made when first needed.
    checker: Option<egui::TextureHandle>,
    /// Read the folder again at the end of this frame (a file was added to it).
    relist: bool,
    /// Start the slideshow as soon as the folder listing has arrived.
    autoplay_pending: bool,
    /// The picture is being reloaded after a save; drop the view rotation when it arrives.
    reset_view_on_reload: bool,

    dir: Option<PathBuf>,
    recursive: bool,
    entries: Vec<Entry>,
    index: usize,
    page: usize,
    listing: Option<Receiver<(u64, Vec<Entry>)>>,
    listing_id: u64,

    shown: Option<Shown>,
    pasted: Option<Arc<Ready>>,
    error: Option<String>,
    loading_since: Option<f64>,
    view: View,
    zoom_request: Option<ZoomRequest>,
    frame: usize,
    frame_due: f64,

    fullscreen: bool,
    slideshow_due: Option<f64>,
    slideshow_owns_fullscreen: bool,
    picker: bool,
    help: bool,
    rename: Option<RenameDialog>,
    toast: Option<(String, f64)>,
    title: String,
    rng: u64,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        request: Option<OpenRequest>,
        requests: Receiver<Option<OpenRequest>>,
    ) -> App {
        let settings = Settings::load();
        apply_theme(&cc.egui_ctx, settings.theme);
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9, |d| d.as_nanos() as u64);
        let mut app = App {
            bindings: input::from_settings(&settings.keys),
            loader: Loader::new(&cc.egui_ctx),
            requests,
            mode: Mode::Viewer,
            from_browser: false,
            thumbs: ThumbLoader::new(&cc.egui_ctx),
            tree: browser::Tree::new(),
            grid: browser::GridState::default(),
            settings_ui: settings_ui::State::default(),
            editor: editor::State::default(),
            batch: batch_ui::State::default(),
            search: library::Search::default(),
            query: folder::Query::default(),
            duplicates: library::Duplicates::default(),
            marked: HashSet::new(),
            subdirs: Vec::new(),
            file_clip: None,
            force_recursive: None,
            back: Vec::new(),
            forward: Vec::new(),
            history_step: false,
            show_hidden: false,
            hidden_formats: HashSet::new(),
            all_entries: Vec::new(),
            last_zoom: None,
            edge: None,
            last_canvas: (Vec2::ZERO, 1.0),
            relist: false,
            checker: None,
            held_keys: Vec::new(),
            autoplay_pending: false,
            reset_view_on_reload: false,
            dir: None,
            recursive: false,
            entries: Vec::new(),
            index: 0,
            page: 0,
            listing: None,
            listing_id: 0,
            shown: None,
            pasted: None,
            error: None,
            loading_since: None,
            view: View::default(),
            zoom_request: None,
            frame: 0,
            frame_due: 0.0,
            fullscreen: false,
            slideshow_due: None,
            slideshow_owns_fullscreen: false,
            picker: false,
            help: false,
            rename: None,
            toast: None,
            title: String::new(),
            rng: seed | 1,
            settings,
        };
        // Started on its own (the exe itself was opened): show the start folder.
        let request = request.or_else(|| {
            app.settings.start_folder().map(|path| OpenRequest { path, recursive: false, slideshow: false })
        });
        if let Some(request) = request {
            app.open(&cc.egui_ctx, request, 0.0);
            // Start decoding before the window is even on screen.
            app.sync_loader();
        }
        app
    }

    // ----- opening and listing ---------------------------------------------------

    fn open(&mut self, ctx: &egui::Context, request: OpenRequest, now: f64) {
        let path = request.path;
        self.editor.end_session();
        // Asked to show something hidden: then hidden things are wanted.
        if !self.show_hidden && folder::is_inside_hidden(&path) {
            self.set_show_hidden(true);
        }
        let came_from = self.dir.clone();
        if path.is_dir() {
            self.tree.reveal(&path);
            self.dir = Some(path);
            self.entries.clear();
            if request.slideshow {
                // From Explorer's "Slideshow with SuperView": straight to the first picture.
                self.recursive = self.force_recursive.unwrap_or(self.settings.explorer_recursive);
                self.mode = Mode::Viewer;
                self.from_browser = false;
                self.autoplay_pending = self.settings.explorer_autoplay;
            } else {
                self.recursive = request.recursive;
                self.mode = Mode::Browser;
                self.grid.reveal = true;
            }
        } else if path.is_file() {
            self.mode = Mode::Viewer;
            self.from_browser = false;
            let meta = std::fs::metadata(&path).ok();
            self.dir = path.parent().map(Path::to_path_buf);
            self.recursive = false;
            // Show the picture right away; its neighbours arrive from the listing thread.
            self.entries = vec![Entry {
                hidden: false,
                size: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()).unwrap_or(std::time::UNIX_EPOCH),
                path,
            }];
        } else {
            self.say(format!("Cannot open {}", path.display()), now);
            return;
        }
        // Remember where we were, for the mouse's back button.
        if came_from != self.dir && !self.history_step {
            if let Some(previous) = came_from.clone() {
                self.back.push(previous);
                if self.back.len() > 200 {
                    self.back.remove(0);
                }
            }
            self.forward.clear();
        }
        self.all_entries = self.entries.clone();
        self.subdirs = self.dir.as_deref().map(|d| browser::subfolders(d, self.show_hidden)).unwrap_or_default();
        self.grid.folder_selected = None;
        // Coming up out of a subfolder (Backspace, "..", Up, mouse back): that subfolder is
        // selected, as in Explorer, instead of the first picture.
        if let Some(previous) = &came_from {
            let same = |a: &Path, b: &Path| a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase();
            if self.dir.as_deref().is_some_and(|d| previous.parent().is_some_and(|p| same(p, d))) {
                self.grid.folder_selected = self.grid_folders().iter().position(|(dir, name)| name != ".." && same(dir.as_path(), previous.as_path()));
            }
        }
        self.marked.clear();
        self.index = 0;
        self.page = 0;
        self.pasted = None;
        self.stop_slideshow(ctx);
        self.start_listing(ctx);
    }

    fn start_listing(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.dir.clone() else { return };
        self.listing_id += 1;
        let (tx, rx) = mpsc::channel();
        self.listing = Some(rx);
        let (id, recursive, ctx) = (self.listing_id, self.recursive, ctx.clone());
        let (key, descending) = (self.settings.sort_key, self.settings.sort_descending);
        std::thread::spawn(move || {
            let mut entries = folder::list(&dir, recursive);
            folder::sort(&mut entries, key, descending);
            let _ = tx.send((id, entries));
            ctx.request_repaint();
        });
    }

    fn poll_listing(&mut self, now: f64) {
        let Some(rx) = &self.listing else { return };
        let Ok((id, mut list)) = rx.try_recv() else { return };
        self.listing = None;
        if id != self.listing_id {
            return;
        }
        let current = self.entries.get(self.index).cloned();
        if let Some(current) = &current {
            // A file opened by hand stays in the list even with an unusual extension.
            if !list.iter().any(|e| e.path == current.path) {
                list.push(current.clone());
                folder::sort(&mut list, self.settings.sort_key, self.settings.sort_descending);
            }
        } else if list.is_empty() {
            self.say("No pictures in this folder".to_string(), now);
        }
        self.all_entries = list;
        self.apply_filter(current.map(|e| e.path), 0);
        self.grid.reveal = true;
        self.update_cached_folders();
    }

    /// Tell the thumbnail loader which of the listed folders hold enough pictures (by
    /// total size) to deserve a cache file. Every subfolder is judged on its own.
    fn update_cached_folders(&self) {
        let mut totals: std::collections::HashMap<&Path, u64> = std::collections::HashMap::new();
        for entry in &self.all_entries {
            if let Some(dir) = entry.path.parent() {
                *totals.entry(dir).or_default() += entry.size;
            }
        }
        let threshold = self.settings.cache_threshold_mb as u64 * 1024 * 1024;
        let folders = totals.into_iter().filter(|(_, bytes)| *bytes >= threshold).map(|(dir, _)| dir.to_path_buf()).collect();
        self.thumbs.set_cached_folders(folders);
    }

    /// Rebuild the visible list from `all_entries`, staying on `keep` when it is still
    /// there and otherwise near `fallback`.
    fn apply_filter(&mut self, keep: Option<PathBuf>, fallback: usize) {
        let (hidden_formats, show_hidden, query) = (&self.hidden_formats, self.show_hidden, &self.query);
        self.entries = self
            .all_entries
            .iter()
            .filter(|e| {
                let wanted_type = Format::from_extension(&e.path).is_none_or(|f| !hidden_formats.contains(&f));
                (wanted_type && (show_hidden || !e.hidden) && query.matches(e)) || Some(&e.path) == keep.as_ref()
            })
            .cloned()
            .collect();
        self.index = keep
            .and_then(|k| self.entries.iter().position(|e| e.path == k))
            .unwrap_or(fallback.min(self.entries.len().saturating_sub(1)));
    }

    /// Go to the previous (`forward == false`) or next folder in the visiting history.
    fn history(&mut self, ctx: &egui::Context, forward: bool, now: f64) {
        if self.editor.is_dirty() {
            self.say("Unsaved edits: Ctrl+S saves them, Esc twice discards them".to_string(), now);
            return;
        }
        let (from, to) = if forward { (&mut self.forward, &mut self.back) } else { (&mut self.back, &mut self.forward) };
        // Folders that were deleted or unplugged meanwhile are skipped.
        let mut target = None;
        while let Some(dir) = from.pop() {
            if dir.is_dir() {
                target = Some(dir);
                break;
            }
        }
        let Some(target) = target else {
            // Nowhere to go back to: in the browser, "back" then means the folder above.
            if !forward && self.mode == Mode::Browser {
                self.grid.go_up = true;
            }
            return;
        };
        if let Some(current) = self.dir.clone() {
            to.push(current);
        }
        self.history_step = true;
        self.open(ctx, OpenRequest { path: target, recursive: false, slideshow: false }, now);
        self.history_step = false;
    }

    /// What Ctrl+C and Ctrl+X take in the browser: the selected folder tile, else the
    /// marked files, else the selected picture.
    fn browser_clip_targets(&self) -> Vec<PathBuf> {
        if let Some(i) = self.grid.folder_selected {
            return self.grid_folders().into_iter().nth(i).filter(|(_, name)| name != "..").map(|(path, _)| path).into_iter().collect();
        }
        self.targets()
    }

    /// Put files or folders on the clipboard, as Explorer would.
    fn clip_files(&mut self, paths: Vec<PathBuf>, cut: bool, now: f64) {
        if paths.is_empty() {
            return;
        }
        let on_system_clipboard = crate::fileclip::set(&paths, cut);
        let what = if paths.len() == 1 { file_name(&paths[0]) } else { format!("{} items", paths.len()) };
        let verb = if cut { "Cut" } else { "Copied" };
        if cfg!(windows) && !on_system_clipboard {
            // Still usable inside SuperView, and worth knowing when Explorer shows nothing.
            self.say(format!("{verb} {what}, but Windows refused the clipboard. Pasting works inside SuperView only"), now + 3.0);
        } else {
            self.say(format!("{verb} {what}. Ctrl+V pastes"), now);
        }
        // SuperView's own copy of the list is only for when the system clipboard did not
        // take it. Otherwise the system clipboard is the one truth: after copying anything
        // else there, Ctrl+V must not bring back these files.
        self.file_clip = if on_system_clipboard { None } else { Some((paths, cut)) };
    }

    /// Paste the clipboard's files into `dest`. Works with files copied in Explorer too.
    fn paste_files(&mut self, dest: PathBuf, now: f64) {
        let Some((paths, cut)) = crate::fileclip::get().or_else(|| self.file_clip.clone()) else {
            self.say("There are no files on the clipboard".to_string(), now);
            return;
        };
        let report = fileops::paste(&paths, &dest, cut);
        if cut {
            // Moved away: they leave the list, and a second paste must not move again.
            self.remove_paths(&report.done);
            self.file_clip = None;
            crate::fileclip::clear();
        }
        let mut text = format!("{} {}", if cut { "Moved" } else { "Pasted" }, report.done.len());
        if !report.skipped.is_empty() {
            text.push_str(&format!(", {} already here", report.skipped.len()));
        }
        if let Some((path, why)) = report.failed.first() {
            text.push_str(&format!(", {} failed ({}: {why})", report.failed.len(), file_name(path)));
        }
        self.say(text, now);
        self.subdirs = self.dir.as_deref().map(|d| browser::subfolders(d, self.show_hidden)).unwrap_or_default();
        self.tree.refresh();
        self.relist = true;
    }

    pub(super) fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        self.tree.set_show_hidden(show);
        self.subdirs = self.dir.as_deref().map(|d| browser::subfolders(d, show)).unwrap_or_default();
        self.grid.folder_selected = None;
        // The selected picture leaves with the rest if it is a hidden one.
        let keep = self.entries.get(self.index).filter(|e| show || !e.hidden).map(|e| e.path.clone());
        self.apply_filter(keep, self.index);
        self.grid.reveal = true;
    }

    fn resort(&mut self) {
        let current = self.current_path().cloned();
        folder::sort(&mut self.all_entries, self.settings.sort_key, self.settings.sort_descending);
        self.apply_filter(current, 0);
        self.grid.reveal = true;
        self.save_settings();
    }

    /// The marked files, or the current one when nothing is marked.
    fn targets(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            self.current_path().filter(|_| self.pasted.is_none()).cloned().into_iter().collect()
        } else {
            self.all_entries.iter().filter(|e| self.marked.contains(&e.path)).map(|e| e.path.clone()).collect()
        }
    }

    fn remove_paths(&mut self, gone: &[PathBuf]) {
        let fallback = self.index;
        let keep = self.current_path().filter(|p| !gone.contains(p)).cloned();
        for path in gone {
            self.loader.forget(path);
            self.thumbs.forget(path);
            self.marked.remove(path);
        }
        self.duplicates.forget(gone);
        self.all_entries.retain(|e| !gone.contains(&e.path));
        self.apply_filter(keep, fallback);
        self.page = 0;
    }

    fn send_to_folder(&mut self, move_files: bool, now: f64) {
        let paths = self.targets();
        if paths.is_empty() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().set_title(if move_files { "Move to folder" } else { "Copy to folder" });
        if let Some(last) = self.settings.last_target.as_ref().filter(|p| Path::new(p).is_dir()) {
            dialog = dialog.set_directory(last);
        }
        let Some(dest) = dialog.pick_folder() else { return };
        let report = fileops::transfer(&paths, &dest, move_files);
        self.settings.last_target = Some(dest.display().to_string());
        self.save_settings();
        if move_files {
            self.remove_paths(&report.done);
        } else {
            for path in &report.done {
                self.marked.remove(path);
            }
        }
        let mut text = report.summary(move_files);
        if let Some((_, why)) = report.failed.first() {
            text.push_str(&format!(" ({why})"));
        }
        self.say(text, now);
    }

    fn save_rotation(&mut self, now: f64) {
        if self.pasted.is_some() {
            return;
        }
        let Some(path) = self.current_path().cloned() else { return };
        let turn = Turn { turns: self.view.turns, flip_h: self.view.flip_h, flip_v: self.view.flip_v };
        if turn.is_identity() {
            self.say("Nothing to save: the picture is not rotated".to_string(), now);
            return;
        }
        let old = self.entries.get(self.index).cloned();
        match save::save_rotated(&path, turn) {
            Ok(outcome) => {
                if old.is_some() {
                    self.thumbs.invalidate(&path);
                }
                if let Ok(meta) = std::fs::metadata(&path) {
                    for list in [&mut self.entries, &mut self.all_entries] {
                        if let Some(e) = list.iter_mut().find(|e| e.path == path) {
                            e.size = meta.len();
                            e.modified = meta.modified().unwrap_or(e.modified);
                        }
                    }
                }
                self.loader.forget(&path);
                self.reset_view_on_reload = true;
                self.say(
                    match outcome {
                        save::Outcome::Lossless => "Saved, without any quality loss".to_string(),
                        save::Outcome::Reencoded => "Saved".to_string(),
                    },
                    now,
                );
            }
            Err(e) => self.say(format!("Could not save {}: {e}", file_name(&path)), now),
        }
    }

    fn set_mode(&mut self, ctx: &egui::Context, mode: Mode) {
        if mode == self.mode {
            return;
        }
        if mode == Mode::Browser {
            self.stop_slideshow(ctx);
            if self.fullscreen {
                self.set_fullscreen(ctx, false);
            }
            if let Some(dir) = &self.dir {
                self.tree.reveal(dir);
            }
            self.grid.reveal = true;
            self.pasted = None;
        } else {
            self.from_browser = true;
        }
        self.mode = mode;
    }

    // ----- current picture -------------------------------------------------------

    fn current_path(&self) -> Option<&PathBuf> {
        self.entries.get(self.index).map(|e| &e.path)
    }

    fn current_key(&self) -> Option<Key> {
        self.current_path().map(|p| (p.clone(), self.page))
    }

    fn sync_loader(&mut self) {
        let Some(current) = self.current_key() else {
            self.loader.want(Vec::new());
            return;
        };
        if self.mode == Mode::Browser {
            // Only the preview pane needs a full picture here.
            self.loader.want(if self.settings.show_preview { vec![current] } else { Vec::new() });
            return;
        }
        let n = self.entries.len();
        let mut keys = vec![current];
        for delta in [1, n.saturating_sub(1), 2] {
            let key = (self.entries[(self.index + delta) % n].path.clone(), 0);
            if !keys.iter().any(|k| k.0 == key.0) {
                keys.push(key);
            }
        }
        self.loader.want(keys);
    }

    fn go_to(&mut self, index: usize) {
        if self.entries.is_empty() {
            return;
        }
        self.index = index.min(self.entries.len() - 1);
        self.page = 0;
        self.pasted = None;
        self.edge = None;
        self.editor.end_session();
        self.grid.reveal = true;
    }

    fn step(&mut self, delta: i64) {
        let n = self.entries.len() as i64;
        if n > 0 {
            self.go_to((self.index as i64 + delta).rem_euclid(n) as usize);
        }
    }

    /// Next / previous asked for by the user. At either end of the list the picture gets
    /// a red border that stays, and for [`WRAP_DELAY`] further pushes the same way do
    /// nothing. After that a push starts over from the other end. Going the other way
    /// clears all of it.
    fn step_by_user(&mut self, direction: i64, now: f64) {
        let n = self.entries.len();
        if n == 0 {
            return;
        }
        let at_edge = if direction > 0 { self.index + 1 >= n } else { self.index == 0 };
        if !at_edge {
            self.edge = None;
            self.step(direction);
            return;
        }
        match &self.edge {
            Some(edge) if edge.direction == direction => {
                if now - edge.since >= WRAP_DELAY {
                    self.edge = None;
                    self.step(direction);
                }
            }
            _ => self.edge = Some(EdgeHit { direction, since: now }),
        }
    }

    fn update_shown(&mut self, now: f64) {
        // A picture being edited is shown from memory, whatever the file says.
        if let Some(session) = &self.editor.session {
            if self.shown.as_ref().is_none_or(|s| !Arc::ptr_eq(&s.ready, &session.current)) {
                let key = if self.pasted.is_none() { self.current_key() } else { None };
                self.shown = Some(Shown { key, ready: session.current.clone() });
                self.frame = 0;
            }
            self.error = None;
            self.loading_since = None;
            return;
        }
        if let Some(pasted) = &self.pasted {
            let already = self.shown.as_ref().is_some_and(|s| Arc::ptr_eq(&s.ready, pasted));
            if !already {
                self.shown = Some(Shown { key: None, ready: pasted.clone() });
                self.on_new_picture(now);
            }
            self.error = None;
            self.loading_since = None;
            return;
        }
        let Some(key) = self.current_key() else {
            self.shown = None;
            self.error = None;
            self.loading_since = None;
            return;
        };
        match self.loader.state(&key) {
            State::Ready(ready) => {
                let ready = ready.clone();
                self.error = None;
                self.loading_since = None;
                let reloaded = self.shown.as_ref().is_some_and(|s| s.key.as_ref() == Some(&key) && !Arc::ptr_eq(&s.ready, &ready));
                if reloaded {
                    // The file changed on disk (saved rotation): swap in the new pixels.
                    self.shown = Some(Shown { key: Some(key), ready });
                    self.frame = 0;
                    if std::mem::take(&mut self.reset_view_on_reload) {
                        self.view.reset_orientation();
                    }
                } else if self.shown.as_ref().is_none_or(|s| s.key.as_ref() != Some(&key)) {
                    let same_file = self.shown.as_ref().is_some_and(|s| s.key.as_ref().is_some_and(|k| k.0 == key.0));
                    self.shown = Some(Shown { key: Some(key), ready });
                    if same_file {
                        // Another page of the same document keeps zoom and rotation.
                        self.frame = 0;
                    } else {
                        self.on_new_picture(now);
                    }
                }
            }
            State::Failed(e) => {
                self.error = Some(e.to_string());
                self.shown = None;
                self.loading_since = None;
            }
            State::Loading => {
                self.error = None;
                self.loading_since.get_or_insert(now);
            }
        }
    }

    fn on_new_picture(&mut self, now: f64) {
        self.frame = 0;
        let delay = self.shown.as_ref().map_or(0, |s| s.ready.loaded.frames[0].delay_ms);
        self.frame_due = now + delay as f64 / 1000.0;
        self.view.reset_orientation();
        self.reset_view_on_reload = false;
        if !self.settings.keep_zoom {
            self.view.reset_placement();
        }
    }

    // ----- commands --------------------------------------------------------------

    fn run(&mut self, command: Command, ctx: &egui::Context, now: f64) {
        // Unsaved edits are never thrown away by moving on.
        let leaves_picture = matches!(
            command,
            Command::Next | Command::Prev | Command::Random | Command::First | Command::Last | Command::Open | Command::Delete | Command::Rename
                | Command::Slideshow | Command::NextPage | Command::PrevPage
        ) || (command == Command::Browser && !self.editor.has_selection())
            || matches!(command, Command::Batch | Command::BatchRename | Command::Info | Command::Search | Command::Duplicates);
        if leaves_picture && self.editor.is_dirty() {
            self.say("Unsaved edits: Ctrl+S saves them, Esc twice discards them".to_string(), now);
            return;
        }
        match command {
            Command::Edit => {
                if self.mode == Mode::Browser {
                    if self.entries.is_empty() {
                        return;
                    }
                    self.set_mode(ctx, Mode::Viewer);
                    self.editor.panel = true;
                } else {
                    self.editor.panel = !self.editor.panel;
                    if !self.editor.panel && !self.editor.is_dirty() {
                        self.editor.end_session();
                    }
                }
            }
            Command::Crop => {
                if self.mode == Mode::Viewer {
                    self.toggle_crop(ctx);
                }
            }
            Command::Search => self.toggle_search(ctx),
            Command::Duplicates => {
                self.set_mode(ctx, Mode::Browser);
                self.duplicates.open = !self.duplicates.open;
            }
            Command::Batch => self.batch.batch_open = !self.batch.batch_open,
            Command::BatchRename => self.batch.rename_open = !self.batch.rename_open,
            Command::Info => self.batch.info_open = !self.batch.info_open,
            Command::Undo => self.undo(now),
            Command::Redo => self.redo(now),
            Command::SaveAs => {
                if self.mode == Mode::Viewer {
                    self.open_save_as(ctx, now);
                }
            }
            Command::Browser if self.editor.has_selection() => self.apply_crop(ctx),
            Command::RotateRight if self.editor.session.is_some() => self.edit_turn(ctx, 1, false, false),
            Command::RotateLeft if self.editor.session.is_some() => self.edit_turn(ctx, 3, false, false),
            Command::FlipH if self.editor.session.is_some() => self.edit_turn(ctx, 0, true, false),
            Command::FlipV if self.editor.session.is_some() => self.edit_turn(ctx, 0, false, true),
            Command::SaveRotation if self.editor.session.is_some() => {
                if self.editor.is_dirty() {
                    self.save_edits(ctx, now);
                } else {
                    self.say("Nothing to save".to_string(), now);
                }
            }
            Command::Back if self.edit_back(now) => {}
            Command::Open => {
                let mut dialog = rfd::FileDialog::new().add_filter("Pictures", &crate::register::EXTENSIONS).add_filter("All files", &["*"]);
                if let Some(dir) = &self.dir {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.pick_file() {
                    self.open(ctx, OpenRequest { path, recursive: false, slideshow: false }, now);
                }
            }
            Command::Next => self.step_by_user(1, now),
            Command::Prev => self.step_by_user(-1, now),
            Command::Random => {
                // Any picture but the current one.
                let n = self.entries.len();
                if n > 1 {
                    let pick = (self.random() % (n as u64 - 1)) as usize;
                    self.go_to(if pick >= self.index { pick + 1 } else { pick });
                }
            }
            Command::First => {
                self.edge = None;
                self.go_to(0);
            }
            Command::Last => {
                self.edge = None;
                self.go_to(usize::MAX);
            }
            Command::ZoomIn => self.zoom_request = Some(ZoomRequest::Factor(ZOOM_STEP)),
            Command::ZoomOut => self.zoom_request = Some(ZoomRequest::Factor(1.0 / ZOOM_STEP)),
            Command::ZoomFit => self.zoom_request = Some(ZoomRequest::Fit),
            Command::ZoomActual => self.zoom_request = Some(ZoomRequest::Actual),
            Command::CycleFit => {
                let i = FitMode::ALL.iter().position(|m| *m == self.settings.fit_mode).unwrap_or(0);
                self.set_fit_mode(FitMode::ALL[(i + 1) % FitMode::ALL.len()], now);
            }
            Command::Fullscreen => {
                if self.mode == Mode::Browser {
                    self.set_mode(ctx, Mode::Viewer);
                }
                self.set_fullscreen(ctx, !self.fullscreen);
            }
            Command::Browser => {
                let next = if self.mode == Mode::Viewer { Mode::Browser } else { Mode::Viewer };
                if next == Mode::Viewer && self.entries.is_empty() {
                    return;
                }
                self.set_mode(ctx, next);
            }
            Command::SaveRotation => self.save_rotation(now),
            Command::Mark => {
                if let Some(path) = self.current_path().filter(|_| self.pasted.is_none()).cloned() {
                    if !self.marked.remove(&path) {
                        self.marked.insert(path);
                    }
                }
            }
            Command::MarkAll => self.marked = self.entries.iter().map(|e| e.path.clone()).collect(),
            Command::ClearMarks => self.marked.clear(),
            Command::ShowInExplorer => {
                // A selected folder tile, else the picture, else the folder being shown.
                let folder = self.grid.folder_selected.filter(|_| self.mode == Mode::Browser);
                let folder = folder.and_then(|i| self.grid_folders().into_iter().nth(i)).map(|(path, _)| path);
                let file = self.current_path().filter(|_| self.pasted.is_none()).cloned();
                if let Some(path) = folder.or(file).or_else(|| self.dir.clone()) {
                    if let Err(e) = show_in_file_manager(&path) {
                        self.say(format!("Could not open Explorer: {e}"), now);
                    }
                }
            }
            Command::Refresh => {
                // Read the folder, the folder tree and the pictures again from disk.
                self.tree.refresh();
                self.thumbs.forget_all();
                if let Some(path) = self.current_path().cloned() {
                    self.loader.forget(&path);
                }
                self.relist = true;
                self.say("Refreshed".to_string(), now);
            }
            Command::CopyTo => self.send_to_folder(false, now),
            Command::MoveTo => self.send_to_folder(true, now),
            Command::Toolbar => {
                self.settings.show_toolbar = !self.settings.show_toolbar;
                self.save_settings();
            }
            Command::Transparency => {
                self.settings.transparency = self.settings.transparency.next();
                self.save_settings();
                self.say(format!("Behind transparency: {}", self.settings.transparency.label()), now);
            }
            Command::Settings => self.settings_ui.open = !self.settings_ui.open,
            Command::RotateRight => self.view.rotate(true),
            Command::RotateLeft => self.view.rotate(false),
            Command::FlipH => self.view.flip_h = !self.view.flip_h,
            Command::FlipV => self.view.flip_v = !self.view.flip_v,
            Command::Delete => self.delete_current(now),
            Command::Rename => {
                if self.pasted.is_none() {
                    if let Some(name) = self.current_path().and_then(|p| p.file_name()) {
                        self.rename = Some(RenameDialog { name: name.to_string_lossy().into_owned(), error: None, focus: true });
                    }
                }
            }
            Command::Copy if self.mode == Mode::Browser => {
                let targets = self.browser_clip_targets();
                self.clip_files(targets, false, now);
            }
            Command::Copy if self.copy_selection(ctx, now) => {}
            Command::Copy => {
                if let Some(shown) = &self.shown {
                    let b = &shown.ready.loaded.frames[self.frame.min(shown.ready.loaded.frames.len() - 1)].bitmap;
                    let image = egui::ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba);
                    ctx.copy_image(image);
                    self.say("Picture copied".to_string(), now);
                }
            }
            Command::CopyPath => {
                if let Some(path) = self.current_path().filter(|_| self.pasted.is_none()) {
                    ctx.copy_text(path.display().to_string());
                    self.say("Path copied".to_string(), now);
                }
            }
            Command::Slideshow => {
                if self.slideshow_due.is_some() {
                    self.stop_slideshow(ctx);
                } else if self.entries.len() > 1 {
                    self.set_mode(ctx, Mode::Viewer);
                    self.slideshow_due = Some(now + self.settings.slideshow_seconds as f64);
                    if !self.fullscreen {
                        self.slideshow_owns_fullscreen = true;
                        self.set_fullscreen(ctx, true);
                    }
                }
            }
            Command::Picker => self.picker = !self.picker,
            Command::StatusBar => {
                self.settings.show_status_bar = !self.settings.show_status_bar;
                self.save_settings();
            }
            Command::NextPage | Command::PrevPage => {
                let pages = self.shown.as_ref().map_or(1, |s| s.ready.loaded.page_count);
                if pages > 1 && self.pasted.is_none() {
                    self.page = if command == Command::NextPage {
                        (self.page + 1).min(pages - 1)
                    } else {
                        self.page.saturating_sub(1)
                    };
                }
            }
            Command::Help => self.help = !self.help,
            Command::Back => {
                if self.duplicates.open {
                    self.duplicates.open = false;
                } else if self.search.open {
                    self.close_search();
                } else if self.batch.batch_open || self.batch.rename_open || self.batch.info_open {
                    // A running batch keeps its window; see `batch_window`.
                    self.batch.batch_open = false;
                    self.batch.rename_open = false;
                    self.batch.info_open = false;
                } else if self.settings_ui.open {
                    self.settings_ui.open = false;
                } else if self.help {
                    self.help = false;
                } else if self.slideshow_due.is_some() {
                    self.stop_slideshow(ctx);
                } else if self.fullscreen {
                    self.set_fullscreen(ctx, false);
                } else if self.mode == Mode::Viewer {
                    // Esc on a picture never closes the program: it shows the thumbnails of
                    // the picture's folder, with the picture selected.
                    self.set_mode(ctx, Mode::Browser);
                } else {
                    // In the thumbnails Esc never closes the program either: it clears the
                    // selection (marked files, a selected folder), and otherwise does nothing.
                    self.marked.clear();
                    self.grid.folder_selected = None;
                    self.grid.anchor = None;
                }
            }
        }
    }

    fn set_fit_mode(&mut self, mode: FitMode, now: f64) {
        self.settings.fit_mode = mode;
        self.view.reset_placement();
        self.save_settings();
        self.say(mode.label().to_string(), now);
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn stop_slideshow(&mut self, ctx: &egui::Context) {
        if self.slideshow_due.take().is_some() && self.slideshow_owns_fullscreen {
            self.set_fullscreen(ctx, false);
        }
        self.slideshow_owns_fullscreen = false;
    }

    fn tick_slideshow(&mut self, ctx: &egui::Context, now: f64) {
        let Some(due) = self.slideshow_due else { return };
        if now < due {
            ctx.request_repaint_after_secs((due - now) as f32);
            return;
        }
        let n = self.entries.len();
        if n < 2 {
            self.stop_slideshow(ctx);
            return;
        }
        if self.settings.slideshow_random {
            // Any picture but the current one.
            let pick = (self.random() % (n as u64 - 1)) as usize;
            self.go_to(if pick >= self.index { pick + 1 } else { pick });
        } else if self.index + 1 == n && !self.settings.slideshow_loop {
            self.stop_slideshow(ctx);
            self.say("Slideshow finished".to_string(), now);
            return;
        } else {
            self.step(1);
        }
        let seconds = self.settings.slideshow_seconds as f64;
        self.slideshow_due = Some(now + seconds);
        ctx.request_repaint_after_secs(seconds as f32);
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Send the marked files, or the current one, to the Recycle Bin.
    fn delete_current(&mut self, now: f64) {
        let paths = self.targets();
        if paths.is_empty() {
            return;
        }
        let mut gone = Vec::new();
        let mut problem = None;
        for path in &paths {
            match fileops::recycle(path) {
                Ok(()) => gone.push(path.clone()),
                Err(e) => problem = Some(format!("Could not delete {}: {e}", file_name(path))),
            }
        }
        self.remove_paths(&gone);
        let text = match (gone.as_slice(), problem) {
            (_, Some(problem)) => problem,
            ([one], None) => format!("Moved to Recycle Bin: {}", file_name(one)),
            (many, None) => format!("Moved {} files to the Recycle Bin", many.len()),
        };
        self.say(text, now);
    }

    fn commit_rename(&mut self, now: f64) {
        let Some(dialog) = &mut self.rename else { return };
        let Some(old) = self.entries.get(self.index).map(|e| e.path.clone()) else {
            self.rename = None;
            return;
        };
        match fileops::rename(&old, &dialog.name) {
            Ok(new) => {
                self.rename = None;
                if new != old {
                    self.loader.forget(&old);
                    self.thumbs.forget(&old);
                    if self.marked.remove(&old) {
                        self.marked.insert(new.clone());
                    }
                    if let Some(e) = self.all_entries.iter_mut().find(|e| e.path == old) {
                        e.path = new.clone();
                    }
                    folder::sort(&mut self.all_entries, self.settings.sort_key, self.settings.sort_descending);
                    self.apply_filter(Some(new.clone()), 0);
                    self.say(format!("Renamed to {}", file_name(&new)), now);
                }
            }
            Err(e) => dialog.error = Some(e.to_string()),
        }
    }

    fn say(&mut self, text: String, now: f64) {
        self.toast = Some((text, now + 2.5));
    }

    fn save_settings(&self) {
        let _ = self.settings.save();
    }

    // ----- input -----------------------------------------------------------------

    fn collect_commands(&mut self, ctx: &egui::Context, now: f64) -> Vec<Command> {
        let mut commands = Vec::new();
        // Keys belong to a text field or an open menu first.
        let typing = self.rename.is_some() || ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx);
        let mut events = ctx.input(|i| i.events.clone());
        events.append(&mut self.held_keys);
        for event in &events {
            match event {
                // The settings window is waiting for the new key of a command.
                egui::Event::Key { key, pressed: true, modifiers, .. } if self.settings_ui.capture.is_some() => {
                    let command = self.settings_ui.capture.take().unwrap();
                    if *key != egui::Key::Escape {
                        let ctrl = modifiers.ctrl || modifiers.command;
                        input::assign(&mut self.bindings, command, *key, ctrl, modifiers.shift);
                        self.settings.keys = Some(input::to_settings(&self.bindings));
                        self.save_settings();
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. }
                    if !typing && self.mode == Mode::Browser && !modifiers.any() && self.browser_key(*key) => {}
                egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if !typing => {
                    let ctrl = modifiers.ctrl || modifiers.command;
                    if let Some(c) = input::lookup(&self.bindings, *key, ctrl, modifiers.shift, modifiers.alt, *repeat) {
                        // In a file with several pages the left and right arrows turn its
                        // pages. PageUp, PageDown and the wheel still change the picture.
                        let pages = self.shown.as_ref().map_or(1, |s| s.ready.loaded.page_count);
                        let paged = self.mode == Mode::Viewer && pages > 1 && self.pasted.is_none();
                        commands.push(match (c, *key) {
                            (Command::Next, egui::Key::ArrowRight) if paged => Command::NextPage,
                            (Command::Prev, egui::Key::ArrowLeft) if paged => Command::PrevPage,
                            _ => c,
                        });
                    }
                }
                egui::Event::Copy if !typing => commands.push(Command::Copy),
                egui::Event::Cut if !typing && self.mode == Mode::Browser => {
                    let targets = self.browser_clip_targets();
                    self.clip_files(targets, true, now);
                }
                // Ctrl+V with files on the clipboard produces no paste event (there is no
                // text or picture to hand over), but the key's release still arrives.
                egui::Event::Key { key: egui::Key::V, pressed: false, modifiers, .. }
                    if !typing && self.mode == Mode::Browser && (modifiers.ctrl || modifiers.command) =>
                {
                    if let Some(dir) = self.dir.clone() {
                        self.paste_files(dir, now);
                    }
                }
                egui::Event::PasteImage(image) if !typing => {
                    self.pasted = Some(Arc::new(Ready::from_color_image(ctx, image)));
                    self.say("Pasted from clipboard".to_string(), now);
                }
                _ => {}
            }
        }
        commands
    }

    /// Keys that mean something else in the thumbnail grid. Returns true when handled.
    fn browser_key(&mut self, key: egui::Key) -> bool {
        let cols = self.grid.columns.max(1) as i64;
        let page = cols * self.grid.visible_rows.max(1) as i64;
        let n = self.entries.len() as i64;
        if key == egui::Key::Enter {
            // Enter on a selected folder goes into it; otherwise it opens the viewer.
            let folders = self.grid_folders();
            let Some((dir, _)) = self.grid.folder_selected.and_then(|i| folders.get(i)) else { return false };
            self.grid.open_dir = Some(dir.clone());
            return true;
        }
        let mut jump = |delta: i64| {
            self.grid.folder_selected = None;
            if n > 0 {
                let target = (self.index as i64 + delta).clamp(0, n - 1);
                self.go_to(target as usize);
            }
        };
        match key {
            egui::Key::ArrowUp => jump(-cols),
            egui::Key::ArrowDown => jump(cols),
            egui::Key::PageUp => jump(-page),
            egui::Key::PageDown => jump(page),
            egui::Key::ArrowLeft => jump(-1),
            egui::Key::ArrowRight => jump(1),
            egui::Key::Space => {
                if let Some(path) = self.current_path().cloned() {
                    if !self.marked.remove(&path) {
                        self.marked.insert(path);
                    }
                }
            }
            egui::Key::Backspace => self.grid.go_back = true,
            _ => return false,
        }
        true
    }

    /// Wheel notches since the last frame; positive is "away from the user".
    fn wheel_steps(ctx: &egui::Context) -> f32 {
        ctx.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                    },
                    _ => 0.0,
                })
                .sum()
        })
    }

    // ----- drawing ---------------------------------------------------------------

    fn menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<Command>, now: f64) {
        enlarge_menu(ui);
        let bindings = self.bindings.clone();
        let mut item = |ui: &mut egui::Ui, command: Command| {
            let button = egui::Button::new(command.label()).shortcut_text(input::shortcut(&bindings, command));
            if ui.add(button).clicked() {
                out.push(command);
                ui.close();
            }
        };
        let has_file = self.current_path().is_some() && self.pasted.is_none();

        item(ui, Command::Open);
        if self.mode == Mode::Browser {
            // Here Ctrl+C / X / V work on files, as in Explorer.
            let file_item = |ui: &mut egui::Ui, label: &str, keys: &str| {
                let clicked = ui.add(egui::Button::new(label).shortcut_text(keys)).clicked();
                if clicked {
                    ui.close();
                }
                clicked
            };
            if has_file && file_item(ui, "Copy file", "Ctrl+C") {
                self.clip_files(self.targets(), false, now);
            }
            if has_file && file_item(ui, "Cut file", "Ctrl+X") {
                self.clip_files(self.targets(), true, now);
            }
            if file_item(ui, "Paste files here", "Ctrl+V") {
                if let Some(dir) = self.dir.clone() {
                    self.paste_files(dir, now);
                }
            }
        } else if self.shown.is_some() {
            item(ui, Command::Copy);
        }
        if has_file {
            item(ui, Command::CopyPath);
            item(ui, Command::ShowInExplorer);
            item(ui, Command::Rename);
            item(ui, Command::Delete);
            item(ui, Command::Mark);
            ui.menu_button("Marked files", |ui| {
                enlarge_menu(ui);
                for c in [Command::CopyTo, Command::MoveTo, Command::MarkAll, Command::ClearMarks] {
                    item(ui, c);
                }
                ui.weak("Copy and move use the current picture when nothing is marked.");
            });
        }
        ui.menu_button("Tools", |ui| {
            enlarge_menu(ui);
            for c in [Command::Info, Command::Batch, Command::BatchRename, Command::Search, Command::Duplicates] {
                item(ui, c);
            }
        });
        ui.separator();
        item(ui, Command::Edit);
        item(ui, Command::Crop);
        if self.mode == Mode::Viewer && self.shown.is_some() && !self.editor.crop_active() {
            let hint = "Dragging then selects an area, even when zoomed in. Esc leaves it.";
            if ui.add(egui::Button::new("Selection mode")).on_hover_text(hint).clicked() {
                self.editor.begin_selection();
                ui.close();
            }
        }
        item(ui, Command::SaveAs);
        item(ui, Command::Browser);
        ui.menu_button("Zoom", |ui| {
            enlarge_menu(ui);
            for c in [Command::ZoomIn, Command::ZoomOut, Command::ZoomActual, Command::ZoomFit] {
                item(ui, c);
            }
            ui.separator();
            let mut mode = self.settings.fit_mode;
            for m in FitMode::ALL {
                ui.radio_value(&mut mode, m, m.label());
            }
            if mode != self.settings.fit_mode {
                self.set_fit_mode(mode, now);
            }
            ui.separator();
            if ui.checkbox(&mut self.settings.keep_zoom, "Keep zoom between pictures").changed() {
                self.save_settings();
            }
        });
        ui.menu_button("Rotate and flip", |ui| {
            enlarge_menu(ui);
            for c in [Command::RotateRight, Command::RotateLeft, Command::FlipH, Command::FlipV] {
                item(ui, c);
            }
            ui.separator();
            item(ui, Command::SaveRotation);
            ui.weak("Until saved, only the view changes.");
        });
        ui.menu_button("Sort by", |ui| {
            enlarge_menu(ui);
            let (mut key, mut descending) = (self.settings.sort_key, self.settings.sort_descending);
            for k in SortKey::ALL {
                ui.radio_value(&mut key, k, k.label());
            }
            ui.separator();
            ui.checkbox(&mut descending, "Descending");
            if key != self.settings.sort_key || descending != self.settings.sort_descending {
                self.settings.sort_key = key;
                self.settings.sort_descending = descending;
                self.resort();
            }
        });
        ui.menu_button("Slideshow", |ui| {
            enlarge_menu(ui);
            item(ui, Command::Slideshow);
            ui.separator();
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Seconds per picture");
                let drag = egui::DragValue::new(&mut self.settings.slideshow_seconds).range(0.5..=3600.0).speed(0.1);
                changed |= ui.add(drag).changed();
            });
            changed |= ui.checkbox(&mut self.settings.slideshow_random, "Random order").changed();
            changed |= ui.checkbox(&mut self.settings.slideshow_loop, "Start over at the end").changed();
            if changed {
                self.save_settings();
            }
        });
        ui.menu_button("View", |ui| {
            enlarge_menu(ui);
            item(ui, Command::Fullscreen);
            item(ui, Command::Toolbar);
            ui.menu_button("Behind transparency", |ui| {
                enlarge_menu(ui);
                let mut backdrop = self.settings.transparency;
                for b in Backdrop::ALL {
                    ui.radio_value(&mut backdrop, b, b.label());
                }
                ui.weak("T switches between them.");
                if backdrop != self.settings.transparency {
                    self.settings.transparency = backdrop;
                    self.save_settings();
                }
            });
            let mut bar = self.settings.show_status_bar;
            if ui.checkbox(&mut bar, "Status bar").changed() {
                self.settings.show_status_bar = bar;
                self.save_settings();
            }
            ui.checkbox(&mut self.picker, "Colour picker");
            ui.separator();
            let mut theme = self.settings.theme;
            ui.radio_value(&mut theme, Theme::System, "Theme: follow Windows");
            ui.radio_value(&mut theme, Theme::Light, "Theme: light");
            ui.radio_value(&mut theme, Theme::Dark, "Theme: dark");
            if theme != self.settings.theme {
                self.settings.theme = theme;
                apply_theme(ui.ctx(), theme);
                self.save_settings();
            }
            ui.separator();
            let mut wheel = self.settings.wheel;
            ui.radio_value(&mut wheel, WheelAction::Zoom, "Mouse wheel zooms");
            ui.radio_value(&mut wheel, WheelAction::Navigate, "Mouse wheel changes picture");
            ui.weak("Ctrl + wheel does the other one.");
            if wheel != self.settings.wheel {
                self.settings.wheel = wheel;
                self.save_settings();
            }
        });
        ui.separator();
        item(ui, Command::Settings);
        item(ui, Command::Help);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, out: &mut Vec<Command>) {
        let commands: Vec<Command> = match &self.settings.toolbar {
            Some(ids) => ids.iter().filter_map(|id| Command::from_id(id)).collect(),
            None => Command::DEFAULT_TOOLBAR.to_vec(),
        };
        ui.horizontal_wrapped(|ui| {
            for command in commands {
                let keys = input::shortcut(&self.bindings, command);
                let tip = if keys.is_empty() { command.label().to_string() } else { format!("{}  ({keys})", command.label()) };
                if ui.button(command.short()).on_hover_text(tip).clicked() {
                    out.push(command);
                }
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let fields = self.settings.status;
        ui.horizontal(|ui| {
            let mut first = true;
            let mut field = |ui: &mut egui::Ui, text: String| {
                if !std::mem::take(&mut first) {
                    ui.separator();
                }
                ui.label(text);
            };
            let Some(entry) = self.entries.get(self.index).filter(|_| self.pasted.is_none()) else {
                field(ui, if self.pasted.is_some() { "Clipboard" } else { "No picture" }.to_string());
                return;
            };
            if fields.name {
                field(ui, file_name(&entry.path));
            }
            if fields.position {
                field(ui, format!("{} / {}", self.index + 1, self.entries.len()));
            }
            if let Some(shown) = self.shown.as_ref().filter(|s| s.key.as_ref().is_some_and(|k| k.0 == entry.path)) {
                let loaded = &shown.ready.loaded;
                if fields.dimensions {
                    field(ui, format!("{} × {}", loaded.width(), loaded.height()));
                }
                if fields.format {
                    field(ui, loaded.format.name().to_string());
                }
                if loaded.page_count > 1 {
                    field(ui, format!("Page {} / {}", loaded.page + 1, loaded.page_count));
                }
                if loaded.is_animated() {
                    field(ui, format!("{} frames", loaded.frames.len()));
                }
            }
            if fields.size {
                field(ui, human_size(entry.size));
            }
            if fields.date {
                field(ui, date_text(entry.modified));
            }
            if let Some(zoom) = self.last_zoom.filter(|_| fields.zoom && self.mode == Mode::Viewer) {
                field(ui, format!("{:.0}%", zoom * 100.0));
            }
            if !self.marked.is_empty() {
                field(ui, format!("{} marked", self.marked.len()));
            }
            if self.recursive {
                field(ui, "with subfolders".to_string());
            }
            if self.listing.is_some() {
                field(ui, "reading folder…".to_string());
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>, now: f64) {
        let ctx = ui.ctx().clone();
        let canvas = ui.max_rect();
        let response = ui.allocate_rect(canvas, Sense::click_and_drag());
        let painter = ui.painter_at(canvas);
        let text_color = ui.visuals().text_color();

        // With a rectangle selected, a double-click must not turn into "crop" (Enter does that).
        if response.double_clicked() && !self.editor.crop_active() {
            commands.push(Command::Browser);
        }
        if response.middle_clicked() {
            commands.push(Command::Fullscreen);
        }
        let mut steps = if response.hovered() { Self::wheel_steps(&ctx) } else { 0.0 };
        if steps != 0.0 && self.slideshow_due.is_some() {
            // Turning the wheel takes over from the slideshow: it stops on the current
            // picture, staying in fullscreen. This first notch does nothing else.
            self.slideshow_due = None;
            self.slideshow_owns_fullscreen = false;
            self.say("Slideshow stopped".to_string(), now);
            steps = 0.0;
        }
        let ctrl = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
        let wheel_zooms = (self.settings.wheel == WheelAction::Zoom) != ctrl;
        if steps != 0.0 && !wheel_zooms {
            commands.push(if steps < 0.0 { Command::Next } else { Command::Prev });
        }

        // While a slow picture (RAW, HEIC, a huge JPEG) is still decoding, its thumbnail
        // stands in for it, so there is something to look at straight away.
        let placeholder = match self.entries.get(self.index) {
            Some(entry) if self.loading_since.is_some() && self.pasted.is_none() => match self.thumbs.get(&entry.clone()) {
                crate::thumbnails::Thumb::Ready(texture) => Some(texture),
                _ => None,
            },
            _ => None,
        };

        // Ask for the neighbours' thumbnails too, so moving on has a stand-in ready at once;
        // but only once the picture on screen is done, so they do not hold it up.
        if self.pasted.is_none() && self.entries.len() > 1 && self.loading_since.is_none() {
            let n = self.entries.len();
            for delta in [1, n - 1] {
                let neighbour = self.entries[(self.index + delta) % n].clone();
                let _ = self.thumbs.get(&neighbour);
            }
        }
        // The last finished picture may belong to a file already passed. It must never be
        // drawn for the current one: when a slow picture finishes after the user moved on,
        // it would look as if the viewer had gone back to it.
        let current_path = self.current_path().cloned();
        let shown_is_current = self.shown.as_ref().is_some_and(|s| match &s.key {
            Some(key) => Some(&key.0) == current_path.as_ref(),
            None => true,
        });

        if let Some(texture) = &placeholder {
            self.zoom_request = None;
            self.last_zoom = None;
            let size = texture.size_vec2();
            let scale = (canvas.width() / size.x).min(canvas.height() / size.y);
            let rect = Rect::from_center_size(canvas.center(), size * scale);
            paint_image(&painter, rect, texture.id(), View::default().uvs());
        } else if let Some(shown) = self.shown.as_ref().filter(|_| shown_is_current) {
            let ready = shown.ready.clone();
            let loaded = &ready.loaded;
            let ppp = ctx.pixels_per_point();
            self.last_canvas = (canvas.size(), ppp);
            let shown_px = self.view.shown_size(loaded.width(), loaded.height());
            let mode = self.settings.fit_mode;
            let canvas_px = canvas.size() * ppp;
            let zoom_now = |view: &View| view.effective_zoom(mode, shown_px, canvas_px);
            let auto_now = |view: &View| View::auto_offset(mode, shown_px * zoom_now(view) / ppp, canvas.size());

            // What a drag does. A picture larger than the window (zoomed in) is moved by
            // dragging. One that fits is selected by dragging. Once selection mode is on
            // (right-click > Selection mode) or a rectangle is on screen, drags select or
            // adjust it even when zoomed in. Shift held while selecting makes the rectangle a square.
            let size_now = shown_px * zoom_now(&self.view) / ppp;
            let pannable = size_now.x > canvas.width() + 1.0 || size_now.y > canvas.height() + 1.0;
            let panning = pannable && !self.editor.crop_active();
            if response.drag_started_by(egui::PointerButton::Primary) && !panning && !self.editor.is_busy() {
                self.editor.begin_selection();
            }

            match self.zoom_request.take() {
                Some(ZoomRequest::Factor(f)) => {
                    let (z, auto) = (zoom_now(&self.view), auto_now(&self.view));
                    self.view.zoom_about(z, z * f, Vec2::ZERO, auto);
                }
                Some(ZoomRequest::Actual) => {
                    let (z, auto) = (zoom_now(&self.view), auto_now(&self.view));
                    self.view.zoom_about(z, 1.0, Vec2::ZERO, auto);
                }
                Some(ZoomRequest::Fit) => self.view.reset_placement(),
                None => {}
            }
            if steps != 0.0 && wheel_zooms {
                let anchor = response.hover_pos().map_or(Vec2::ZERO, |p| p - canvas.center());
                let (z, auto) = (zoom_now(&self.view), auto_now(&self.view));
                self.view.zoom_about(z, z * ZOOM_STEP.powf(steps), anchor, auto);
            }
            if response.dragged_by(egui::PointerButton::Primary) && panning {
                if self.view.zoom.is_none() {
                    self.view.offset = auto_now(&self.view);
                    self.view.zoom = Some(zoom_now(&self.view));
                }
                self.view.offset += response.drag_delta();
            }

            let zoom = zoom_now(&self.view);
            let size = shown_px * zoom / ppp;
            let offset = if self.view.zoom.is_some() {
                self.view.offset = View::clamp_offset(self.view.offset, size, canvas.size(), DRAG_OVERSHOOT);
                self.view.offset
            } else {
                auto_now(&self.view)
            };
            let mut rect = Rect::from_center_size(canvas.center() + offset, size);
            // Sit exactly on the pixel grid so 100% is not blurred by half-pixel offsets.
            let snapped = (rect.min.to_vec2() * ppp).round() / ppp;
            rect = Rect::from_min_size(snapped.to_pos2(), size);

            let frame = self.frame.min(ready.textures.len() - 1);
            let texture = self.editor.preview_texture().unwrap_or(ready.textures[frame].id());
            if ready.transparent {
                self.paint_backdrop(&ctx, &painter, rect.intersect(canvas));
            }
            if !self.editor.paint_straighten(&painter, rect, texture, (loaded.width(), loaded.height())) {
                paint_image(&painter, rect, texture, self.view.uvs());
            }
            self.editor.crop_ui(&response, &painter, rect, shown_px.x as u32, shown_px.y as u32, panning);
            self.last_zoom = Some(zoom);
            // Red border: the user pushed against the end of the list. It stays until
            // they move on.
            if self.edge.is_some() {
                let red = Color32::from_rgb(230, 40, 40);
                painter.rect_stroke(rect.intersect(canvas), 0.0, (4.0, red), egui::StrokeKind::Inside);
            }
            if self.current_path().is_some_and(|p| self.marked.contains(p)) && self.pasted.is_none() {
                badge(&painter, canvas.right_top() + vec2(-10.0, 10.0), Align2::RIGHT_TOP, "Marked");
            }

            if loaded.page_count > 1 && self.pasted.is_none() {
                page_badge(&painter, canvas, loaded.page + 1, loaded.page_count);
                if let Some(command) = page_bar(ui, canvas, loaded.page + 1, loaded.page_count) {
                    commands.push(command);
                }
            }

            // Animation playback.
            if loaded.is_animated() {
                if now >= self.frame_due {
                    self.frame = (frame + 1) % loaded.frames.len();
                    self.frame_due = now + loaded.frames[self.frame].delay_ms as f64 / 1000.0;
                }
                ctx.request_repaint_after_secs((self.frame_due - now).max(0.0) as f32);
            }

            // Zoom readout while it is not the plain fit.
            if self.view.zoom.is_some() {
                let label = format!("{:.0}%", zoom * 100.0);
                badge(&painter, canvas.right_bottom() - vec2(10.0, 10.0), Align2::RIGHT_BOTTOM, &label);
            }

            if self.picker {
                if let Some(pos) = response.hover_pos() {
                    let bitmap = &loaded.frames[frame].bitmap;
                    let hit = self.view.pixel_at(pos, rect, loaded.width(), loaded.height());
                    if let Some(([r, g, b, a], (x, y))) = hit.and_then(|(x, y)| Some((bitmap.pixel(x, y)?, (x, y)))) {
                        let hex = format!("#{r:02X}{g:02X}{b:02X}");
                        let text = format!("{hex}   rgb({r}, {g}, {b})   alpha {a}   x {x}, y {y}");
                        let anchor = pos + vec2(18.0, 18.0);
                        let swatch = Rect::from_min_size(anchor, vec2(22.0, 22.0));
                        badge(&painter, anchor + vec2(28.0, 0.0), Align2::LEFT_TOP, &text);
                        painter.rect_filled(swatch, 3.0, Color32::from_rgb(r, g, b));
                        painter.rect_stroke(swatch, 3.0, (1.0, Color32::WHITE), egui::StrokeKind::Outside);
                        if response.clicked() {
                            ctx.copy_text(hex.clone());
                            self.say(format!("Copied {hex}"), now);
                        }
                    }
                }
            }
        } else if let Some(error) = &self.error {
            self.last_zoom = None;
            let name = self.current_path().map(|p| file_name(p)).unwrap_or_default();
            let text = format!("Cannot show {name}\n{error}");
            painter.text(canvas.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), text_color);
        } else if self.current_path().is_none() && self.listing.is_none() {
            let text = "Drop a picture or a folder here\nCtrl+O to open   ·   F1 for keys   ·   right-click for the menu";
            painter.text(canvas.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), text_color);
        }

        // With no picture on screen (unreadable file) the border goes around the window.
        if !shown_is_current && placeholder.is_none() && self.edge.is_some() {
            let red = Color32::from_rgb(230, 40, 40);
            painter.rect_stroke(canvas, 0.0, (4.0, red), egui::StrokeKind::Inside);
        }

        if let Some(label) = self.editor.working_on() {
            badge(&painter, canvas.center_top() + vec2(0.0, 12.0), Align2::CENTER_TOP, &format!("{label}…"));
            ctx.request_repaint_after_secs(0.1);
        }
        if self.loading_since.is_some_and(|since| now - since > 0.15) {
            badge(&painter, canvas.left_top() + vec2(10.0, 10.0), Align2::LEFT_TOP, "Loading…");
            ctx.request_repaint_after_secs(0.1);
        } else if self.loading_since.is_some() {
            ctx.request_repaint_after_secs(0.16);
        }

        if let Some((text, until)) = &self.toast {
            if now < *until {
                badge(&painter, canvas.left_bottom() + vec2(10.0, -10.0), Align2::LEFT_BOTTOM, text);
                ctx.request_repaint_after_secs((*until - now) as f32);
            } else {
                self.toast = None;
            }
        }

        response.context_menu(|ui| {
            if self.editor.has_selection() {
                self.selection_menu(ui, commands, now);
            } else {
                self.menu(ui, commands, now);
            }
        });
    }

    /// Fill `area` with what should show through a transparent picture.
    fn paint_backdrop(&mut self, ctx: &egui::Context, painter: &egui::Painter, area: Rect) {
        match self.settings.transparency {
            Backdrop::Normal => {}
            Backdrop::Black => {
                painter.rect_filled(area, 0.0, Color32::BLACK);
            }
            Backdrop::White => {
                painter.rect_filled(area, 0.0, Color32::WHITE);
            }
            Backdrop::Checkerboard => {
                const CELL: f32 = 8.0;
                let tile = self.checker.get_or_insert_with(|| {
                    let (light, dark) = (Color32::from_gray(204), Color32::from_gray(153));
                    let image = egui::ColorImage::new([2, 2], vec![light, dark, dark, light]);
                    let options = egui::TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Nearest,
                        wrap_mode: egui::TextureWrapMode::Repeat,
                        mipmap_mode: None,
                    };
                    ctx.load_texture("checkerboard", image, options)
                });
                // Texture coordinates follow the screen, so the squares keep their size
                // and stay put however the picture is zoomed or moved.
                let to_uv = |p: egui::Pos2| egui::pos2(p.x / (2.0 * CELL), p.y / (2.0 * CELL));
                let mut mesh = egui::Mesh::with_texture(tile.id());
                mesh.add_rect_with_uv(area, Rect::from_min_max(to_uv(area.min), to_uv(area.max)), Color32::WHITE);
                painter.add(egui::Shape::mesh(mesh));
            }
        }
    }

    fn rename_dialog(&mut self, ctx: &egui::Context, now: f64) {
        let Some(dialog) = &mut self.rename else { return };
        let mut commit = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::unique("rename")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Rename");
            ui.add_space(6.0);
            let edit = ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(f32::INFINITY));
            if dialog.focus {
                edit.request_focus();
                dialog.focus = false;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                commit |= ui.button("Rename").clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if commit {
            self.commit_rename(now);
        } else if cancel || modal.should_close() {
            self.rename = None;
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.help;
        egui::Window::new("Keyboard shortcuts").open(&mut open).collapsible(false).resizable(false).vscroll(true).show(ctx, |ui| {
            egui::Grid::new("keys").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                for command in Command::ALL {
                    let keys = input::all_shortcuts(&self.bindings, command);
                    if keys.is_empty() {
                        continue;
                    }
                    ui.label(command.label());
                    ui.label(keys.join(",  "));
                    ui.end_row();
                }
                for (what, how) in [
                    ("Paste a picture", "Ctrl+V"),
                    ("Zoom at the pointer", "Mouse wheel"),
                    ("Previous / next folder", "Mouse back / forward, Alt+Left / Alt+Right"),
                    ("Select a rectangle", "Drag"),
                    ("Move the picture", "Shift + drag"),
                    ("Menu for the selection", "Right-click"),
                    ("Browser", "Double-click"),
                    ("Fullscreen / windowed", "Middle-click"),
                ] {
                    ui.label(what);
                    ui.label(how);
                    ui.end_row();
                }
            });
        });
        self.help = open;
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Tab is one of SuperView's own keys. Leave it to a text field being typed in.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let held = &mut self.held_keys;
        raw_input.events.retain(|event| {
            let tab = matches!(event, egui::Event::Key { key: egui::Key::Tab, .. });
            if tab {
                held.push(event.clone());
            }
            !tab
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.loader.set_max_texture_side(ctx.input(|i| i.max_texture_side));

        // Requests from a second launch.
        let mut raise = false;
        while let Ok(request) = self.requests.try_recv() {
            raise = true;
            if let Some(request) = request {
                self.open(&ctx, request, now);
            }
        }
        if raise {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // The mouse's back and forward buttons, and Alt+Left / Alt+Right as in Explorer.
        let (go_back, go_forward) = ctx.input(|i| {
            // The modifiers come from the key event itself: a quick tap can be over
            // before the frame's own modifier state has caught up.
            let alt_key = |wanted: egui::Key| {
                i.events.iter().any(|e| {
                    matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                        if *key == wanted && modifiers.alt && !modifiers.ctrl && !modifiers.shift)
                })
            };
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1) || alt_key(egui::Key::ArrowLeft),
                i.pointer.button_pressed(egui::PointerButton::Extra2) || alt_key(egui::Key::ArrowRight),
            )
        });
        if go_back {
            self.history(&ctx, false, now);
        } else if go_forward {
            self.history(&ctx, true, now);
        }

        let dropped = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| !p.as_os_str().is_empty()));
        if let Some(path) = dropped {
            self.open(&ctx, OpenRequest { path, recursive: false, slideshow: false }, now);
        }

        self.poll_listing(now);
        if self.autoplay_pending && self.listing.is_none() {
            self.autoplay_pending = false;
            if self.entries.len() > 1 && self.slideshow_due.is_none() {
                self.run(Command::Slideshow, &ctx, now);
            }
        }
        let mut commands = self.collect_commands(&ctx, now);
        for command in commands.drain(..) {
            self.run(command, &ctx, now);
        }
        self.tick_slideshow(&ctx, now);

        self.poll_edit(now);
        self.poll_batch(now);
        self.poll_duplicates(now);
        self.sync_loader();
        self.loader.poll();
        self.update_shown(now);

        self.thumbs.begin_frame();
        if self.mode == Mode::Viewer && self.settings.show_toolbar && !self.fullscreen {
            egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, &mut commands));
        }
        if self.settings.show_status_bar && !self.fullscreen {
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        }
        if self.mode == Mode::Browser {
            self.browser_panels(ui, &mut commands, now);
        }
        let background = if self.fullscreen {
            Color32::BLACK
        } else if ui.visuals().dark_mode {
            Color32::from_gray(18)
        } else {
            Color32::from_gray(228)
        };
        if self.mode == Mode::Viewer && self.editor.panel && !self.fullscreen {
            egui::Panel::right("edit").resizable(true).default_size(300.0).min_size(260.0).show(ui, |ui| self.edit_panel(ui, now));
        }
        if self.mode == Mode::Browser {
            egui::CentralPanel::default().show(ui, |ui| self.grid(ui, &mut commands, now));
        } else {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(background)).show(ui, |ui| {
                self.canvas(ui, &mut commands, now);
            });
        }
        self.thumbs.end_frame();
        self.rename_dialog(&ctx, now);
        self.help_window(&ctx);
        self.settings_window(&ctx, now);
        self.save_as_window(&ctx, now);
        self.batch_window(&ctx, now);
        self.rename_window(&ctx, now);
        self.info_window(&ctx, now);
        self.duplicates_window(&ctx, &mut commands);

        // Folder changes asked for by the browser.
        if let Some(dir) = self.grid.open_dir.take() {
            self.open(&ctx, OpenRequest { path: dir, recursive: false, slideshow: false }, now);
        }
        if let Some((path, action)) = self.grid.folder_action.take() {
            use browser::FolderAction;
            match action {
                FolderAction::Open => self.grid.open_dir = Some(path),
                FolderAction::OpenInExplorer => {
                    if let Err(e) = show_in_file_manager(&path) {
                        self.say(format!("Could not open Explorer: {e}"), now);
                    }
                }
                FolderAction::Slideshow { subfolders } => {
                    self.force_recursive = Some(subfolders);
                    self.open(&ctx, OpenRequest { path, recursive: subfolders, slideshow: true }, now);
                    self.force_recursive = None;
                }
                FolderAction::Copy => self.clip_files(vec![path], false, now),
                FolderAction::Cut => self.clip_files(vec![path], true, now),
                FolderAction::Paste => self.paste_files(path, now),
            }
        }
        if std::mem::take(&mut self.grid.go_back) {
            // Back to where we came from; in the folder SuperView started in there is
            // no such place, so go up instead.
            if self.back.iter().any(|d| d.is_dir()) {
                self.history(&ctx, false, now);
            } else {
                self.grid.go_up = true;
            }
        }
        if std::mem::take(&mut self.grid.go_up) {
            if let Some(parent) = self.dir.as_ref().and_then(|d| d.parent()).map(Path::to_path_buf) {
                self.open(&ctx, OpenRequest { path: parent, recursive: false, slideshow: false }, now);
            }
        }
        if std::mem::take(&mut self.relist) {
            self.start_listing(&ctx);
        }
        if let Some(recursive) = self.grid.set_recursive.take() {
            self.recursive = recursive;
            self.start_listing(&ctx);
        }

        // Commands from the mouse and the menu take effect on the next frame.
        if !commands.is_empty() {
            for command in commands {
                self.run(command, &ctx, now);
            }
            ctx.request_repaint();
        }

        let title = match (self.pasted.is_some(), self.current_path()) {
            (true, _) => "Clipboard - SuperView".to_string(),
            (false, Some(path)) => format!("{} - SuperView", file_name(path)),
            (false, None) => "SuperView".to_string(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
}

/// How far past its edge a zoomed-in picture can be dragged, in points.
const DRAG_OVERSHOOT: f32 = 100.0;

/// Show `path` in the system's file manager (Explorer on Windows): a folder is opened, a
/// file is shown selected in its folder.
fn show_in_file_manager(path: &Path) -> std::io::Result<()> {
    let mut command = std::process::Command::new(if cfg!(windows) { "explorer.exe" } else { "xdg-open" });
    if path.is_dir() {
        command.arg(path);
    } else if cfg!(windows) {
        // Explorer wants "/select," and the path as one argument, quoted only around the path.
        #[cfg(windows)]
        std::os::windows::process::CommandExt::raw_arg(&mut command, format!("/select,\"{}\"", path.display()));
    } else {
        command.arg(path.parent().unwrap_or(path));
    }
    command.spawn().map(|_| ())
}

/// Right-click menus are drawn half as large again as ordinary text, for easier reading.
const MENU_SCALE: f32 = 1.5;

/// Enlarge everything a menu draws. Sizes are taken from the app-wide style each time,
/// so calling this on a menu that is already enlarged changes nothing.
pub(super) fn enlarge_menu(ui: &mut egui::Ui) {
    let base = ui.ctx().global_style();
    let style = ui.style_mut();
    for (name, font) in style.text_styles.iter_mut() {
        if let Some(original) = base.text_styles.get(name) {
            font.size = original.size * MENU_SCALE;
        }
    }
    style.spacing.button_padding = base.spacing.button_padding * MENU_SCALE;
    style.spacing.item_spacing = base.spacing.item_spacing * MENU_SCALE;
    style.spacing.interact_size = base.spacing.interact_size * MENU_SCALE;
    style.spacing.icon_width = base.spacing.icon_width * MENU_SCALE;
    style.spacing.icon_width_inner = base.spacing.icon_width_inner * MENU_SCALE;
    style.spacing.icon_spacing = base.spacing.icon_spacing * MENU_SCALE;
}

/// Draw a texture into `rect` with the given corner texture coordinates.
fn paint_image(painter: &egui::Painter, rect: Rect, texture: egui::TextureId, uvs: [egui::Pos2; 4]) {
    let mut mesh = egui::Mesh::with_texture(texture);
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn date_text(time: std::time::SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = time.into();
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Small text label on a dark plate, readable over any picture.
fn badge(painter: &egui::Painter, pos: egui::Pos2, anchor: Align2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(13.0), Color32::WHITE);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.rect_filled(rect.expand2(vec2(7.0, 4.0)), 4.0, Color32::from_black_alpha(190));
    painter.galley(rect.min, galley, Color32::WHITE);
}

/// A book with the page number under it, at the right edge of the picture area: the sign
/// of a file with several pages.
fn page_badge(painter: &egui::Painter, canvas: Rect, page: usize, count: usize) {
    let book = vec2(72.0, 56.0);
    let text = format!("{page} / {count}");
    let galley = painter.layout_no_wrap(text, FontId::proportional(20.0), Color32::WHITE);
    let plate = Rect::from_center_size(
        egui::pos2(canvas.right() - 16.0 - (book.x + 28.0) / 2.0, canvas.center().y),
        vec2(book.x.max(galley.size().x) + 28.0, book.y + galley.size().y + 34.0),
    );
    painter.rect_filled(plate, 10.0, Color32::from_black_alpha(170));

    // An open book: two pages meeting at the spine, with a few lines of text.
    let area = Rect::from_min_size(egui::pos2(plate.center().x - book.x / 2.0, plate.top() + 12.0), book);
    let stroke = egui::Stroke::new(2.5, Color32::WHITE);
    let spine = area.center().x;
    let (top, bottom, dip) = (area.top() + 6.0, area.bottom() - 2.0, 7.0);
    for side in [-1.0_f32, 1.0] {
        let outer = spine + side * book.x / 2.0;
        let page = vec![
            egui::pos2(spine, top + dip),
            egui::pos2(spine + side * book.x * 0.25, top),
            egui::pos2(outer, top + 2.0),
            egui::pos2(outer, bottom - dip + 2.0),
            egui::pos2(spine + side * book.x * 0.25, bottom - dip),
            egui::pos2(spine, bottom),
        ];
        painter.add(egui::Shape::closed_line(page, stroke));
        for row in 0..3 {
            let y = top + 15.0 + row as f32 * 9.0;
            let thin = egui::Stroke::new(1.5, Color32::from_white_alpha(170));
            painter.line_segment([egui::pos2(spine + side * 8.0, y + 2.0), egui::pos2(outer - side * 7.0, y)], thin);
        }
    }
    painter.galley(egui::pos2(plate.center().x - galley.size().x / 2.0, area.bottom() + 10.0), galley, Color32::WHITE);
}

/// Previous page, the page number and next page, at the bottom of the picture area of a
/// file with several pages. Returns the command of the arrow that was clicked.
fn page_bar(ui: &egui::Ui, canvas: Rect, page: usize, count: usize) -> Option<Command> {
    const MARGIN: f32 = 15.0;
    const BUTTON: f32 = 46.0;
    let painter = ui.painter_at(canvas);
    let galley = painter.layout_no_wrap(format!("{page} / {count}"), FontId::proportional(22.0), Color32::WHITE);
    let gap = 22.0;
    let size = vec2(BUTTON * 2.0 + galley.size().x + gap * 2.0 + MARGIN * 2.0, BUTTON + MARGIN * 2.0);
    let plate = Rect::from_center_size(egui::pos2(canvas.center().x, canvas.bottom() - MARGIN - size.y / 2.0), size);
    painter.rect_filled(plate, size.y / 2.0, Color32::from_black_alpha(185));
    painter.galley(plate.center() - galley.size() / 2.0, galley, Color32::WHITE);

    let mut clicked = None;
    let sides = [(-1.0_f32, Command::PrevPage, page > 1, "Previous page"), (1.0, Command::NextPage, page < count, "Next page")];
    for (side, command, enabled, tip) in sides {
        let centre = egui::pos2(plate.center().x + side * (plate.width() / 2.0 - MARGIN - BUTTON / 2.0), plate.center().y);
        let rect = Rect::from_center_size(centre, vec2(BUTTON, BUTTON));
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let response = ui.interact(rect, ui.scope_id().with(("page_bar", command.id())), sense);
        let hovered = enabled && response.hovered();
        let ink = if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) };
        let fill = if hovered { Color32::from_white_alpha(70) } else { Color32::from_white_alpha(26) };
        painter.circle_filled(centre, BUTTON / 2.0, fill);
        painter.circle_stroke(centre, BUTTON / 2.0, egui::Stroke::new(1.5, Color32::from_white_alpha(if enabled { 150 } else { 50 })));
        // An arrow: a shaft and a head with round joints.
        let stroke = egui::Stroke::new(3.0, ink);
        let (tip_x, tail_x, head) = (centre.x + side * 11.0, centre.x - side * 11.0, 9.0);
        painter.line_segment([egui::pos2(tail_x, centre.y), egui::pos2(tip_x, centre.y)], stroke);
        let head_points = vec![
            egui::pos2(tip_x - side * head, centre.y - head),
            egui::pos2(tip_x, centre.y),
            egui::pos2(tip_x - side * head, centre.y + head),
        ];
        painter.add(egui::Shape::line(head_points, stroke));
        for point in [egui::pos2(tail_x, centre.y), egui::pos2(tip_x, centre.y)] {
            painter.circle_filled(point, 1.5, ink);
        }
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.on_hover_text(tip).clicked() {
            clicked = Some(command);
        }
    }
    clicked
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.2} {}", UNITS[unit])
    } else if value < 100.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_size(512), "512 bytes");
        assert_eq!(human_size(1536), "1.50 KB");
        assert_eq!(human_size(2_500_000), "2.38 MB");
        assert_eq!(human_size(45 * 1024 * 1024), "45.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.00 GB");
    }
}
