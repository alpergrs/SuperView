use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};
use sv_core::decode::raw::Linear;
use sv_core::decode::Bitmap;
use sv_core::dupes;
use sv_core::edit::{self, RawParams};
use sv_core::folder::{self, Entry, Query};
use sv_core::save::{self, SaveFormat};

fn dir() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("sv-test").tempdir().unwrap()
}

fn entry(path: &str, size: u64, secs: u64) -> Entry {
    Entry { path: path.into(), size, modified: SystemTime::UNIX_EPOCH + Duration::from_secs(secs), hidden: false }
}

#[test]
fn search_matches_words_sizes_and_dates() {
    let beach = entry("/photos/2023 Summer/Beach_Sunset.JPG", 3_000_000, 1_690_000_000);
    let scan = entry("/photos/scans/invoice-0042.png", 200_000, 1_600_000_000);
    let q = |text: &str| Query { text: text.into(), ..Query::default() };

    assert!(Query::default().is_empty() && Query::default().matches(&beach));
    assert!(!q("x").is_empty());
    assert!(q("beach").matches(&beach) && !q("beach").matches(&scan));
    assert!(q("SUNSET beach").matches(&beach), "any order, any letter case");
    assert!(q("summer sunset").matches(&beach), "folder names count too");
    assert!(!q("beach winter").matches(&beach), "every word must be there");
    assert!(q(".png").matches(&scan) && q("0042").matches(&scan));

    let big = Query { min_bytes: Some(1_000_000), ..Query::default() };
    let small = Query { max_bytes: Some(1_000_000), ..Query::default() };
    assert!(big.matches(&beach) && !big.matches(&scan));
    assert!(small.matches(&scan) && !small.matches(&beach));

    let t = |secs| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
    let recent = Query { from: t(1_650_000_000), ..Query::default() };
    let old = Query { until: t(1_650_000_000), ..Query::default() };
    assert!(recent.matches(&beach) && !recent.matches(&scan));
    assert!(old.matches(&scan) && !old.matches(&beach));
    let both = Query { text: "beach".into(), min_bytes: Some(1), from: t(1), until: t(2_000_000_000), ..Query::default() };
    assert!(both.matches(&beach) && !both.matches(&scan));
}

#[test]
fn days_parse_as_whole_days_months_and_years() {
    let day = folder::parse_day("2024-05-31", false).unwrap();
    let day_end = folder::parse_day("2024-05-31", true).unwrap();
    assert_eq!(day_end.duration_since(day).unwrap(), Duration::from_secs(86_400));
    let month = folder::parse_day("2024-02", false).unwrap();
    let month_end = folder::parse_day("2024-02", true).unwrap();
    assert_eq!(month_end.duration_since(month).unwrap(), Duration::from_secs(29 * 86_400), "leap February");
    let year_end = folder::parse_day("2023", true).unwrap();
    assert_eq!(year_end, folder::parse_day("2024", false).unwrap());
    assert_eq!(folder::parse_day("2024.12.31", true), folder::parse_day("2025-01-01", false));
    for bad in ["", "soon", "2024-13", "2024-02-30", "24-01-01", "2024-01-01-01"] {
        assert_eq!(folder::parse_day(bad, false), None, "{bad}");
    }
}

/// A picture with a recognisable layout: a bright diagonal band on a dark gradient.
fn scene(w: u32, h: u32, shift: u32) -> Bitmap {
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let band = ((fx + fy * 0.7 + shift as f32 * 0.13) * 3.0).sin() * 0.5 + 0.5;
            let v = (band * 200.0 + fx * 40.0) as u8;
            rgba.extend_from_slice(&[v, (v as f32 * 0.8) as u8, 255 - v, 255]);
        }
    }
    Bitmap { width: w, height: h, rgba }
}

fn write(path: &Path, bitmap: &Bitmap, format: SaveFormat, quality: u8) -> Entry {
    std::fs::write(path, save::encode(bitmap, format, quality, None).unwrap()).unwrap();
    let meta = std::fs::metadata(path).unwrap();
    Entry { path: path.to_path_buf(), size: meta.len(), modified: meta.modified().unwrap(), hidden: false }
}

fn names(groups: &[Vec<PathBuf>]) -> Vec<Vec<String>> {
    groups.iter().map(|g| g.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()).collect()
}

#[test]
fn exact_duplicates_are_grouped_and_lookalikes_of_equal_size_are_not() {
    let d = dir();
    let a = write(&d.path().join("a.png"), &scene(64, 48, 0), SaveFormat::Png, 0);
    let copy = d.path().join("copy of a.png");
    std::fs::copy(&a.path, &copy).unwrap();
    let copy = Entry { path: copy, ..a.clone() };
    let other = write(&d.path().join("other.png"), &scene(64, 48, 3), SaveFormat::Png, 0);
    // Same length as a.png but different bytes: must not be called a duplicate.
    let mut bytes = std::fs::read(&a.path).unwrap();
    let last = bytes.len() - 20;
    bytes[last] ^= 0xFF;
    let twin = d.path().join("same size.bin.png");
    std::fs::write(&twin, &bytes).unwrap();
    let twin = Entry { path: twin, ..a.clone() };

    let progress = AtomicUsize::new(0);
    let groups = dupes::find_exact(&[a.clone(), other, twin, copy], &progress, &AtomicBool::new(false));
    assert_eq!(names(&groups), [["a.png", "copy of a.png"]]);
    assert_eq!(progress.load(Ordering::Relaxed), 4);

    assert!(dupes::find_exact(std::slice::from_ref(&a), &AtomicUsize::new(0), &AtomicBool::new(false)).is_empty());
    let stopped = dupes::find_exact(&[a.clone(), Entry { path: d.path().join("copy of a.png"), ..a }], &AtomicUsize::new(0), &AtomicBool::new(true));
    assert!(stopped.is_empty(), "a cancelled search reports nothing rather than guesses");
}

#[test]
fn lookalikes_survive_resizing_and_recompression_but_different_pictures_do_not_match() {
    let d = dir();
    let original = scene(800, 600, 0);
    let big = write(&d.path().join("original.png"), &original, SaveFormat::Png, 0);
    let small = write(&d.path().join("small.jpg"), &edit::resize(&original, 200, 150), SaveFormat::Jpeg, 60);
    let webp = write(&d.path().join("converted.webp"), &original, SaveFormat::WebP, 70);
    let different = write(&d.path().join("different.png"), &scene(800, 600, 9), SaveFormat::Png, 0);
    // Same content but another shape is not offered as a duplicate.
    let cropped = write(&d.path().join("tall.png"), &edit::crop(&original, 0, 0, 300, 600), SaveFormat::Png, 0);

    assert!((dupes::look(&original) ^ dupes::look(&edit::resize(&original, 200, 150))).count_ones() <= 4);
    assert!((dupes::look(&original) ^ dupes::look(&scene(800, 600, 9))).count_ones() > 12);

    // The same layout in other colours is a different picture.
    let mut tinted = scene(800, 600, 0);
    for p in tinted.rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    assert!((dupes::look(&original) ^ dupes::look(&tinted)).count_ones() <= 16, "the light-and-dark fingerprint alone is fooled");
    let recoloured = write(&d.path().join("recoloured.png"), &tinted, SaveFormat::Png, 0);

    let all = [different, small, big, cropped, webp, recoloured];
    let groups = dupes::find_similar(&all, 6, &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
    assert_eq!(groups.len(), 1, "{:?}", names(&groups));
    let mut found = names(&groups).remove(0);
    assert_eq!(found[0], "original.png", "the largest file comes first, as the one to keep");
    found.sort();
    assert_eq!(found, ["converted.webp", "original.png", "small.jpg"]);
}

fn grey_ramp() -> Linear {
    // 256 pixels from black to full scale, neutral grey.
    let mut rgb = Vec::new();
    for i in 0..256u32 {
        let v = (i * 257) as u16;
        rgb.extend_from_slice(&[v, v, v]);
    }
    Linear { width: 256, height: 1, rgb }
}

#[test]
fn raw_develop_applies_the_tone_curve_and_each_slider() {
    let ramp = grey_ramp();
    let plain = edit::develop(&ramp, &RawParams::default());
    assert_eq!((plain.width, plain.height), (256, 1));
    let at = |b: &Bitmap, x: u32| b.pixel(x, 0).unwrap();
    assert_eq!(at(&plain, 0), [0, 0, 0, 255]);
    assert_eq!(at(&plain, 255), [255, 255, 255, 255]);
    // Linear 18% grey lands near sRGB 118: the tone curve is applied.
    let mid = at(&plain, 46)[0];
    assert!((112..=124).contains(&mid), "{mid}");
    assert!(plain.rgba.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]), "neutral stays neutral");

    let brighter = edit::develop(&ramp, &RawParams { exposure: 1.0, ..RawParams::default() });
    let darker = edit::develop(&ramp, &RawParams { exposure: -1.0, ..RawParams::default() });
    assert!(at(&brighter, 46)[0] > mid && at(&darker, 46)[0] < mid);
    // One stop up makes linear 0.18 look like linear 0.36 did.
    assert!((at(&brighter, 46)[0] as i32 - at(&plain, 92)[0] as i32).abs() <= 2);

    let warm = at(&edit::develop(&ramp, &RawParams { temperature: 60.0, ..RawParams::default() }), 100);
    assert!(warm[0] > warm[1] && warm[1] > warm[2], "warmer is more red, less blue: {warm:?}");
    let cool = at(&edit::develop(&ramp, &RawParams { temperature: -60.0, ..RawParams::default() }), 100);
    assert!(cool[2] > cool[1] && cool[1] > cool[0]);
    let magenta = at(&edit::develop(&ramp, &RawParams { tint: 60.0, ..RawParams::default() }), 100);
    assert!(magenta[1] < magenta[0] && magenta[0] == magenta[2]);

    // Pushed two stops, the upper half burns out; highlight recovery keeps it apart.
    let pushed = RawParams { exposure: 2.0, ..RawParams::default() };
    let burnt = edit::develop(&ramp, &pushed);
    let saved = edit::develop(&ramp, &RawParams { highlights: 100.0, ..pushed });
    assert_eq!(at(&burnt, 80)[0], 255);
    assert_eq!(at(&burnt, 120)[0], 255);
    assert!(at(&saved, 80)[0] < at(&saved, 120)[0], "detail is back between tones that had both gone white");
    assert!(at(&saved, 20)[0] > 0);
    // The brightest recorded value still reaches white.
    assert_eq!(at(&saved, 255)[0], 255);
}

#[test]
fn raw_preview_copy_is_smaller_and_keeps_the_layout() {
    let mut rgb = Vec::new();
    for _ in 0..300 {
        for x in 0..400u32 {
            rgb.extend_from_slice(if x < 200 { &[60000u16, 0, 0] } else { &[0u16, 0, 60000] });
        }
    }
    let full = Linear { width: 400, height: 300, rgb };
    let small = full.downscaled(100);
    assert_eq!((small.width, small.height), (100, 75));
    assert_eq!(&small.rgb[..3], &[60000, 0, 0]);
    assert_eq!(&small.rgb[small.rgb.len() - 3..], &[0, 0, 60000]);
    let same = full.downscaled(1000);
    assert_eq!((same.width, same.height), (400, 300));
}

#[test]
fn pasting_copies_and_moves_files_and_folders_without_overwriting() {
    use sv_core::fileops;
    let d = dir();
    let (src, dest) = (d.path().join("src"), d.path().join("dest"));
    std::fs::create_dir_all(src.join("album/inner")).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(src.join("a.jpg"), b"A").unwrap();
    std::fs::write(src.join("album/b.jpg"), b"B").unwrap();
    std::fs::write(src.join("album/inner/c.jpg"), b"C").unwrap();
    std::fs::write(dest.join("a.jpg"), b"already here").unwrap();
    let list = |p: &Path| {
        let mut v: Vec<String> = std::fs::read_dir(p).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };

    // Copy a file and a whole folder; the name clash gets a number instead of an overwrite.
    let report = fileops::paste(&[src.join("a.jpg"), src.join("album")], &dest, false);
    assert_eq!((report.done.len(), report.failed.len()), (2, 0), "{:?}", report.failed);
    assert_eq!(list(&dest), ["a (2).jpg", "a.jpg", "album"]);
    assert_eq!(std::fs::read(dest.join("a.jpg")).unwrap(), b"already here");
    assert_eq!(std::fs::read(dest.join("a (2).jpg")).unwrap(), b"A");
    assert_eq!(std::fs::read(dest.join("album/inner/c.jpg")).unwrap(), b"C");
    assert!(src.join("a.jpg").exists() && src.join("album/b.jpg").exists(), "copying leaves the originals");

    // Pasting into the same folder makes a copy next to the original.
    fileops::paste(&[src.join("a.jpg")], &src, false);
    fileops::paste(&[src.join("a.jpg")], &src, false);
    assert_eq!(list(&src), ["a (2).jpg", "a (3).jpg", "a.jpg", "album"]);

    // Cut and paste moves; into its own folder it does nothing.
    let same = fileops::paste(&[src.join("a (3).jpg")], &src, true);
    assert_eq!((same.done.len(), same.skipped.len()), (0, 1));
    let moved = fileops::paste(&[src.join("a (3).jpg"), src.join("album")], &d.path().join("dest/album"), true);
    assert_eq!(moved.done.len(), 2, "{:?}", moved.failed);
    assert_eq!(list(&src), ["a (2).jpg", "a.jpg"]);
    assert_eq!(list(&dest.join("album")), ["a (3).jpg", "album", "b.jpg", "inner"]);

    // A folder cannot go inside itself, and a vanished source is reported.
    let silly = fileops::paste(&[dest.join("album")], &dest.join("album/inner"), false);
    assert!(silly.failed[0].1.contains("inside itself"));
    let gone = fileops::paste(&[src.join("nope.jpg")], &dest, false);
    assert!(gone.failed[0].1.contains("no longer exists"));
}
