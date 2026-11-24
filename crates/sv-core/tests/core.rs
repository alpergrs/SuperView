use image::{codecs::gif::GifEncoder, Delay, Frame, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use std::io::Cursor;
use std::time::{Duration, SystemTime};
use sv_core::decode::{self, Format};
use sv_core::folder::{self, natural_cmp, Entry, SortKey};
use sv_core::settings::{FitMode, Settings};
use sv_core::{fileops, folder::list};

/// 4x2 picture: left half red, right half blue.
fn sample() -> RgbImage {
    RgbImage::from_fn(4, 2, |x, _| if x < 2 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) })
}

fn encode(img: &RgbImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn close(a: [u8; 4], b: [u8; 3]) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() < 40)
}

#[test]
fn decodes_every_basic_format() {
    for (fmt, ours) in [
        (ImageFormat::Png, Format::Png),
        (ImageFormat::Bmp, Format::Bmp),
        (ImageFormat::Tiff, Format::Tiff),
        (ImageFormat::WebP, Format::WebP),
        (ImageFormat::Gif, Format::Gif),
    ] {
        let bytes = encode(&sample(), fmt);
        assert_eq!(Format::from_magic(&bytes), Some(ours), "{fmt:?}");
        let loaded = decode::load_bytes(&bytes, ours, 0).unwrap();
        assert_eq!((loaded.width(), loaded.height()), (4, 2), "{fmt:?}");
        assert!(!loaded.is_animated());
        let b = &loaded.frames[0].bitmap;
        assert!(close(b.pixel(0, 0).unwrap(), [255, 0, 0]), "{fmt:?}");
        assert!(close(b.pixel(3, 1).unwrap(), [0, 0, 255]), "{fmt:?}");
        assert_eq!(b.pixel(4, 0), None);
    }
}

/// Insert an EXIF APP1 segment carrying only the orientation tag after the SOI marker.
fn with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&tiff);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[test]
fn jpeg_decodes_and_follows_exif_orientation() {
    // Big flat blocks so JPEG compression keeps the colours recognisable.
    let img = RgbImage::from_fn(32, 16, |x, _| if x < 16 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) });
    let plain = encode(&img, ImageFormat::Jpeg);
    assert_eq!(Format::from_magic(&plain), Some(Format::Jpeg));

    let loaded = decode::load_bytes(&plain, Format::Jpeg, 0).unwrap();
    assert_eq!((loaded.width(), loaded.height()), (32, 16));
    assert!(close(loaded.frames[0].bitmap.pixel(2, 8).unwrap(), [255, 0, 0]));

    // Orientation 6: the stored picture must be turned 90 degrees clockwise, so the
    // red left half ends up on top.
    let turned = decode::load_bytes(&with_orientation(&plain, 6), Format::Jpeg, 0).unwrap();
    assert_eq!((turned.width(), turned.height()), (16, 32));
    let b = &turned.frames[0].bitmap;
    assert!(close(b.pixel(8, 2).unwrap(), [255, 0, 0]));
    assert!(close(b.pixel(8, 29).unwrap(), [0, 0, 255]));
}

#[test]
fn animated_gif_keeps_frames_and_delays() {
    let mut bytes = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut bytes);
        for (i, ms) in [(0u8, 100u32), (1, 250), (2, 0)] {
            let img = RgbaImage::from_pixel(3, 3, Rgba([i * 100, 0, 0, 255]));
            enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(ms, 1))).unwrap();
        }
    }
    let loaded = decode::load_bytes(&bytes, Format::Gif, 0).unwrap();
    assert!(loaded.is_animated());
    assert_eq!(loaded.frames.len(), 3);
    let delays: Vec<u32> = loaded.frames.iter().map(|f| f.delay_ms).collect();
    assert_eq!(delays, vec![100, 250, 100], "zero delay is raised to 100 ms");
}

#[test]
fn multi_page_tiff_pages_are_addressable() {
    use tiff::encoder::{colortype, TiffEncoder};
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut bytes).unwrap();
        enc.write_image::<colortype::RGB8>(2, 1, &[255, 0, 0, 0, 255, 0]).unwrap();
        enc.write_image::<colortype::Gray8>(3, 2, &[0, 128, 255, 10, 20, 30]).unwrap();
        enc.write_image::<colortype::RGBA8>(1, 1, &[1, 2, 3, 4]).unwrap();
    }
    let bytes = bytes.into_inner();

    let p0 = decode::load_bytes(&bytes, Format::Tiff, 0).unwrap();
    assert_eq!((p0.page, p0.page_count), (0, 3));
    assert_eq!(p0.frames[0].bitmap.pixel(1, 0), Some([0, 255, 0, 255]));

    let p1 = decode::load_bytes(&bytes, Format::Tiff, 1).unwrap();
    assert_eq!((p1.width(), p1.height(), p1.page), (3, 2, 1));
    assert_eq!(p1.frames[0].bitmap.pixel(1, 0), Some([128, 128, 128, 255]));

    let p2 = decode::load_bytes(&bytes, Format::Tiff, 2).unwrap();
    assert_eq!(p2.frames[0].bitmap.pixel(0, 0), Some([1, 2, 3, 4]));

    // Asking past the end gives the last page instead of failing.
    assert_eq!(decode::load_bytes(&bytes, Format::Tiff, 99).unwrap().page, 2);
}

#[test]
fn rejects_garbage() {
    assert!(decode::load_bytes(b"not an image at all", Format::Jpeg, 0).is_err());
    assert!(decode::load_bytes(b"", Format::Png, 0).is_err());
    assert_eq!(Format::from_magic(b"hello"), None);
}

#[test]
fn wrong_extension_is_detected_by_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("really-a-png.jpg");
    std::fs::write(&path, encode(&sample(), ImageFormat::Png)).unwrap();
    assert_eq!(decode::load(&path, 0).unwrap().format, Format::Png);
}

#[test]
fn natural_order_matches_explorer() {
    let mut names = vec!["img10.jpg", "IMG2.jpg", "img1.jpg", "a.jpg", "img02.jpg", "img2a.jpg"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["a.jpg", "img1.jpg", "IMG2.jpg", "img02.jpg", "img2a.jpg", "img10.jpg"]);
}

#[test]
fn sorting_by_each_key() {
    let t = |s| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
    let e = |p: &str, size, secs| Entry { path: p.into(), size, modified: t(secs), hidden: false };
    let mut v = vec![e("b.png", 30, 1), e("a.jpg", 10, 3), e("c.gif", 20, 2)];
    let names = |v: &[Entry]| v.iter().map(|e| e.path.to_str().unwrap().to_string()).collect::<Vec<_>>();

    folder::sort(&mut v, SortKey::Name, false);
    assert_eq!(names(&v), ["a.jpg", "b.png", "c.gif"]);
    folder::sort(&mut v, SortKey::Name, true);
    assert_eq!(names(&v), ["c.gif", "b.png", "a.jpg"]);
    folder::sort(&mut v, SortKey::Size, false);
    assert_eq!(names(&v), ["a.jpg", "c.gif", "b.png"]);
    folder::sort(&mut v, SortKey::Date, false);
    assert_eq!(names(&v), ["b.png", "c.gif", "a.jpg"]);
    folder::sort(&mut v, SortKey::Type, false);
    assert_eq!(names(&v), ["c.gif", "a.jpg", "b.png"]);
}

#[test]
fn listing_recurses_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    for name in ["one.jpg", "two.PNG", "notes.txt", "sub/three.webp"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    assert_eq!(list(dir.path(), false).len(), 2);
    assert_eq!(list(dir.path(), true).len(), 3);
}

#[test]
fn rename_keeps_extension_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jpg");
    let b = dir.path().join("b.jpg");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();

    assert!(fileops::rename(&a, "b").is_err(), "must not overwrite b.jpg");
    assert!(fileops::rename(&a, "bad:name").is_err());
    assert!(fileops::rename(&a, "  ").is_err());
    assert!(a.exists());

    let renamed = fileops::rename(&a, "holiday").unwrap();
    assert_eq!(renamed, dir.path().join("holiday.jpg"));
    assert!(renamed.exists() && !a.exists());
    assert_eq!(fileops::rename(&renamed, "x.png").unwrap(), dir.path().join("x.png"));
    assert_eq!(std::fs::read(&b).unwrap(), b"b");
}

#[test]
fn settings_survive_a_round_trip_and_bad_input() {
    let s = Settings {
        fit_mode: FitMode::Fill,
        sort_key: SortKey::Date,
        slideshow_seconds: 7.5,
        ..Settings::default()
    };
    let text = toml::to_string_pretty(&s).unwrap();
    assert_eq!(Settings::from_toml(&text), s);

    assert_eq!(Settings::from_toml("this is {{ not toml"), Settings::default());
    // Unknown keys and missing keys are tolerated; silly values are clamped.
    let partial = Settings::from_toml("slideshow_seconds = 0.0\nfuture_option = 1\n");
    assert_eq!(partial.slideshow_seconds, 0.5);
    assert_eq!(partial.fit_mode, FitMode::ShrinkToFit);
}

#[test]
fn subfolders_come_after_their_parents_pictures_in_tree_order() {
    let t = SystemTime::UNIX_EPOCH;
    let e = |p: &str, size| Entry { path: p.into(), size, modified: t, hidden: false };
    let mut v = vec![
        e("A/A3/z.jpg", 1),
        e("A/A1/A1B/k.jpg", 1),
        e("A/A10/q.jpg", 1),
        e("A/zebra.jpg", 5),
        e("A/A2/m.jpg", 1),
        e("A/A1/b.jpg", 9),
        e("A/A1/A1A/x.jpg", 1),
        e("A/apple.jpg", 7),
        e("A/A1/a.jpg", 3),
        e("A/A1/A1C/c.jpg", 1),
    ];
    let names = |v: &[Entry]| v.iter().map(|e| e.path.to_str().unwrap().replace('\\', "/")).collect::<Vec<_>>();

    folder::sort(&mut v, SortKey::Name, false);
    assert_eq!(
        names(&v),
        [
            "A/apple.jpg",
            "A/zebra.jpg",
            "A/A1/a.jpg",
            "A/A1/b.jpg",
            "A/A1/A1A/x.jpg",
            "A/A1/A1B/k.jpg",
            "A/A1/A1C/c.jpg",
            "A/A2/m.jpg",
            "A/A3/z.jpg",
            "A/A10/q.jpg",
        ]
    );

    // Another sort key reorders pictures inside each folder, never the folders.
    folder::sort(&mut v, SortKey::Size, true);
    let got = names(&v);
    assert_eq!(&got[..4], ["A/apple.jpg", "A/zebra.jpg", "A/A1/b.jpg", "A/A1/a.jpg"]);
    assert_eq!(&got[4..], ["A/A1/A1A/x.jpg", "A/A1/A1B/k.jpg", "A/A1/A1C/c.jpg", "A/A2/m.jpg", "A/A3/z.jpg", "A/A10/q.jpg"]);
}

#[test]
fn hidden_pictures_and_hidden_folders_are_flagged() {
    // A plainly named folder: the default temporary name starts with a dot, which
    // would itself count as hidden.
    let dir = tempfile::Builder::new().prefix("sv-test").tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".secret/deeper")).unwrap();
    std::fs::create_dir_all(dir.path().join("plain")).unwrap();
    for name in ["a.jpg", ".hidden.jpg", ".secret/b.jpg", ".secret/deeper/c.jpg", "plain/d.jpg"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    let flag = |name: &str, entries: &[Entry]| entries.iter().find(|e| e.path.ends_with(name)).map(|e| e.hidden);

    let flat = list(dir.path(), false);
    assert_eq!(flat.len(), 2);
    assert_eq!(flag("a.jpg", &flat), Some(false));
    assert_eq!(flag(".hidden.jpg", &flat), Some(true));

    let deep = list(dir.path(), true);
    assert_eq!(deep.len(), 5);
    assert_eq!(flag("b.jpg", &deep), Some(true), "inside a hidden folder");
    assert_eq!(flag("c.jpg", &deep), Some(true), "inside a folder inside a hidden folder");
    assert_eq!(flag("d.jpg", &deep), Some(false));

    assert!(folder::is_hidden_path(&dir.path().join(".secret")));
    assert!(!folder::is_hidden_path(&dir.path().join("plain")));

    // Anywhere below a hidden folder counts, however deep; a plain path does not.
    assert!(folder::is_inside_hidden(&dir.path().join(".secret/deeper/c.jpg")));
    assert!(folder::is_inside_hidden(&dir.path().join(".secret/deeper")));
    assert!(folder::is_inside_hidden(&dir.path().join(".hidden.jpg")));
    // The temporary folder itself may sit below a hidden one (on Windows it lives in
    // AppData), so a plain path is only as hidden as the folder the test runs in.
    let surroundings = folder::is_inside_hidden(dir.path());
    assert_eq!(folder::is_inside_hidden(&dir.path().join("plain/d.jpg")), surroundings);
    assert!(!folder::is_inside_hidden(std::path::Path::new("/")), "a root is never 'hidden'");
}
