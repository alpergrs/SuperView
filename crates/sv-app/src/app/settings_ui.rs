//! The settings window.

use super::*;
use crate::register;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    General,
    Keys,
    Toolbar,
    StatusBar,
    Windows,
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    tab: Tab,
    /// The command whose new key is being waited for.
    pub capture: Option<Command>,
    message: Option<String>,
    /// Bytes used by the thumbnail cache; measured when the window opens.
    cache_size: Option<u64>,
    cache_message: Option<String>,
}

impl App {
    pub(super) fn settings_window(&mut self, ctx: &egui::Context, now: f64) {
        if !self.settings_ui.open {
            self.settings_ui.capture = None;
            self.settings_ui.cache_size = None;
            self.settings_ui.cache_message = None;
            return;
        }
        if self.settings_ui.cache_size.is_none() {
            self.settings_ui.cache_size = Some(self.thumbs.cache_size());
        }
        let mut open = true;
        egui::Window::new("Settings").open(&mut open).collapsible(false).default_size([560.0, 480.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let tab = &mut self.settings_ui.tab;
                ui.selectable_value(tab, Tab::General, "General");
                ui.selectable_value(tab, Tab::Keys, "Keys");
                ui.selectable_value(tab, Tab::Toolbar, "Toolbar");
                ui.selectable_value(tab, Tab::StatusBar, "Status bar");
                ui.selectable_value(tab, Tab::Windows, "Windows");
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match self.settings_ui.tab {
                Tab::General => self.settings_general(ui, now),
                Tab::Keys => self.settings_keys(ui),
                Tab::Toolbar => self.settings_toolbar(ui),
                Tab::StatusBar => self.settings_status(ui),
                Tab::Windows => self.settings_windows(ui),
            });
        });
        self.settings_ui.open = open;
    }

    fn settings_general(&mut self, ui: &mut egui::Ui, now: f64) {
        let before = self.settings.clone();
        ui.strong("Start folder");
        ui.weak("Shown when SuperView is opened by itself, without a picture.");
        let shown = self.settings.start_folder().map(|p| p.display().to_string()).unwrap_or_else(|| "(none)".to_string());
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(shown).truncate());
        });
        ui.horizontal(|ui| {
            if ui.button("Choose…").clicked() {
                let mut picker = rfd::FileDialog::new().set_title("Start folder");
                if let Some(dir) = self.settings.start_folder() {
                    picker = picker.set_directory(dir);
                }
                if let Some(dir) = picker.pick_folder() {
                    self.settings.start_folder = Some(dir.display().to_string());
                    self.save_settings();
                }
            }
            if ui.add_enabled(self.settings.start_folder.is_some(), egui::Button::new("Back to Pictures")).clicked() {
                self.settings.start_folder = None;
                self.save_settings();
            }
        });
        ui.add_space(8.0);
        ui.strong("Thumbnail cache");
        ui.label(format!("Cache size: {}", human_size(self.settings_ui.cache_size.unwrap_or(0))));
        ui.horizontal(|ui| {
            ui.label("Keep thumbnails on disk for folders with more than");
            ui.add(egui::DragValue::new(&mut self.settings.cache_threshold_mb).range(1..=100_000).suffix(" MB"));
            ui.label("of pictures");
        });
        ui.horizontal(|ui| {
            let default = sv_core::settings::DEFAULT_CACHE_THRESHOLD_MB;
            let tip = format!("Back to {default} MB, and delete the cache of every folder smaller than that");
            if ui.button("Reset to default").on_hover_text(tip).clicked() {
                self.settings.cache_threshold_mb = default;
                let removed = self.thumbs.remove_small_caches(default as u64 * 1024 * 1024);
                self.settings_ui.cache_size = Some(self.thumbs.cache_size());
                self.settings_ui.cache_message = Some(match removed {
                    0 => format!("Back to {default} MB. No small caches to remove."),
                    1 => format!("Back to {default} MB. Removed the cache of 1 small folder."),
                    n => format!("Back to {default} MB. Removed the caches of {n} small folders."),
                });
            }
            if ui.button("Clear cache").on_hover_text("Delete all cached thumbnails. They are made again when needed.").clicked() {
                self.thumbs.clear_cache();
                self.settings_ui.cache_size = Some(self.thumbs.cache_size());
                self.settings_ui.cache_message = Some("Cache cleared.".to_string());
            }
        });
        if let Some(message) = &self.settings_ui.cache_message {
            ui.weak(message);
        }
        ui.weak("One cache file per folder, kept in %LOCALAPPDATA%\\SuperView\\cache. Smaller folders are never cached.");
        ui.add_space(8.0);
        ui.strong("Behind transparent pictures");
        ui.horizontal_wrapped(|ui| {
            for b in sv_core::settings::Backdrop::ALL {
                ui.radio_value(&mut self.settings.transparency, b, b.label());
            }
        });
        ui.weak("T switches between them while viewing.");
        ui.add_space(8.0);
        ui.strong("Theme");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.settings.theme, Theme::System, "Follow Windows");
            ui.radio_value(&mut self.settings.theme, Theme::Light, "Light");
            ui.radio_value(&mut self.settings.theme, Theme::Dark, "Dark");
        });
        ui.add_space(8.0);
        ui.strong("Mouse wheel");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.settings.wheel, WheelAction::Zoom, "Zooms");
            ui.radio_value(&mut self.settings.wheel, WheelAction::Navigate, "Changes picture");
        });
        ui.weak("Ctrl + wheel does the other one.");
        ui.add_space(8.0);
        ui.strong("Opening a picture");
        let mut mode = self.settings.fit_mode;
        egui::ComboBox::from_id_salt("fit").selected_text(mode.label()).show_ui(ui, |ui| {
            for m in FitMode::ALL {
                ui.selectable_value(&mut mode, m, m.label());
            }
        });
        if mode != self.settings.fit_mode {
            self.set_fit_mode(mode, now);
        }
        ui.checkbox(&mut self.settings.keep_zoom, "Keep zoom between pictures");
        ui.add_space(8.0);
        ui.strong("Slideshow");
        ui.horizontal(|ui| {
            ui.label("Seconds per picture");
            ui.add(egui::DragValue::new(&mut self.settings.slideshow_seconds).range(0.5..=3600.0).speed(0.1));
        });
        ui.checkbox(&mut self.settings.slideshow_random, "Random order");
        ui.checkbox(&mut self.settings.slideshow_loop, "Start over at the end");
        ui.add_space(8.0);
        ui.strong("\"Slideshow with SuperView\" on a folder in Explorer");
        ui.checkbox(&mut self.settings.explorer_recursive, "Include subfolders");
        ui.weak("Order: the folder's own pictures first, then each subfolder in turn, each followed by its own subfolders.");
        ui.checkbox(&mut self.settings.explorer_autoplay, "Start playing automatically");

        if self.settings != before {
            if self.settings.theme != before.theme {
                apply_theme(ui.ctx(), self.settings.theme);
            }
            if self.settings.cache_threshold_mb != before.cache_threshold_mb {
                self.update_cached_folders();
            }
            self.save_settings();
        }
    }

    fn settings_keys(&mut self, ui: &mut egui::Ui) {
        ui.weak("Set key: press the new key, with Ctrl or Shift if you like. A key already in use moves to the new command.");
        ui.add_space(4.0);
        let mut changed = false;
        egui::Grid::new("key_grid").num_columns(4).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
            for command in Command::ALL {
                ui.label(command.label());
                if self.settings_ui.capture == Some(command) {
                    ui.strong("Press a key…  (Esc cancels)");
                } else {
                    ui.label(input::all_shortcuts(&self.bindings, command).join(",  "));
                }
                if command == Command::Copy {
                    // Ctrl+C is the system copy shortcut and stays fixed.
                    ui.label("");
                    ui.label("");
                } else {
                    if ui.button("Set key").clicked() {
                        self.settings_ui.capture = Some(command);
                    }
                    if ui.button("Clear").clicked() {
                        self.bindings.retain(|b| b.command != command);
                        changed = true;
                    }
                }
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        if ui.button("Reset all keys to the defaults").clicked() {
            self.bindings = input::default_bindings();
            self.settings.keys = None;
            self.settings_ui.capture = None;
            self.save_settings();
        } else if changed {
            self.settings.keys = Some(input::to_settings(&self.bindings));
            self.save_settings();
        }
    }

    fn settings_toolbar(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.settings.show_toolbar, "Show the toolbar in the viewer").changed() {
            self.save_settings();
        }
        let label = "Thumbnail view: buttons as icons (point at one to see its name)";
        if ui.checkbox(&mut self.settings.browser_icons, label).changed() {
            self.save_settings();
        }
        ui.add_space(4.0);
        ui.strong("Buttons");
        let mut chosen: Vec<Command> = match &self.settings.toolbar {
            Some(ids) => ids.iter().filter_map(|id| Command::from_id(id)).collect(),
            None => Command::DEFAULT_TOOLBAR.to_vec(),
        };
        let mut changed = false;
        ui.columns(2, |columns| {
            for (i, command) in Command::ALL.into_iter().enumerate() {
                let mut on = chosen.contains(&command);
                if columns[i % 2].checkbox(&mut on, command.label()).changed() {
                    changed = true;
                    if on {
                        chosen.push(command);
                    } else {
                        chosen.retain(|c| *c != command);
                    }
                }
            }
        });
        ui.add_space(8.0);
        if ui.button("Reset to the default buttons").clicked() {
            self.settings.toolbar = None;
            self.save_settings();
        } else if changed {
            // Buttons keep the order of the command list, whatever order they were ticked in.
            let ordered: Vec<String> = Command::ALL.into_iter().filter(|c| chosen.contains(c)).map(Command::id).collect();
            self.settings.toolbar = Some(ordered);
            self.save_settings();
        }
    }

    fn settings_status(&mut self, ui: &mut egui::Ui) {
        let before = (self.settings.show_status_bar, self.settings.status);
        ui.checkbox(&mut self.settings.show_status_bar, "Show the status bar");
        ui.add_space(4.0);
        ui.strong("Fields");
        let f = &mut self.settings.status;
        ui.checkbox(&mut f.name, "File name");
        ui.checkbox(&mut f.position, "Position in the folder (12 / 340)");
        ui.checkbox(&mut f.dimensions, "Width × height");
        ui.checkbox(&mut f.format, "Format");
        ui.checkbox(&mut f.size, "File size");
        ui.checkbox(&mut f.date, "Date modified");
        ui.checkbox(&mut f.zoom, "Zoom");
        if before != (self.settings.show_status_bar, self.settings.status) {
            self.save_settings();
        }
    }

    fn settings_windows(&mut self, ui: &mut egui::Ui) {
        ui.label("Add SuperView to Windows Explorer for your user account:");
        ui.label("  •  \"Open with\" for JPEG, PNG, GIF, BMP, WebP, TIFF, HEIC, AVIF, PSD and camera RAW files");
        ui.label("  •  \"Open with SuperView\" when you right-click a picture");
        ui.label("  •  \"Slideshow with SuperView\" when you right-click a folder: opens its first picture");
        ui.weak("Whether subfolders are included is on the General tab and can be changed at any time.");
        ui.add_space(6.0);
        ui.weak("Windows decides the default app itself. After adding, right-click a picture, choose Open with, then Choose another app, pick SuperView and tick Always.");
        ui.weak("If you move superview.exe, add it again from its new place.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(register::SUPPORTED, egui::Button::new("Add to Explorer")).clicked() {
                self.settings_ui.message = Some(match register::register() {
                    Ok(()) => "Added. New Explorer windows show the entries.".to_string(),
                    Err(e) => format!("Could not add: {e}"),
                });
            }
            if ui.add_enabled(register::SUPPORTED, egui::Button::new("Remove from Explorer")).clicked() {
                self.settings_ui.message = Some(match register::unregister() {
                    Ok(()) => "Removed.".to_string(),
                    Err(e) => format!("Could not remove: {e}"),
                });
            }
        });
        if let Some(message) = &self.settings_ui.message {
            ui.add_space(4.0);
            ui.label(message);
        }
    }
}
