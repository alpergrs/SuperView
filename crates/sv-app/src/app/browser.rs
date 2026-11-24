//! Browser mode: folder tree, thumbnail grid and preview pane.

use super::icons::{self, Icon};
use super::*;
use crate::thumbnails::Thumb;
use std::collections::HashMap;

/// What the right-click menu of a folder can ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FolderAction {
    Open,
    OpenInExplorer,
    Slideshow { subfolders: bool },
    Copy,
    Cut,
    Paste,
}

/// The right-click menu of a folder, in the grid and in the tree. `own` is false for the
/// ".." tile, which stands for the parent folder and cannot itself be copied.
fn folder_menu(ui: &mut egui::Ui, own: bool) -> Option<FolderAction> {
    enlarge_menu(ui);
    let mut chosen = None;
    let mut item = |ui: &mut egui::Ui, label: &str, keys: &str, action: FolderAction| {
        if ui.add(egui::Button::new(label).shortcut_text(keys)).clicked() {
            chosen = Some(action);
            ui.close();
        }
    };
    item(ui, "Open", "", FolderAction::Open);
    item(ui, "Open in Explorer", "", FolderAction::OpenInExplorer);
    item(ui, "Slideshow", "", FolderAction::Slideshow { subfolders: false });
    item(ui, "Slideshow with subfolders", "", FolderAction::Slideshow { subfolders: true });
    ui.separator();
    if own {
        item(ui, "Copy", "Ctrl+C", FolderAction::Copy);
        item(ui, "Cut", "Ctrl+X", FolderAction::Cut);
    }
    item(ui, "Paste here", "", FolderAction::Paste);
    chosen
}

/// Drive letters in use, one bit per letter from A, without touching any drive.
#[cfg(windows)]
fn drive_letters() -> u32 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalDrives() -> u32;
    }
    // SAFETY: takes nothing and only returns a bit mask.
    unsafe { GetLogicalDrives() }
}

#[cfg(not(windows))]
fn drive_letters() -> u32 {
    0
}

/// Folder tree. Folders are read from disk the first time they are opened.
pub struct Tree {
    roots: Vec<(String, PathBuf)>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
    expanded: HashSet<PathBuf>,
    scroll_to: Option<PathBuf>,
    show_hidden: bool,
    /// Whether a folder has any subfolder, looked up once when it first comes into view.
    has_subfolders: HashMap<PathBuf, bool>,
    /// How many of those look-ups are still allowed in this frame.
    lookups_left: usize,
    /// Left edge of the tree's contents on screen.
    content_left: f32,
    /// Something chosen from a folder's right-click menu.
    pub action: Option<(PathBuf, FolderAction)>,
}

/// Folder names in the tree are this much larger than ordinary text.
const TREE_TEXT_SCALE: f32 = 1.3;

fn has_subfolder(dir: &Path, show_hidden: bool) -> bool {
    let Ok(read) = std::fs::read_dir(dir) else { return false };
    read.filter_map(Result::ok).any(|e| e.file_type().is_ok_and(|t| t.is_dir()) && (show_hidden || !is_hidden(&e)))
}

fn is_hidden(entry: &std::fs::DirEntry) -> bool {
    // "$Recycle.Bin" and friends are never worth showing.
    entry.file_name().to_string_lossy().starts_with('$')
        || entry.metadata().is_ok_and(|meta| folder::is_hidden(&entry.path(), &meta))
}

pub(super) fn subfolders(dir: &Path, show_hidden: bool) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && (show_hidden || !is_hidden(e)))
        .map(|e| e.path())
        .collect();
    out.sort_by(|a, b| folder::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out
}

impl Tree {
    pub fn new() -> Tree {
        let mut roots = Vec::new();
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
        if let Some(home) = &home {
            for name in ["Pictures", "Desktop", "Downloads", "Documents"] {
                let path = home.join(name);
                if path.is_dir() {
                    roots.push((name.to_string(), path));
                }
            }
        }
        if cfg!(windows) {
            // Asking Windows which drive letters are in use returns at once. Touching each
            // drive instead (to see whether it exists) can stall for seconds on a mapped
            // network drive whose server is slow or away, freezing the window at start-up.
            let in_use = drive_letters();
            for (i, letter) in ('A'..='Z').enumerate() {
                if letter >= 'C' && in_use & (1 << i) != 0 {
                    roots.push((format!("{letter}:"), PathBuf::from(format!("{letter}:\\"))));
                }
            }
        } else {
            if let Some(home) = home {
                roots.push(("Home".to_string(), home));
            }
            roots.push(("/".to_string(), PathBuf::from("/")));
        }
        Tree {
            roots,
            children: HashMap::new(),
            expanded: HashSet::new(),
            scroll_to: None,
            show_hidden: false,
            has_subfolders: HashMap::new(),
            lookups_left: 0,
            content_left: 0.0,
            action: None,
        }
    }

    /// Forget what was read from disk: folders were added, moved or removed.
    pub fn refresh(&mut self) {
        self.children.clear();
        self.has_subfolders.clear();
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        if show != self.show_hidden {
            self.show_hidden = show;
            // Folder contents are remembered; read them again with the new rule.
            self.children.clear();
            self.has_subfolders.clear();
        }
    }

    /// Open the tree down to `dir` and bring it into view.
    pub fn reveal(&mut self, dir: &Path) {
        for ancestor in dir.ancestors().skip(1) {
            self.expanded.insert(ancestor.to_path_buf());
        }
        self.scroll_to = Some(dir.to_path_buf());
    }

    fn show(&mut self, ui: &mut egui::Ui, current: Option<&Path>) -> Option<PathBuf> {
        let mut clicked = None;
        // Spread the disk look-ups of a long folder list over a few frames.
        self.lookups_left = 40;
        self.content_left = ui.max_rect().left();
        let roots = self.roots.clone();
        for (i, (label, path)) in roots.iter().enumerate() {
            self.node(ui, path, label, 0, i, current, &mut clicked);
        }
        self.scroll_to = None;
        clicked
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        label: &str,
        depth: usize,
        root: usize,
        current: Option<&Path>,
        clicked: &mut Option<PathBuf>,
    ) {
        let expanded = self.expanded.contains(path);
        // No "+" next to a folder that has nothing to open.
        let known_empty = match self.children.get(path) {
            Some(children) => children.is_empty(),
            // Top-level entries (drives, Pictures, ...) are not looked into until opened:
            // a network drive could hold up the window. They get a "+" regardless.
            None if depth == 0 => false,
            None => match self.has_subfolders.get(path) {
                Some(has) => !has,
                None if self.lookups_left > 0 => {
                    self.lookups_left -= 1;
                    let has = has_subfolder(path, self.show_hidden);
                    self.has_subfolders.insert(path.to_path_buf(), has);
                    !has
                }
                None => {
                    // Not looked up yet: come back next frame.
                    ui.ctx().request_repaint();
                    false
                }
            },
        };
        let text_size = egui::TextStyle::Body.resolve(ui.style()).size * TREE_TEXT_SCALE;
        ui.push_id((root, path), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 14.0 * TREE_TEXT_SCALE);
                let arrow = if known_empty { " " } else if expanded { "−" } else { "+" };
                let arrow = egui::RichText::new(arrow).monospace().size(text_size);
                if ui.add(egui::Button::new(arrow).frame(false).small()).clicked() && !known_empty {
                    if expanded {
                        self.expanded.remove(path);
                    } else {
                        self.expanded.insert(path.to_path_buf());
                    }
                }
                let response = ui.selectable_label(current == Some(path), egui::RichText::new(label).size(text_size));
                if response.clicked() {
                    *clicked = Some(path.to_path_buf());
                }
                response.context_menu(|ui| {
                    if let Some(action) = folder_menu(ui, true) {
                        self.action = Some((path.to_path_buf(), action));
                    }
                });
                // A double-click on the name opens or closes the folder's branch, the
                // same as its + / − sign.
                if response.double_clicked() && !known_empty {
                    if expanded {
                        self.expanded.remove(path);
                    } else {
                        self.expanded.insert(path.to_path_buf());
                    }
                }
                if self.scroll_to.as_deref() == Some(path) {
                    // Bring the row into view vertically without sliding the tree
                    // sideways, which would cut off the left edge.
                    let row = response.rect;
                    let left = self.content_left;
                    let target = Rect::from_min_max(egui::pos2(left, row.top()), egui::pos2(left + 1.0, row.bottom()));
                    ui.scroll_to_rect(target, Some(egui::Align::Center));
                }
            });
        });
        if self.expanded.contains(path) {
            let children = self.children.entry(path.to_path_buf()).or_insert_with(|| subfolders(path, self.show_hidden)).clone();
            for child in &children {
                let name = file_name(child);
                self.node(ui, child, &name, depth + 1, root, current, clicked);
            }
        }
    }
}

/// A plain folder symbol, drawn so it looks the same in every font and theme.
fn paint_folder(painter: &egui::Painter, square: Rect) {
    let size = square.width() * 0.62;
    let body = Rect::from_center_size(square.center() + vec2(0.0, size * 0.05), vec2(size, size * 0.72));
    let tab = Rect::from_min_size(body.left_top() - vec2(0.0, size * 0.12), vec2(size * 0.42, size * 0.2));
    let back = Color32::from_rgb(214, 160, 52);
    let front = Color32::from_rgb(240, 196, 88);
    painter.rect_filled(tab, size * 0.05, back);
    painter.rect_filled(body, size * 0.06, back);
    let face = Rect::from_min_max(body.left_top() + vec2(0.0, size * 0.1), body.right_bottom());
    painter.rect_filled(face, size * 0.06, front);
}

/// File and folder names under the thumbnails: a quarter larger than ordinary text.
const LABEL_SIZE: f32 = 15.0;
const LABEL_HEIGHT: f32 = 25.0;

/// How much faster than normal the mouse wheel scrolls the thumbnail grid.
const GRID_WHEEL_SPEED: f32 = 3.0;

/// What the grid remembers between frames, and requests it hands back to the app.
#[derive(Default)]
pub struct GridState {
    pub columns: usize,
    pub visible_rows: usize,
    /// Scroll so the selected picture is visible.
    pub reveal: bool,
    /// Top and bottom of the visible part of the grid, in grid coordinates.
    view: (f32, f32),
    /// Which subfolder tile is selected, if any.
    pub folder_selected: Option<usize>,
    /// Something chosen from a folder's right-click menu.
    pub folder_action: Option<(PathBuf, FolderAction)>,
    pub open_dir: Option<PathBuf>,
    /// Go to the parent folder (the Up button).
    pub go_up: bool,
    /// Backspace: back to the folder we came from, or up when there is none.
    pub go_back: bool,
    pub set_recursive: Option<bool>,
    /// Where a Shift+click range starts: the picture last clicked without Shift.
    pub anchor: Option<PathBuf>,
}

impl App {
    /// The folder tiles of the grid, in order: ".." for the parent folder (unless this
    /// is the top of a drive), then the subfolders. With "Subfolders" on, their pictures
    /// are already in the list, so only ".." remains.
    pub(super) fn grid_folders(&self) -> Vec<(PathBuf, String)> {
        let mut folders = Vec::new();
        if let Some(parent) = self.dir.as_ref().and_then(|d| d.parent()) {
            folders.push((parent.to_path_buf(), "..".to_string()));
        }
        if !self.recursive {
            folders.extend(self.subdirs.iter().map(|d| (d.clone(), file_name(d))));
        }
        folders
    }

    pub(super) fn browser_panels(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>, now: f64) {
        egui::Panel::top("browser_bar").show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| self.browser_bar(ui, commands));
            self.search_bar(ui);
            ui.add_space(2.0);
        });
        if self.settings.show_tree {
            egui::Panel::left("tree").resizable(true).default_size(230.0).min_size(120.0).show(ui, |ui| {
                // Twice the usual wheel distance: the list of folders is long.
                egui::ScrollArea::both().auto_shrink(false).wheel_scroll_multiplier(vec2(1.0, 2.0)).show(ui, |ui| {
                    if let Some(dir) = self.tree.show(ui, self.dir.as_deref()) {
                        self.grid.open_dir = Some(dir);
                    }
                    if let Some(action) = self.tree.action.take() {
                        self.grid.folder_action = Some(action);
                    }
                });
            });
        }
        if self.settings.show_preview {
            egui::Panel::right("preview").resizable(true).default_size(320.0).min_size(160.0).show(ui, |ui| {
                self.preview(ui, now);
            });
        }
    }

    /// A button of the top bar: an icon or a word, as chosen in Settings. `selected` makes
    /// it an on/off switch shown pressed while on.
    fn bar_button(&self, ui: &mut egui::Ui, icon: Icon, text: &str, tip: String, selected: Option<bool>) -> bool {
        let response = if self.settings.browser_icons {
            icons::button(ui, icon, selected.unwrap_or(false))
        } else {
            match selected {
                Some(on) => ui.selectable_label(on, text),
                None => ui.button(text),
            }
        };
        response.on_hover_text(tip).clicked()
    }

    fn browser_bar(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        if self.bar_button(ui, Icon::Up, "Up", "Parent folder".to_string(), None) {
            self.grid.go_up = true;
        }
        let dir = self.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        ui.strong(dir);
        ui.separator();

        let mut recursive = self.recursive;
        if ui.checkbox(&mut recursive, "Subfolders").changed() {
            self.grid.set_recursive = Some(recursive);
        }
        let mut show_hidden = self.show_hidden;
        if ui.checkbox(&mut show_hidden, "Hidden files").on_hover_text("Show hidden and system files and folders").changed() {
            self.set_show_hidden(show_hidden);
        }
        let (mut key, mut descending) = (self.settings.sort_key, self.settings.sort_descending);
        egui::ComboBox::from_id_salt("sort").selected_text(format!("Sort: {}", key.label())).show_ui(ui, |ui| {
            for k in SortKey::ALL {
                ui.selectable_value(&mut key, k, k.label());
            }
        });
        ui.checkbox(&mut descending, "Descending");
        if key != self.settings.sort_key || descending != self.settings.sort_descending {
            self.settings.sort_key = key;
            self.settings.sort_descending = descending;
            self.resort();
        }

        let hidden = self.hidden_formats.len();
        let types = if hidden == 0 { "File types: all".to_string() } else { format!("File types: {} hidden", hidden) };
        ui.menu_button(types, |ui| {
            let mut changed = false;
            for format in Format::ALL {
                let mut shown = !self.hidden_formats.contains(&format);
                if ui.checkbox(&mut shown, format.name()).changed() {
                    changed = true;
                    if shown {
                        self.hidden_formats.remove(&format);
                    } else {
                        self.hidden_formats.insert(format);
                    }
                }
            }
            if changed {
                let (keep, fallback) = (self.current_path().cloned(), self.index);
                // The selected picture goes too if its type was just hidden.
                let keep = keep.filter(|p| Format::from_extension(p).is_none_or(|f| !self.hidden_formats.contains(&f)));
                self.apply_filter(keep, fallback);
                self.grid.reveal = true;
            }
        });

        ui.label("Size");
        let slider = ui.add(egui::Slider::new(&mut self.settings.thumb_size, 64.0..=256.0).show_value(false));
        if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
            self.save_settings();
        }
        ui.separator();

        let marked = self.marked.len();
        if marked > 0 {
            ui.strong(format!("{marked} marked"));
        }
        let target = if marked > 0 { "marked files" } else { "selected file" };
        let tip = |command: Command, about: &str| {
            let keys = input::shortcut(&self.bindings, command);
            let label = command.label().trim_end_matches('…');
            let keys = if keys.is_empty() { String::new() } else { format!("  ({keys})") };
            if about.is_empty() { format!("{label}{keys}") } else { format!("{label}: {about}{keys}") }
        };
        let mut press = |ui: &mut egui::Ui, icon: Icon, command: Command, about: &str| {
            if self.bar_button(ui, icon, command.short(), tip(command, about), None) {
                commands.push(command);
            }
        };
        press(ui, Icon::CopyTo, Command::CopyTo, target);
        press(ui, Icon::MoveTo, Command::MoveTo, target);
        press(ui, Icon::Delete, Command::Delete, target);
        press(ui, Icon::MarkAll, Command::MarkAll, "");
        if marked > 0 {
            press(ui, Icon::Unmark, Command::ClearMarks, "");
        }
        press(ui, Icon::Batch, Command::Batch, "");
        press(ui, Icon::Rename, Command::BatchRename, "");
        press(ui, Icon::Info, Command::Info, "");
        press(ui, Icon::Search, Command::Search, "");
        press(ui, Icon::Duplicates, Command::Duplicates, "");
        ui.separator();
        let (tree, preview) = (self.settings.show_tree, self.settings.show_preview);
        let tree_tip = format!("Folder tree: {}", if tree { "shown" } else { "hidden" });
        let preview_tip = format!("Preview: {}", if preview { "shown" } else { "hidden" });
        let mut changed = false;
        if self.bar_button(ui, Icon::Folders, "Folders", tree_tip, Some(tree)) {
            self.settings.show_tree = !tree;
            changed = true;
        }
        if self.bar_button(ui, Icon::Preview, "Preview", preview_tip, Some(preview)) {
            self.settings.show_preview = !preview;
            changed = true;
        }
        if changed {
            self.save_settings();
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui, now: f64) {
        let Some(entry) = self.entries.get(self.index).cloned() else {
            ui.weak("Nothing selected");
            return;
        };
        let info_height = 112.0;
        let area = vec2(ui.available_width(), (ui.available_height() - info_height).max(80.0));
        let (rect, _) = ui.allocate_exact_size(area, Sense::hover());
        let painter = ui.painter_at(rect);
        let shown = self.shown.as_ref().filter(|s| s.key.as_ref().is_some_and(|k| k.0 == entry.path));
        let mut dimensions = None;
        if let Some(shown) = shown {
            let loaded = &shown.ready.loaded;
            dimensions = Some((loaded.width(), loaded.height(), loaded.format));
            let size = self.view.shown_size(loaded.width(), loaded.height());
            let scale = (rect.width() / size.x).min(rect.height() / size.y).min(1.0);
            let frame = self.frame.min(shown.ready.textures.len() - 1);
            let target = Rect::from_center_size(rect.center(), size * scale);
            paint_image(&painter, target, shown.ready.textures[frame].id(), self.view.uvs());
            if loaded.is_animated() {
                if now >= self.frame_due {
                    self.frame = (frame + 1) % loaded.frames.len();
                    self.frame_due = now + loaded.frames[self.frame].delay_ms as f64 / 1000.0;
                }
                ui.ctx().request_repaint_after_secs((self.frame_due - now).max(0.0) as f32);
            }
        } else if let Some(error) = &self.error {
            painter.text(rect.center(), Align2::CENTER_CENTER, error, FontId::proportional(13.0), ui.visuals().weak_text_color());
        } else if let Thumb::Ready(texture) = self.thumbs.get(&entry) {
            // The quick stand-in while the full picture decodes.
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let target = Rect::from_center_size(rect.center(), size * scale);
            paint_image(&painter, target, texture.id(), View::default().uvs());
        }

        ui.add_space(6.0);
        ui.add(egui::Label::new(egui::RichText::new(file_name(&entry.path)).strong()).truncate());
        if let Some((w, h, format)) = dimensions {
            ui.label(format!("{w} × {h}   {}", format.name()));
        } else {
            ui.label(" ");
        }
        ui.label(human_size(entry.size));
        ui.label(format!("Modified {}", date_text(entry.modified)));
        if self.recursive {
            if let Some(parent) = entry.path.parent() {
                ui.add(egui::Label::new(egui::RichText::new(parent.display().to_string()).weak()).truncate());
            }
        }
    }

    pub(super) fn grid(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>, now: f64) {
        // Like Explorer: the folder's subfolders first, then its pictures. With
        // "Subfolders" on, their pictures are already in the list instead.
        let folders = self.grid_folders();
        let folder_count = folders.len();
        if self.entries.is_empty() && folders.is_empty() {
            let text = if self.listing.is_some() {
                "Reading folder…"
            } else if self.dir.is_none() {
                "Pick a folder on the left, or drop one here"
            } else {
                "No pictures in this folder"
            };
            ui.centered_and_justified(|ui| ui.weak(text));
            return;
        }

        // Ctrl + wheel over the grid changes the thumbnail size.
        let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        if ctrl_held && ui.rect_contains_pointer(ui.max_rect()) {
            let steps = Self::wheel_steps(ui.ctx());
            if steps != 0.0 {
                let size = (self.settings.thumb_size + steps * 16.0).clamp(64.0, 256.0);
                if size != self.settings.thumb_size {
                    self.settings.thumb_size = size;
                    self.save_settings();
                    // Keep the selected picture in view while the rows reflow.
                    self.grid.reveal = true;
                }
            }
        }

        let thumb = self.settings.thumb_size;
        let cell = vec2(thumb + 14.0, thumb + 14.0 + LABEL_HEIGHT);
        let width = ui.available_width();
        let columns = ((width / cell.x).floor() as usize).max(1);
        let rows = (folder_count + self.entries.len()).div_ceil(columns);
        let view_height = ui.available_height();
        self.grid.columns = columns;
        self.grid.visible_rows = ((view_height / cell.y).floor() as usize).max(1);

        // Three times the usual wheel distance: thumbnails are tall, and folders long.
        let mut area = egui::ScrollArea::vertical().auto_shrink(false).wheel_scroll_multiplier(vec2(1.0, GRID_WHEEL_SPEED));
        if std::mem::take(&mut self.grid.reveal) {
            let slot = self.grid.folder_selected.unwrap_or(self.index + folder_count);
            let top = (slot / columns) as f32 * cell.y;
            let (view_top, view_bottom) = self.grid.view;
            if top < view_top {
                area = area.vertical_scroll_offset(top);
            } else if top + cell.y > view_bottom {
                area = area.vertical_scroll_offset(top + cell.y - view_height);
            }
        }

        let selection = ui.visuals().selection.bg_fill;
        let hover = ui.visuals().widgets.hovered.weak_bg_fill;
        let text_color = ui.visuals().text_color();
        let faint = ui.visuals().faint_bg_color;
        let accent = ui.visuals().hyperlink_color;
        let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        let shift = ui.input(|i| i.modifiers.shift);

        area.show_viewport(ui, |ui, viewport| {
            ui.set_width(width);
            ui.set_height(rows as f32 * cell.y);
            self.grid.view = (viewport.min.y, viewport.max.y);
            let origin = ui.min_rect().min;
            let first = (viewport.min.y / cell.y).floor().max(0.0) as usize;
            let last = ((viewport.max.y / cell.y).ceil() as usize).min(rows);
            // Centre the columns in the available width.
            let margin = ((width - columns as f32 * cell.x) / 2.0).max(0.0);

            for row in first..last {
                for column in 0..columns {
                    let slot = row * columns + column;
                    let min = origin + vec2(margin + column as f32 * cell.x, row as f32 * cell.y);
                    let rect = Rect::from_min_size(min, cell);
                    let painter = ui.painter_at(rect);

                    if let Some((folder, name)) = folders.get(slot) {
                        let response = ui.interact(rect, ui.scope_id().with(("folder", slot)), Sense::click());
                        if self.grid.folder_selected == Some(slot) {
                            painter.rect_filled(rect.shrink(2.0), 5.0, selection);
                        } else if response.hovered() {
                            painter.rect_filled(rect.shrink(2.0), 5.0, hover);
                        }
                        let square = Rect::from_min_size(min + vec2(7.0, 7.0), vec2(thumb, thumb));
                        paint_folder(&painter, square);
                        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), FontId::proportional(LABEL_SIZE), text_color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(thumb);
                        let galley = painter.layout_job(job);
                        let label = egui::pos2(square.center().x - galley.size().x / 2.0, square.max.y + 4.0);
                        painter.galley(label, galley, text_color);
                        if response.double_clicked() {
                            self.grid.open_dir = Some(folder.clone());
                        } else if response.clicked() || response.secondary_clicked() {
                            self.grid.folder_selected = Some(slot);
                        }
                        response.context_menu(|ui| {
                            if let Some(action) = folder_menu(ui, name != "..") {
                                self.grid.folder_action = Some((folder.clone(), action));
                            }
                        });
                        continue;
                    }

                    let index = slot - folder_count;
                    let Some(entry) = self.entries.get(index).cloned() else { break };
                    let response = ui.interact(rect, ui.scope_id().with(("cell", index)), Sense::click());

                    if index == self.index && self.grid.folder_selected.is_none() {
                        painter.rect_filled(rect.shrink(2.0), 5.0, selection);
                    } else if response.hovered() {
                        painter.rect_filled(rect.shrink(2.0), 5.0, hover);
                    }

                    let square = Rect::from_min_size(min + vec2(7.0, 7.0), vec2(thumb, thumb));
                    match self.thumbs.get(&entry) {
                        Thumb::Ready(texture) => {
                            let size = texture.size_vec2();
                            let scale = (thumb / size.x).min(thumb / size.y);
                            let target = Rect::from_center_size(square.center(), size * scale);
                            paint_image(&painter, target, texture.id(), View::default().uvs());
                        }
                        Thumb::Pending => {
                            painter.rect_filled(square.shrink(thumb * 0.2), 4.0, faint);
                        }
                        Thumb::Failed => {
                            painter.text(square.center(), Align2::CENTER_CENTER, "cannot read", FontId::proportional(12.0), text_color);
                        }
                    }

                    if self.marked.contains(&entry.path) {
                        let centre = square.left_top() + vec2(13.0, 13.0);
                        painter.circle_filled(centre, 11.0, accent);
                        let stroke = egui::Stroke::new(2.0, Color32::WHITE);
                        painter.line_segment([centre + vec2(-5.0, 0.0), centre + vec2(-1.5, 4.0)], stroke);
                        painter.line_segment([centre + vec2(-1.5, 4.0), centre + vec2(5.5, -4.0)], stroke);
                    }

                    let mut job = egui::text::LayoutJob::simple_singleline(file_name(&entry.path), FontId::proportional(LABEL_SIZE), text_color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(thumb);
                    let galley = painter.layout_job(job);
                    let label = egui::pos2(square.center().x - galley.size().x / 2.0, square.max.y + 4.0);
                    painter.galley(label, galley, text_color);

                    if response.clicked() || response.secondary_clicked() {
                        self.grid.folder_selected = None;
                    }
                    if response.double_clicked() {
                        self.go_to(index);
                        commands.push(Command::Browser);
                    } else if response.clicked() {
                        if shift {
                            // Everything from the anchor (or the picture we are on) to this
                            // one, both ends included, as in Explorer.
                            let anchor = self.grid.anchor.as_ref().and_then(|a| self.entries.iter().position(|e| &e.path == a));
                            let from = anchor.unwrap_or(self.index).min(self.entries.len().saturating_sub(1));
                            let (lo, hi) = (from.min(index), from.max(index));
                            self.marked = self.entries[lo..=hi].iter().map(|e| e.path.clone()).collect();
                            if self.grid.anchor.is_none() {
                                self.grid.anchor = self.entries.get(from).map(|e| e.path.clone());
                            }
                        } else if ctrl {
                            // The picture we were on and this one are both selected; a second
                            // Ctrl+click on a selected picture takes it out again.
                            if self.marked.is_empty() {
                                if let Some(current) = self.entries.get(self.index).filter(|e| e.path != entry.path) {
                                    self.marked.insert(current.path.clone());
                                }
                            }
                            if !self.marked.remove(&entry.path) {
                                self.marked.insert(entry.path.clone());
                            }
                            self.grid.anchor = Some(entry.path.clone());
                        } else {
                            self.grid.anchor = Some(entry.path.clone());
                        }
                        self.go_to(index);
                    }
                    if response.secondary_clicked() {
                        self.go_to(index);
                    }
                    response.context_menu(|ui| self.menu(ui, commands, now));
                }
            }
        });
        // Clicking never needs the view to jump: the picture was visible already.
        if ui.input(|i| i.pointer.any_click()) {
            self.grid.reveal = false;
        }
    }
}
