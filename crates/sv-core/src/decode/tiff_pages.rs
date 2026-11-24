//! TIFF, including multi-page files. The `image` crate only reads the first page, so
//! other pages are decoded with the `tiff` crate directly.

use super::{generic, Bitmap, DecodeError, Format, Loaded};
use std::io::Cursor;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;
use tiff::ColorType;

fn err(e: tiff::TiffError) -> DecodeError {
    DecodeError::Decode(e.to_string())
}

pub(crate) fn page_count(data: &[u8]) -> usize {
    let Ok(mut decoder) = Decoder::new(Cursor::new(data)) else { return 1 };
    let mut count = 1;
    while decoder.more_images() && count < 100_000 {
        if decoder.next_image().is_err() {
            break;
        }
        count += 1;
    }
    count
}

pub(crate) fn load(data: &[u8], page: usize) -> Result<Loaded, DecodeError> {
    let page_count = page_count(data);
    let page = page.min(page_count - 1);
    let bitmap = if page == 0 {
        // The `image` crate covers more TIFF variants and applies orientation.
        generic::load(data, Format::Tiff).or_else(|_| decode_page(data, 0))?
    } else {
        decode_page(data, page)?
    };
    Ok(Loaded {
        format: Format::Tiff,
        frames: vec![super::Frame { bitmap, delay_ms: 0 }],
        page,
        page_count,
    })
}

fn decode_page(data: &[u8], page: usize) -> Result<Bitmap, DecodeError> {
    let mut decoder = Decoder::new(Cursor::new(data)).map_err(err)?;
    decoder.seek_to_image(page).map_err(err)?;
    let (width, height) = decoder.dimensions().map_err(err)?;
    let color = decoder.colortype().map_err(err)?;
    // PhotometricInterpretation 0 = WhiteIsZero (typical for faxes and scans).
    let white_is_zero = decoder.get_tag_u32(Tag::PhotometricInterpretation).ok() == Some(0);
    let result = decoder.read_image().map_err(err)?;

    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![255u8; w * h * 4];
    let unsupported = || DecodeError::Decode(format!("unsupported TIFF pixel layout {color:?}"));
    let px = rgba.chunks_exact_mut(4);

    match (result, color) {
        (DecodingResult::U8(buf), ColorType::Gray(1)) => {
            let stride = w.div_ceil(8);
            if buf.len() < stride * h {
                return Err(unsupported());
            }
            for (i, p) in px.enumerate() {
                let (x, y) = (i % w, i / w);
                let bit = (buf[y * stride + x / 8] >> (7 - (x % 8))) & 1;
                let v = if (bit == 1) != white_is_zero { 255 } else { 0 };
                p[..3].fill(v);
            }
        }
        (DecodingResult::U8(buf), ColorType::Gray(8)) if buf.len() >= w * h => {
            for (p, &g) in px.zip(&buf) {
                p[..3].fill(if white_is_zero { 255 - g } else { g });
            }
        }
        (DecodingResult::U8(buf), ColorType::GrayA(8)) if buf.len() >= w * h * 2 => {
            for (p, s) in px.zip(buf.chunks_exact(2)) {
                p[..3].fill(s[0]);
                p[3] = s[1];
            }
        }
        (DecodingResult::U8(buf), ColorType::RGB(8)) if buf.len() >= w * h * 3 => {
            for (p, s) in px.zip(buf.chunks_exact(3)) {
                p[..3].copy_from_slice(s);
            }
        }
        (DecodingResult::U8(buf), ColorType::RGBA(8)) if buf.len() >= w * h * 4 => {
            rgba.copy_from_slice(&buf[..w * h * 4]);
        }
        (DecodingResult::U8(buf), ColorType::CMYK(8)) if buf.len() >= w * h * 4 => {
            for (p, s) in px.zip(buf.chunks_exact(4)) {
                let k = 255 - s[3] as u32;
                p[0] = ((255 - s[0] as u32) * k / 255) as u8;
                p[1] = ((255 - s[1] as u32) * k / 255) as u8;
                p[2] = ((255 - s[2] as u32) * k / 255) as u8;
            }
        }
        (DecodingResult::U16(buf), ColorType::Gray(16)) if buf.len() >= w * h => {
            for (p, &g) in px.zip(&buf) {
                let g = (g >> 8) as u8;
                p[..3].fill(if white_is_zero { 255 - g } else { g });
            }
        }
        (DecodingResult::U16(buf), ColorType::RGB(16)) if buf.len() >= w * h * 3 => {
            for (p, s) in px.zip(buf.chunks_exact(3)) {
                p[0] = (s[0] >> 8) as u8;
                p[1] = (s[1] >> 8) as u8;
                p[2] = (s[2] >> 8) as u8;
            }
        }
        (DecodingResult::U16(buf), ColorType::RGBA(16)) if buf.len() >= w * h * 4 => {
            for (p, s) in px.zip(buf.chunks_exact(4)) {
                for c in 0..4 {
                    p[c] = (s[c] >> 8) as u8;
                }
            }
        }
        _ => return Err(unsupported()),
    }
    Ok(Bitmap { width, height, rgba })
}
