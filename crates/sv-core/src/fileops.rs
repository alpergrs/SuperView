//! Operations that change files on disk.

use std::io;
use std::path::{Path, PathBuf};

const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Rename a file inside its folder. `new_name` is a bare file name; when it has no
/// extension the old one is kept. Never overwrites an existing file.
pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidInput, msg.to_string());
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name == "." || new_name == ".." {
        return Err(invalid("The name is empty."));
    }
    if new_name.chars().any(|c| FORBIDDEN.contains(&c) || c.is_control()) {
        return Err(invalid("A file name cannot contain < > : \" / \\ | ? *"));
    }
    let mut target = path.with_file_name(new_name);
    if target.extension().is_none() {
        if let Some(ext) = path.extension() {
            target.set_extension(ext);
        }
    }
    if target == path {
        return Ok(target);
    }
    // A change of letter case only is the same file on Windows; allow it.
    let same_file = target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
    if target.exists() && !same_file {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "A file with that name already exists.",
        ));
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// Move a file to the Recycle Bin.
pub fn recycle(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// What happened to a batch of files sent to another folder.
#[derive(Debug, Default, PartialEq)]
pub struct TransferReport {
    /// Source paths that were copied or moved.
    pub done: Vec<PathBuf>,
    /// Left alone because the destination already has a file of that name.
    pub skipped: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

impl TransferReport {
    pub fn summary(&self, moved: bool) -> String {
        let mut text = format!("{} {}", if moved { "Moved" } else { "Copied" }, self.done.len());
        if !self.skipped.is_empty() {
            text.push_str(&format!(", skipped {} already there", self.skipped.len()));
        }
        if !self.failed.is_empty() {
            text.push_str(&format!(", {} failed", self.failed.len()));
        }
        text
    }
}

/// Copy or move files into `dest`. Never overwrites: a name that already exists in
/// `dest` is skipped.
pub fn transfer(paths: &[PathBuf], dest: &Path, move_files: bool) -> TransferReport {
    let mut report = TransferReport::default();
    for path in paths {
        let Some(name) = path.file_name() else { continue };
        let target = dest.join(name);
        if target.exists() {
            report.skipped.push(path.clone());
            continue;
        }
        let result = if move_files {
            // A plain rename fails across drives; fall back to copy + delete.
            std::fs::rename(path, &target).or_else(|_| {
                std::fs::copy(path, &target).and_then(|_| std::fs::remove_file(path))
            })
        } else {
            std::fs::copy(path, &target).map(|_| ())
        };
        match result {
            Ok(()) => report.done.push(path.clone()),
            Err(e) => report.failed.push((path.clone(), e.to_string())),
        }
    }
    report
}

/// A name for `target` that is free: itself, or "name (2).ext", "name (3).ext", ...
fn free_name(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| target.with_file_name(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("some number is free")
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Paste files and folders into `dest`, as copies or (for cut items) by moving them.
/// Nothing is ever overwritten: a name already taken gets " (2)" added, as Explorer does.
/// Moving something into the folder it is already in does nothing, and a folder cannot
/// be pasted into itself.
pub fn paste(paths: &[PathBuf], dest: &Path, move_items: bool) -> TransferReport {
    let mut report = TransferReport::default();
    for path in paths {
        let Some(name) = path.file_name() else { continue };
        if !path.exists() {
            report.failed.push((path.clone(), "it no longer exists".to_string()));
            continue;
        }
        if path.is_dir() && dest.starts_with(path) {
            report.failed.push((path.clone(), "a folder cannot be put inside itself".to_string()));
            continue;
        }
        if move_items && path.parent() == Some(dest) {
            report.skipped.push(path.clone());
            continue;
        }
        let target = free_name(&dest.join(name));
        let result = if move_items {
            // A plain rename fails across drives; fall back to copy, then remove.
            std::fs::rename(path, &target).or_else(|_| {
                copy_tree(path, &target)?;
                if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                }
            })
        } else {
            copy_tree(path, &target)
        };
        match result {
            Ok(()) => report.done.push(path.clone()),
            Err(e) => report.failed.push((path.clone(), e.to_string())),
        }
    }
    report
}
