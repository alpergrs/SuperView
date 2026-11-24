use image::{ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use std::io::Cursor;
use std::time::{Duration, SystemTime};
use sv_core::decode::{self, Bitmap, Format};
use sv_core::fileops;
use sv_core::save::{self, Outcome, SaveError, Turn};
use sv_core::thumbs::{self, CacheStore, THUMB_SIDE};

fn encode(img: &RgbImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// Left half red, right half blue.
fn two_tone(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, _| if x < w / 2 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) })
}

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

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 180 && p[2] < 80
}

fn is_blue(p: [u8; 4]) -> bool {
    p[2] > 180 && p[0] < 80
}

fn shown(bytes: &[u8], format: Format) -> Bitmap {
    decode::load_bytes(bytes, format, 0).unwrap().frames.swap_remove(0).bitmap
}

const RIGHT: Turn = Turn { turns: 1, flip_h: false, flip_v: false };

#[test]
fn identity_is_recognised_however_it_is_reached() {
    assert!(Turn::default().is_identity());
    assert!(Turn { turns: 4, flip_h: false, flip_v: false }.is_identity());
    // Two flips equal a half turn, so adding a half turn undoes them.
    assert!(Turn { turns: 2, flip_h: true, flip_v: true }.is_identity());
    assert!(!RIGHT.is_identity());
}

#[test]
fn block_aligned_jpeg_rotates_without_recompression() {
    let jpeg = encode(&two_tone(64, 32), ImageFormat::Jpeg);
    let (out, outcome) = save::rotated_bytes(&jpeg, Format::Jpeg, RIGHT).unwrap();
    assert_eq!(outcome, Outcome::Lossless);
    let b = shown(&out, Format::Jpeg);
    assert_eq!((b.width, b.height), (32, 64));
    // Clockwise: the red left half is now on top.
    assert!(is_red(b.pixel(16, 8).unwrap()));
    assert!(is_blue(b.pixel(16, 56).unwrap()));

    // Four right turns saved one after another give back the same pixels exactly.
    let mut bytes = jpeg.clone();
    for _ in 0..4 {
        bytes = save::rotated_bytes(&bytes, Format::Jpeg, RIGHT).unwrap().0;
    }
    assert_eq!(shown(&bytes, Format::Jpeg).rgba, shown(&jpeg, Format::Jpeg).rgba);
}

#[test]
fn saved_jpeg_looks_like_the_view_even_with_exif_orientation() {
    // Stored landscape, EXIF says "turn right to display": shown 32x64, red on top.
    let stored = with_orientation(&encode(&two_tone(64, 32), ImageFormat::Jpeg), 6);
    let before = shown(&stored, Format::Jpeg);
    assert_eq!((before.width, before.height), (32, 64));

    // The user turns it right once more: red ends up on the right.
    let (out, _) = save::rotated_bytes(&stored, Format::Jpeg, RIGHT).unwrap();
    let after = shown(&out, Format::Jpeg);
    assert_eq!((after.width, after.height), (64, 32));
    assert!(is_red(after.pixel(56, 16).unwrap()));
    assert!(is_blue(after.pixel(8, 16).unwrap()));

    // The EXIF block is still there and no longer asks for a rotation.
    assert!(out.windows(6).any(|w| w == b"Exif\0\0"));
    let mut copy = out.clone();
    assert!(save::reset_exif_orientation(&mut copy));
    assert_eq!(copy, out, "orientation was already reset to normal");
}

#[test]
fn odd_sized_jpeg_keeps_every_pixel_and_its_metadata() {
    // 50x30 is not a whole number of JPEG blocks.
    let stored = with_orientation(&encode(&two_tone(50, 30), ImageFormat::Jpeg), 1);
    let flip = Turn { turns: 0, flip_h: true, flip_v: false };
    let (out, outcome) = save::rotated_bytes(&stored, Format::Jpeg, flip).unwrap();
    assert_eq!(outcome, Outcome::Reencoded);
    let b = shown(&out, Format::Jpeg);
    assert_eq!((b.width, b.height), (50, 30), "no edge was trimmed");
    assert!(is_blue(b.pixel(5, 15).unwrap()));
    assert!(is_red(b.pixel(45, 15).unwrap()));
    assert!(out.windows(6).any(|w| w == b"Exif\0\0"), "EXIF survived the re-encode");
}

#[test]
fn every_turn_and_flip_matches_the_bitmap_transform() {
    let jpeg = encode(&two_tone(64, 32), ImageFormat::Jpeg);
    let original = shown(&jpeg, Format::Jpeg);
    for turns in 0..4 {
        for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)] {
            let turn = Turn { turns, flip_h, flip_v };
            let saved = shown(&save::rotated_bytes(&jpeg, Format::Jpeg, turn).unwrap().0, Format::Jpeg);
            let expected = original.turned(turns, flip_h, flip_v);
            assert_eq!((saved.width, saved.height), (expected.width, expected.height), "{turn:?}");
            assert_eq!(saved.rgba, expected.rgba, "{turn:?}");
        }
    }
}

#[test]
fn png_keeps_transparency() {
    let img = RgbaImage::from_fn(4, 2, |x, _| if x == 0 { Rgba([9, 8, 7, 0]) } else { Rgba([10, 20, 30, 255]) });
    let mut bytes = Cursor::new(Vec::new());
    img.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let (out, outcome) = save::rotated_bytes(bytes.get_ref(), Format::Png, RIGHT).unwrap();
    assert_eq!(outcome, Outcome::Reencoded);
    let b = shown(&out, Format::Png);
    assert_eq!((b.width, b.height), (2, 4));
    assert_eq!(b.pixel(0, 0).unwrap()[3], 0, "the transparent column is now the top row");
    assert_eq!(b.pixel(0, 3), Some([10, 20, 30, 255]));
}

#[test]
fn animations_are_refused() {
    use image::codecs::gif::GifEncoder;
    let mut bytes = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut bytes);
        for c in [0u8, 200] {
            let img = RgbaImage::from_pixel(3, 3, Rgba([c, 0, 0, 255]));
            enc.encode_frame(image::Frame::new(img)).unwrap();
        }
    }
    assert!(matches!(save::rotated_bytes(&bytes, Format::Gif, RIGHT), Err(SaveError::MultiFrame)));
}

#[test]
fn saving_overwrites_in_place_and_keeps_the_dates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, encode(&two_tone(64, 32), ImageFormat::Jpeg)).unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
    drop(file);

    assert_eq!(save::save_rotated(&path, RIGHT).unwrap(), Outcome::Lossless);
    let b = decode::load(&path, 0).unwrap();
    assert_eq!((b.width(), b.height()), (32, 64));
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 1, "no temporary file left behind: {names:?}");
}

#[test]
fn thumbnails_are_small_and_upright() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.jpg");
    std::fs::write(&big, encode(&two_tone(1600, 800), ImageFormat::Jpeg)).unwrap();
    let t = thumbs::make(&big).unwrap();
    assert_eq!((t.width, t.height), (THUMB_SIDE, THUMB_SIDE / 2));
    assert!(is_red(t.pixel(20, 60).unwrap()) && is_blue(t.pixel(230, 60).unwrap()));

    let turned = dir.path().join("turned.jpg");
    std::fs::write(&turned, with_orientation(&encode(&two_tone(1600, 800), ImageFormat::Jpeg), 6)).unwrap();
    let t = thumbs::make(&turned).unwrap();
    assert_eq!((t.width, t.height), (THUMB_SIDE / 2, THUMB_SIDE));
    assert!(is_red(t.pixel(60, 20).unwrap()) && is_blue(t.pixel(60, 230).unwrap()));

    let small = dir.path().join("small.png");
    std::fs::write(&small, encode(&two_tone(40, 20), ImageFormat::Png)).unwrap();
    let t = thumbs::make(&small).unwrap();
    assert_eq!((t.width, t.height), (40, 20), "small pictures are not enlarged");
}

fn cache_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(root).map(|r| r.map(|e| e.unwrap().path()).collect()).unwrap_or_default()
}

#[test]
fn folder_cache_is_one_file_per_folder_and_follows_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cache");
    let pics = dir.path().join("pics");
    std::fs::create_dir_all(&pics).unwrap();
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let names = ["a.png", "b.png", "c.png"];
    for n in names {
        std::fs::write(pics.join(n), encode(&two_tone(600, 300), ImageFormat::Png)).unwrap();
    }

    let store = CacheStore::new(root.clone());
    assert!(store.get(&pics.join("a.png"), 5, t0).is_none());
    for n in names {
        thumbs::load_or_make(Some(&store), &pics.join(n), 5, t0).unwrap();
    }
    assert_eq!(cache_files(&root).len(), 1, "three pictures, one cache file");
    assert!(store.total_size() > 0);
    assert!(cache_files(&pics).len() == 3, "nothing is written into the picture folder");

    // A fresh store (a new session) reads the same thumbnails back from disk.
    let again = CacheStore::new(root.clone());
    let cached = again.get(&pics.join("B.PNG"), 5, t0).expect("found, ignoring letter case");
    assert_eq!((cached.width, cached.height), (THUMB_SIDE, THUMB_SIDE / 2));
    assert!(is_red(cached.pixel(10, 10).unwrap()));

    // A different size or time is a different picture as far as the cache goes.
    assert!(again.get(&pics.join("b.png"), 6, t0).is_none());
    assert!(again.get(&pics.join("b.png"), 5, t0 + Duration::from_secs(1)).is_none());

    // Forgetting survives a restart too, and leaves the others alone.
    again.forget(&pics.join("b.png"));
    let third = CacheStore::new(root.clone());
    assert!(third.get(&pics.join("b.png"), 5, t0).is_none());
    assert!(third.get(&pics.join("c.png"), 5, t0).is_some());
}

#[test]
fn without_a_store_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let pic = dir.path().join("pic.png");
    std::fs::write(&pic, encode(&two_tone(600, 300), ImageFormat::Png)).unwrap();
    let t = thumbs::load_or_make(None, &pic, 1, SystemTime::UNIX_EPOCH).unwrap();
    assert_eq!(t.width, THUMB_SIDE);
    assert_eq!(cache_files(dir.path()).len(), 1, "only the picture itself");
}

#[test]
fn a_damaged_tail_is_cut_off_and_the_rest_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cache");
    let pics = dir.path().join("pics");
    std::fs::create_dir_all(&pics).unwrap();
    let t0 = SystemTime::UNIX_EPOCH;
    for n in ["a.png", "b.png"] {
        std::fs::write(pics.join(n), encode(&two_tone(600, 300), ImageFormat::Png)).unwrap();
        thumbs::load_or_make(Some(&CacheStore::new(root.clone())), &pics.join(n), 1, t0).unwrap();
    }
    let file = cache_files(&root).remove(0);
    let len = std::fs::metadata(&file).unwrap().len();
    // Simulate a crash in the middle of writing the second thumbnail.
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(len - 100).unwrap();

    let store = CacheStore::new(root.clone());
    assert!(store.get(&pics.join("a.png"), 1, t0).is_some());
    assert!(store.get(&pics.join("b.png"), 1, t0).is_none());
    thumbs::load_or_make(Some(&store), &pics.join("b.png"), 1, t0).unwrap();
    assert!(CacheStore::new(root).get(&pics.join("b.png"), 1, t0).is_some());
}

#[test]
fn small_and_vanished_folders_lose_their_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cache");
    let store = CacheStore::new(root.clone());
    let t0 = SystemTime::UNIX_EPOCH;
    let mut sizes = Vec::new();
    for (name, side) in [("small", 40u32), ("big", 400), ("gone", 40)] {
        let folder = dir.path().join(name);
        std::fs::create_dir_all(&folder).unwrap();
        let pic = folder.join("p.bmp");
        std::fs::write(&pic, encode(&two_tone(side, side), ImageFormat::Bmp)).unwrap();
        thumbs::load_or_make(Some(&store), &pic, 1, t0).unwrap();
        sizes.push(thumbs::picture_bytes(&folder));
    }
    assert_eq!(cache_files(&root).len(), 3);
    assert!(sizes[0] < 100_000 && sizes[1] > 100_000);
    std::fs::remove_dir_all(dir.path().join("gone")).unwrap();

    // Start-up tidy only removes the cache of the folder that no longer exists.
    store.tidy(u64::MAX);
    assert_eq!(cache_files(&root).len(), 2);

    assert_eq!(store.remove_small(100_000), 1);
    assert_eq!(cache_files(&root).len(), 1);
    assert!(store.get(&dir.path().join("big").join("p.bmp"), 1, t0).is_some());

    store.clear();
    assert_eq!(store.total_size(), 0);
    assert!(store.get(&dir.path().join("big").join("p.bmp"), 1, t0).is_none());
}

#[test]
fn transfer_copies_moves_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let (src, dest) = (dir.path().join("src"), dir.path().join("dest"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    let files: Vec<_> = ["a.jpg", "b.jpg", "c.jpg"].iter().map(|n| src.join(n)).collect();
    for f in &files {
        std::fs::write(f, f.file_name().unwrap().to_string_lossy().as_bytes()).unwrap();
    }
    std::fs::write(dest.join("b.jpg"), b"already here").unwrap();

    let copied = fileops::transfer(&files, &dest, false);
    assert_eq!((copied.done.len(), copied.skipped.len(), copied.failed.len()), (2, 1, 0));
    assert_eq!(copied.summary(false), "Copied 2, skipped 1 already there");
    assert_eq!(std::fs::read(dest.join("b.jpg")).unwrap(), b"already here");
    assert!(files.iter().all(|f| f.exists()), "copy leaves the originals");

    let other = dir.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let moved = fileops::transfer(&files, &other, true);
    assert_eq!(moved.done.len(), 3);
    assert!(files.iter().all(|f| !f.exists()));
    assert_eq!(std::fs::read(other.join("c.jpg")).unwrap(), b"c.jpg");
}
