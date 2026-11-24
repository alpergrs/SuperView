//! Writing a rotated or mirrored picture back over its file (Ctrl+S).
//!
//! JPEGs are transformed without recompression whenever libjpeg can do it exactly.
//! The file keeps its creation and modification dates.

use crate::decode::{self, jpeg, Bitmap, Format};
use image::metadata::Orientation;
use std::io::Cursor;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Decode(#[from] decode::DecodeError),
    #[error("Animations and multi-page files cannot be saved rotated.")]
    MultiFrame,
    #[error("{0} files can be viewed but not saved.")]
    ReadOnly(&'static str),
    #[error("{0}")]
    Encode(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The JPEG data was rearranged, not recompressed.
    Lossless,
    /// The picture was encoded again (JPEG at quality 95, other formats lossless).
    Reencoded,
}

/// A view change: quarter turns clockwise, then mirroring as seen on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Turn {
    pub turns: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// 2x2 integer matrix acting on image coordinates (x right, y down).
type Matrix = [[i8; 2]; 2];

const IDENTITY: Matrix = [[1, 0], [0, 1]];
const ROT90: Matrix = [[0, -1], [1, 0]];
const ROT180: Matrix = [[-1, 0], [0, -1]];
const ROT270: Matrix = [[0, 1], [-1, 0]];
const HFLIP: Matrix = [[-1, 0], [0, 1]];
const VFLIP: Matrix = [[1, 0], [0, -1]];
const TRANSPOSE: Matrix = [[0, 1], [1, 0]];
const TRANSVERSE: Matrix = [[0, -1], [-1, 0]];

/// `a` after `b`.
fn mul(a: Matrix, b: Matrix) -> Matrix {
    let mut m = [[0; 2]; 2];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j];
        }
    }
    m
}

impl Turn {
    pub fn is_identity(self) -> bool {
        self.matrix() == IDENTITY
    }

    fn matrix(self) -> Matrix {
        let mut m = match self.turns % 4 {
            1 => ROT90,
            2 => ROT180,
            3 => ROT270,
            _ => IDENTITY,
        };
        if self.flip_h {
            m = mul(HFLIP, m);
        }
        if self.flip_v {
            m = mul(VFLIP, m);
        }
        m
    }
}

fn orientation_matrix(o: Orientation) -> Matrix {
    match o {
        Orientation::NoTransforms => IDENTITY,
        Orientation::FlipHorizontal => HFLIP,
        Orientation::Rotate180 => ROT180,
        Orientation::FlipVertical => VFLIP,
        Orientation::Rotate90FlipH => TRANSPOSE,
        Orientation::Rotate90 => ROT90,
        Orientation::Rotate270FlipH => TRANSVERSE,
        Orientation::Rotate270 => ROT270,
    }
}

fn jpeg_op(m: Matrix) -> turbojpeg::TransformOp {
    use turbojpeg::TransformOp as Op;
    match m {
        ROT90 => Op::Rot90,
        ROT180 => Op::Rot180,
        ROT270 => Op::Rot270,
        HFLIP => Op::Hflip,
        VFLIP => Op::Vflip,
        TRANSPOSE => Op::Transpose,
        TRANSVERSE => Op::Transverse,
        _ => Op::None,
    }
}

/// Set the EXIF orientation of a JPEG to "normal" in place. Returns whether a tag was
/// found. The pixels now carry the orientation, so the tag must no longer turn them.
pub fn reset_exif_orientation(jpeg: &mut [u8]) -> bool {
    let mut pos = 2;
    while pos + 4 <= jpeg.len() && jpeg[pos] == 0xFF {
        let marker = jpeg[pos + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]) as usize;
        let (start, end) = (pos + 4, pos + 2 + len);
        if len < 2 || end > jpeg.len() {
            break;
        }
        if marker == 0xE1 && jpeg[start..end].starts_with(b"Exif\0\0") {
            return patch_tiff_orientation(&mut jpeg[start + 6..end]);
        }
        pos = end;
    }
    false
}

fn patch_tiff_orientation(tiff: &mut [u8]) -> bool {
    if tiff.len() < 8 {
        return false;
    }
    let little = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return false,
    };
    let u16_at = |t: &[u8], i: usize| {
        let b = [t[i], t[i + 1]];
        if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }
    };
    let b4 = [tiff[4], tiff[5], tiff[6], tiff[7]];
    let ifd = if little { u32::from_le_bytes(b4) } else { u32::from_be_bytes(b4) } as usize;
    if ifd + 2 > tiff.len() {
        return false;
    }
    let count = u16_at(tiff, ifd) as usize;
    for i in 0..count {
        let entry = ifd + 2 + i * 12;
        if entry + 12 > tiff.len() {
            break;
        }
        if u16_at(tiff, entry) == 0x0112 {
            let value = if little { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            tiff[entry + 8..entry + 10].copy_from_slice(&value);
            return true;
        }
    }
    false
}

/// A JPEG without its descriptive metadata: EXIF (with GPS), XMP, IPTC and comments go.
/// The colour profile and the markers a decoder needs stay, so the picture looks the same.
pub fn strip_jpeg_metadata(jpeg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(jpeg.len());
    out.extend_from_slice(&jpeg[..2.min(jpeg.len())]);
    let mut pos = 2;
    while pos + 4 <= jpeg.len() && jpeg[pos] == 0xFF {
        let marker = jpeg[pos + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]) as usize;
        let end = pos + 2 + len;
        if len < 2 || end > jpeg.len() {
            break;
        }
        // Keep JFIF (APP0), ICC profile (APP2) and Adobe colour info (APP14).
        let descriptive = matches!(marker, 0xE1 | 0xE3..=0xED | 0xEF | 0xFE);
        if !descriptive {
            out.extend_from_slice(&jpeg[pos..end]);
        }
        pos = end;
    }
    out.extend_from_slice(&jpeg[pos.min(jpeg.len())..]);
    out
}

/// The metadata segments (EXIF, ICC profile, IPTC, comments) of a JPEG, as raw bytes
/// ready to be inserted into another JPEG.
fn jpeg_metadata(jpeg: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 2;
    while pos + 4 <= jpeg.len() && jpeg[pos] == 0xFF {
        let marker = jpeg[pos + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]) as usize;
        let end = pos + 2 + len;
        if len < 2 || end > jpeg.len() {
            break;
        }
        // APP1..APP15 and COM. APP0 (JFIF) is written fresh by the encoder.
        if (0xE1..=0xEF).contains(&marker) || marker == 0xFE {
            out.extend_from_slice(&jpeg[pos..end]);
        }
        pos = end;
    }
    out
}

/// Insert metadata segments after the SOI marker and any JFIF header.
fn insert_metadata(jpeg: &[u8], metadata: &[u8]) -> Vec<u8> {
    let mut at = 2;
    if jpeg.len() > 6 && jpeg[2] == 0xFF && jpeg[3] == 0xE0 {
        at = 4 + u16::from_be_bytes([jpeg[4], jpeg[5]]) as usize;
    }
    let at = at.min(jpeg.len());
    let mut out = Vec::with_capacity(jpeg.len() + metadata.len());
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(metadata);
    out.extend_from_slice(&jpeg[at..]);
    out
}

fn rotate_jpeg(data: &[u8], turn: Turn) -> Result<(Vec<u8>, Outcome), SaveError> {
    let encode_err = |e: turbojpeg::Error| SaveError::Encode(e.to_string());
    // What the viewer showed is the stored pixels, turned by EXIF, then by the user.
    let total = mul(turn.matrix(), orientation_matrix(jpeg::exif_orientation(data)));

    let mut transform = turbojpeg::Transform::default();
    transform.op = jpeg_op(total);
    transform.perfect = true;
    if let Ok(out) = turbojpeg::transform(&transform, data) {
        let mut out = out.to_vec();
        reset_exif_orientation(&mut out);
        return Ok((out, Outcome::Lossless));
    }

    // The size is not a whole number of JPEG blocks, so an exact transform would have
    // to cut off an edge. Re-encode instead and keep every pixel and all metadata.
    let loaded = decode::load_bytes(data, Format::Jpeg, 0)?;
    let bitmap = loaded.frames[0].bitmap.turned(turn.turns, turn.flip_h, turn.flip_v);
    let subsamp = turbojpeg::read_header(data).map(|h| h.subsamp).unwrap_or(turbojpeg::Subsamp::Sub2x2);
    let image = turbojpeg::Image {
        pixels: bitmap.rgba.as_slice(),
        width: bitmap.width as usize,
        pitch: bitmap.width as usize * 4,
        height: bitmap.height as usize,
        format: turbojpeg::PixelFormat::RGBA,
    };
    let encoded = turbojpeg::compress(image, 95, subsamp).map_err(encode_err)?;
    let mut out = insert_metadata(&encoded, &jpeg_metadata(data));
    reset_exif_orientation(&mut out);
    Ok((out, Outcome::Reencoded))
}

pub(crate) fn encode_other(bitmap: &Bitmap, format: Format) -> Result<Vec<u8>, SaveError> {
    let img = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba.clone())
        .ok_or_else(|| SaveError::Encode("bad pixel buffer".into()))?;
    let opaque = bitmap.rgba.chunks_exact(4).all(|p| p[3] == 255);
    let mut dynimg = image::DynamicImage::ImageRgba8(img);
    // BMP has no reliable alpha; drop the channel when it carries nothing anyway.
    if opaque || format == Format::Bmp {
        dynimg = image::DynamicImage::ImageRgb8(dynimg.into_rgb8());
    }
    let mut out = Cursor::new(Vec::new());
    dynimg
        .write_to(&mut out, decode::generic::image_format(format))
        .map_err(|e| SaveError::Encode(e.to_string()))?;
    Ok(out.into_inner())
}

/// The bytes of `data` with `turn` applied.
pub fn rotated_bytes(data: &[u8], format: Format, turn: Turn) -> Result<(Vec<u8>, Outcome), SaveError> {
    if format.is_read_only() {
        return Err(SaveError::ReadOnly(format.name()));
    }
    if format == Format::Jpeg {
        return rotate_jpeg(data, turn);
    }
    let loaded = decode::load_bytes(data, format, 0)?;
    if loaded.is_animated() || loaded.page_count > 1 {
        return Err(SaveError::MultiFrame);
    }
    let bitmap = loaded.frames[0].bitmap.turned(turn.turns, turn.flip_h, turn.flip_v);
    Ok((encode_other(&bitmap, format)?, Outcome::Reencoded))
}

/// Replace the contents of `path`. The new bytes go to a temporary file first, so a
/// failure never leaves a half-written picture, and the file keeps its creation and
/// modification dates.
pub(crate) fn replace_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.svtmp"));
    let result = (|| -> std::io::Result<()> {
        std::fs::write(&temp, bytes)?;
        copy_times(&meta, &temp)?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Overwrite `path` with its rotated self, keeping its dates.
pub fn save_rotated(path: &Path, turn: Turn) -> Result<Outcome, SaveError> {
    let data = std::fs::read(path)?;
    let format = Format::detect(&data, path).ok_or(decode::DecodeError::Unsupported)?;
    let (bytes, outcome) = rotated_bytes(&data, format, turn)?;
    replace_file(path, &bytes)?;
    Ok(outcome)
}

/// File formats a picture can be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    Jpeg,
    Png,
    WebP,
    Bmp,
    Tiff,
}

impl SaveFormat {
    pub const ALL: [SaveFormat; 5] = [SaveFormat::Jpeg, SaveFormat::Png, SaveFormat::WebP, SaveFormat::Bmp, SaveFormat::Tiff];

    pub fn name(self) -> &'static str {
        match self {
            SaveFormat::Jpeg => "JPEG",
            SaveFormat::Png => "PNG",
            SaveFormat::WebP => "WebP",
            SaveFormat::Bmp => "BMP",
            SaveFormat::Tiff => "TIFF",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            SaveFormat::Jpeg => "jpg",
            SaveFormat::Png => "png",
            SaveFormat::WebP => "webp",
            SaveFormat::Bmp => "bmp",
            SaveFormat::Tiff => "tif",
        }
    }

    /// Whether the format trades file size against quality.
    pub fn has_quality(self) -> bool {
        matches!(self, SaveFormat::Jpeg | SaveFormat::WebP)
    }

    /// The format to offer first when saving a picture that came from `format`.
    pub fn for_source(format: Format) -> SaveFormat {
        match format {
            Format::Png | Format::Gif | Format::Psd => SaveFormat::Png,
            Format::WebP => SaveFormat::WebP,
            Format::Bmp => SaveFormat::Bmp,
            Format::Tiff => SaveFormat::Tiff,
            Format::Jpeg | Format::Heif | Format::Raw => SaveFormat::Jpeg,
        }
    }

    /// The format a file of this kind is written back in; `None` for kinds SuperView
    /// only reads.
    pub fn of_file(format: Format) -> Option<SaveFormat> {
        Some(match format {
            Format::Jpeg => SaveFormat::Jpeg,
            Format::Png => SaveFormat::Png,
            Format::WebP => SaveFormat::WebP,
            Format::Bmp => SaveFormat::Bmp,
            Format::Tiff => SaveFormat::Tiff,
            Format::Gif | Format::Heif | Format::Raw | Format::Psd => return None,
        })
    }
}

/// Encode a picture. `quality` (1 to 100) matters for JPEG and WebP only. When
/// `metadata_from` holds the bytes of a JPEG, a JPEG result carries over its EXIF and
/// colour profile, with the orientation reset since the pixels are already upright.
pub fn encode(bitmap: &Bitmap, format: SaveFormat, quality: u8, metadata_from: Option<&[u8]>) -> Result<Vec<u8>, SaveError> {
    let quality = quality.clamp(1, 100);
    match format {
        SaveFormat::Jpeg => {
            // JPEG has no transparency: transparent parts become white.
            let mut flat = bitmap.rgba.clone();
            for p in flat.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a != 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
                    }
                    p[3] = 255;
                }
            }
            let image = turbojpeg::Image {
                pixels: flat.as_slice(),
                width: bitmap.width as usize,
                pitch: bitmap.width as usize * 4,
                height: bitmap.height as usize,
                format: turbojpeg::PixelFormat::RGBA,
            };
            // Full colour resolution at high quality, the usual halved one below.
            let subsamp = if quality >= 90 { turbojpeg::Subsamp::None } else { turbojpeg::Subsamp::Sub2x2 };
            let encoded = turbojpeg::compress(image, quality as i32, subsamp).map_err(|e| SaveError::Encode(e.to_string()))?;
            let mut out = match metadata_from.filter(|d| Format::from_magic(d) == Some(Format::Jpeg)) {
                Some(original) => insert_metadata(&encoded, &jpeg_metadata(original)),
                None => encoded.to_vec(),
            };
            reset_exif_orientation(&mut out);
            Ok(out)
        }
        SaveFormat::WebP => {
            let encoder = webp::Encoder::from_rgba(&bitmap.rgba, bitmap.width, bitmap.height);
            Ok(encoder.encode(quality as f32).to_vec())
        }
        SaveFormat::Png => encode_other(bitmap, Format::Png),
        SaveFormat::Bmp => encode_other(bitmap, Format::Bmp),
        SaveFormat::Tiff => encode_other(bitmap, Format::Tiff),
    }
}

/// Overwrite `path` with an edited picture, in the file's own format and keeping its
/// dates. JPEGs are written at quality 95 with their metadata.
pub fn save_edited(path: &Path, bitmap: &Bitmap) -> Result<(), SaveError> {
    let data = std::fs::read(path)?;
    let format = Format::detect(&data, path).ok_or(decode::DecodeError::Unsupported)?;
    let Some(target) = SaveFormat::of_file(format) else {
        return Err(if format == Format::Gif { SaveError::MultiFrame } else { SaveError::ReadOnly(format.name()) });
    };
    // Lossy WebP would lose quality on every save; write it losslessly instead.
    let bytes = if target == SaveFormat::WebP { encode_other(bitmap, Format::WebP)? } else { encode(bitmap, target, 95, Some(&data))? };
    replace_file(path, &bytes)?;
    Ok(())
}

pub(crate) fn copy_times(from: &std::fs::Metadata, to: &Path) -> std::io::Result<()> {
    let mut times = std::fs::FileTimes::new();
    if let Ok(t) = from.modified() {
        times = times.set_modified(t);
    }
    if let Ok(t) = from.accessed() {
        times = times.set_accessed(t);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        if let Ok(t) = from.created() {
            times = times.set_created(t);
        }
    }
    std::fs::OpenOptions::new().write(true).open(to)?.set_times(times)
}
