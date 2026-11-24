//! HEIC / HEIF and AVIF through the Windows Imaging Component, the same decoder the
//! Windows Photos app uses. It needs Microsoft's "HEIF Image Extensions" (and "HEVC
//! Video Extensions" for HEIC, "AV1 Video Extension" for AVIF) to be installed.

use super::{Bitmap, DecodeError};

#[cfg(windows)]
mod wic {
    use super::{Bitmap, DecodeError};
    use windows::core::Interface;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapFrameDecode, IWICBitmapSource,
        IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

    /// An embedded thumbnail smaller than this is too coarse to show.
    const MIN_THUMB_SIDE: u32 = 160;

    fn failed(e: windows::core::Error) -> DecodeError {
        DecodeError::Decode(format!(
            "Windows could not decode this file ({e}). HEIC needs the \"HEIF Image Extensions\" and \"HEVC Video Extensions\" from the Microsoft Store; AVIF needs the \"AV1 Video Extension\"."
        ))
    }

    /// # Safety
    /// COM must be initialised on this thread, and `data` must outlive the returned frame.
    unsafe fn open(data: &[u8]) -> Result<(IWICImagingFactory, IWICBitmapFrameDecode), DecodeError> {
        // Fine to call on a thread that is already initialised.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(failed)?;
        let stream = factory.CreateStream().map_err(failed)?;
        stream.InitializeFromMemory(data).map_err(failed)?;
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .map_err(failed)?;
        let frame = decoder.GetFrame(0).map_err(failed)?;
        Ok((factory, frame))
    }

    /// # Safety
    /// `source` must be a live WIC bitmap source.
    unsafe fn pixels(factory: &IWICImagingFactory, source: &IWICBitmapSource) -> Result<Bitmap, DecodeError> {
        let converter = factory.CreateFormatConverter().map_err(failed)?;
        converter
            .Initialize(source, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(failed)?;
        let (mut width, mut height) = (0u32, 0u32);
        converter.GetSize(&mut width, &mut height).map_err(failed)?;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut rgba).map_err(failed)?;
        Ok(Bitmap { width, height, rgba })
    }

    pub(crate) fn load(data: &[u8]) -> Result<Bitmap, DecodeError> {
        // SAFETY: `data` outlives every COM object created here; all are dropped on return.
        unsafe {
            let (factory, frame) = open(data)?;
            pixels(&factory, &frame.cast::<IWICBitmapSource>().map_err(failed)?)
        }
    }

    /// The small picture stored inside the file, when there is a usable one. Reading it
    /// skips decoding the full photo, which is where nearly all the time goes.
    pub(crate) fn thumbnail(data: &[u8]) -> Result<Bitmap, DecodeError> {
        // SAFETY: as in `load`.
        unsafe {
            let (factory, frame) = open(data)?;
            let (mut full_w, mut full_h) = (0u32, 0u32);
            frame.GetSize(&mut full_w, &mut full_h).map_err(failed)?;
            if let Ok(small) = frame.GetThumbnail() {
                if let Ok(bitmap) = pixels(&factory, &small) {
                    // The stored thumbnail is only trusted when it stands the same way
                    // up as the photo; otherwise it was not rotated like the photo is.
                    let same_way = (bitmap.width >= bitmap.height) == (full_w >= full_h);
                    if same_way && bitmap.width.max(bitmap.height) >= MIN_THUMB_SIDE {
                        return Ok(bitmap);
                    }
                }
            }
            pixels(&factory, &frame.cast::<IWICBitmapSource>().map_err(failed)?)
        }
    }
}

#[cfg(windows)]
pub(crate) use wic::{load, thumbnail};

#[cfg(not(windows))]
pub(crate) fn load(_data: &[u8]) -> Result<Bitmap, DecodeError> {
    Err(DecodeError::Decode("HEIC and AVIF are decoded by Windows and only work there.".into()))
}

#[cfg(not(windows))]
pub(crate) fn thumbnail(data: &[u8]) -> Result<Bitmap, DecodeError> {
    load(data)
}
