use std::time::{Duration, SystemTime};
use sv_core::decode::{self, Bitmap, Format};
use sv_core::edit::{self, Adjust};
use sv_core::save::{self, SaveError, SaveFormat};

fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Bitmap {
    Bitmap { width: w, height: h, rgba: rgba.iter().copied().cycle().take((w * h * 4) as usize).collect() }
}

/// Each pixel's red is its x, green its y: easy to see where a pixel came from.
fn coordinates(w: u32, h: u32) -> Bitmap {
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[x as u8, y as u8, 0, 255]);
        }
    }
    Bitmap { width: w, height: h, rgba }
}

/// Left half dark, right half light, with a little pseudo-random grain on top.
fn noisy_edge(w: u32, h: u32) -> Bitmap {
    let mut seed = 12345u32;
    let mut rgba = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let grain = ((seed >> 24) % 21) as i32 - 10;
            let base = if x < w / 2 { 60 } else { 190 };
            let v = (base + grain).clamp(0, 255) as u8;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Bitmap { width: w, height: h, rgba }
}

fn spread(b: &Bitmap, x0: u32, x1: u32) -> f32 {
    let values: Vec<f32> = (0..b.height).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| b.pixel(x, y).unwrap()[0] as f32).collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
}

#[test]
fn crop_takes_the_right_pixels_and_stays_inside_the_picture() {
    let src = coordinates(20, 10);
    let c = edit::crop(&src, 5, 2, 8, 4);
    assert_eq!((c.width, c.height), (8, 4));
    assert_eq!(c.pixel(0, 0), Some([5, 2, 0, 255]));
    assert_eq!(c.pixel(7, 3), Some([12, 5, 0, 255]));

    let over = edit::crop(&src, 15, 8, 100, 100);
    assert_eq!((over.width, over.height), (5, 2));
    let outside = edit::crop(&src, 500, 500, 0, 0);
    assert_eq!((outside.width, outside.height), (1, 1));
}

#[test]
fn resize_changes_the_size_and_keeps_the_picture() {
    let src = solid(400, 200, [10, 200, 90, 255]);
    let small = edit::resize(&src, 100, 50);
    assert_eq!((small.width, small.height), (100, 50));
    assert_eq!(small.pixel(50, 25), Some([10, 200, 90, 255]));
    let big = edit::resize(&small, 300, 300);
    assert_eq!((big.width, big.height, big.rgba.len()), (300, 300, 300 * 300 * 4));

    let mut halves = solid(200, 100, [255, 0, 0, 255]);
    for y in 0..100 {
        for x in 100..200 {
            let i = (y * 200 + x) * 4;
            halves.rgba[i..i + 3].copy_from_slice(&[0, 0, 255]);
        }
    }
    let r = edit::resize(&halves, 50, 25);
    assert!(r.pixel(5, 12).unwrap()[0] > 240 && r.pixel(45, 12).unwrap()[2] > 240, "left stays red, right stays blue");
}

#[test]
fn straighten_turns_clockwise_and_leaves_no_empty_corners() {
    let white = solid(400, 300, [255, 255, 255, 255]);
    assert_eq!(edit::straighten(&white, 0.0).rgba, white.rgba);

    for degrees in [3.0f32, -7.5, 15.0, 44.0] {
        let out = edit::straighten(&white, degrees);
        assert!(out.width < 400 && out.height < 300, "{degrees}: cut down to {}x{}", out.width, out.height);
        assert!(out.rgba.iter().all(|v| *v == 255), "{degrees}: every pixel comes from inside the picture");
        let (kw, kh) = edit::straighten_keeps(400, 300, degrees);
        assert!((kw * 400.0 - out.width as f32).abs() <= 1.0 && (kh * 300.0 - out.height as f32).abs() <= 1.0);
    }

    // A picture with a white top half: after a clockwise turn the boundary slopes down
    // to the right, so the top-right corner region shows more white than the top-left.
    let mut halves = solid(400, 400, [0, 0, 0, 255]);
    halves.rgba[..400 * 200 * 4].fill(255);
    let turned = edit::straighten(&halves, 10.0);
    let (cy, right, left) = (turned.height / 2, turned.width - 10, 10);
    assert!(turned.pixel(right, cy + 15).unwrap()[0] > 200, "white reaches below centre on the right");
    assert!(turned.pixel(left, cy - 15).unwrap()[0] < 60, "black reaches above centre on the left");
}

#[test]
fn neutral_adjustment_changes_nothing() {
    let src = noisy_edge(64, 16);
    assert!(Adjust::default().is_neutral());
    assert_eq!(Adjust::default().apply(&src).rgba, src.rgba);
}

#[test]
fn each_adjustment_moves_the_picture_the_expected_way() {
    let grey = solid(8, 8, [100, 100, 100, 200]);
    let px = |a: Adjust| a.apply(&grey).pixel(0, 0).unwrap();

    assert!(px(Adjust { brightness: 40.0, ..Adjust::default() })[0] > 100);
    assert!(px(Adjust { brightness: -40.0, ..Adjust::default() })[0] < 100);
    assert!(px(Adjust { gamma: 2.0, ..Adjust::default() })[0] > 100);
    assert!(px(Adjust { gamma: 0.5, ..Adjust::default() })[0] < 100);
    // Below middle grey, more contrast means darker.
    assert!(px(Adjust { contrast: 50.0, ..Adjust::default() })[0] < 100);
    assert!(px(Adjust { contrast: -50.0, ..Adjust::default() })[0] > 100);

    let warm = px(Adjust { red: 50.0, blue: -50.0, ..Adjust::default() });
    assert!(warm[0] > 100 && warm[1] == 100 && warm[2] < 100);
    assert_eq!(warm[3], 200, "transparency is never touched");

    let colour = solid(8, 8, [200, 100, 50, 255]);
    let grey_out = Adjust { saturation: -100.0, ..Adjust::default() }.apply(&colour).pixel(0, 0).unwrap();
    assert!(grey_out[0] == grey_out[1] && grey_out[1] == grey_out[2], "no saturation is grey: {grey_out:?}");
    let vivid = Adjust { saturation: 60.0, ..Adjust::default() }.apply(&colour).pixel(0, 0).unwrap();
    assert!(vivid[0] > 200 && vivid[2] < 50);
}

#[test]
fn blur_and_sharpen_leave_flat_areas_alone_and_act_on_edges() {
    let flat = solid(40, 40, [120, 60, 30, 255]);
    assert_eq!(edit::blur(&flat, 3.0).rgba, flat.rgba);
    assert_eq!(edit::sharpen(&flat, 80.0).rgba, flat.rgba);

    let mut edge = solid(40, 10, [50, 50, 50, 255]);
    for y in 0..10 {
        for x in 20..40 {
            let i = (y * 40 + x) * 4;
            edge.rgba[i..i + 3].fill(200);
        }
    }
    let soft = edit::blur(&edge, 2.0);
    let v = soft.pixel(19, 5).unwrap()[0];
    assert!(v > 60 && v < 190, "the edge is smeared: {v}");
    assert_eq!(soft.pixel(2, 5).unwrap()[0], 50, "far from the edge nothing changes");

    let crisp = edit::sharpen(&edge, 100.0);
    assert!(crisp.pixel(19, 5).unwrap()[0] < 50, "dark side of the edge gets darker");
    assert!(crisp.pixel(20, 5).unwrap()[0] > 200, "light side gets lighter");
    assert_eq!(edit::blur(&edge, 0.0).rgba, edge.rgba);
}

#[test]
fn denoise_removes_grain_but_keeps_the_edge() {
    let src = noisy_edge(120, 60);
    let clean = edit::denoise(&src, 80.0);
    assert!(spread(&clean, 5, 50) < spread(&src, 5, 50) * 0.6, "grain is reduced on the dark side");
    assert!(spread(&clean, 70, 115) < spread(&src, 70, 115) * 0.6, "and on the light side");
    let (dark, light) = (clean.pixel(58, 30).unwrap()[0], clean.pixel(61, 30).unwrap()[0]);
    assert!(dark < 90 && light > 160, "the edge is still sharp: {dark} / {light}");
    assert_eq!(edit::denoise(&src, 0.0).rgba, src.rgba);
}

#[test]
fn auto_fix_stretches_a_flat_picture_and_leaves_a_good_one() {
    // Tones squeezed between 80 and 160.
    let mut hazy = solid(100, 10, [0, 0, 0, 255]);
    for (i, p) in hazy.rgba.chunks_exact_mut(4).enumerate() {
        let v = 80 + ((i % 100) * 80 / 99) as u8;
        p[..3].fill(v);
    }
    let fixed = edit::auto_fix(&hazy);
    let (min, max) = fixed.rgba.chunks_exact(4).fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
    assert!(min <= 5 && max >= 250, "now uses the full range: {min}..{max}");

    let mut full = solid(256, 1, [0, 0, 0, 255]);
    for (i, p) in full.rgba.chunks_exact_mut(4).enumerate() {
        p[..3].fill(i as u8);
    }
    // A picture that already spans black to white barely moves.
    let fixed = edit::auto_fix(&full);
    let worst = fixed.rgba.iter().zip(&full.rgba).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 3, "changed by up to {worst}");
    let flat = solid(10, 10, [90, 90, 90, 255]);
    assert_eq!(edit::auto_fix(&flat).rgba, flat.rgba);
}

fn detect(bytes: &[u8]) -> Format {
    Format::from_magic(bytes).unwrap()
}

#[test]
fn every_save_format_reads_back() {
    let src = coordinates(64, 48);
    for (format, expect) in [
        (SaveFormat::Jpeg, Format::Jpeg),
        (SaveFormat::Png, Format::Png),
        (SaveFormat::WebP, Format::WebP),
        (SaveFormat::Bmp, Format::Bmp),
        (SaveFormat::Tiff, Format::Tiff),
    ] {
        let bytes = save::encode(&src, format, 90, None).unwrap();
        assert_eq!(detect(&bytes), expect, "{}", format.name());
        let back = decode::load_bytes(&bytes, expect, 0).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48), "{}", format.name());
        if !format.has_quality() {
            assert_eq!(back.frames[0].bitmap.rgba, src.rgba, "{} is exact", format.name());
        }
        assert!(!format.extension().is_empty());
    }
}

#[test]
fn lower_quality_means_a_smaller_file() {
    let src = noisy_edge(256, 256);
    for format in [SaveFormat::Jpeg, SaveFormat::WebP] {
        let high = save::encode(&src, format, 95, None).unwrap().len();
        let mid = save::encode(&src, format, 60, None).unwrap().len();
        let low = save::encode(&src, format, 20, None).unwrap().len();
        assert!(high > mid && mid > low, "{}: {high} > {mid} > {low}", format.name());
    }
}

#[test]
fn jpeg_export_carries_metadata_and_flattens_transparency() {
    // A JPEG with an EXIF block saying "rotate me".
    let plain = save::encode(&solid(32, 16, [200, 0, 0, 255]), SaveFormat::Jpeg, 90, None).unwrap();
    let mut with_exif = vec![0xFF, 0xD8, 0xFF, 0xE1, 0, 34];
    with_exif.extend_from_slice(b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0");
    with_exif.extend_from_slice(&plain[2..]);
    assert_eq!(decode::load_bytes(&with_exif, Format::Jpeg, 0).unwrap().width(), 16, "the test file really is rotated");

    let edited = solid(40, 20, [0, 0, 200, 255]);
    let out = save::encode(&edited, SaveFormat::Jpeg, 90, Some(&with_exif)).unwrap();
    assert!(out.windows(6).any(|w| w == b"Exif\0\0"), "EXIF came along");
    let back = decode::load_bytes(&out, Format::Jpeg, 0).unwrap();
    assert_eq!((back.width(), back.height()), (40, 20), "and no longer rotates the saved pixels");

    let clear = solid(16, 16, [0, 0, 0, 0]);
    let jpeg = save::encode(&clear, SaveFormat::Jpeg, 90, None).unwrap();
    let p = decode::load_bytes(&jpeg, Format::Jpeg, 0).unwrap().frames[0].bitmap.pixel(8, 8).unwrap();
    assert!(p[0] > 245 && p[1] > 245 && p[2] > 245, "transparent becomes white, not black: {p:?}");
}

#[test]
fn saving_an_edit_overwrites_in_the_same_format_and_keeps_the_date() {
    let dir = tempfile::tempdir().unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_400_000_000);
    for (name, format) in [("a.jpg", SaveFormat::Jpeg), ("b.png", SaveFormat::Png), ("c.webp", SaveFormat::WebP), ("d.tif", SaveFormat::Tiff)] {
        let path = dir.path().join(name);
        std::fs::write(&path, save::encode(&solid(64, 32, [200, 0, 0, 255]), format, 90, None).unwrap()).unwrap();
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(old)).unwrap();
        let before = detect(&std::fs::read(&path).unwrap());

        save::save_edited(&path, &solid(20, 10, [0, 0, 200, 255])).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(detect(&bytes), before, "{name} keeps its format");
        let back = decode::load(&path, 0).unwrap();
        assert_eq!((back.width(), back.height()), (20, 10), "{name}");
        assert!(back.frames[0].bitmap.pixel(5, 5).unwrap()[2] > 150, "{name} has the new pixels");
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old, "{name}");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4, "no temporary files left");

    let psd = dir.path().join("x.psd");
    std::fs::write(&psd, b"8BPS\0\x01").unwrap();
    assert!(matches!(save::save_edited(&psd, &solid(2, 2, [0; 4])), Err(SaveError::ReadOnly(_))));
    assert_eq!(std::fs::read(&psd).unwrap(), b"8BPS\0\x01");
}

#[test]
fn save_as_offers_a_sensible_first_format() {
    assert_eq!(SaveFormat::for_source(Format::Raw), SaveFormat::Jpeg);
    assert_eq!(SaveFormat::for_source(Format::Heif), SaveFormat::Jpeg);
    assert_eq!(SaveFormat::for_source(Format::Psd), SaveFormat::Png);
    assert_eq!(SaveFormat::for_source(Format::Tiff), SaveFormat::Tiff);
}

#[test]
fn blurring_a_region_hides_it_and_touches_nothing_else() {
    // Fine black and white stripes: readable detail.
    let mut src = solid(120, 80, [0, 0, 0, 255]);
    for (i, p) in src.rgba.chunks_exact_mut(4).enumerate() {
        if (i % 120) % 2 == 0 {
            p[..3].fill(255);
        }
    }
    let out = edit::blur_region(&src, 40, 20, 40, 30);
    assert_eq!((out.width, out.height), (120, 80));
    let inside = out.pixel(60, 35).unwrap()[0];
    assert!(inside > 90 && inside < 170, "stripes are smeared to grey: {inside}");
    for (x, y) in [(10, 10), (39, 35), (80, 35), (60, 19), (60, 50), (119, 79)] {
        assert_eq!(out.pixel(x, y), src.pixel(x, y), "({x}, {y}) is outside the region");
    }
    // A region hanging over the edge is cut to the picture instead of failing.
    let edge = edit::blur_region(&src, 100, 60, 500, 500);
    assert_eq!(edge.rgba.len(), src.rgba.len());
}
