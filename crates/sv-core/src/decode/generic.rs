//! Still images through the `image` crate.

use super::{Bitmap, DecodeError, Format};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::io::Cursor;

pub(crate) fn image_format(format: Format) -> ImageFormat {
    match format {
        Format::Jpeg => ImageFormat::Jpeg,
        Format::Png => ImageFormat::Png,
        Format::Gif => ImageFormat::Gif,
        Format::Bmp => ImageFormat::Bmp,
        Format::WebP => ImageFormat::WebP,
        Format::Tiff => ImageFormat::Tiff,
        // Never reached: these formats have their own decoders and are not written.
        Format::Heif | Format::Raw | Format::Psd => ImageFormat::Png,
    }
}

pub(crate) fn load(data: &[u8], format: Format) -> Result<Bitmap, DecodeError> {
    let mut reader = ImageReader::with_format(Cursor::new(data), image_format(format));
    reader.no_limits();
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(Bitmap::from_dynamic(img))
}
