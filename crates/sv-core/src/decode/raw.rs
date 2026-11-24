//! Camera RAW. Every RAW file carries a finished JPEG made by the camera; showing that
//! is instant and matches what the camera's screen showed. Only when a file has no
//! usable one is the sensor data developed, which takes a few seconds.

use super::{Bitmap, DecodeError};
use image::metadata::Orientation;
use image::DynamicImage;
use rawler::decoders::RawDecodeParams;
use rawler::imgop::develop::RawDevelop;
use rawler::rawsource::RawSource;

/// An embedded picture smaller than this is a thumbnail, not something to view.
const MIN_PREVIEW_SIDE: u32 = 1000;

fn err(e: impl std::fmt::Display) -> DecodeError {
    DecodeError::Decode(format!("RAW: {e}"))
}

fn finish(mut image: DynamicImage, orientation: Orientation) -> Bitmap {
    image.apply_orientation(orientation);
    Bitmap::from_dynamic(image)
}

fn long_side(image: &DynamicImage) -> u32 {
    image.width().max(image.height())
}

struct Opened<'a> {
    source: RawSource,
    decoder: Box<dyn rawler::decoders::Decoder>,
    params: RawDecodeParams,
    _data: std::marker::PhantomData<&'a [u8]>,
}

impl Opened<'_> {
    fn new(data: &[u8]) -> Result<Opened<'_>, DecodeError> {
        let source = RawSource::new_from_slice(data);
        let decoder = rawler::get_decoder(&source).map_err(err)?;
        Ok(Opened { source, decoder, params: RawDecodeParams::default(), _data: std::marker::PhantomData })
    }

    /// The camera's orientation for the picture. Embedded JPEGs and the sensor data
    /// are both stored unrotated.
    fn orientation(&self) -> Orientation {
        self.decoder
            .raw_metadata(&self.source, &self.params)
            .ok()
            .and_then(|m| m.exif.orientation)
            .and_then(|o| Orientation::from_exif(o as u8))
            .unwrap_or(Orientation::NoTransforms)
    }

    fn full(&self) -> Option<DynamicImage> {
        self.decoder.full_image(&self.source, &self.params).ok().flatten()
    }

    fn preview(&self) -> Option<DynamicImage> {
        self.decoder.preview_image(&self.source, &self.params).ok().flatten()
    }

    fn small(&self) -> Option<DynamicImage> {
        self.decoder.thumbnail_image(&self.source, &self.params).ok().flatten()
    }

    fn develop(&self) -> Result<DynamicImage, DecodeError> {
        let raw = self.decoder.raw_image(&self.source, &self.params, false).map_err(err)?;
        RawDevelop::default()
            .develop_intermediate(&raw)
            .map_err(err)?
            .to_dynamic_image()
            .ok_or_else(|| err("could not develop the sensor data"))
    }
}

pub(crate) fn load(data: &[u8]) -> Result<Bitmap, DecodeError> {
    let raw = Opened::new(data)?;
    let orientation = raw.orientation();
    // The largest embedded picture wins, if it is big enough to look at.
    let embedded = [raw.full(), raw.preview()].into_iter().flatten().max_by_key(long_side);
    match embedded {
        Some(image) if long_side(&image) >= MIN_PREVIEW_SIDE => Ok(finish(image, orientation)),
        small => match raw.develop() {
            Ok(image) => Ok(finish(image, orientation)),
            // A tiny embedded picture still beats an error.
            Err(e) => small.map(|image| finish(image, orientation)).ok_or(e),
        },
    }
}

/// Sensor data developed to linear light: demosaiced, white-balanced as shot and
/// colour-corrected, but without the tone curve. The starting point for exposure and
/// colour changes, which need more precision than a finished 8-bit picture has.
pub struct Linear {
    pub width: u32,
    pub height: u32,
    /// `width * height * 3` values, red green blue.
    pub rgb: Vec<u16>,
}

impl Linear {
    /// A smaller copy for previews, no side longer than `max_side`.
    pub fn downscaled(&self, max_side: u32) -> Linear {
        let longest = self.width.max(self.height);
        if longest <= max_side || max_side == 0 {
            return Linear { width: self.width, height: self.height, rgb: self.rgb.clone() };
        }
        let step = longest as f64 / max_side as f64;
        let width = ((self.width as f64 / step) as u32).max(1);
        let height = ((self.height as f64 / step) as u32).max(1);
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for y in 0..height {
            let sy = ((y as f64 * step) as usize).min(self.height as usize - 1);
            for x in 0..width {
                let sx = ((x as f64 * step) as usize).min(self.width as usize - 1);
                let i = (sy * self.width as usize + sx) * 3;
                rgb.extend_from_slice(&self.rgb[i..i + 3]);
            }
        }
        Linear { width, height, rgb }
    }
}

/// Develop a RAW file from its sensor data. Takes a few seconds for a large file.
pub fn develop_linear(data: &[u8]) -> Result<Linear, DecodeError> {
    use rawler::imgop::develop::ProcessingStep as Step;
    super::guarded(|| {
        let raw = Opened::new(data)?;
        let orientation = raw.orientation();
        let sensor = raw.decoder.raw_image(&raw.source, &raw.params, false).map_err(err)?;
        // Everything but the final tone curve.
        let steps = [Step::Rescale, Step::Demosaic, Step::FujiRotate, Step::CropActiveArea, Step::WhiteBalance, Step::Calibrate, Step::CropDefault];
        let mut image = RawDevelop::new_with(&steps)
            .develop_intermediate(&sensor)
            .map_err(err)?
            .to_dynamic_image()
            .ok_or_else(|| err("could not develop the sensor data"))?;
        image.apply_orientation(orientation);
        let rgb = image.into_rgb16();
        Ok(Linear { width: rgb.width(), height: rgb.height(), rgb: rgb.into_raw() })
    })
}

/// The quickest picture the file can give, for thumbnails.
pub(crate) fn thumbnail(data: &[u8]) -> Result<Bitmap, DecodeError> {
    let raw = Opened::new(data)?;
    let orientation = raw.orientation();
    let image = match raw.small().or_else(|| raw.preview()).or_else(|| raw.full()) {
        Some(image) => image,
        None => raw.develop()?,
    };
    Ok(finish(image, orientation))
}
