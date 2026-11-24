//! Library tools: the search bar and the duplicate finder.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use sv_core::dupes;
use sv_core::folder::Query;

/// What is typed into the search bar.
#[derive(Default)]
pub struct Search {
    pub open: bool,
    text: String,
    min_mb: String,
    max_mb: String,
    from: String,
    until: String,
    focus: bool,
}

impl Search {
    /// The query for what is typed. Fields that do not parse are ignored (and shown red).
    fn query(&self) -> Query {
        let megabytes = |text: &str| text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| *v >= 0.0).map(|v| (v * 1024.0 * 1024.0) as u64);
        Query {
            text: self.text.clone(),
            min_bytes: megabytes(&self.min_mb),
            max_bytes: megabytes(&self.max_mb),
            from: folder::parse_day(&self.from, false),
            until: folder::parse_day(&self.until, true),
        }
    }
}

struct Running {
    progress: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    total: usize,
    result: Receiver<Option<Vec<Vec<PathBuf>>>>,
}

pub struct Duplicates {
    pub open: bool,
    similar: bool,
    tolerance: u32,
    running: Option<Running>,
    /// Each group: files with their sizes, the largest first.
    groups: Vec<Vec<(PathBuf, u64)>>,
    message: Option<String>,
}

impl Default for Duplicates {
    fn default() -> Self {
        Duplicates { open: false, similar: false, tolerance: 6, running: None, groups: Vec::new(), message: None }
    }
}

impl Duplicates {
    /// Files that no longer exist leave the results; a group needs at least two.
    pub fn forget(&mut self, gone: &[PathBuf]) {
        for group in &mut self.groups {
            group.retain(|(path, _)| !gone.contains(path));
        }
        self.groups.retain(|g| g.len() > 1);
    }
}

impl App {
    pub(super) fn toggle_search(&mut self, ctx: &egui::Context) {
        if self.search.open {
            self.close_search();
        } else {
            self.set_mode(ctx, Mode::Browser);
            self.search.open = true;
            self.search.focus = true;
        }
    }

    pub(super) fn close_search(&mut self) {
        self.search = Search::default();
        self.set_query(Query::default());
    }

    fn set_query(&mut self, query: Query) {
        if query != self.query {
            self.query = query;
            let keep = self.current_path().cloned();
            // The selected picture is dropped too when it does not match.
            let keep = keep.filter(|p| self.all_entries.iter().any(|e| &e.path == p && self.query.matches(e)));
            self.apply_filter(keep, 0);
            self.grid.reveal = true;
        }
    }

    /// The search row under the browser's top bar.
    pub(super) fn search_bar(&mut self, ui: &mut egui::Ui) {
        if !self.search.open {
            return;
        }
        let bad = ui.visuals().error_fg_color;
        let mut close = false;
        ui.horizontal_wrapped(|ui| {
            let s = &mut self.search;
            ui.label("Search");
            let text = ui.add(egui::TextEdit::singleline(&mut s.text).desired_width(220.0).hint_text("words in the name or folder"));
            if std::mem::take(&mut s.focus) {
                text.request_focus();
            }
            let small = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, valid: bool| {
                ui.label(label);
                let mut edit = egui::TextEdit::singleline(value).desired_width(78.0).hint_text(hint);
                if !valid {
                    edit = edit.text_color(bad);
                }
                ui.add(edit);
            };
            let number_ok = |t: &str| t.trim().is_empty() || t.trim().replace(',', ".").parse::<f64>().is_ok_and(|v| v >= 0.0);
            let day_ok = |t: &str| t.trim().is_empty() || folder::parse_day(t, false).is_some();
            let (min_ok, max_ok, from_ok, until_ok) = (number_ok(&s.min_mb), number_ok(&s.max_mb), day_ok(&s.from), day_ok(&s.until));
            small(ui, "larger than", &mut s.min_mb, "MB", min_ok);
            small(ui, "smaller than", &mut s.max_mb, "MB", max_ok);
            small(ui, "from", &mut s.from, "2024-05-31", from_ok);
            small(ui, "to", &mut s.until, "2024 or 2024-06", until_ok);
            close = ui.button("Close").on_hover_text("Esc").clicked();
        });
        if close {
            self.close_search();
            return;
        }
        let query = self.search.query();
        self.set_query(query);
        if !self.query.is_empty() {
            let scope = if self.recursive { "this folder and its subfolders" } else { "this folder (tick Subfolders to search deeper)" };
            ui.weak(format!("{} of {} pictures in {scope}", self.entries.len(), self.all_entries.len()));
        }
    }

    pub(super) fn poll_duplicates(&mut self, now: f64) {
        let Some(running) = &self.duplicates.running else { return };
        let Ok(result) = running.result.try_recv() else { return };
        let cancelled = running.cancel.load(Ordering::Relaxed);
        self.duplicates.running = None;
        let sizes: std::collections::HashMap<&PathBuf, u64> = self.all_entries.iter().map(|e| (&e.path, e.size)).collect();
        match result {
            None => self.duplicates.message = Some(format!("Too many pictures to compare by look (more than {}). Search a smaller folder, or use exact matching.", dupes::MAX_SIMILAR)),
            Some(groups) => {
                self.duplicates.groups = groups.into_iter().map(|g| g.into_iter().map(|p| (sizes.get(&p).copied().unwrap_or(0), p)).map(|(s, p)| (p, s)).collect()).collect();
                let files: usize = self.duplicates.groups.iter().map(Vec::len).sum();
                let text = match (self.duplicates.groups.len(), cancelled) {
                    (_, true) => "Stopped. Nothing is reported from an unfinished search.".to_string(),
                    (0, false) => "No duplicates found.".to_string(),
                    (n, false) => format!("{n} groups, {files} files."),
                };
                if cancelled {
                    self.duplicates.groups.clear();
                }
                self.say(text.clone(), now);
                self.duplicates.message = Some(text);
            }
        }
    }

    pub(super) fn duplicates_window(&mut self, ctx: &egui::Context, commands: &mut Vec<Command>) {
        if !self.duplicates.open {
            return;
        }
        let total = self.entries.len();
        let mut open = true;
        let mut start = false;
        let mut jump: Option<PathBuf> = None;
        let mut mark_extras = false;
        egui::Window::new("Duplicates").open(&mut open).collapsible(false).default_width(520.0).default_height(460.0).show(ctx, |ui| {
            let d = &mut self.duplicates;
            let scope = if self.recursive { "with subfolders" } else { "tick Subfolders in the top bar to include subfolders" };
            ui.label(format!("Searches the {total} pictures shown ({scope})."));
            ui.add_enabled_ui(d.running.is_none(), |ui| {
                ui.radio_value(&mut d.similar, false, "Exactly the same file contents");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.similar, true, "Pictures that look the same (resized, re-saved, converted)");
                });
                ui.add_enabled(d.similar, egui::Slider::new(&mut d.tolerance, 0..=14).text("looseness")).on_hover_text("0 finds near-identical pictures only. Higher finds more, and more false matches.");
            });
            if let Some(running) = &d.running {
                let done = running.progress.load(Ordering::Relaxed);
                ui.add(egui::ProgressBar::new(done as f32 / running.total.max(1) as f32).text(format!("{done} / {}", running.total)));
                if ui.button("Stop").clicked() {
                    running.cancel.store(true, Ordering::Relaxed);
                }
                ui.ctx().request_repaint_after_secs(0.1);
            } else {
                start = ui.add_enabled(total > 1, egui::Button::new("Find duplicates")).clicked();
            }
            if let Some(message) = &d.message {
                ui.label(message);
            }
            if d.groups.is_empty() {
                return;
            }
            ui.separator();
            ui.horizontal(|ui| {
                mark_extras = ui.button("Mark all but the largest of each group").clicked();
                if ui.button("Unmark all").clicked() {
                    commands.push(Command::ClearMarks);
                }
                let marked = self.marked.len();
                if ui.add_enabled(marked > 0, egui::Button::new(format!("Delete {marked} marked"))).on_hover_text("To the Recycle Bin").clicked() {
                    commands.push(Command::Delete);
                }
            });
            ui.weak("Click a name to look at the picture. Marked files can also be moved with F7.");
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for (n, group) in d.groups.iter().enumerate() {
                    ui.add_space(4.0);
                    ui.strong(format!("Group {}", n + 1));
                    for (path, size) in group {
                        ui.horizontal(|ui| {
                            let mut marked = self.marked.contains(path);
                            if ui.checkbox(&mut marked, "").changed() {
                                if marked {
                                    self.marked.insert(path.clone());
                                } else {
                                    self.marked.remove(path);
                                }
                            }
                            let current = self.entries.get(self.index).is_some_and(|e| &e.path == path);
                            if ui.selectable_label(current, file_name(path)).clicked() {
                                jump = Some(path.clone());
                            }
                            ui.weak(human_size(*size));
                            if let Some(parent) = path.parent() {
                                ui.add(egui::Label::new(egui::RichText::new(parent.display().to_string()).weak()).truncate());
                            }
                        });
                    }
                }
            });
        });

        if mark_extras {
            for group in &self.duplicates.groups {
                for (path, _) in group.iter().skip(1) {
                    self.marked.insert(path.clone());
                }
            }
        }
        if let Some(path) = jump {
            if let Some(index) = self.entries.iter().position(|e| e.path == path) {
                self.go_to(index);
            }
        }
        if start {
            let entries = self.entries.clone();
            let (similar, tolerance) = (self.duplicates.similar, self.duplicates.tolerance);
            let (progress, cancel) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
            let (tx, rx) = mpsc::channel();
            {
                let (progress, cancel, ctx) = (progress.clone(), cancel.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let result = if similar { dupes::find_similar(&entries, tolerance, &progress, &cancel) } else { Some(dupes::find_exact(&entries, &progress, &cancel)) };
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
            }
            self.duplicates.groups.clear();
            self.duplicates.message = None;
            self.duplicates.running = Some(Running { progress, cancel, total, result: rx });
        }
        self.duplicates.open = open || self.duplicates.running.is_some();
    }
}
