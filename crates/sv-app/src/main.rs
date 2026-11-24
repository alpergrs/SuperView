// No console window behind the viewer in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod fileclip;
mod input;
mod instance;
mod loader;
mod register;
mod thumbnails;
mod viewer;

use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;

/// "Show this file or folder", from the command line or from a second launch.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRequest {
    pub path: PathBuf,
    /// Include subfolders. Only set by `--recursive`, which is what the Explorer
    /// "Browse with SuperView" folder entry passes.
    pub recursive: bool,
    /// Set by `--slideshow`, the Explorer "Slideshow with SuperView" folder entry: open
    /// the first picture in the viewer. Subfolders and auto-play follow the settings.
    pub slideshow: bool,
}

fn parse_args(args: impl Iterator<Item = std::ffi::OsString>) -> Option<OpenRequest> {
    let mut recursive = false;
    let mut slideshow = false;
    let mut path = None;
    for arg in args {
        match arg.to_str() {
            Some("--recursive") | Some("-r") => recursive = true,
            Some("--slideshow") => slideshow = true,
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => {}
        }
    }
    let path = path?;
    let path = std::path::absolute(&path).unwrap_or(path);
    Some(OpenRequest { path, recursive, slideshow })
}

fn main() -> eframe::Result {
    // Explorer integration can also be switched from a script or an installer.
    match std::env::args().nth(1).as_deref() {
        Some("--register") => return finish(register::register()),
        Some("--unregister") => return finish(register::unregister()),
        _ => {}
    }
    let request = parse_args(std::env::args_os().skip(1));

    let (tx, rx) = mpsc::channel();
    let waker = instance::Waker::default();
    if !instance::claim(request.as_ref(), tx, waker.clone()) {
        // Another SuperView window took the request.
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SuperView")
            .with_app_id("superview")
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([320.0, 240.0])
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default())
            .with_drag_and_drop(true),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "SuperView",
        options,
        Box::new(move |cc| {
            waker.set(cc.egui_ctx.clone());
            Ok(Box::new(app::App::new(cc, request, rx)))
        }),
    )
}

fn finish(result: Result<(), String>) -> eframe::Result {
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Option<OpenRequest> {
        parse_args(list.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn recursive_flag_in_any_position() {
        assert_eq!(args(&[]), None);
        assert_eq!(args(&["--recursive"]), None);
        let a = args(&["--recursive", "/pics"]).unwrap();
        let b = args(&["/pics", "-r"]).unwrap();
        assert!(a.recursive && b.recursive);
        assert_eq!(a.path, b.path);
        assert!(!args(&["/pics/a.jpg"]).unwrap().recursive);
        let s = args(&["--slideshow", "/pics"]).unwrap();
        assert!(s.slideshow && !s.recursive);
        assert!(!a.slideshow);
    }
}
