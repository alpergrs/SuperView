//! Telling Windows about SuperView: "Open with" for pictures and the right-click
//! entries in Explorer. Everything goes under the current user's registry hive, so no
//! administrator rights are needed and nothing outside this user's account changes.

/// File extensions SuperView offers itself for.
pub const EXTENSIONS: [&str; 36] = [
    "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "bmp", "dib", "webp", "tif", "tiff", "heic", "heif", "hif", "avif",
    "psd", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf", "pef", "srw", "erf", "kdc",
    "dcr", "mos", "mrw",
];

#[cfg(windows)]
mod imp {
    use super::EXTENSIONS;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const PROG_ID: &str = "SuperView.Image";

    fn classes() -> std::io::Result<RegKey> {
        Ok(RegKey::predef(HKEY_CURRENT_USER).create_subkey("Software\\Classes")?.0)
    }

    fn set(classes: &RegKey, key: &str, name: &str, value: &str) -> std::io::Result<()> {
        classes.create_subkey(key)?.0.set_value(name, &value.to_string())
    }

    pub fn register() -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let exe = exe.display().to_string();
        let open = format!("\"{exe}\" \"%1\"");
        let icon = format!("\"{exe}\",0");
        let c = classes()?;

        // The file type Windows shows in "Open with" and Default apps.
        set(&c, PROG_ID, "", "Picture")?;
        set(&c, &format!("{PROG_ID}\\DefaultIcon"), "", &icon)?;
        set(&c, &format!("{PROG_ID}\\shell\\open\\command"), "", &open)?;
        set(&c, "Applications\\superview.exe", "FriendlyAppName", "SuperView")?;
        set(&c, "Applications\\superview.exe\\shell\\open\\command", "", &open)?;
        for ext in EXTENSIONS {
            set(&c, &format!(".{ext}\\OpenWithProgids"), PROG_ID, "")?;
            set(&c, "Applications\\superview.exe\\SupportedTypes", &format!(".{ext}"), "")?;
        }

        // Right-click a picture. (Windows counts RAW, HEIC and PSD files as pictures too
        // once it knows a program for them; "Open with" always works.)
        set(&c, "SystemFileAssociations\\image\\shell\\SuperView", "", "Open with SuperView")?;
        set(&c, "SystemFileAssociations\\image\\shell\\SuperView", "Icon", &icon)?;
        set(&c, "SystemFileAssociations\\image\\shell\\SuperView\\command", "", &open)?;

        // Right-click a folder, or the empty space inside one: show its pictures from the
        // first one on. Whether subfolders are included is a setting read at start-up.
        for (key, arg) in [("Directory\\shell\\SuperView", "%1"), ("Directory\\Background\\shell\\SuperView", "%V")] {
            set(&c, key, "", "Slideshow with SuperView")?;
            set(&c, key, "Icon", &icon)?;
            set(&c, &format!("{key}\\command"), "", &format!("\"{exe}\" --slideshow \"{arg}\""))?;
        }
        Ok(())
    }

    pub fn unregister() -> std::io::Result<()> {
        let c = classes()?;
        for key in [
            PROG_ID,
            "Applications\\superview.exe",
            "SystemFileAssociations\\image\\shell\\SuperView",
            "Directory\\shell\\SuperView",
            "Directory\\Background\\shell\\SuperView",
        ] {
            match c.delete_subkey_all(key) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        for ext in EXTENSIONS {
            if let Ok(key) = c.open_subkey_with_flags(format!(".{ext}\\OpenWithProgids"), winreg::enums::KEY_SET_VALUE) {
                let _ = key.delete_value(PROG_ID);
            }
        }
        Ok(())
    }
}

pub const SUPPORTED: bool = cfg!(windows);

#[cfg(windows)]
pub fn register() -> Result<(), String> {
    imp::register().map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn unregister() -> Result<(), String> {
    imp::unregister().map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn register() -> Result<(), String> {
    Err("Only available on Windows.".to_string())
}

#[cfg(not(windows))]
pub fn unregister() -> Result<(), String> {
    Err("Only available on Windows.".to_string())
}

#[cfg(test)]
mod tests {
    use super::EXTENSIONS;
    use std::path::Path;
    use sv_core::decode::{self, Format};

    #[test]
    fn every_registered_extension_can_be_opened_and_none_is_missing() {
        for ext in EXTENSIONS {
            assert!(decode::is_supported(Path::new(&format!("x.{ext}"))), "{ext}");
        }
        for raw in Format::RAW_EXTENSIONS {
            assert!(EXTENSIONS.contains(&raw), "{raw}");
        }
        let mut sorted = EXTENSIONS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), EXTENSIONS.len(), "no duplicates");
    }
}
