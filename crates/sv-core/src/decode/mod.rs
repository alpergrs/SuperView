//! Image decoding. Every decoder produces straight (non-premultiplied) RGBA8 with the
//! EXIF orientation already applied.

mod anim;
pub(crate) mod generic;
pub(crate) mod heif;
pub(crate) mod jpeg;
mod psd_file;
pub mod raw;
mod tiff_pages;

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("cannot read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a supported image format")]
    Unsupported,
    #[error("{0}")]
    Decode(String),
}

impl From<image::ImageError> for DecodeError {
    fn from(e: image::ImageError) -> Self {
        DecodeError::Decode(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    Bmp,
    WebP,
    Tiff,
    /// HEIC / HEIF and AVIF, decoded by Windows.
    Heif,
    /// Camera RAW of any make.
    Raw,
    Psd,
}

impl Format {
    pub const ALL: [Format; 9] = [
        Format::Jpeg,
        Format::Png,
        Format::Gif,
        Format::Bmp,
        Format::WebP,
        Format::Tiff,
        Format::Heif,
        Format::Raw,
        Format::Psd,
    ];

    /// Camera RAW file extensions.
    pub const RAW_EXTENSIONS: [&'static str; 19] = [
        "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf", "pef", "srw", "erf", "kdc",
        "dcr", "mos", "mrw",
    ];

    /// Formats SuperView shows but cannot write back.
    pub fn is_read_only(self) -> bool {
        matches!(self, Format::Heif | Format::Raw | Format::Psd)
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::Gif => "GIF",
            Format::Bmp => "BMP",
            Format::WebP => "WebP",
            Format::Tiff => "TIFF",
            Format::Heif => "HEIC / AVIF",
            Format::Raw => "Camera RAW",
            Format::Psd => "PSD",
        }
    }

    pub fn from_extension(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "jpg" | "jpeg" | "jpe" | "jfif" => Format::Jpeg,
            "png" | "apng" => Format::Png,
            "gif" => Format::Gif,
            "bmp" | "dib" => Format::Bmp,
            "webp" => Format::WebP,
            "tif" | "tiff" => Format::Tiff,
            "heic" | "heif" | "hif" | "avif" => Format::Heif,
            "psd" => Format::Psd,
            raw if Format::RAW_EXTENSIONS.contains(&raw) => Format::Raw,
            _ => return None,
        })
    }

    /// Identify a file by its first bytes. Files with a wrong extension are common.
    pub fn from_magic(head: &[u8]) -> Option<Format> {
        if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Format::Jpeg)
        } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Format::Png)
        } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            Some(Format::Gif)
        } else if head.starts_with(b"BM") {
            Some(Format::Bmp)
        } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
            Some(Format::WebP)
        } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
            Some(Format::Tiff)
        } else if head.starts_with(b"8BPS") {
            Some(Format::Psd)
        } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
            // The same container holds HEIC, AVIF and Canon's CR3.
            Some(if &head[8..12] == b"crx " { Format::Raw } else { Format::Heif })
        } else if head.starts_with(b"FUJIFILMCCD-RAW") || head.starts_with(b"IIRO") || head.starts_with(b"IIRS") || head.starts_with(b"IIU\0") {
            Some(Format::Raw)
        } else {
            None
        }
    }

    /// What a file is, from its contents and its name. Most camera RAW files look like
    /// TIFF from the inside, so for those the extension decides.
    pub fn detect(head: &[u8], path: &Path) -> Option<Format> {
        match Format::from_extension(path) {
            Some(Format::Raw) => Some(Format::Raw),
            by_name => Format::from_magic(head).or(by_name),
        }
    }
}

/// True when the file name looks like something SuperView can open.
pub fn is_supported(path: &Path) -> bool {
    Format::from_extension(path).is_some()
}

#[derive(Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, straight alpha.
    pub rgba: Vec<u8>,
}

impl Bitmap {
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        Some([self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]])
    }

    /// A copy scaled down so neither side exceeds `max_side`, or `None` when the
    /// bitmap already fits. Used for pictures larger than the GPU's texture limit.
    pub fn downscaled(&self, max_side: u32) -> Option<Bitmap> {
        let longest = self.width.max(self.height);
        if longest <= max_side || max_side == 0 {
            return None;
        }
        let scale = max_side as f64 / longest as f64;
        let w = ((self.width as f64 * scale).round() as u32).clamp(1, max_side);
        let h = ((self.height as f64 * scale).round() as u32).clamp(1, max_side);
        let src = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())?;
        let out = image::imageops::resize(&src, w, h, image::imageops::FilterType::Triangle);
        Some(Bitmap { width: w, height: h, rgba: out.into_raw() })
    }

    /// A copy turned by quarter turns clockwise, then mirrored.
    pub fn turned(&self, turns: u8, flip_h: bool, flip_v: bool) -> Bitmap {
        use image::imageops;
        let mut img = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .expect("bitmap buffer matches its size");
        img = match turns % 4 {
            1 => imageops::rotate90(&img),
            2 => imageops::rotate180(&img),
            3 => imageops::rotate270(&img),
            _ => img,
        };
        if flip_h {
            imageops::flip_horizontal_in_place(&mut img);
        }
        if flip_v {
            imageops::flip_vertical_in_place(&mut img);
        }
        Bitmap { width: img.width(), height: img.height(), rgba: img.into_raw() }
    }

    pub(crate) fn from_dynamic(img: image::DynamicImage) -> Bitmap {
        let rgba = img.into_rgba8();
        Bitmap { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
    }
}

pub struct Frame {
    pub bitmap: Bitmap,
    /// How long the frame stays on screen. 0 for still images.
    pub delay_ms: u32,
}

pub struct Loaded {
    pub format: Format,
    /// One frame for stills, several for animations.
    pub frames: Vec<Frame>,
    /// Which page this is (multi-page TIFF), zero based.
    pub page: usize,
    pub page_count: usize,
}

impl Loaded {
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    pub fn width(&self) -> u32 {
        self.frames[0].bitmap.width
    }

    pub fn height(&self) -> u32 {
        self.frames[0].bitmap.height
    }

    pub fn bytes(&self) -> usize {
        self.frames.iter().map(|f| f.bitmap.bytes()).sum()
    }

    pub(crate) fn still(format: Format, bitmap: Bitmap) -> Loaded {
        Loaded { format, frames: vec![Frame { bitmap, delay_ms: 0 }], page: 0, page_count: 1 }
    }
}

/// Load a file from disk. `page` selects a page of a multi-page TIFF and is ignored
/// (clamped) for everything else.
pub fn load(path: &Path, page: usize) -> Result<Loaded, DecodeError> {
    let data = std::fs::read(path)?;
    let format = Format::detect(&data, path).ok_or(DecodeError::Unsupported)?;
    load_bytes(&data, format, page)
}

/// Run a decoder so that a bug in it (a panic on a strange file) becomes an ordinary
/// "cannot show this file" instead of taking the program down.
pub(crate) fn guarded<T>(work: impl FnOnce() -> Result<T, DecodeError>) -> Result<T, DecodeError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(DecodeError::Decode("the file is damaged or of an unknown variant".into())))
}

pub fn load_bytes(data: &[u8], format: Format, page: usize) -> Result<Loaded, DecodeError> {
    let loaded = guarded(|| {
        Ok(match format {
            Format::Jpeg => jpeg::load(data)?,
            Format::Gif | Format::Png | Format::WebP => anim::load(data, format)?,
            Format::Tiff => tiff_pages::load(data, page)?,
            Format::Bmp => Loaded::still(format, generic::load(data, format)?),
            Format::Heif => Loaded::still(format, heif::load(data)?),
            Format::Raw => Loaded::still(format, raw::load(data)?),
            Format::Psd => Loaded::still(format, psd_file::load(data)?),
        })
    })?;
    if loaded.frames.is_empty() || loaded.width() == 0 || loaded.height() == 0 {
        return Err(DecodeError::Decode("image has no pixels".into()));
    }
    Ok(loaded)
}
