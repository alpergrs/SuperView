//! Files on the Windows clipboard, the way Explorer puts them there: a list of paths
//! plus a note saying whether they were copied or cut. So a file copied in SuperView can
//! be pasted in Explorer and the other way round.

use std::path::PathBuf;

#[cfg(windows)]
mod imp {
    use clipboard_win::{raw, Clipboard};
    use std::path::PathBuf;

    /// Explorer's own values for "Preferred DropEffect".
    const COPY: u32 = 1;
    const MOVE: u32 = 2;

    fn effect_format() -> Option<u32> {
        raw::register_format("Preferred DropEffect").map(|f| f.get())
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> *mut core::ffi::c_void;
    }

    /// Open the clipboard as SuperView's own window. Opened without naming a window,
    /// emptying the clipboard leaves it ownerless and Windows refuses the data that follows.
    fn open_as_window() -> Option<Clipboard> {
        // SAFETY: a plain query with no arguments; a null result is handled.
        let window = unsafe { GetActiveWindow() };
        if window.is_null() {
            Clipboard::new_attempts(10).ok()
        } else {
            Clipboard::new_attempts_for(window as _, 10).ok()
        }
    }

    pub fn set(paths: &[PathBuf], cut: bool) -> bool {
        let Some(_open) = open_as_window() else { return false };
        // Emptying is what makes this window the clipboard's owner. The file list call
        // below does not empty by itself, and data added by a non-owner is not offered
        // to other programs such as Explorer.
        if raw::empty().is_err() {
            return false;
        }
        let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        if raw::set_file_list(&list).is_err() {
            return false;
        }
        if let Some(format) = effect_format() {
            let effect = if cut { MOVE } else { COPY };
            let _ = raw::set_without_clear(format, &effect.to_le_bytes());
        }
        true
    }

    pub fn get() -> Option<(Vec<PathBuf>, bool)> {
        let _open = Clipboard::new_attempts(10).ok()?;
        let mut paths = Vec::new();
        raw::get_file_list_path(&mut paths).ok()?;
        if paths.is_empty() {
            return None;
        }
        let mut effect = [0u8; 4];
        let cut = effect_format().is_some_and(|f| raw::get(f, &mut effect).is_ok() && u32::from_le_bytes(effect) & MOVE != 0);
        Some((paths, cut))
    }

    pub fn clear() {
        if let Some(_open) = open_as_window() {
            let _ = raw::empty();
        }
    }
}

/// Put files on the system clipboard. Returns false where that is not possible (the
/// caller then keeps its own copy of the list).
#[cfg(windows)]
pub fn set(paths: &[PathBuf], cut: bool) -> bool {
    imp::set(paths, cut)
}

/// The files on the system clipboard and whether they were cut.
#[cfg(windows)]
pub fn get() -> Option<(Vec<PathBuf>, bool)> {
    imp::get()
}

/// Empty the clipboard, after cut files have been moved.
#[cfg(windows)]
pub fn clear() {
    imp::clear();
}

#[cfg(not(windows))]
pub fn set(_paths: &[PathBuf], _cut: bool) -> bool {
    false
}

#[cfg(not(windows))]
pub fn get() -> Option<(Vec<PathBuf>, bool)> {
    None
}

#[cfg(not(windows))]
pub fn clear() {}
