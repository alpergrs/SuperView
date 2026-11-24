//! Listing the pictures of a folder and putting them in order.

use crate::decode;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    /// Hidden or system file, or inside a hidden folder below the listed one.
    pub hidden: bool,
}

/// Whether Windows marks the file or folder as hidden or system. Names starting with a
/// dot count as hidden too, as they do everywhere else.
pub fn is_hidden(path: &Path, meta: &std::fs::Metadata) -> bool {
    if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
        if meta.file_attributes() & HIDDEN_OR_SYSTEM != 0 {
            return true;
        }
    }
    let _ = meta;
    false
}

/// `is_hidden` for a path given on its own, e.g. the file the program was started with.
pub fn is_hidden_path(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| is_hidden(path, &meta))
}

/// Whether the path, or any folder above it, is hidden or system: a picture deep inside
/// `AppData` counts. Drive roots are skipped: Windows marks every one of them as a
/// hidden system folder, which says nothing about what is on the drive.
pub fn is_inside_hidden(path: &Path) -> bool {
    path.ancestors().filter(|p| p.file_name().is_some()).any(is_hidden_path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Date,
    Size,
    Type,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [SortKey::Name, SortKey::Date, SortKey::Size, SortKey::Type];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Date => "Date modified",
            SortKey::Size => "Size",
            SortKey::Type => "Type",
        }
    }
}

/// List supported images in `dir`. Subfolders are only entered when `recursive` is set.
/// Unreadable entries are skipped.
pub fn list(dir: &Path, recursive: bool) -> Vec<Entry> {
    let mut walker = walkdir::WalkDir::new(dir).min_depth(1).follow_links(false);
    if !recursive {
        walker = walker.max_depth(1);
    }
    let mut out = Vec::new();
    // Folders are visited before their contents, so a hidden folder is known by the
    // time its pictures come up.
    let mut hidden_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for item in walker.into_iter().filter_map(Result::ok) {
        let in_hidden_dir = item.path().parent().is_some_and(|p| hidden_dirs.contains(p));
        if item.file_type().is_dir() {
            if in_hidden_dir || item.metadata().is_ok_and(|m| is_hidden(item.path(), &m)) {
                hidden_dirs.insert(item.path().to_path_buf());
            }
            continue;
        }
        if !item.file_type().is_file() || !decode::is_supported(item.path()) {
            continue;
        }
        let Ok(meta) = item.metadata() else { continue };
        out.push(Entry {
            hidden: in_hidden_dir || is_hidden(item.path(), &meta),
            path: item.into_path(),
            size: meta.len(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    out
}

/// Put entries in viewing order.
///
/// Pictures of one folder stay together and are ordered by `key`. Across folders the
/// order is the folder tree read top-down: a folder's own pictures first, then each of
/// its subfolders in name order, each followed by its own subfolders. So with
/// `A/{A1/{A1A, A1B}, A2}` the order is A, A1, A1A, A1B, A2.
pub fn sort(entries: &mut [Entry], key: SortKey, descending: bool) {
    entries.sort_by(|a, b| {
        let (dir_a, dir_b) = (a.path.parent(), b.path.parent());
        if dir_a != dir_b {
            let mut ca = dir_a.into_iter().flat_map(Path::components);
            let mut cb = dir_b.into_iter().flat_map(Path::components);
            loop {
                match (ca.next(), cb.next()) {
                    (Some(x), Some(y)) if x == y => continue,
                    (Some(x), Some(y)) => {
                        return natural_cmp(&x.as_os_str().to_string_lossy(), &y.as_os_str().to_string_lossy());
                    }
                    // `a` sits directly in a folder that `b` is deeper inside of.
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (None, None) => break,
                }
            }
        }
        let name = |e: &Entry| e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let by_name = || natural_cmp(&name(a), &name(b));
        let ord = match key {
            SortKey::Name => by_name(),
            SortKey::Date => a.modified.cmp(&b.modified).then_with(by_name),
            SortKey::Size => a.size.cmp(&b.size).then_with(by_name),
            SortKey::Type => extension(&a.path).cmp(&extension(&b.path)).then_with(by_name),
        };
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Case-insensitive comparison that orders digit runs by value, the way Explorer does:
/// `img2` comes before `img10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let strip = |s: &[char]| -> Vec<char> {
                let first = s.iter().position(|c| *c != '0').unwrap_or(s.len());
                s[first..].to_vec()
            };
            let (na, nb) = (strip(&a[si..i]), strip(&b[sj..j]));
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let ord = a[i].to_lowercase().cmp(b[j].to_lowercase());
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j)).then_with(|| a.cmp(&b))
}

/// What the search box asks for. Every part that is set must match.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    /// Words that must all appear in the file's name or folder path, in any order,
    /// ignoring letter case.
    pub text: String,
    pub min_bytes: Option<u64>,
    pub max_bytes: Option<u64>,
    /// Modified at or after this moment.
    pub from: Option<SystemTime>,
    /// Modified before this moment.
    pub until: Option<SystemTime>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.min_bytes.is_none() && self.max_bytes.is_none() && self.from.is_none() && self.until.is_none()
    }

    pub fn matches(&self, entry: &Entry) -> bool {
        if self.min_bytes.is_some_and(|min| entry.size < min) || self.max_bytes.is_some_and(|max| entry.size > max) {
            return false;
        }
        if self.from.is_some_and(|from| entry.modified < from) || self.until.is_some_and(|until| entry.modified >= until) {
            return false;
        }
        let path = entry.path.to_string_lossy().to_lowercase();
        self.text.split_whitespace().all(|word| path.contains(&word.to_lowercase()))
    }
}

/// The start of a day written as `2024-05-31` (also `2024-05` or `2024`), in local time.
/// With `end` set, the start of the day, month or year after it, so a range can include
/// the whole of its last day.
pub fn parse_day(text: &str, end: bool) -> Option<SystemTime> {
    use chrono::{Datelike, Local, NaiveDate, TimeZone};
    let parts: Vec<u32> = text.trim().split(['-', '.', '/']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let (date, next) = match parts[..] {
        [y] => (NaiveDate::from_ymd_opt(y as i32, 1, 1)?, NaiveDate::from_ymd_opt(y as i32 + 1, 1, 1)?),
        [y, m] => {
            let first = NaiveDate::from_ymd_opt(y as i32, m, 1)?;
            let (ny, nm) = if m == 12 { (y as i32 + 1, 1) } else { (y as i32, m + 1) };
            (first, NaiveDate::from_ymd_opt(ny, nm, 1)?)
        }
        [y, m, d] => {
            let day = NaiveDate::from_ymd_opt(y as i32, m, d)?;
            (day, day.succ_opt()?)
        }
        _ => return None,
    };
    if !(1000..=9999).contains(&date.year()) {
        return None;
    }
    let chosen = if end { next } else { date };
    let local = Local.from_local_datetime(&chosen.and_hms_opt(0, 0, 0)?).earliest()?;
    Some(local.into())
}
