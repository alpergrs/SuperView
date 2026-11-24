//! User settings, stored as TOML in `%APPDATA%\SuperView\settings.toml`.

use crate::folder::SortKey;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FitMode {
    /// Shrink pictures larger than the window, show small ones at 100%.
    #[default]
    ShrinkToFit,
    /// Always scale to the window, enlarging small pictures too.
    FitWindow,
    FitWidth,
    /// Cover the whole window, cropping the overflow.
    Fill,
    /// Always open at 100%.
    Actual,
}

impl FitMode {
    pub const ALL: [FitMode; 5] =
        [FitMode::ShrinkToFit, FitMode::FitWindow, FitMode::FitWidth, FitMode::Fill, FitMode::Actual];

    pub fn label(self) -> &'static str {
        match self {
            FitMode::ShrinkToFit => "Fit, large pictures only",
            FitMode::FitWindow => "Fit to window",
            FitMode::FitWidth => "Fit width",
            FitMode::Fill => "Fill window",
            FitMode::Actual => "Actual size (100%)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WheelAction {
    #[default]
    Zoom,
    Navigate,
}

pub const DEFAULT_CACHE_THRESHOLD_MB: u32 = 100;

/// What is drawn behind a picture's transparent parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    #[default]
    Checkerboard,
    Black,
    White,
    /// The viewer's own background colour.
    Normal,
}

impl Backdrop {
    pub const ALL: [Backdrop; 4] = [Backdrop::Checkerboard, Backdrop::Black, Backdrop::White, Backdrop::Normal];

    pub fn label(self) -> &'static str {
        match self {
            Backdrop::Checkerboard => "Checkerboard",
            Backdrop::Black => "Black",
            Backdrop::White => "White",
            Backdrop::Normal => "Normal background",
        }
    }

    /// The next one in the list, for the key that cycles through them.
    pub fn next(self) -> Backdrop {
        let i = Backdrop::ALL.iter().position(|b| *b == self).unwrap_or(0);
        Backdrop::ALL[(i + 1) % Backdrop::ALL.len()]
    }
}

/// One user-defined key. Stored by name so the file stays readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    pub command: String,
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
}

/// Which pieces of information the status bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusFields {
    pub name: bool,
    pub position: bool,
    pub dimensions: bool,
    pub format: bool,
    pub size: bool,
    pub date: bool,
    pub zoom: bool,
}

impl Default for StatusFields {
    fn default() -> Self {
        StatusFields { name: true, position: true, dimensions: true, format: true, size: true, date: false, zoom: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub sort_key: SortKey,
    pub sort_descending: bool,
    pub fit_mode: FitMode,
    /// Keep zoom and position when moving to the next picture.
    pub keep_zoom: bool,
    /// What the plain mouse wheel does; Ctrl+wheel does the other thing.
    pub wheel: WheelAction,
    pub show_status_bar: bool,
    pub slideshow_seconds: f32,
    pub slideshow_random: bool,
    pub slideshow_loop: bool,
    /// "Slideshow with SuperView" on a folder in Explorer also shows its subfolders.
    pub explorer_recursive: bool,
    /// ... and starts playing by itself instead of waiting on the first picture.
    pub explorer_autoplay: bool,
    pub status: StatusFields,
    pub show_toolbar: bool,
    /// Command names of the toolbar buttons. `None` means the built-in set.
    pub toolbar: Option<Vec<String>>,
    /// Thumbnail edge length in the browser, in points.
    pub thumb_size: f32,
    /// What shows through the transparent parts of a picture.
    pub transparency: Backdrop,
    /// A folder gets a thumbnail cache file once its pictures add up to this many MB.
    pub cache_threshold_mb: u32,
    pub show_tree: bool,
    pub show_preview: bool,
    /// The thumbnail view's top bar shows icons instead of words.
    pub browser_icons: bool,
    /// Folder last used for "copy to" / "move to".
    pub last_target: Option<String>,
    /// User-defined keys. `None` means the built-in keys.
    pub keys: Option<Vec<KeyBinding>>,
    /// The folder shown when SuperView is started on its own, without a picture or folder.
    /// `None` means the user's Pictures folder.
    pub start_folder: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            sort_key: SortKey::Name,
            sort_descending: false,
            fit_mode: FitMode::ShrinkToFit,
            keep_zoom: false,
            wheel: WheelAction::Zoom,
            show_status_bar: true,
            slideshow_seconds: 4.0,
            slideshow_random: false,
            slideshow_loop: true,
            explorer_recursive: true,
            explorer_autoplay: false,
            status: StatusFields::default(),
            show_toolbar: false,
            toolbar: None,
            thumb_size: 160.0,
            transparency: Backdrop::Checkerboard,
            cache_threshold_mb: DEFAULT_CACHE_THRESHOLD_MB,
            show_tree: true,
            show_preview: true,
            browser_icons: true,
            last_target: None,
            keys: None,
            start_folder: None,
        }
    }
}

/// The user's own Pictures folder (`C:\Users\<name>\Pictures` on Windows), when it exists.
pub fn pictures_folder() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let pictures = PathBuf::from(home).join("Pictures");
    pictures.is_dir().then_some(pictures)
}

impl Settings {
    /// Where to start when opened without a picture: the chosen folder if it still
    /// exists, otherwise Pictures.
    pub fn start_folder(&self) -> Option<PathBuf> {
        self.start_folder.as_ref().map(PathBuf::from).filter(|p| p.is_dir()).or_else(pictures_folder)
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("SuperView").join("settings.toml"))
    }

    /// Missing or damaged settings fall back to the defaults.
    pub fn load() -> Settings {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| Self::from_toml(&text))
            .unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Settings {
        let mut s: Settings = toml::from_str(text).unwrap_or_default();
        s.slideshow_seconds = s.slideshow_seconds.clamp(0.5, 3600.0);
        s.thumb_size = s.thumb_size.clamp(64.0, 256.0);
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }
}
