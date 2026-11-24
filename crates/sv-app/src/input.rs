//! Commands and their keys. Everything SuperView can do goes through [`Command`], so
//! menus, the toolbar, the help sheet and user-defined keys share one table.

use eframe::egui::Key;
use sv_core::settings::KeyBinding;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Open,
    Next,
    Prev,
    First,
    Last,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomActual,
    CycleFit,
    Fullscreen,
    Browser,
    RotateRight,
    RotateLeft,
    FlipH,
    FlipV,
    SaveRotation,
    SaveAs,
    Edit,
    Crop,
    Undo,
    Redo,
    Delete,
    Rename,
    Copy,
    CopyPath,
    Mark,
    MarkAll,
    ClearMarks,
    CopyTo,
    MoveTo,
    Batch,
    BatchRename,
    Info,
    Search,
    Duplicates,
    Slideshow,
    Picker,
    StatusBar,
    Toolbar,
    Transparency,
    NextPage,
    PrevPage,
    Settings,
    Help,
    Back,
}

impl Command {
    pub const ALL: [Command; 46] = [
        Command::Open,
        Command::Next,
        Command::Prev,
        Command::First,
        Command::Last,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::ZoomFit,
        Command::ZoomActual,
        Command::CycleFit,
        Command::Fullscreen,
        Command::Browser,
        Command::RotateRight,
        Command::RotateLeft,
        Command::FlipH,
        Command::FlipV,
        Command::SaveRotation,
        Command::SaveAs,
        Command::Edit,
        Command::Crop,
        Command::Undo,
        Command::Redo,
        Command::Delete,
        Command::Rename,
        Command::Copy,
        Command::CopyPath,
        Command::Mark,
        Command::MarkAll,
        Command::ClearMarks,
        Command::CopyTo,
        Command::MoveTo,
        Command::Batch,
        Command::BatchRename,
        Command::Info,
        Command::Search,
        Command::Duplicates,
        Command::Slideshow,
        Command::Picker,
        Command::StatusBar,
        Command::Toolbar,
        Command::Transparency,
        Command::NextPage,
        Command::PrevPage,
        Command::Settings,
        Command::Help,
        Command::Back,
    ];

    /// The toolbar a fresh install shows.
    pub const DEFAULT_TOOLBAR: [Command; 14] = [
        Command::Open,
        Command::Browser,
        Command::Prev,
        Command::Next,
        Command::ZoomOut,
        Command::ZoomIn,
        Command::ZoomFit,
        Command::ZoomActual,
        Command::RotateLeft,
        Command::RotateRight,
        Command::SaveRotation,
        Command::Delete,
        Command::Slideshow,
        Command::Fullscreen,
    ];

    /// Stable name used in the settings file.
    pub fn id(self) -> String {
        format!("{self:?}")
    }

    pub fn from_id(id: &str) -> Option<Command> {
        Command::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Command::Open => "Open…",
            Command::Next => "Next picture",
            Command::Prev => "Previous picture",
            Command::First => "First picture",
            Command::Last => "Last picture",
            Command::ZoomIn => "Zoom in",
            Command::ZoomOut => "Zoom out",
            Command::ZoomFit => "Reset zoom to fit mode",
            Command::ZoomActual => "Actual size (100%)",
            Command::CycleFit => "Next fit mode",
            Command::Fullscreen => "Fullscreen",
            Command::Browser => "Browser / viewer",
            Command::RotateRight => "Rotate right",
            Command::RotateLeft => "Rotate left",
            Command::FlipH => "Flip horizontally",
            Command::FlipV => "Flip vertically",
            Command::SaveRotation => "Save (rotation or edits) to the file",
            Command::SaveAs => "Save as… / save for web",
            Command::Edit => "Edit panel",
            Command::Crop => "Crop",
            Command::Undo => "Undo edit",
            Command::Redo => "Redo edit",
            Command::Delete => "Delete to Recycle Bin",
            Command::Rename => "Rename…",
            Command::Copy => "Copy picture",
            Command::CopyPath => "Copy file path",
            Command::Mark => "Mark / unmark",
            Command::MarkAll => "Mark all",
            Command::ClearMarks => "Unmark all",
            Command::CopyTo => "Copy to folder…",
            Command::MoveTo => "Move to folder…",
            Command::Batch => "Batch convert / resize / rotate…",
            Command::BatchRename => "Batch rename…",
            Command::Info => "Information (metadata)…",
            Command::Search => "Search",
            Command::Duplicates => "Find duplicates…",
            Command::Slideshow => "Start / stop slideshow",
            Command::Picker => "Colour picker",
            Command::StatusBar => "Status bar",
            Command::Toolbar => "Toolbar",
            Command::Transparency => "Background behind transparency",
            Command::NextPage => "Next page",
            Command::PrevPage => "Previous page",
            Command::Settings => "Settings…",
            Command::Help => "Keyboard shortcuts",
            Command::Back => "Back / close",
        }
    }

    /// Short text for a toolbar button.
    pub fn short(self) -> &'static str {
        match self {
            Command::Open => "Open",
            Command::Next => "Next",
            Command::Prev => "Prev",
            Command::First => "First",
            Command::Last => "Last",
            Command::ZoomIn => "+",
            Command::ZoomOut => "−",
            Command::ZoomFit => "Fit",
            Command::ZoomActual => "1:1",
            Command::CycleFit => "Fit mode",
            Command::Fullscreen => "Fullscreen",
            Command::Browser => "Browser",
            Command::RotateRight => "Rotate R",
            Command::RotateLeft => "Rotate L",
            Command::FlipH => "Flip H",
            Command::FlipV => "Flip V",
            Command::SaveRotation => "Save",
            Command::SaveAs => "Save as",
            Command::Edit => "Edit",
            Command::Crop => "Crop",
            Command::Undo => "Undo",
            Command::Redo => "Redo",
            Command::Delete => "Delete",
            Command::Rename => "Rename",
            Command::Copy => "Copy",
            Command::CopyPath => "Copy path",
            Command::Mark => "Mark",
            Command::MarkAll => "Mark all",
            Command::ClearMarks => "Unmark",
            Command::CopyTo => "Copy to",
            Command::MoveTo => "Move to",
            Command::Batch => "Batch",
            Command::BatchRename => "Batch rename",
            Command::Info => "Info",
            Command::Search => "Search",
            Command::Duplicates => "Duplicates",
            Command::Slideshow => "Slideshow",
            Command::Picker => "Picker",
            Command::StatusBar => "Status",
            Command::Toolbar => "Toolbar",
            Command::Transparency => "Backdrop",
            Command::NextPage => "Next page",
            Command::PrevPage => "Prev page",
            Command::Settings => "Settings",
            Command::Help => "Keys",
            Command::Back => "Back",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Binding {
    pub ctrl: bool,
    pub shift: bool,
    pub key: Key,
    pub command: Command,
}

impl Binding {
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(match self.key {
            Key::ArrowLeft => "Left",
            Key::ArrowRight => "Right",
            Key::ArrowUp => "Up",
            Key::ArrowDown => "Down",
            Key::Plus | Key::Equals => "+",
            Key::Minus => "-",
            Key::Comma => ",",
            Key::Num0 => "0",
            Key::Num1 => "1",
            other => other.name(),
        });
        s
    }

    /// Whether holding the key down repeats the command.
    pub fn repeats(&self) -> bool {
        matches!(self.command, Command::Next | Command::Prev | Command::ZoomIn | Command::ZoomOut)
    }
}

pub fn default_bindings() -> Vec<Binding> {
    use Command::*;
    let b = |key, command| Binding { ctrl: false, shift: false, key, command };
    let ctrl = |key, command| Binding { ctrl: true, ..b(key, command) };
    vec![
        ctrl(Key::O, Open),
        b(Key::ArrowRight, Next),
        b(Key::Space, Next),
        b(Key::PageDown, Next),
        b(Key::ArrowLeft, Prev),
        b(Key::Backspace, Prev),
        b(Key::PageUp, Prev),
        b(Key::Home, First),
        b(Key::End, Last),
        b(Key::Plus, ZoomIn),
        b(Key::Equals, ZoomIn),
        b(Key::Minus, ZoomOut),
        b(Key::Num0, ZoomFit),
        b(Key::Num1, ZoomActual),
        b(Key::F, CycleFit),
        b(Key::F11, Fullscreen),
        b(Key::Enter, Browser),
        b(Key::R, RotateRight),
        b(Key::L, RotateLeft),
        b(Key::H, FlipH),
        b(Key::V, FlipV),
        ctrl(Key::S, SaveRotation),
        Binding { shift: true, ..ctrl(Key::S, SaveAs) },
        b(Key::E, Edit),
        b(Key::C, Crop),
        ctrl(Key::Z, Undo),
        ctrl(Key::Y, Redo),
        b(Key::Delete, Delete),
        b(Key::F2, Rename),
        Binding { shift: true, ..ctrl(Key::C, CopyPath) },
        b(Key::M, Mark),
        b(Key::Insert, Mark),
        ctrl(Key::A, MarkAll),
        Binding { shift: true, ..ctrl(Key::A, ClearMarks) },
        b(Key::F8, CopyTo),
        b(Key::F7, MoveTo),
        ctrl(Key::B, Batch),
        ctrl(Key::R, BatchRename),
        b(Key::I, Info),
        ctrl(Key::F, Search),
        ctrl(Key::D, Duplicates),
        b(Key::S, Slideshow),
        b(Key::P, Picker),
        b(Key::B, StatusBar),
        b(Key::Tab, Toolbar),
        b(Key::T, Transparency),
        ctrl(Key::PageDown, NextPage),
        ctrl(Key::PageUp, PrevPage),
        ctrl(Key::Comma, Settings),
        b(Key::F1, Help),
        b(Key::Escape, Back),
    ]
}

/// Bindings from the settings file; anything unreadable in it is skipped.
pub fn from_settings(saved: &Option<Vec<KeyBinding>>) -> Vec<Binding> {
    match saved {
        None => default_bindings(),
        Some(list) => {
            let mut bindings: Vec<Binding> = list
                .iter()
                .filter_map(|k| {
                    Some(Binding {
                        ctrl: k.ctrl,
                        shift: k.shift,
                        key: Key::from_name(&k.key)?,
                        command: Command::from_id(&k.command)?,
                    })
                })
                .collect();
            // Commands that are newer than the saved keys get their default keys, as long
            // as the user has not given those keys to something else.
            let known: Vec<String> = list.iter().map(|k| k.command.clone()).collect();
            for default in default_bindings() {
                let new_command = !known.contains(&default.command.id());
                let key_free = !bindings.iter().any(|b| b.key == default.key && b.ctrl == default.ctrl && b.shift == default.shift);
                if new_command && key_free {
                    bindings.push(default);
                }
            }
            bindings
        }
    }
}

pub fn to_settings(bindings: &[Binding]) -> Vec<KeyBinding> {
    bindings
        .iter()
        .map(|b| KeyBinding { command: b.command.id(), key: b.key.name().to_string(), ctrl: b.ctrl, shift: b.shift })
        .collect()
}

/// The shortcut shown next to a command in menus: its first binding.
pub fn shortcut(bindings: &[Binding], command: Command) -> String {
    if command == Command::Copy {
        return "Ctrl+C".to_string();
    }
    bindings.iter().find(|b| b.command == command).map(Binding::text).unwrap_or_default()
}

/// All keys of a command, for the help sheet and the settings window.
pub fn all_shortcuts(bindings: &[Binding], command: Command) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    if command == Command::Copy {
        keys.push("Ctrl+C".to_string());
    }
    for b in bindings.iter().filter(|b| b.command == command) {
        if !keys.contains(&b.text()) {
            keys.push(b.text());
        }
    }
    keys
}

/// Which command a key press triggers, if any.
pub fn lookup(bindings: &[Binding], key: Key, ctrl: bool, shift: bool, alt: bool, repeat: bool) -> Option<Command> {
    if alt {
        return None;
    }
    bindings
        .iter()
        .find(|b| b.key == key && b.ctrl == ctrl && b.shift == shift && (b.repeats() || !repeat))
        .map(|b| b.command)
}

/// Give `command` the key, taking it away from whatever had it before.
pub fn assign(bindings: &mut Vec<Binding>, command: Command, key: Key, ctrl: bool, shift: bool) {
    bindings.retain(|b| !(b.key == key && b.ctrl == ctrl && b.shift == shift));
    bindings.push(Binding { ctrl, shift, key, command });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_must_match_exactly() {
        let b = default_bindings();
        assert_eq!(lookup(&b, Key::PageDown, false, false, false, false), Some(Command::Next));
        assert_eq!(lookup(&b, Key::PageDown, true, false, false, false), Some(Command::NextPage));
        assert_eq!(lookup(&b, Key::O, false, false, false, false), None);
        assert_eq!(lookup(&b, Key::C, true, true, false, false), Some(Command::CopyPath));
        assert_eq!(lookup(&b, Key::R, false, false, true, false), None);
        assert_eq!(lookup(&b, Key::S, true, false, false, false), Some(Command::SaveRotation));
        assert_eq!(lookup(&b, Key::S, false, false, false, false), Some(Command::Slideshow));
    }

    #[test]
    fn only_some_keys_repeat() {
        let b = default_bindings();
        assert_eq!(lookup(&b, Key::ArrowRight, false, false, false, true), Some(Command::Next));
        assert_eq!(lookup(&b, Key::Delete, false, false, false, true), None);
        assert_eq!(lookup(&b, Key::S, true, false, false, true), None);
    }

    #[test]
    fn menus_show_the_first_binding() {
        let b = default_bindings();
        assert_eq!(shortcut(&b, Command::Next), "Right");
        assert_eq!(shortcut(&b, Command::Open), "Ctrl+O");
        assert_eq!(shortcut(&b, Command::CopyPath), "Ctrl+Shift+C");
        assert_eq!(shortcut(&b, Command::Copy), "Ctrl+C");
        assert_eq!(all_shortcuts(&b, Command::Mark), ["M", "Insert"]);
        assert_eq!(shortcut(&b, Command::Transparency), "T");
        assert_eq!(shortcut(&b, Command::Toolbar), "Tab");
    }

    #[test]
    fn every_command_has_a_stable_unique_name() {
        let mut ids: Vec<String> = Command::ALL.iter().map(|c| c.id()).collect();
        for c in Command::ALL {
            assert_eq!(Command::from_id(&c.id()), Some(c));
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), Command::ALL.len());
        assert_eq!(Command::from_id("Nonsense"), None);
    }

    #[test]
    fn bindings_survive_the_settings_file() {
        let defaults = default_bindings();
        let restored = from_settings(&Some(to_settings(&defaults)));
        assert_eq!(restored, defaults);
        assert_eq!(from_settings(&None), defaults);
        // Unknown entries are dropped, not fatal; commands missing from the file get
        // their default keys.
        let odd = vec![KeyBinding { command: "Gone".into(), key: "Q".into(), ctrl: false, shift: false }];
        assert_eq!(from_settings(&Some(odd)), defaults);

        // A command added later gets its default key, unless the user's keys use it.
        let mut saved = to_settings(&defaults);
        saved.retain(|k| k.command != "Transparency");
        saved.iter_mut().filter(|k| k.command == "Slideshow").for_each(|k| k.key = "T".into());
        let restored = from_settings(&Some(saved));
        assert_eq!(lookup(&restored, Key::T, false, false, false, false), Some(Command::Slideshow));
        assert!(restored.iter().all(|b| b.command != Command::Transparency), "its key is taken, so it has none");
    }

    #[test]
    fn assigning_a_key_takes_it_from_the_previous_owner() {
        let mut b = default_bindings();
        assign(&mut b, Command::Delete, Key::S, false, false);
        assert_eq!(lookup(&b, Key::S, false, false, false, false), Some(Command::Delete));
        assert_eq!(shortcut(&b, Command::Slideshow), "");
        // Ctrl+S is a different key and stays.
        assert_eq!(lookup(&b, Key::S, true, false, false, false), Some(Command::SaveRotation));
    }
}
