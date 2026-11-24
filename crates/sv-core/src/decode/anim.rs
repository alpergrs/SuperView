//! GIF, PNG/APNG and WebP: animated when the file is, a single frame otherwise.

use super::{generic, Bitmap, DecodeError, Format, Frame, Loaded};
use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
use image::{AnimationDecoder, Frames};
use std::io::Cursor;

/// Decoded animation frames are kept in memory; stop before they eat the machine.
const MAX_ANIMATION_BYTES: usize = 768 * 1024 * 1024;

pub(crate) fn load(data: &[u8], format: Format) -> Result<Loaded, DecodeError> {
    let frames = match format {
        Format::Gif => Some(collect(GifDecoder::new(Cursor::new(data))?.into_frames())?),
        Format::Png => {
            let decoder = PngDecoder::new(Cursor::new(data))?;
            if decoder.is_apng()? {
                Some(collect(decoder.apng()?.into_frames())?)
            } else {
                None
            }
        }
        Format::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(data))?;
            if decoder.has_animation() {
                Some(collect(decoder.into_frames())?)
            } else {
                None
            }
        }
        _ => None,
    };
    match frames {
        Some(frames) if !frames.is_empty() => {
            Ok(Loaded { format, frames, page: 0, page_count: 1 })
        }
        _ => Ok(Loaded::still(format, generic::load(data, format)?)),
    }
}

fn collect(frames: Frames<'_>) -> Result<Vec<Frame>, DecodeError> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for frame in frames {
        let frame = match frame {
            Ok(f) => f,
            // A truncated animation still shows the frames that decoded.
            Err(_) if !out.is_empty() => break,
            Err(e) => return Err(e.into()),
        };
        let (num, den) = frame.delay().numer_denom_ms();
        let mut delay_ms = num.checked_div(den).unwrap_or(0);
        // Browsers treat near-zero delays as 100 ms; so do we.
        if delay_ms < 20 {
            delay_ms = 100;
        }
        let buf = frame.into_buffer();
        let bitmap = Bitmap { width: buf.width(), height: buf.height(), rgba: buf.into_raw() };
        total += bitmap.bytes();
        out.push(Frame { bitmap, delay_ms });
        if total > MAX_ANIMATION_BYTES {
            break;
        }
    }
    if out.len() == 1 {
        out[0].delay_ms = 0;
    }
    Ok(out)
}
