use std::path::Path;
use sv_core::decode::{self, Format};
use sv_core::save::{self, SaveError, Turn};
use sv_core::thumbs;

/// A minimal flat 8-bit RGB Photoshop file: 2x1, a red pixel then a blue one.
fn tiny_psd() -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"8BPS");
    f.extend_from_slice(&1u16.to_be_bytes()); // version
    f.extend_from_slice(&[0; 6]); // reserved
    f.extend_from_slice(&3u16.to_be_bytes()); // channels
    f.extend_from_slice(&1u32.to_be_bytes()); // height
    f.extend_from_slice(&2u32.to_be_bytes()); // width
    f.extend_from_slice(&8u16.to_be_bytes()); // bits per channel
    f.extend_from_slice(&3u16.to_be_bytes()); // colour mode: RGB
    f.extend_from_slice(&0u32.to_be_bytes()); // colour mode data
    f.extend_from_slice(&0u32.to_be_bytes()); // image resources
    f.extend_from_slice(&0u32.to_be_bytes()); // layers and masks
    f.extend_from_slice(&0u16.to_be_bytes()); // raw, uncompressed
    f.extend_from_slice(&[255, 0]); // red plane
    f.extend_from_slice(&[0, 0]); // green plane
    f.extend_from_slice(&[0, 255]); // blue plane
    f
}

#[test]
fn files_are_recognised_by_content_and_raw_by_name() {
    let p = |name: &str| Path::new(name).to_path_buf();
    assert_eq!(Format::detect(b"8BPS\0\x01", &p("a.psd")), Some(Format::Psd));
    assert_eq!(Format::detect(b"\0\0\0\x18ftypheic\0\0\0\0", &p("a.heic")), Some(Format::Heif));
    assert_eq!(Format::detect(b"\0\0\0\x1cftypavif\0\0\0\0", &p("a.avif")), Some(Format::Heif));
    assert_eq!(Format::detect(b"\0\0\0\x18ftypcrx \0\0\0\0", &p("a.cr3")), Some(Format::Raw));
    assert_eq!(Format::detect(b"FUJIFILMCCD-RAW 0201", &p("a.raf")), Some(Format::Raw));

    // NEF, ARW, DNG and friends are TIFF containers: only the name tells them apart.
    let tiff_header = b"II*\0\x08\0\0\0";
    for raw in Format::RAW_EXTENSIONS {
        assert_eq!(Format::detect(tiff_header, &p(&format!("shot.{raw}"))), Some(Format::Raw), "{raw}");
        assert_eq!(Format::detect(tiff_header, &p(&format!("SHOT.{}", raw.to_uppercase()))), Some(Format::Raw));
    }
    assert_eq!(Format::detect(tiff_header, &p("scan.tif")), Some(Format::Tiff));
    // A JPEG renamed to .psd is still a JPEG.
    assert_eq!(Format::detect(&[0xFF, 0xD8, 0xFF, 0xE0], &p("odd.psd")), Some(Format::Jpeg));
}

#[test]
fn psd_shows_its_flattened_picture() {
    let loaded = decode::load_bytes(&tiny_psd(), Format::Psd, 0).unwrap();
    assert_eq!((loaded.width(), loaded.height()), (2, 1));
    let b = &loaded.frames[0].bitmap;
    assert_eq!(b.pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(b.pixel(1, 0), Some([0, 0, 255, 255]));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flat.psd");
    std::fs::write(&path, tiny_psd()).unwrap();
    let thumb = thumbs::make(&path).unwrap();
    assert_eq!((thumb.width, thumb.height), (2, 1));
}

#[test]
fn damaged_files_give_an_error_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        ("broken.nef", &b"II*\0\x08\0\0\0 this is not really a raw file"[..]),
        ("empty.cr2", &b""[..]),
        ("broken.psd", &b"8BPS and then nothing sensible"[..]),
        ("broken.heic", &b"\0\0\0\x18ftypheic garbage"[..]),
        ("broken.dng", &[0u8; 64][..]),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        assert!(decode::load(&path, 0).is_err(), "{name}");
        assert!(thumbs::make(&path).is_err(), "{name}");
    }
}

#[test]
fn view_only_formats_refuse_to_be_saved() {
    let turn = Turn { turns: 1, flip_h: false, flip_v: false };
    for format in [Format::Psd, Format::Raw, Format::Heif] {
        assert!(format.is_read_only());
        assert!(matches!(save::rotated_bytes(&tiny_psd(), format, turn), Err(SaveError::ReadOnly(_))));
    }
    assert!(!Format::Jpeg.is_read_only());

    // The file on disk is left exactly as it was.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flat.psd");
    std::fs::write(&path, tiny_psd()).unwrap();
    assert!(save::save_rotated(&path, turn).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), tiny_psd());
}
