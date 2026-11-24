//! Photoshop files: the flattened picture, as Photoshop itself would show it.

use super::{Bitmap, DecodeError};

pub(crate) fn load(data: &[u8]) -> Result<Bitmap, DecodeError> {
    let psd = psd::Psd::from_bytes(data).map_err(|e| DecodeError::Decode(format!("PSD: {e}")))?;
    let (width, height) = (psd.width(), psd.height());
    let rgba = psd.rgba();
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(DecodeError::Decode("PSD: unexpected pixel data".into()));
    }
    Ok(Bitmap { width, height, rgba })
}
