//! Reading and changing the descriptive information stored inside a picture.

use crate::decode::{self, Format};
use crate::save::{self, Turn};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata;
use std::io::Cursor;
use std::path::Path;

/// The fields SuperView can change. In [`write`], `None` leaves a field alone and an
/// empty string removes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub description: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    /// When the picture was taken, as `YYYY-MM-DD HH:MM:SS`.
    pub taken: Option<String>,
}

impl Fields {
    pub fn is_empty(&self) -> bool {
        *self == Fields::default()
    }
}

/// Everything read from a picture: the editable fields plus camera details for display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub fields: Fields,
    /// Label and value pairs such as ("Camera", "Canon EOS R6").
    pub details: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum MetaError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0} files cannot have their information changed.")]
    Unsupported(&'static str),
    #[error("The date must look like 2024-05-31 14:30:00.")]
    BadDate,
    #[error("{0}")]
    Failed(String),
}

/// `2024-05-31 14:30:00` (also with `T`, or without seconds) to EXIF's
/// `2024:05:31 14:30:00`.
pub fn to_exif_date(text: &str) -> Option<String> {
    let text = text.trim().replace('T', " ");
    let (date, time) = text.split_once(' ').unwrap_or((text.as_str(), "00:00:00"));
    let d: Vec<u32> = date.split(['-', ':', '.', '/']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<u32> = time.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [y, m, day] = d[..] else { return None };
    let (h, min, s) = match t[..] {
        [h, min] => (h, min, 0),
        [h, min, s] => (h, min, s),
        _ => return None,
    };
    let valid = (1000..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&day) && h < 24 && min < 60 && s < 60;
    valid.then(|| format!("{y:04}:{m:02}:{day:02} {h:02}:{min:02}:{s:02}"))
}

/// EXIF's `2024:05:31 14:30:00` to `2024-05-31 14:30:00`.
pub fn from_exif_date(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches('\0');
    let (date, time) = text.split_once(' ')?;
    let date = date.replace(':', "-");
    (date.len() == 10 && time.len() >= 5).then(|| format!("{date} {time}"))
}

fn text_of(exif: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let field = exif.get_field(tag, exif::In::PRIMARY)?;
    let text = match &field.value {
        exif::Value::Ascii(parts) => parts.iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect::<Vec<_>>().join(" "),
        _ => field.display_value().with_unit(exif).to_string(),
    };
    let text = text.trim().trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Read the information stored in a picture. A picture without any gives an empty result.
pub fn read_bytes(data: &[u8]) -> Info {
    let Ok(exif) = exif::Reader::new().read_from_container(&mut Cursor::new(data)) else {
        return Info { fields: read_fields_fallback(data), details: Vec::new() };
    };
    let fields = Fields {
        description: text_of(&exif, exif::Tag::ImageDescription),
        artist: text_of(&exif, exif::Tag::Artist),
        copyright: text_of(&exif, exif::Tag::Copyright),
        taken: text_of(&exif, exif::Tag::DateTimeOriginal)
            .or_else(|| text_of(&exif, exif::Tag::DateTime))
            .and_then(|d| from_exif_date(&d)),
    };
    let mut details = Vec::new();
    let camera = [text_of(&exif, exif::Tag::Make), text_of(&exif, exif::Tag::Model)].into_iter().flatten().collect::<Vec<_>>().join(" ");
    if !camera.is_empty() {
        details.push(("Camera".to_string(), camera));
    }
    for (label, tag) in [
        ("Lens", exif::Tag::LensModel),
        ("Exposure", exif::Tag::ExposureTime),
        ("Aperture", exif::Tag::FNumber),
        ("ISO", exif::Tag::PhotographicSensitivity),
        ("Focal length", exif::Tag::FocalLength),
        ("Flash", exif::Tag::Flash),
        ("Software", exif::Tag::Software),
    ] {
        if let Some(value) = text_of(&exif, tag) {
            details.push((label.to_string(), value));
        }
    }
    Info { fields, details }
}

/// A second reader for files the first one finds nothing in. In PNGs the information can
/// sit after the picture data, where the first reader does not look.
fn read_fields_fallback(data: &[u8]) -> Fields {
    let Some(kind) = Format::from_magic(data).and_then(container) else { return Fields::default() };
    let read = || -> Option<Fields> {
        let meta = Metadata::new_from_vec(&data.to_vec(), kind).ok()?;
        let text = |probe: ExifTag| -> Option<String> {
            let found = meta.get_tag(&probe).next()?;
            let value = match found {
                ExifTag::ImageDescription(s) | ExifTag::Artist(s) | ExifTag::Copyright(s) | ExifTag::DateTimeOriginal(s) | ExifTag::CreateDate(s) => s,
                _ => return None,
            };
            let value = value.trim().trim_end_matches('\0').trim().to_string();
            (!value.is_empty()).then_some(value)
        };
        Some(Fields {
            description: text(ExifTag::ImageDescription(String::new())),
            artist: text(ExifTag::Artist(String::new())),
            copyright: text(ExifTag::Copyright(String::new())),
            taken: text(ExifTag::DateTimeOriginal(String::new())).or_else(|| text(ExifTag::CreateDate(String::new()))).and_then(|d| from_exif_date(&d)),
        })
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).ok().flatten().unwrap_or_default()
}

pub fn read(path: &Path) -> Info {
    std::fs::read(path).map(|data| read_bytes(&data)).unwrap_or_default()
}

fn container(format: Format) -> Option<FileExtension> {
    Some(match format {
        Format::Jpeg => FileExtension::JPEG,
        Format::Png => FileExtension::PNG { as_zTXt_chunk: false },
        Format::WebP => FileExtension::WEBP,
        Format::Tiff => FileExtension::TIFF,
        Format::Gif | Format::Bmp | Format::Heif | Format::Raw | Format::Psd => return None,
    })
}

/// Whether [`write`] can change the information of this kind of file.
pub fn can_write(format: Format) -> bool {
    container(format).is_some()
}

/// The bytes of a picture with the given fields changed.
pub fn written_bytes(data: &[u8], format: Format, changes: &Fields) -> Result<Vec<u8>, MetaError> {
    let kind = container(format).ok_or(MetaError::Unsupported(format.name()))?;
    let taken = match &changes.taken {
        Some(text) if !text.trim().is_empty() => Some(Some(to_exif_date(text).ok_or(MetaError::BadDate)?)),
        Some(_) => Some(None),
        None => None,
    };
    let mut out = data.to_vec();
    // A library bug on an odd file must not take the program down.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        let mut meta = Metadata::new_from_vec(&out, kind).unwrap_or_else(|_| Metadata::new());
        let mut text = |value: &Option<String>, make: fn(String) -> ExifTag| {
            if let Some(value) = value {
                if value.trim().is_empty() {
                    meta.remove_tag(make(String::new()));
                } else {
                    meta.set_tag(make(value.trim().to_string()));
                }
            }
        };
        text(&changes.description, ExifTag::ImageDescription);
        text(&changes.artist, ExifTag::Artist);
        text(&changes.copyright, ExifTag::Copyright);
        match taken {
            Some(Some(date)) => {
                meta.set_tag(ExifTag::DateTimeOriginal(date.clone()));
                meta.set_tag(ExifTag::CreateDate(date));
            }
            Some(None) => {
                meta.remove_tag(ExifTag::DateTimeOriginal(String::new()));
                meta.remove_tag(ExifTag::CreateDate(String::new()));
            }
            None => {}
        }
        meta.write_to_vec(&mut out, kind).map_err(|e| e.to_string())
    }));
    match result {
        Ok(Ok(())) => Ok(out),
        Ok(Err(e)) => Err(MetaError::Failed(e)),
        Err(_) => Err(MetaError::Failed("this file's information could not be rewritten".to_string())),
    }
}

/// Change fields of a file in place. The picture itself is not recompressed, and the
/// file keeps its creation and modification dates.
pub fn write(path: &Path, changes: &Fields) -> Result<(), MetaError> {
    if changes.is_empty() {
        return Ok(());
    }
    let data = std::fs::read(path)?;
    let format = Format::detect(&data, path).ok_or(MetaError::Unsupported("These"))?;
    let out = written_bytes(&data, format, changes)?;
    // Refuse to write something that no longer opens.
    decode::load_bytes(&out, format, 0).map_err(|e| MetaError::Failed(format!("the result would not open ({e}); the file was left alone")))?;
    save::replace_file(path, &out)?;
    Ok(())
}

/// The bytes of a picture with all descriptive metadata removed (camera, dates, GPS
/// position, comments). A rotation that was only noted in the metadata is applied to
/// the picture first, so it still looks the same.
pub fn stripped_bytes(data: &[u8], format: Format) -> Result<Vec<u8>, MetaError> {
    let failed = |e: save::SaveError| MetaError::Failed(e.to_string());
    match format {
        Format::Jpeg => {
            let (upright, _) = save::rotated_bytes(data, Format::Jpeg, Turn::default()).map_err(failed)?;
            Ok(save::strip_jpeg_metadata(&upright))
        }
        Format::Png | Format::WebP | Format::Tiff | Format::Bmp => {
            let loaded = decode::load_bytes(data, format, 0).map_err(|e| MetaError::Failed(e.to_string()))?;
            if loaded.is_animated() || loaded.page_count > 1 {
                return Err(MetaError::Failed("animations and multi-page files are left alone".to_string()));
            }
            save::encode_other(&loaded.frames[0].bitmap, format).map_err(failed)
        }
        Format::Gif | Format::Heif | Format::Raw | Format::Psd => Err(MetaError::Unsupported(format.name())),
    }
}

pub fn strip(path: &Path) -> Result<(), MetaError> {
    let data = std::fs::read(path)?;
    let format = Format::detect(&data, path).ok_or(MetaError::Unsupported("These"))?;
    let out = stripped_bytes(&data, format)?;
    save::replace_file(path, &out)?;
    Ok(())
}
