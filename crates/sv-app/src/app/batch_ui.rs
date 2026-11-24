//! Windows for working on many files at once: batch processing, batch rename, and the
//! information (metadata) editor.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use sv_core::batch::{self, Job, Output, Resize};
use sv_core::metadata::{self, Fields, Info};
use sv_core::save::SaveFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeMode {
    LongSide,
    Width,
    Height,
    Percent,
}

impl ResizeMode {
    const ALL: [ResizeMode; 4] = [ResizeMode::LongSide, ResizeMode::Width, ResizeMode::Height, ResizeMode::Percent];

    fn label(self) -> &'static str {
        match self {
            ResizeMode::LongSide => "Longest side",
            ResizeMode::Width => "Width",
            ResizeMode::Height => "Height",
            ResizeMode::Percent => "Percent",
        }
    }
}

struct Running {
    progress: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    total: usize,
    result: Receiver<batch::Report>,
}

pub struct State {
    pub batch_open: bool,
    pub rename_open: bool,
    pub info_open: bool,

    convert: bool,
    format: SaveFormat,
    quality: u8,
    resize: bool,
    resize_mode: ResizeMode,
    resize_value: f32,
    enlarge: bool,
    /// Quarter turns clockwise.
    turns: u8,
    auto_fix: bool,
    strip: bool,
    to_folder: bool,
    folder: Option<PathBuf>,
    running: Option<Running>,
    summary: Option<String>,
    failures: Vec<String>,

    pattern: String,
    start: u32,
    rename_error: Option<String>,

    /// The file whose information is loaded into the editor.
    info_for: Option<PathBuf>,
    info: Info,
    /// Description, artist, copyright, date taken.
    text: [String; 4],
    /// With several files: which fields to write.
    change: [bool; 4],
    info_message: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            batch_open: false,
            rename_open: false,
            info_open: false,
            convert: false,
            format: SaveFormat::Jpeg,
            quality: 90,
            resize: false,
            resize_mode: ResizeMode::LongSide,
            resize_value: 1920.0,
            enlarge: false,
            turns: 0,
            auto_fix: false,
            strip: false,
            to_folder: false,
            folder: None,
            running: None,
            summary: None,
            failures: Vec::new(),
            pattern: "{name}_{n:3}".to_string(),
            start: 1,
            rename_error: None,
            info_for: None,
            info: Info::default(),
            text: Default::default(),
            change: [false; 4],
            info_message: None,
        }
    }
}

impl State {
    fn job(&self) -> Job {
        let value = self.resize_value.max(1.0);
        Job {
            convert: self.convert.then_some(self.format),
            quality: self.quality,
            resize: self.resize.then_some(match self.resize_mode {
                ResizeMode::LongSide => Resize::LongSide(value as u32),
                ResizeMode::Width => Resize::Width(value as u32),
                ResizeMode::Height => Resize::Height(value as u32),
                ResizeMode::Percent => Resize::Percent(value),
            }),
            enlarge: self.enlarge,
            turn: sv_core::save::Turn { turns: self.turns, flip_h: false, flip_v: false },
            auto_fix: self.auto_fix,
            strip: self.strip,
            output: match (&self.folder, self.to_folder) {
                (Some(dir), true) => Output::Folder(dir.clone()),
                _ => Output::SameFolder,
            },
        }
    }
}

impl App {
    /// What a batch works on: the marked files, or every picture in the list when
    /// nothing is marked.
    fn batch_targets(&self) -> (Vec<PathBuf>, String) {
        if self.marked.is_empty() {
            let all: Vec<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
            let text = format!("All {} pictures shown. Mark files first to work on only some.", all.len());
            (all, text)
        } else {
            let marked: Vec<PathBuf> = self.all_entries.iter().filter(|e| self.marked.contains(&e.path)).map(|e| e.path.clone()).collect();
            let text = format!("{} marked files", marked.len());
            (marked, text)
        }
    }

    /// Something on disk changed under these paths: drop what is cached about them.
    fn files_changed(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            self.loader.forget(&path);
            self.thumbs.invalidate(&path);
        }
        self.relist = true;
    }

    pub(super) fn poll_batch(&mut self, now: f64) {
        let Some(running) = &self.batch.running else { return };
        let Ok(report) = running.result.try_recv() else { return };
        self.batch.running = None;
        self.batch.summary = Some(report.summary());
        self.batch.failures = report.failed.iter().map(|(path, why)| format!("{}: {why}", file_name(path))).collect();
        self.say(format!("Batch: {}", report.summary()), now);
        self.files_changed(report.written.into_iter().map(|(_, dest)| dest));
    }

    pub(super) fn batch_window(&mut self, ctx: &egui::Context, _now: f64) {
        if !self.batch.batch_open {
            return;
        }
        let (targets, description) = self.batch_targets();
        let mut open = true;
        let mut start = false;
        egui::Window::new("Batch").open(&mut open).collapsible(false).resizable(false).default_width(430.0).show(ctx, |ui| {
            let b = &mut self.batch;
            ui.label(description);
            ui.separator();
            ui.add_enabled_ui(b.running.is_none(), |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut b.convert, "Convert to");
                    ui.add_enabled_ui(b.convert, |ui| {
                        egui::ComboBox::from_id_salt("batch_format").selected_text(b.format.name()).show_ui(ui, |ui| {
                            for f in SaveFormat::ALL {
                                ui.selectable_value(&mut b.format, f, f.name());
                            }
                        });
                    });
                });
                ui.add(egui::Slider::new(&mut b.quality, 1..=100).text("Quality (JPEG and WebP results)"));

                ui.horizontal(|ui| {
                    ui.checkbox(&mut b.resize, "Resize");
                    ui.add_enabled_ui(b.resize, |ui| {
                        egui::ComboBox::from_id_salt("batch_resize").selected_text(b.resize_mode.label()).show_ui(ui, |ui| {
                            for m in ResizeMode::ALL {
                                ui.selectable_value(&mut b.resize_mode, m, m.label());
                            }
                        });
                        let (range, suffix) = if b.resize_mode == ResizeMode::Percent { (1.0..=1000.0, " %") } else { (1.0..=60_000.0, " px") };
                        ui.add(egui::DragValue::new(&mut b.resize_value).range(range).suffix(suffix).speed(1.0));
                    });
                });
                ui.add_enabled(b.resize, egui::Checkbox::new(&mut b.enlarge, "Also enlarge pictures that are smaller"));

                ui.horizontal(|ui| {
                    ui.label("Rotate");
                    ui.selectable_value(&mut b.turns, 0, "No");
                    ui.selectable_value(&mut b.turns, 3, "Left");
                    ui.selectable_value(&mut b.turns, 1, "Right");
                    ui.selectable_value(&mut b.turns, 2, "180°");
                });
                ui.checkbox(&mut b.auto_fix, "Auto-fix (stretch flat tones to full contrast)");
                ui.checkbox(&mut b.strip, "Remove information (camera, dates, GPS position, comments)");
                ui.separator();

                ui.radio_value(&mut b.to_folder, false, "Results in the same folder");
                ui.weak("Files that keep their format are replaced, keeping their dates. Converted files are added next to the originals.");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut b.to_folder, true, "Results in another folder");
                    if ui.button("Choose…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().set_title("Folder for the results").pick_folder() {
                            b.folder = Some(dir);
                            b.to_folder = true;
                        }
                    }
                });
                if let Some(dir) = &b.folder {
                    ui.add(egui::Label::new(egui::RichText::new(dir.display().to_string()).weak()).truncate());
                }
                ui.weak("An existing file is never overwritten there; such files are skipped.");
            });
            ui.separator();

            let job = b.job();
            let folder_missing = b.to_folder && b.folder.is_none();
            if let Some(running) = &b.running {
                let done = running.progress.load(Ordering::Relaxed);
                ui.add(egui::ProgressBar::new(done as f32 / running.total.max(1) as f32).text(format!("{done} / {}", running.total)));
                if ui.button("Stop").clicked() {
                    running.cancel.store(true, Ordering::Relaxed);
                }
                ui.ctx().request_repaint_after_secs(0.1);
            } else {
                let ready = job.does_something() && !targets.is_empty() && !folder_missing;
                let label = format!("Start on {} files", targets.len());
                start = ui.add_enabled(ready, egui::Button::new(label)).clicked();
                if !job.does_something() {
                    ui.weak("Tick at least one thing to do.");
                } else if folder_missing {
                    ui.weak("Choose the folder for the results.");
                }
            }
            if let Some(summary) = &b.summary {
                ui.label(summary);
            }
            if !b.failures.is_empty() {
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for line in &b.failures {
                        ui.weak(line);
                    }
                });
            }
        });

        if start {
            let job = self.batch.job();
            let (progress, cancel) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
            let (tx, rx) = mpsc::channel();
            let total = targets.len();
            {
                let (progress, cancel, ctx) = (progress.clone(), cancel.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let report = batch::run(&targets, &job, &progress, &cancel);
                    let _ = tx.send(report);
                    ctx.request_repaint();
                });
            }
            self.batch.summary = None;
            self.batch.failures.clear();
            self.batch.running = Some(Running { progress, cancel, total, result: rx });
        }
        // The window stays while work is in progress, so it can still be stopped.
        self.batch.batch_open = open || self.batch.running.is_some();
    }

    pub(super) fn rename_window(&mut self, ctx: &egui::Context, now: f64) {
        if !self.batch.rename_open {
            return;
        }
        let (targets, description) = self.batch_targets();
        let plan = batch::plan_rename(&targets, &self.batch.pattern, self.batch.start);
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Batch rename").open(&mut open).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
            ui.label(description);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("New name");
                ui.add(egui::TextEdit::singleline(&mut self.batch.pattern).desired_width(240.0));
                ui.label("first number");
                ui.add(egui::DragValue::new(&mut self.batch.start).range(0..=9_999_999));
            });
            ui.weak("{name} current name   ·   {n} number   ·   {n:3} number as 001   ·   {date} and {time} when the picture was taken");
            ui.weak("The file type ending always stays.");
            ui.separator();
            egui::Grid::new("rename_preview").num_columns(2).spacing([18.0, 3.0]).show(ui, |ui| {
                for (path, name) in plan.items.iter().take(8) {
                    ui.label(file_name(path));
                    ui.strong(name);
                    ui.end_row();
                }
            });
            if plan.items.len() > 8 {
                ui.weak(format!("… and {} more", plan.items.len() - 8));
            }
            for problem in &plan.problems {
                ui.colored_label(ui.visuals().error_fg_color, problem);
            }
            if let Some(error) = &self.batch.rename_error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(4.0);
            let ready = plan.problems.is_empty() && !plan.items.is_empty();
            apply = ui.add_enabled(ready, egui::Button::new(format!("Rename {} files", plan.items.len()))).clicked();
        });

        if apply {
            match batch::apply_rename(&plan) {
                Ok(done) => {
                    let current = self.current_path().cloned();
                    let mut follow = current.clone();
                    for (old, new) in &done {
                        self.loader.forget(old);
                        self.thumbs.forget(old);
                        if self.marked.remove(old) {
                            self.marked.insert(new.clone());
                        }
                        if let Some(e) = self.all_entries.iter_mut().find(|e| e.path == *old) {
                            e.path = new.clone();
                        }
                        if current.as_ref() == Some(old) {
                            follow = Some(new.clone());
                        }
                    }
                    folder::sort(&mut self.all_entries, self.settings.sort_key, self.settings.sort_descending);
                    self.apply_filter(follow, self.index);
                    self.grid.reveal = true;
                    self.batch.rename_error = None;
                    self.batch.rename_open = false;
                    self.say(format!("Renamed {} files", done.len()), now);
                    return;
                }
                Err(e) => {
                    self.batch.rename_error = Some(e);
                    // Some files may have been renamed before the failure.
                    self.relist = true;
                }
            }
        }
        self.batch.rename_open = open;
    }

    pub(super) fn info_window(&mut self, ctx: &egui::Context, now: f64) {
        if !self.batch.info_open {
            self.batch.info_for = None;
            return;
        }
        let targets = self.targets();
        let single = if targets.len() == 1 { targets.first().cloned() } else { None };
        // Load the stored values whenever another file comes into focus.
        if self.batch.info_for != single {
            self.batch.info_for = single.clone();
            self.batch.info_message = None;
            self.batch.change = [false; 4];
            self.batch.info = single.as_deref().map(metadata::read).unwrap_or_default();
            let f = &self.batch.info.fields;
            self.batch.text = [
                f.description.clone().unwrap_or_default(),
                f.artist.clone().unwrap_or_default(),
                f.copyright.clone().unwrap_or_default(),
                f.taken.clone().unwrap_or_default(),
            ];
        }
        let writable = |path: &PathBuf| sv_core::decode::Format::from_extension(path).is_some_and(metadata::can_write);
        let can_write = !targets.is_empty() && targets.iter().all(writable);

        let mut open = true;
        let mut save = false;
        egui::Window::new("Information").open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
            let b = &mut self.batch;
            match &single {
                Some(path) => {
                    ui.strong(file_name(path));
                }
                None if targets.is_empty() => {
                    ui.weak("No picture selected.");
                    return;
                }
                None => {
                    ui.strong(format!("{} marked files", targets.len()));
                    ui.weak("Tick the fields to write to all of them.");
                }
            }
            if single.is_some() && !b.info.details.is_empty() {
                egui::Grid::new("info_details").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                    for (label, value) in &b.info.details {
                        ui.weak(label);
                        ui.label(value);
                        ui.end_row();
                    }
                });
            }
            ui.separator();
            let many = single.is_none();
            egui::Grid::new("info_fields").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Description", "Artist", "Copyright", "Date taken"].into_iter().enumerate() {
                    if many {
                        ui.checkbox(&mut b.change[i], "");
                    } else {
                        ui.label("");
                    }
                    ui.label(label);
                    let edit = egui::TextEdit::singleline(&mut b.text[i]).desired_width(250.0);
                    let edit = if i == 3 { edit.hint_text("2024-05-31 14:30:00") } else { edit };
                    if ui.add_enabled(can_write, edit).changed() && many {
                        b.change[i] = true;
                    }
                    ui.end_row();
                }
            });
            ui.weak("An empty field removes that information. The picture itself is not recompressed and the file keeps its dates.");
            ui.add_space(4.0);
            if can_write {
                let label = if many { format!("Write to {} files", targets.len()) } else { "Save".to_string() };
                let anything = !many || b.change.iter().any(|c| *c);
                save = ui.add_enabled(anything, egui::Button::new(label)).clicked();
            } else {
                ui.colored_label(ui.visuals().warn_fg_color, "Information can be changed in JPEG, PNG, WebP and TIFF files only.");
            }
            if let Some(message) = &b.info_message {
                ui.label(message);
            }
        });

        if save {
            let b = &self.batch;
            let many = single.is_none();
            let old = &b.info.fields;
            let stored = [&old.description, &old.artist, &old.copyright, &old.taken];
            // One file: write what differs from what is stored. Many: write what is ticked.
            let pick = |i: usize| -> Option<String> {
                let wanted = if many { b.change[i] } else { b.text[i].trim() != stored[i].as_deref().unwrap_or("") };
                wanted.then(|| b.text[i].trim().to_string())
            };
            let changes = Fields { description: pick(0), artist: pick(1), copyright: pick(2), taken: pick(3) };
            let mut failed = Vec::new();
            for path in &targets {
                if let Err(e) = metadata::write(path, &changes) {
                    failed.push(format!("{}: {e}", file_name(path)));
                }
            }
            let written = targets.len() - failed.len();
            self.batch.info_message = Some(match failed.first() {
                None if changes.is_empty() => "Nothing was changed.".to_string(),
                None => format!("Saved to {written} file{}.", if written == 1 { "" } else { "s" }),
                Some(first) => format!("{written} saved, {} failed. {first}", failed.len()),
            });
            if written > 0 && !changes.is_empty() {
                self.say("Information saved".to_string(), now);
                // Show what is now stored (file sizes changed too).
                self.batch.info_for = None;
                let message = self.batch.info_message.take();
                self.files_changed(targets);
                self.batch.info_message = message;
            }
        }
        self.batch.info_open = open;
    }
}
