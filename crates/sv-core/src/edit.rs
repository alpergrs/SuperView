//! Picture editing. Every function takes a bitmap and returns a new one, so undo is
//! simply keeping the old one.

use crate::decode::raw::Linear;
use crate::decode::Bitmap;
use rayon::prelude::*;

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// The part of the picture inside the rectangle. The rectangle is cut to the picture
/// and never becomes empty.
pub fn crop(src: &Bitmap, x: u32, y: u32, width: u32, height: u32) -> Bitmap {
    let x = x.min(src.width.saturating_sub(1));
    let y = y.min(src.height.saturating_sub(1));
    let width = width.clamp(1, src.width - x);
    let height = height.clamp(1, src.height - y);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in y..y + height {
        let start = (row as usize * src.width as usize + x as usize) * 4;
        rgba.extend_from_slice(&src.rgba[start..start + width as usize * 4]);
    }
    Bitmap { width, height, rgba }
}

/// Scale to exactly `width` x `height` with a high-quality (Lanczos) filter.
pub fn resize(src: &Bitmap, width: u32, height: u32) -> Bitmap {
    use fast_image_resize as fr;
    let (width, height) = (width.max(1), height.max(1));
    if (width, height) == (src.width, src.height) {
        return Bitmap { width, height, rgba: src.rgba.clone() };
    }
    let fast = || -> Option<Vec<u8>> {
        let source = fr::images::Image::from_vec_u8(src.width, src.height, src.rgba.clone(), fr::PixelType::U8x4).ok()?;
        let mut target = fr::images::Image::new(width, height, fr::PixelType::U8x4);
        let options = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3));
        fr::Resizer::new().resize(&source, &mut target, &options).ok()?;
        Some(target.into_vec())
    };
    let rgba = fast().unwrap_or_else(|| {
        let img = image::RgbaImage::from_raw(src.width, src.height, src.rgba.clone()).expect("bitmap buffer matches its size");
        image::imageops::resize(&img, width, height, image::imageops::FilterType::Lanczos3).into_raw()
    });
    Bitmap { width, height, rgba }
}

/// Size of the largest upright rectangle that fits inside a `w` x `h` rectangle turned
/// by `radians`.
fn inscribed(w: f64, h: f64, radians: f64) -> (f64, f64) {
    let (sin, cos) = (radians.sin().abs(), radians.cos().abs());
    let (long, short) = if w >= h { (w, h) } else { (h, w) };
    if short <= 2.0 * sin * cos * long || (sin - cos).abs() < 1e-10 {
        let x = 0.5 * short;
        if w >= h {
            (x / sin, x / cos)
        } else {
            (x / cos, x / sin)
        }
    } else {
        let cos_2a = cos * cos - sin * sin;
        ((w * cos - h * sin) / cos_2a, (h * cos - w * sin) / cos_2a)
    }
}

/// How much of the picture is left after straightening by `degrees`, as a fraction of
/// its width and height. Used to show the crop before it happens.
pub fn straighten_keeps(width: u32, height: u32, degrees: f32) -> (f32, f32) {
    let (w, h) = inscribed(width as f64, height as f64, (degrees as f64).to_radians());
    ((w / width as f64).min(1.0) as f32, (h / height as f64).min(1.0) as f32)
}

/// Turn the picture by a few degrees (positive is clockwise) to level a horizon, and
/// cut away the empty corners the turn leaves.
pub fn straighten(src: &Bitmap, degrees: f32) -> Bitmap {
    if degrees.abs() < 0.005 {
        return Bitmap { width: src.width, height: src.height, rgba: src.rgba.clone() };
    }
    let radians = (degrees as f64).to_radians();
    let (sw, sh) = (src.width as f64, src.height as f64);
    let (fw, fh) = inscribed(sw, sh, radians);
    let width = (fw.floor() as u32).clamp(1, src.width);
    let height = (fh.floor() as u32).clamp(1, src.height);
    let (sin, cos) = (radians.sin() as f32, radians.cos() as f32);
    let (cx, cy) = ((sw as f32 - 1.0) / 2.0, (sh as f32 - 1.0) / 2.0);
    let (ox, oy) = ((width as f32 - 1.0) / 2.0, (height as f32 - 1.0) / 2.0);
    let (max_x, max_y) = (src.width as i32 - 1, src.height as i32 - 1);
    let stride = src.width as usize * 4;

    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    rgba.par_chunks_mut(width as usize * 4).enumerate().for_each(|(y, row)| {
        let dy = y as f32 - oy;
        for (x, out) in row.chunks_exact_mut(4).enumerate() {
            let dx = x as f32 - ox;
            // Where this output pixel was before the clockwise turn.
            let sx = cx + dx * cos + dy * sin;
            let sy = cy - dx * sin + dy * cos;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - x0, sy - y0);
            let xa = (x0 as i32).clamp(0, max_x) as usize;
            let xb = (x0 as i32 + 1).clamp(0, max_x) as usize;
            let ya = (y0 as i32).clamp(0, max_y) as usize;
            let yb = (y0 as i32 + 1).clamp(0, max_y) as usize;
            for (c, slot) in out.iter_mut().enumerate() {
                let p = |xx: usize, yy: usize| src.rgba[yy * stride + xx * 4 + c] as f32;
                let top = p(xa, ya) * (1.0 - tx) + p(xb, ya) * tx;
                let bottom = p(xa, yb) * (1.0 - tx) + p(xb, yb) * tx;
                *slot = clamp_u8(top * (1.0 - ty) + bottom * ty);
            }
        }
    });
    Bitmap { width, height, rgba }
}

/// Tone and colour settings. Zero (and gamma 1) means "leave as is"; the sliders run
/// from -100 to 100.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    /// 1.0 is neutral; above brightens the mid-tones, below darkens them.
    pub gamma: f32,
    pub saturation: f32,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust { brightness: 0.0, contrast: 0.0, gamma: 1.0, saturation: 0.0, red: 0.0, green: 0.0, blue: 0.0 }
    }
}

impl Adjust {
    pub fn is_neutral(&self) -> bool {
        *self == Adjust::default()
    }

    fn table(&self, balance: f32) -> [u8; 256] {
        let contrast = if self.contrast >= 0.0 { 1.0 + self.contrast / 50.0 } else { 1.0 + self.contrast / 100.0 };
        let gamma = 1.0 / self.gamma.clamp(0.1, 10.0);
        let mut table = [0u8; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut v = i as f32 / 255.0;
            v += balance * 0.002 + self.brightness * 0.004;
            v = (v - 0.5) * contrast + 0.5;
            v = v.clamp(0.0, 1.0).powf(gamma);
            *slot = clamp_u8(v * 255.0);
        }
        table
    }

    pub fn apply(&self, src: &Bitmap) -> Bitmap {
        let mut rgba = src.rgba.clone();
        if self.is_neutral() {
            return Bitmap { width: src.width, height: src.height, rgba };
        }
        let tables = [self.table(self.red), self.table(self.green), self.table(self.blue)];
        let saturation = 1.0 + self.saturation / 100.0;
        let recolour = self.saturation != 0.0;
        rgba.par_chunks_mut(4 * 4096).for_each(|chunk| {
            for p in chunk.chunks_exact_mut(4) {
                let (mut r, mut g, mut b) = (tables[0][p[0] as usize], tables[1][p[1] as usize], tables[2][p[2] as usize]);
                if recolour {
                    let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
                    r = clamp_u8(luma + (r as f32 - luma) * saturation);
                    g = clamp_u8(luma + (g as f32 - luma) * saturation);
                    b = clamp_u8(luma + (b as f32 - luma) * saturation);
                }
                p[0] = r;
                p[1] = g;
                p[2] = b;
            }
        });
        Bitmap { width: src.width, height: src.height, rgba }
    }
}

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil().max(1.0) as i32;
    let mut kernel: Vec<f32> = (-radius..=radius).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);
    kernel
}

/// Soften the picture. `sigma` is the blur radius in pixels.
pub fn blur(src: &Bitmap, sigma: f32) -> Bitmap {
    if sigma <= 0.05 {
        return Bitmap { width: src.width, height: src.height, rgba: src.rgba.clone() };
    }
    // A wide blur has no fine detail left, so it can be done on a smaller copy and
    // scaled back up: the same look for a fraction of the work.
    const DIRECT_LIMIT: f32 = 4.0;
    if sigma > DIRECT_LIMIT && src.width.min(src.height) >= 16 {
        let shrink = sigma / DIRECT_LIMIT;
        let small_w = ((src.width as f32 / shrink).round() as u32).max(4);
        let small_h = ((src.height as f32 / shrink).round() as u32).max(4);
        let small = blur_direct(&resize(src, small_w, small_h), DIRECT_LIMIT);
        return resize(&small, src.width, src.height);
    }
    blur_direct(src, sigma)
}

fn blur_direct(src: &Bitmap, sigma: f32) -> Bitmap {
    let kernel = gaussian_kernel(sigma);
    let radius = (kernel.len() / 2) as i32;
    let (w, h) = (src.width as usize, src.height as usize);

    // Horizontal pass, then vertical: the same result as a full 2D blur, far cheaper.
    let mut across = vec![0u8; src.rgba.len()];
    across.par_chunks_mut(w * 4).zip(src.rgba.par_chunks(w * 4)).for_each(|(out, row)| {
        for x in 0..w {
            let mut sum = [0f32; 4];
            for (k, weight) in kernel.iter().enumerate() {
                let xx = (x as i32 + k as i32 - radius).clamp(0, w as i32 - 1) as usize;
                for c in 0..4 {
                    sum[c] += row[xx * 4 + c] as f32 * weight;
                }
            }
            for c in 0..4 {
                out[x * 4 + c] = clamp_u8(sum[c]);
            }
        }
    });
    let mut rgba = vec![0u8; src.rgba.len()];
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, out)| {
        for x in 0..w {
            let mut sum = [0f32; 4];
            for (k, weight) in kernel.iter().enumerate() {
                let yy = (y as i32 + k as i32 - radius).clamp(0, h as i32 - 1) as usize;
                for c in 0..4 {
                    sum[c] += across[(yy * w + x) * 4 + c] as f32 * weight;
                }
            }
            for c in 0..4 {
                out[x * 4 + c] = clamp_u8(sum[c]);
            }
        }
    });
    Bitmap { width: src.width, height: src.height, rgba }
}

/// Blur only a rectangle of the picture, e.g. to hide a face or a number plate. The
/// strength follows the size of the rectangle, so the result is unreadable at any size.
pub fn blur_region(src: &Bitmap, x: u32, y: u32, width: u32, height: u32) -> Bitmap {
    let patch = crop(src, x, y, width, height);
    let sigma = (patch.width.max(patch.height) as f32 / 12.0).clamp(4.0, 60.0);
    let soft = blur(&patch, sigma);
    let (x, y) = (x.min(src.width.saturating_sub(1)) as usize, y.min(src.height.saturating_sub(1)) as usize);
    let mut rgba = src.rgba.clone();
    let stride = src.width as usize * 4;
    for (row, line) in soft.rgba.chunks_exact(soft.width as usize * 4).enumerate() {
        let start = (y + row) * stride + x * 4;
        rgba[start..start + line.len()].copy_from_slice(line);
    }
    Bitmap { width: src.width, height: src.height, rgba }
}

/// Sharpen with an unsharp mask. `amount` runs from 0 to 100.
pub fn sharpen(src: &Bitmap, amount: f32) -> Bitmap {
    let strength = amount.clamp(0.0, 100.0) / 100.0 * 1.5;
    let soft = blur(src, 1.2);
    let mut rgba = src.rgba.clone();
    rgba.par_chunks_mut(4 * 4096).zip(soft.rgba.par_chunks(4 * 4096)).for_each(|(out, soft)| {
        for (p, s) in out.chunks_exact_mut(4).zip(soft.chunks_exact(4)) {
            for c in 0..3 {
                let v = p[c] as f32;
                p[c] = clamp_u8(v + (v - s[c] as f32) * strength);
            }
        }
    });
    Bitmap { width: src.width, height: src.height, rgba }
}

/// Smooth away grain while keeping edges: each pixel is averaged only with neighbours
/// of similar brightness. `strength` runs from 0 to 100.
pub fn denoise(src: &Bitmap, strength: f32) -> Bitmap {
    let strength = strength.clamp(0.0, 100.0);
    if strength == 0.0 {
        return Bitmap { width: src.width, height: src.height, rgba: src.rgba.clone() };
    }
    const RADIUS: i32 = 2;
    let range_sigma = 4.0 + strength * 0.4;
    let mut range = [0f32; 256];
    for (d, slot) in range.iter_mut().enumerate() {
        *slot = (-((d * d) as f32) / (2.0 * range_sigma * range_sigma)).exp();
    }
    let mut spatial = [[0f32; 5]; 5];
    for (j, row) in spatial.iter_mut().enumerate() {
        for (i, slot) in row.iter_mut().enumerate() {
            let (dx, dy) = (i as f32 - 2.0, j as f32 - 2.0);
            *slot = (-(dx * dx + dy * dy) / (2.0 * 1.5 * 1.5)).exp();
        }
    }
    let luma = |p: &[u8]| (p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8;
    let (w, h) = (src.width as i32, src.height as i32);
    let stride = src.width as usize * 4;

    let mut rgba = src.rgba.clone();
    rgba.par_chunks_mut(stride).enumerate().for_each(|(y, out)| {
        for x in 0..w {
            let centre = &src.rgba[y * stride + x as usize * 4..][..4];
            let centre_luma = luma(centre) as i32;
            let (mut sum, mut total) = ([0f32; 3], 0f32);
            for dy in -RADIUS..=RADIUS {
                let yy = (y as i32 + dy).clamp(0, h - 1) as usize;
                for dx in -RADIUS..=RADIUS {
                    let xx = (x + dx).clamp(0, w - 1) as usize;
                    let p = &src.rgba[yy * stride + xx * 4..][..4];
                    let diff = (luma(p) as i32 - centre_luma).unsigned_abs() as usize;
                    let weight = spatial[(dy + RADIUS) as usize][(dx + RADIUS) as usize] * range[diff.min(255)];
                    for c in 0..3 {
                        sum[c] += p[c] as f32 * weight;
                    }
                    total += weight;
                }
            }
            for c in 0..3 {
                out[x as usize * 4 + c] = clamp_u8(sum[c] / total);
            }
        }
    });
    Bitmap { width: src.width, height: src.height, rgba }
}

/// One-click fix for flat, hazy pictures: stretch the tones so the darkest parts become
/// black and the brightest white, ignoring the most extreme half percent at each end.
/// Colours keep their balance because all channels are stretched alike.
pub fn auto_fix(src: &Bitmap) -> Bitmap {
    let mut histogram = [0u64; 256];
    for p in src.rgba.chunks_exact(4) {
        histogram[((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as usize] += 1;
    }
    let pixels: u64 = histogram.iter().sum();
    let clip = pixels / 200;
    let mut low = 0usize;
    let mut seen = 0u64;
    while low < 255 && seen + histogram[low] <= clip {
        seen += histogram[low];
        low += 1;
    }
    let mut high = 255usize;
    seen = 0;
    while high > low && seen + histogram[high] <= clip {
        seen += histogram[high];
        high -= 1;
    }
    let mut rgba = src.rgba.clone();
    // Nothing to gain on a picture that already uses the full range, or is one flat tone.
    if high - low < 8 || (low == 0 && high == 255) {
        return Bitmap { width: src.width, height: src.height, rgba };
    }
    let mut table = [0u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = clamp_u8((i as f32 - low as f32) * 255.0 / (high - low) as f32);
    }
    rgba.par_chunks_mut(4 * 4096).for_each(|chunk| {
        for p in chunk.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = table[p[c] as usize];
            }
        }
    });
    Bitmap { width: src.width, height: src.height, rgba }
}

/// Settings for developing a RAW file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawParams {
    /// Stops of exposure, -3 to +3.
    pub exposure: f32,
    /// -100 (cooler, bluer) to 100 (warmer, more orange), relative to the camera's setting.
    pub temperature: f32,
    /// -100 (greener) to 100 (more magenta).
    pub tint: f32,
    /// 0 to 100: how strongly bright areas are pulled back to recover their detail.
    pub highlights: f32,
}

impl Default for RawParams {
    fn default() -> Self {
        RawParams { exposure: 0.0, temperature: 0.0, tint: 0.0, highlights: 0.0 }
    }
}

fn srgb_curve(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// Turn linear sensor data into a finished picture. Exposure and colour are changed in
/// linear light, before the tone curve, which is what makes RAW adjustments cleaner than
/// the same sliders on a JPEG.
pub fn develop(linear: &Linear, params: &RawParams) -> Bitmap {
    let exposure = 2f32.powf(params.exposure.clamp(-5.0, 5.0));
    let warm = 2f32.powf(params.temperature.clamp(-100.0, 100.0) / 100.0 * 0.5);
    let magenta = 2f32.powf(params.tint.clamp(-100.0, 100.0) / 100.0 * 0.35);
    let gains = [exposure * warm, exposure / magenta, exposure / warm];
    let recover = params.highlights.clamp(0.0, 100.0) / 100.0;

    // The whole chain is per channel, so one lookup table per channel does it.
    let tables: Vec<Vec<u8>> = gains
        .iter()
        .map(|gain| {
            (0..=u16::MAX)
                .map(|v| {
                    let x = v as f32 / 65535.0 * gain;
                    // A soft shoulder (extended Reinhard) that reaches white only at the
                    // brightest value the sensor recorded, so tones pushed past white
                    // by the exposure slider get their separation back.
                    let white = gain.max(1.25);
                    let rolled = x * (1.0 + x / (white * white)) / (1.0 + x);
                    let toned = x.min(1.0) * (1.0 - recover) + rolled.min(1.0) * recover;
                    clamp_u8(srgb_curve(toned.clamp(0.0, 1.0)) * 255.0)
                })
                .collect()
        })
        .collect();

    let mut rgba = vec![255u8; linear.width as usize * linear.height as usize * 4];
    rgba.par_chunks_mut(4 * 4096).zip(linear.rgb.par_chunks(3 * 4096)).for_each(|(out, src)| {
        for (p, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
            p[0] = tables[0][s[0] as usize];
            p[1] = tables[1][s[1] as usize];
            p[2] = tables[2][s[2] as usize];
        }
    });
    Bitmap { width: linear.width, height: linear.height, rgba }
}
