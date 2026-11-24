//! JPEG through libjpeg-turbo, with the `image` crate as a fallback for the rare files
//! libjpeg-turbo refuses.

use super::{generic, Bitmap, DecodeError, Format, Loaded};
use image::metadata::Orientation;
use image::{DynamicImage, RgbaImage};
use std::io::Cursor;

pub(crate) fn exif_orientation(data: &[u8]) -> Orientation {
    let read = || -> Option<Orientation> {
        let exif = exif::Reader::new().read_from_container(&mut Cursor::new(data)).ok()?;
        let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        Orientation::from_exif(field.value.get_uint(0)? as u8)
    };
    read().unwrap_or(Orientation::NoTransforms)
}

pub(crate) fn load(data: &[u8]) -> Result<Loaded, DecodeError> {
    let bitmap = match turbojpeg::decompress(data, turbojpeg::PixelFormat::RGBA) {
        Ok(img) => {
            let (w, h) = (img.width as u32, img.height as u32);
            let mut pixels = img.pixels;
            if img.pitch != img.width * 4 {
                // Drop row padding.
                let mut tight = Vec::with_capacity(img.width * img.height * 4);
                for row in pixels.chunks(img.pitch).take(img.height) {
                    tight.extend_from_slice(&row[..img.width * 4]);
                }
                pixels = tight;
            }
            let orientation = exif_orientation(data);
            if orientation == Orientation::NoTransforms {
                Bitmap { width: w, height: h, rgba: pixels }
            } else {
                let buf = RgbaImage::from_raw(w, h, pixels)
                    .ok_or_else(|| DecodeError::Decode("bad JPEG buffer".into()))?;
                let mut dynimg = DynamicImage::ImageRgba8(buf);
                dynimg.apply_orientation(orientation);
                Bitmap::from_dynamic(dynimg)
            }
        }
        Err(_) => generic::load(data, Format::Jpeg)?,
    };
    Ok(Loaded::still(Format::Jpeg, bitmap))
}
