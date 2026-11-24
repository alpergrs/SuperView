use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};
use sv_core::batch::{self, Job, Output, Resize};
use sv_core::decode::{self, Bitmap, Format};
use sv_core::metadata::{self, Fields};
use sv_core::save::{self, SaveFormat, Turn};

fn two_tone(w: u32, h: u32) -> Bitmap {
    let mut rgba = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(if x < w / 2 { &[220, 30, 30, 255] } else { &[30, 30, 220, 255] });
        }
    }
    Bitmap { width: w, height: h, rgba }
}

fn write_picture(path: &Path, format: SaveFormat, w: u32, h: u32) {
    std::fs::write(path, save::encode(&two_tone(w, h), format, 90, None).unwrap()).unwrap();
}

fn dir() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("sv-test").tempdir().unwrap()
}

fn names(folder: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn size_of(path: &Path) -> (u32, u32) {
    let l = decode::load(path, 0).unwrap();
    (l.width(), l.height())
}

#[test]
fn dates_convert_both_ways_and_bad_ones_are_refused() {
    assert_eq!(metadata::to_exif_date("2024-05-31 14:30:05").as_deref(), Some("2024:05:31 14:30:05"));
    assert_eq!(metadata::to_exif_date(" 2024-05-31T14:30 ").as_deref(), Some("2024:05:31 14:30:00"));
    assert_eq!(metadata::to_exif_date("2024-05-31").as_deref(), Some("2024:05:31 00:00:00"));
    assert_eq!(metadata::from_exif_date("2024:05:31 14:30:05").as_deref(), Some("2024-05-31 14:30:05"));
    for bad in ["", "yesterday", "2024-13-01 10:00:00", "2024-05-31 25:00:00", "31-05-2024 10:00"] {
        assert_eq!(metadata::to_exif_date(bad), None, "{bad}");
    }
}

#[test]
fn metadata_is_written_read_back_and_leaves_the_picture_alone() {
    let d = dir();
    for (name, format) in [("a.jpg", SaveFormat::Jpeg), ("b.png", SaveFormat::Png), ("c.tif", SaveFormat::Tiff)] {
        let path = d.path().join(name);
        write_picture(&path, format, 64, 32);
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_300_000_000);
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
        let before = decode::load(&path, 0).unwrap().frames.remove(0).bitmap.rgba;
        assert!(metadata::read(&path).fields.is_empty(), "{name} starts without information");

        let changes = Fields {
            description: Some("Harbour at dusk".into()),
            artist: Some("Alper".into()),
            copyright: Some("(c) 2024".into()),
            taken: Some("2024-05-31 18:45:10".into()),
        };
        metadata::write(&path, &changes).unwrap();
        assert_eq!(metadata::read(&path).fields, changes, "{name}");
        assert_eq!(decode::load(&path, 0).unwrap().frames.remove(0).bitmap.rgba, before, "{name}: pixels untouched");
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old, "{name}: file date kept");

        // Change one field, clear another, leave the rest.
        metadata::write(&path, &Fields { artist: Some("Someone Else".into()), copyright: Some(String::new()), ..Fields::default() }).unwrap();
        let now = metadata::read(&path).fields;
        assert_eq!(now.artist.as_deref(), Some("Someone Else"), "{name}");
        assert_eq!(now.copyright, None, "{name}: an empty value removes the field");
        assert_eq!(now.description.as_deref(), Some("Harbour at dusk"), "{name}: untouched field stays");
        assert_eq!(now.taken.as_deref(), Some("2024-05-31 18:45:10"), "{name}");
    }
    assert_eq!(names(d.path()).len(), 3, "no temporary files left");
}

#[test]
fn bad_dates_and_read_only_formats_are_rejected_without_touching_the_file() {
    let d = dir();
    let jpg = d.path().join("a.jpg");
    write_picture(&jpg, SaveFormat::Jpeg, 32, 16);
    let before = std::fs::read(&jpg).unwrap();
    assert!(metadata::write(&jpg, &Fields { taken: Some("not a date".into()), ..Fields::default() }).is_err());
    assert_eq!(std::fs::read(&jpg).unwrap(), before);
    assert!(metadata::write(&jpg, &Fields::default()).is_ok(), "no changes is fine and does nothing");
    assert_eq!(std::fs::read(&jpg).unwrap(), before);

    assert!(metadata::can_write(Format::Jpeg) && metadata::can_write(Format::Png));
    assert!(!metadata::can_write(Format::Raw) && !metadata::can_write(Format::Psd) && !metadata::can_write(Format::Gif));
}

#[test]
fn stripping_removes_information_and_keeps_the_picture_upright() {
    let d = dir();
    let path = d.path().join("a.jpg");
    write_picture(&path, SaveFormat::Jpeg, 64, 32);
    metadata::write(&path, &Fields { artist: Some("Alper".into()), taken: Some("2024-01-02 03:04:05".into()), ..Fields::default() }).unwrap();
    let before = decode::load(&path, 0).unwrap().frames.remove(0).bitmap.rgba;

    metadata::strip(&path).unwrap();
    assert!(metadata::read(&path).fields.is_empty());
    assert!(!std::fs::read(&path).unwrap().windows(4).any(|w| w == b"Exif"));
    assert_eq!(decode::load(&path, 0).unwrap().frames.remove(0).bitmap.rgba, before, "same pixels, no recompression");

    // A JPEG that relies on its EXIF orientation: stored 64x32, shown 32x64.
    let plain = save::encode(&two_tone(64, 32), SaveFormat::Jpeg, 90, None).unwrap();
    let mut turned = vec![0xFF, 0xD8, 0xFF, 0xE1, 0, 34];
    turned.extend_from_slice(b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0");
    turned.extend_from_slice(&plain[2..]);
    let shown = decode::load_bytes(&turned, Format::Jpeg, 0).unwrap();
    assert_eq!((shown.width(), shown.height()), (32, 64));
    let stripped = metadata::stripped_bytes(&turned, Format::Jpeg).unwrap();
    let after = decode::load_bytes(&stripped, Format::Jpeg, 0).unwrap();
    assert_eq!((after.width(), after.height()), (32, 64), "still upright without the EXIF note");
    assert_eq!(after.frames[0].bitmap.rgba, shown.frames[0].bitmap.rgba);
}

#[test]
fn resize_rules() {
    assert_eq!(Resize::LongSide(1000).target(4000, 3000, false), (1000, 750));
    assert_eq!(Resize::LongSide(1000).target(3000, 4000, false), (750, 1000));
    assert_eq!(Resize::Width(800).target(4000, 3000, false), (800, 600));
    assert_eq!(Resize::Height(600).target(4000, 3000, false), (800, 600));
    assert_eq!(Resize::Percent(25.0).target(4000, 3000, false), (1000, 750));
    // Small pictures are left alone unless enlarging is asked for.
    assert_eq!(Resize::LongSide(1000).target(400, 300, false), (400, 300));
    assert_eq!(Resize::LongSide(1000).target(400, 300, true), (1000, 750));
    assert_eq!(Resize::Percent(200.0).target(400, 300, false), (400, 300));
}

#[test]
fn batch_convert_and_resize_into_another_folder() {
    let d = dir();
    let (src, out) = (d.path().join("src"), d.path().join("out"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    let files: Vec<PathBuf> = ["one.png", "two.bmp", "three.jpg"].iter().map(|n| src.join(n)).collect();
    write_picture(&files[0], SaveFormat::Png, 400, 200);
    write_picture(&files[1], SaveFormat::Bmp, 200, 400);
    write_picture(&files[2], SaveFormat::Jpeg, 100, 50);
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_200_000_000);
    std::fs::OpenOptions::new().write(true).open(&files[0]).unwrap().set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();

    let job = Job { convert: Some(SaveFormat::Jpeg), quality: 80, resize: Some(Resize::LongSide(100)), output: Output::Folder(out.clone()), ..Job::default() };
    let (progress, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
    let report = batch::run(&files, &job, &progress, &cancel);
    assert_eq!((report.written.len(), report.failed.len()), (3, 0), "{:?}", report.failed);
    assert_eq!(progress.load(Ordering::Relaxed), 3);
    assert_eq!(names(&out), ["one.jpg", "three.jpg", "two.jpg"]);
    assert_eq!(size_of(&out.join("one.jpg")), (100, 50));
    assert_eq!(size_of(&out.join("two.jpg")), (50, 100));
    assert_eq!(size_of(&out.join("three.jpg")), (100, 50));
    assert_eq!(std::fs::metadata(out.join("one.jpg")).unwrap().modified().unwrap(), old, "the copy keeps the original's date");
    assert_eq!(names(&src), ["one.png", "three.jpg", "two.bmp"], "originals untouched");
    assert_eq!(size_of(&files[0]), (400, 200));

    // Running it again never overwrites what is already there.
    let again = batch::run(&files, &job, &AtomicUsize::new(0), &cancel);
    assert_eq!((again.written.len(), again.failed.len()), (0, 3));
    assert!(again.failed[0].1.contains("already exists"));
    assert_eq!(again.summary(), "0 done, 3 skipped");
}

#[test]
fn batch_in_place_keeps_format_and_dates_and_rotates_jpeg_losslessly() {
    let d = dir();
    let jpg = d.path().join("a.jpg");
    let png = d.path().join("b.png");
    write_picture(&jpg, SaveFormat::Jpeg, 64, 32);
    write_picture(&png, SaveFormat::Png, 64, 32);
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_100_000_000);
    for p in [&jpg, &png] {
        std::fs::OpenOptions::new().write(true).open(p).unwrap().set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
    }
    let original = decode::load(&jpg, 0).unwrap().frames.remove(0).bitmap;

    let right = Job { turn: Turn { turns: 1, flip_h: false, flip_v: false }, ..Job::default() };
    let report = batch::run(&[jpg.clone(), png.clone()], &right, &AtomicUsize::new(0), &AtomicBool::new(false));
    assert_eq!(report.failed.len(), 0, "{:?}", report.failed);
    assert_eq!(names(d.path()), ["a.jpg", "b.png"]);
    for p in [&jpg, &png] {
        assert_eq!(size_of(p), (32, 64));
        assert_eq!(std::fs::metadata(p).unwrap().modified().unwrap(), old);
    }
    // Three more right turns bring the JPEG back to exactly the pixels it started with.
    for _ in 0..3 {
        batch::process(&jpg, &right).unwrap();
    }
    assert_eq!(decode::load(&jpg, 0).unwrap().frames.remove(0).bitmap.rgba, original.rgba);
}

#[test]
fn batch_reports_what_it_cannot_do_and_can_be_cancelled() {
    let d = dir();
    let good = d.path().join("good.png");
    let broken = d.path().join("broken.jpg");
    let psd = d.path().join("layers.psd");
    write_picture(&good, SaveFormat::Png, 40, 20);
    std::fs::write(&broken, b"\xFF\xD8\xFF not a jpeg").unwrap();
    std::fs::write(&psd, b"8BPS\0\x01").unwrap();

    assert!(!Job::default().does_something());
    assert!(batch::process(&good, &Job::default()).unwrap_err().contains("nothing to do"));

    let job = Job { resize: Some(Resize::Percent(50.0)), ..Job::default() };
    let report = batch::run(&[good.clone(), broken.clone(), psd.clone()], &job, &AtomicUsize::new(0), &AtomicBool::new(false));
    assert_eq!(report.written.len(), 1);
    assert_eq!(report.failed.len(), 2);
    assert!(report.failed.iter().any(|(p, why)| p == &psd && why.contains("need a format")));
    assert_eq!(size_of(&good), (20, 10));
    assert_eq!(std::fs::read(&broken).unwrap(), b"\xFF\xD8\xFF not a jpeg", "a file that failed is left as it was");

    // A resize that would not change the size leaves the file byte for byte as it was.
    let before = std::fs::read(&good).unwrap();
    let no_change = Job { resize: Some(Resize::LongSide(5000)), ..Job::default() };
    assert!(batch::process(&good, &no_change).unwrap_err().contains("nothing to do"));
    assert_eq!(std::fs::read(&good).unwrap(), before);

    let stopped = batch::run(std::slice::from_ref(&good), &job, &AtomicUsize::new(0), &AtomicBool::new(true));
    assert!(stopped.cancelled && stopped.written.is_empty());
    assert_eq!(size_of(&good), (20, 10), "nothing ran after the stop");
}

#[test]
fn batch_strip_removes_information_from_every_file() {
    let d = dir();
    let jpg = d.path().join("a.jpg");
    write_picture(&jpg, SaveFormat::Jpeg, 64, 32);
    metadata::write(&jpg, &Fields { artist: Some("Alper".into()), ..Fields::default() }).unwrap();
    let pixels = decode::load(&jpg, 0).unwrap().frames.remove(0).bitmap.rgba;

    batch::process(&jpg, &Job { strip: true, ..Job::default() }).unwrap();
    assert!(metadata::read(&jpg).fields.is_empty());
    assert_eq!(decode::load(&jpg, 0).unwrap().frames.remove(0).bitmap.rgba, pixels);

    // Converting normally carries the information along; with strip it does not.
    metadata::write(&jpg, &Fields { artist: Some("Alper".into()), ..Fields::default() }).unwrap();
    let out = d.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let resize = Job { resize: Some(Resize::Percent(50.0)), output: Output::Folder(out.clone()), ..Job::default() };
    batch::process(&jpg, &resize).unwrap();
    assert_eq!(metadata::read(&out.join("a.jpg")).fields.artist.as_deref(), Some("Alper"));
    std::fs::remove_file(out.join("a.jpg")).unwrap();
    batch::process(&jpg, &Job { strip: true, ..resize }).unwrap();
    assert!(metadata::read(&out.join("a.jpg")).fields.is_empty());
}

fn touch(folder: &Path, list: &[&str]) -> Vec<PathBuf> {
    list.iter()
        .map(|n| {
            let p = folder.join(n);
            std::fs::write(&p, n.as_bytes()).unwrap();
            p
        })
        .collect()
}

#[test]
fn rename_patterns() {
    let d = dir();
    let files = touch(d.path(), &["IMG_0042.JPG", "holiday.png", "x.gif"]);
    let new = |pattern: &str, start: u32| -> Vec<String> { batch::plan_rename(&files, pattern, start).items.into_iter().map(|(_, n)| n).collect() };

    assert_eq!(new("trip_{n:3}", 1), ["trip_001.JPG", "trip_002.png", "trip_003.gif"]);
    assert_eq!(new("{name}-web", 1), ["IMG_0042-web.JPG", "holiday-web.png", "x-web.gif"]);
    assert_eq!(new("{n}_{name}", 9), ["9_IMG_0042.JPG", "10_holiday.png", "11_x.gif"]);

    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 * 365 * 40);
    std::fs::OpenOptions::new().write(true).open(&files[2]).unwrap().set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
    let dated = new("{date}_{n}", 1);
    assert!(dated[2].starts_with("2009-12-") || dated[2].starts_with("2010-01-"), "{}", dated[2]);
    assert!(dated[2].ends_with("_3.gif"));

    for (pattern, expect) in [
        ("", "empty"),
        ("{nope}_{n}", "not a known placeholder"),
        ("{n", "no matching"),
        ("a/b{n}", "cannot have"),
    ] {
        let plan = batch::plan_rename(&files, pattern, 1);
        assert!(plan.problems.iter().any(|p| p.contains(expect)), "{pattern}: {:?}", plan.problems);
        assert!(batch::apply_rename(&plan).is_err(), "{pattern}");
    }
    assert_eq!(names(d.path()), ["IMG_0042.JPG", "holiday.png", "x.gif"], "refused plans rename nothing");

    // Two files of the same type cannot share a name; different types can.
    let twins = touch(d.path(), &["p.jpg", "q.jpg"]);
    let plan = batch::plan_rename(&twins, "same", 1);
    assert!(plan.problems.iter().any(|p| p.contains("More than one file")), "{:?}", plan.problems);
    assert!(batch::plan_rename(&files, "same", 1).problems.is_empty());
}

#[test]
fn rename_never_overwrites_a_bystander_but_can_swap_names() {
    let d = dir();
    let files = touch(d.path(), &["a.jpg", "b.jpg"]);
    touch(d.path(), &["pic_1.jpg"]);

    let clash = batch::plan_rename(&files, "pic_{n}", 1);
    assert!(clash.problems.iter().any(|p| p.contains("already exists")), "{:?}", clash.problems);
    assert!(batch::apply_rename(&clash).is_err());
    assert_eq!(std::fs::read(d.path().join("pic_1.jpg")).unwrap(), b"pic_1.jpg");

    let ok = batch::plan_rename(&files, "pic_{n}", 5);
    assert!(ok.problems.is_empty());
    let done = batch::apply_rename(&ok).unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(names(d.path()), ["pic_1.jpg", "pic_5.jpg", "pic_6.jpg"]);
    assert_eq!(std::fs::read(d.path().join("pic_5.jpg")).unwrap(), b"a.jpg");

    // pic_5 -> pic_6 and pic_6 -> pic_7 overlap: the old pic_6 must not be lost.
    let chain = [d.path().join("pic_5.jpg"), d.path().join("pic_6.jpg")];
    batch::apply_rename(&batch::plan_rename(&chain, "pic_{n}", 6)).unwrap();
    assert_eq!(names(d.path()), ["pic_1.jpg", "pic_6.jpg", "pic_7.jpg"]);
    assert_eq!(std::fs::read(d.path().join("pic_6.jpg")).unwrap(), b"a.jpg");
    assert_eq!(std::fs::read(d.path().join("pic_7.jpg")).unwrap(), b"b.jpg");
}
