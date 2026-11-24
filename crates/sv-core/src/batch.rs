//! Doing the same thing to many pictures: convert, resize, rotate, fix, strip, rename.

use crate::decode::{self, Format};
use crate::save::{self, SaveFormat, Turn};
use crate::{edit, metadata};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resize {
    /// The longer side becomes this many pixels.
    LongSide(u32),
    Width(u32),
    Height(u32),
    Percent(f32),
}

impl Resize {
    /// The size a `width` x `height` picture ends up with.
    pub fn target(self, width: u32, height: u32, enlarge: bool) -> (u32, u32) {
        let (w, h) = (width.max(1) as f64, height.max(1) as f64);
        let mut scale = match self {
            Resize::LongSide(px) => px as f64 / w.max(h),
            Resize::Width(px) => px as f64 / w,
            Resize::Height(px) => px as f64 / h,
            Resize::Percent(p) => p as f64 / 100.0,
        };
        if !enlarge {
            scale = scale.min(1.0);
        }
        (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Next to the original. A file that keeps its format is replaced in place; a
    /// converted one becomes a new file and the original stays.
    SameFolder,
    Folder(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// `None` keeps each file's own format.
    pub convert: Option<SaveFormat>,
    /// For JPEG and WebP results, 1 to 100.
    pub quality: u8,
    pub resize: Option<Resize>,
    /// Allow resizing to make small pictures larger.
    pub enlarge: bool,
    pub turn: Turn,
    pub auto_fix: bool,
    /// Remove camera details, dates, GPS position and comments from the result.
    pub strip: bool,
    pub output: Output,
}

impl Default for Job {
    fn default() -> Self {
        Job { convert: None, quality: 90, resize: None, enlarge: false, turn: Turn::default(), auto_fix: false, strip: false, output: Output::SameFolder }
    }
}

impl Job {
    /// Whether the job would change anything at all.
    pub fn does_something(&self) -> bool {
        self.convert.is_some() || self.resize.is_some() || !self.turn.is_identity() || self.auto_fix || self.strip
    }
}

/// Run the job on one file. Returns where the result was written.
pub fn process(path: &Path, job: &Job) -> Result<PathBuf, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let format = Format::detect(&data, path).ok_or("not a picture SuperView can read")?;
    let own = SaveFormat::of_file(format);
    let target = job.convert.or(own).ok_or_else(|| format!("{} files need a format to convert to", format.name()))?;
    let converting = Some(target) != own;
    let needs_pixels = converting || job.resize.is_some() || job.auto_fix;

    let bytes = if !needs_pixels {
        // Nothing here requires recompressing: rotate JPEGs losslessly, strip in place.
        let mut bytes = if job.turn.is_identity() {
            data.clone()
        } else {
            save::rotated_bytes(&data, format, job.turn).map_err(|e| e.to_string())?.0
        };
        if job.strip {
            bytes = metadata::stripped_bytes(&bytes, format).map_err(|e| e.to_string())?;
        }
        // Unchanged and staying where it is: nothing to write. Sent to another folder,
        // an unchanged picture is still wanted there, as a plain copy.
        if bytes == data && job.output == Output::SameFolder {
            return Err("nothing to do for this file".to_string());
        }
        bytes
    } else {
        let loaded = decode::load_bytes(&data, format, 0).map_err(|e| e.to_string())?;
        if loaded.is_animated() || loaded.page_count > 1 {
            return Err("animations and multi-page files are left alone".to_string());
        }
        let mut bitmap = loaded.frames.into_iter().next().map(|f| f.bitmap).ok_or("no picture in the file")?;
        if !job.turn.is_identity() {
            bitmap = bitmap.turned(job.turn.turns, job.turn.flip_h, job.turn.flip_v);
        }
        if job.auto_fix {
            bitmap = edit::auto_fix(&bitmap);
        }
        let mut resized = false;
        if let Some(resize) = job.resize {
            let (w, h) = resize.target(bitmap.width, bitmap.height, job.enlarge);
            if (w, h) != (bitmap.width, bitmap.height) {
                bitmap = edit::resize(&bitmap, w, h);
                resized = true;
            }
        }
        // A picture already small enough is not recompressed just for having been looked at.
        if !resized && !converting && !job.auto_fix {
            let rest = Job { resize: None, ..job.clone() };
            return process(path, &rest);
        }
        let keep_metadata = if job.strip { None } else { Some(data.as_slice()) };
        save::encode(&bitmap, target, job.quality, keep_metadata).map_err(|e| e.to_string())?
    };

    let folder = match &job.output {
        Output::SameFolder => path.parent().ok_or("the file has no folder")?.to_path_buf(),
        Output::Folder(dir) => dir.clone(),
    };
    let name = if converting {
        Path::new(path.file_stem().ok_or("the file has no name")?).with_extension(target.extension())
    } else {
        PathBuf::from(path.file_name().ok_or("the file has no name")?)
    };
    let dest = folder.join(name);
    if dest == path {
        save::replace_file(path, &bytes).map_err(|e| e.to_string())?;
    } else {
        if dest.exists() {
            return Err(format!("{} already exists there", dest.file_name().unwrap_or_default().to_string_lossy()));
        }
        std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
        // The new file keeps the original's dates, so date sorting still makes sense.
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = save::copy_times(&meta, &dest);
        }
    }
    Ok(dest)
}

#[derive(Debug, Default)]
pub struct Report {
    /// Source file and the file its result went to.
    pub written: Vec<(PathBuf, PathBuf)>,
    pub failed: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut text = format!("{} done", self.written.len());
        if !self.failed.is_empty() {
            text.push_str(&format!(", {} skipped", self.failed.len()));
        }
        if self.cancelled {
            text.push_str(", stopped early");
        }
        text
    }
}

/// Run a job over many files, several at a time. `progress` counts finished files;
/// setting `cancel` stops before the files not yet started.
pub fn run(paths: &[PathBuf], job: &Job, progress: &AtomicUsize, cancel: &AtomicBool) -> Report {
    let results: Vec<(PathBuf, Option<Result<PathBuf, String>>)> = paths
        .par_iter()
        .map(|path| {
            if cancel.load(Ordering::Relaxed) {
                return (path.clone(), None);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(path, job)))
                .unwrap_or_else(|_| Err("this file could not be processed".to_string()));
            progress.fetch_add(1, Ordering::Relaxed);
            (path.clone(), Some(result))
        })
        .collect();
    let mut report = Report::default();
    for (path, result) in results {
        match result {
            Some(Ok(dest)) => report.written.push((path, dest)),
            Some(Err(e)) => report.failed.push((path, e)),
            None => report.cancelled = true,
        }
    }
    report
}

/// The result of applying a rename pattern to a list of files.
#[derive(Debug, Default, PartialEq)]
pub struct RenamePlan {
    /// Each file with its new name (name only, same folder).
    pub items: Vec<(PathBuf, String)>,
    /// Reasons the plan cannot be carried out. Empty means it is safe.
    pub problems: Vec<String>,
}

const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

fn date_parts(path: &Path) -> (String, String) {
    // The date the picture was taken when the file says so, otherwise the file's own.
    if let Some(taken) = metadata::read(path).fields.taken {
        if let Some((date, time)) = taken.split_once(' ') {
            return (date.to_string(), time.replace(':', "-"));
        }
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    let local: chrono::DateTime<chrono::Local> = modified.into();
    (local.format("%Y-%m-%d").to_string(), local.format("%H-%M-%S").to_string())
}

/// Work out new names from a pattern. Placeholders:
///
/// - `{name}` the current name without extension
/// - `{n}` a running number, `{n:3}` padded with zeros to three digits
/// - `{date}` and `{time}` when the picture was taken (or the file's date)
///
/// The extension always stays. Nothing is renamed here; see [`apply_rename`].
pub fn plan_rename(paths: &[PathBuf], pattern: &str, start: u32) -> RenamePlan {
    let mut plan = RenamePlan::default();
    let pattern = pattern.trim();
    if pattern.is_empty() {
        plan.problems.push("The pattern is empty.".to_string());
        return plan;
    }
    let uses_dates = pattern.contains("{date}") || pattern.contains("{time}");
    for (i, path) in paths.iter().enumerate() {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (date, time) = if uses_dates { date_parts(path) } else { Default::default() };
        let number = start as u64 + i as u64;
        let mut name = String::new();
        let mut rest = pattern;
        while let Some(open) = rest.find('{') {
            name.push_str(&rest[..open]);
            let Some(close) = rest[open..].find('}') else {
                plan.problems.push("A { has no matching }.".to_string());
                return plan;
            };
            let token = &rest[open + 1..open + close];
            match token.split_once(':') {
                None if token == "name" => name.push_str(&stem),
                None if token == "date" => name.push_str(&date),
                None if token == "time" => name.push_str(&time),
                None if token == "n" => name.push_str(&number.to_string()),
                Some(("n", width)) if width.parse::<usize>().is_ok_and(|w| (1..=12).contains(&w)) => {
                    name.push_str(&format!("{number:0width$}", width = width.parse::<usize>().unwrap_or(1)));
                }
                _ => {
                    plan.problems.push(format!("{{{token}}} is not a known placeholder."));
                    return plan;
                }
            }
            rest = &rest[open + close + 1..];
        }
        name.push_str(rest);
        if let Some(ext) = path.extension() {
            name.push('.');
            name.push_str(&ext.to_string_lossy());
        }
        plan.items.push((path.clone(), name));
    }

    if let Some((_, bad)) = plan.items.iter().find(|(_, n)| n.chars().any(|c| FORBIDDEN.contains(&c) || c.is_control())) {
        plan.problems.push(format!("\"{bad}\" contains a character that file names cannot have."));
    }
    // Two files must not end up with one name, and no bystander may be overwritten.
    let sources: std::collections::HashSet<PathBuf> = paths.iter().map(|p| lower(p)).collect();
    let mut seen = std::collections::HashSet::new();
    for (path, name) in &plan.items {
        let target = path.with_file_name(name);
        if !seen.insert(lower(&target)) {
            plan.problems.push(format!("More than one file would be called \"{name}\". Add {{n}} to the pattern."));
            break;
        }
        if target.exists() && !sources.contains(&lower(&target)) {
            plan.problems.push(format!("\"{name}\" already exists and is not one of the files being renamed."));
            break;
        }
    }
    plan
}

fn lower(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

/// Carry out a plan. Files first move to temporary names, so a plan that swaps names
/// (a becomes b while b becomes a) works. Returns old and new path of every file.
pub fn apply_rename(plan: &RenamePlan) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if let Some(problem) = plan.problems.first() {
        return Err(problem.clone());
    }
    let stamp = std::process::id();
    let mut parked: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    let undo = |parked: &[(PathBuf, PathBuf, PathBuf)]| {
        for (original, temp, _) in parked {
            let _ = std::fs::rename(temp, original);
        }
    };
    for (i, (path, name)) in plan.items.iter().enumerate() {
        let target = path.with_file_name(name);
        if target == *path {
            continue;
        }
        let temp = path.with_file_name(format!(".svrename-{stamp}-{i}.tmp"));
        if let Err(e) = std::fs::rename(path, &temp) {
            undo(&parked);
            return Err(format!("Could not rename {}: {e}", path.display()));
        }
        parked.push((path.clone(), temp, target));
    }
    let mut done = Vec::new();
    for (original, temp, target) in &parked {
        if let Err(e) = std::fs::rename(temp, target) {
            // Put this one back; the ones already renamed stay renamed.
            let _ = std::fs::rename(temp, original);
            return Err(format!("Could not rename {}: {e}", original.display()));
        }
        done.push((original.clone(), target.clone()));
    }
    Ok(done)
}
