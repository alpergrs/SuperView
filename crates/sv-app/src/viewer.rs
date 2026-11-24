//! Where the picture sits on screen: zoom, pan, quarter turns and flips.
//!
//! Zoom is in screen pixels per image pixel (1.0 = 100%). Positions are in egui points.

use eframe::egui::{pos2, vec2, Pos2, Rect, Vec2};
use sv_core::settings::FitMode;

pub const MIN_ZOOM: f32 = 0.01;
pub const MAX_ZOOM: f32 = 64.0;

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// `None` follows the fit mode (and the window size); `Some` is a zoom the user chose.
    pub zoom: Option<f32>,
    /// Picture centre relative to the canvas centre. Only used with a chosen zoom.
    pub offset: Vec2,
    /// Quarter turns clockwise, 0..=3.
    pub turns: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: None, offset: Vec2::ZERO, turns: 0, flip_h: false, flip_v: false }
    }
}

impl View {
    pub fn reset_placement(&mut self) {
        self.zoom = None;
        self.offset = Vec2::ZERO;
    }

    pub fn reset_orientation(&mut self) {
        self.turns = 0;
        self.flip_h = false;
        self.flip_v = false;
    }

    pub fn rotate(&mut self, clockwise: bool) {
        // A flip mirrors the sense of rotation; compensate so "right" always looks right.
        let mirrored = self.flip_h != self.flip_v;
        let step = if clockwise != mirrored { 1 } else { 3 };
        self.turns = (self.turns + step) % 4;
        self.offset = Vec2::ZERO;
    }

    /// Picture size in image pixels as shown, after quarter turns.
    pub fn shown_size(&self, width: u32, height: u32) -> Vec2 {
        if self.turns % 2 == 1 {
            vec2(height as f32, width as f32)
        } else {
            vec2(width as f32, height as f32)
        }
    }

    pub fn fit_zoom(mode: FitMode, shown: Vec2, canvas_px: Vec2) -> f32 {
        let (zx, zy) = (canvas_px.x / shown.x.max(1.0), canvas_px.y / shown.y.max(1.0));
        let zoom = match mode {
            FitMode::ShrinkToFit => zx.min(zy).min(1.0),
            FitMode::FitWindow => zx.min(zy),
            FitMode::FitWidth => zx,
            FitMode::Fill => zx.max(zy),
            FitMode::Actual => 1.0,
        };
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    }

    pub fn effective_zoom(&self, mode: FitMode, shown: Vec2, canvas_px: Vec2) -> f32 {
        self.zoom.unwrap_or_else(|| Self::fit_zoom(mode, shown, canvas_px))
    }

    /// Change zoom keeping the point under `anchor` (relative to the canvas centre) still.
    pub fn zoom_about(&mut self, current: f32, new_zoom: f32, anchor: Vec2, auto_offset: Vec2) {
        let new_zoom = new_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let offset = if self.zoom.is_some() { self.offset } else { auto_offset };
        self.offset = anchor - (anchor - offset) * (new_zoom / current);
        self.zoom = Some(new_zoom);
    }

    /// Where the fit mode puts a picture that is larger than the canvas: fit-width starts
    /// at the top, everything else is centred.
    pub fn auto_offset(mode: FitMode, size: Vec2, canvas: Vec2) -> Vec2 {
        if mode == FitMode::FitWidth && size.y > canvas.y {
            vec2(0.0, (size.y - canvas.y) / 2.0)
        } else {
            Vec2::ZERO
        }
    }

    /// Keep the picture from being dragged off the canvas: centred on an axis where it
    /// fits. Where it is larger than the canvas, it can be dragged `overshoot` points past
    /// its edge, showing a strip of background, so the edge itself is easy to look at.
    pub fn clamp_offset(offset: Vec2, size: Vec2, canvas: Vec2, overshoot: f32) -> Vec2 {
        let limit = |picture: f32, room: f32| if picture > room { (picture - room) / 2.0 + overshoot } else { 0.0 };
        let (lx, ly) = (limit(size.x, canvas.x), limit(size.y, canvas.y));
        vec2(offset.x.clamp(-lx, lx), offset.y.clamp(-ly, ly))
    }

    /// Texture coordinates for the corners of the on-screen rectangle, in the order
    /// top-left, top-right, bottom-right, bottom-left.
    pub fn uvs(&self) -> [Pos2; 4] {
        let src = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let k = self.turns as usize % 4;
        let mut uv = [src[0]; 4];
        for (i, slot) in uv.iter_mut().enumerate() {
            *slot = src[(i + 4 - k) % 4];
        }
        if self.flip_h {
            uv.swap(0, 1);
            uv.swap(3, 2);
        }
        if self.flip_v {
            uv.swap(0, 3);
            uv.swap(1, 2);
        }
        uv
    }

    /// The image pixel under a screen position, undoing flips and turns.
    pub fn pixel_at(&self, pos: Pos2, rect: Rect, width: u32, height: u32) -> Option<(u32, u32)> {
        if !rect.contains(pos) || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        let mut x = (pos.x - rect.min.x) / rect.width();
        let mut y = (pos.y - rect.min.y) / rect.height();
        if self.flip_h {
            x = 1.0 - x;
        }
        if self.flip_v {
            y = 1.0 - y;
        }
        for _ in 0..self.turns % 4 {
            (x, y) = (y, 1.0 - x);
        }
        let px = ((x * width as f32) as u32).min(width.saturating_sub(1));
        let py = ((y * height as f32) as u32).min(height.saturating_sub(1));
        Some((px, py))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Vec2 = Vec2::new(1000.0, 500.0);

    #[test]
    fn fit_modes() {
        let big = vec2(4000.0, 1000.0);
        let small = vec2(100.0, 100.0);
        assert_eq!(View::fit_zoom(FitMode::ShrinkToFit, big, CANVAS), 0.25);
        assert_eq!(View::fit_zoom(FitMode::ShrinkToFit, small, CANVAS), 1.0);
        assert_eq!(View::fit_zoom(FitMode::FitWindow, small, CANVAS), 5.0);
        assert_eq!(View::fit_zoom(FitMode::FitWidth, small, CANVAS), 10.0);
        assert_eq!(View::fit_zoom(FitMode::Fill, big, CANVAS), 0.5);
        assert_eq!(View::fit_zoom(FitMode::Actual, big, CANVAS), 1.0);
    }

    #[test]
    fn zoom_keeps_the_anchor_still() {
        let mut v = View::default();
        let anchor = vec2(100.0, -50.0);
        // The image point under the anchor before: (anchor - offset) / zoom.
        let before = (anchor - Vec2::ZERO) / 1.0;
        v.zoom_about(1.0, 2.0, anchor, Vec2::ZERO);
        let after = (anchor - v.offset) / 2.0;
        assert!((before - after).length() < 1e-4);
        v.zoom_about(2.0, 1000.0, anchor, Vec2::ZERO);
        assert_eq!(v.zoom, Some(MAX_ZOOM));
    }

    #[test]
    fn offset_is_centred_when_the_picture_fits() {
        let c = View::clamp_offset(vec2(300.0, 300.0), vec2(800.0, 900.0), CANVAS, 0.0);
        assert_eq!(c, vec2(0.0, 200.0));
    }

    #[test]
    fn a_large_picture_can_be_dragged_a_little_past_its_edge() {
        // 900 tall in a 500 canvas: the edge is reached at 200, and 100 more is allowed.
        let c = View::clamp_offset(vec2(300.0, 1000.0), vec2(800.0, 900.0), CANVAS, 100.0);
        assert_eq!(c, vec2(0.0, 300.0), "the axis where the picture fits stays centred");
        let c = View::clamp_offset(vec2(0.0, -1000.0), vec2(800.0, 900.0), CANVAS, 100.0);
        assert_eq!(c, vec2(0.0, -300.0));
    }

    #[test]
    fn fit_width_starts_at_the_top() {
        let off = View::auto_offset(FitMode::FitWidth, vec2(1000.0, 3000.0), CANVAS);
        assert_eq!(off, vec2(0.0, 1250.0));
        assert_eq!(View::auto_offset(FitMode::Fill, vec2(1000.0, 3000.0), CANVAS), Vec2::ZERO);
    }

    #[test]
    fn turning_swaps_the_shown_size() {
        let mut v = View::default();
        assert_eq!(v.shown_size(40, 20), vec2(40.0, 20.0));
        v.rotate(true);
        assert_eq!(v.shown_size(40, 20), vec2(20.0, 40.0));
    }

    /// The pixel found under a screen corner must be the one whose texture coordinate
    /// is drawn at that corner, for every combination of turn and flip.
    #[test]
    fn picking_agrees_with_drawing() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 100.0));
        let inset = 0.01;
        let corners = [
            pos2(rect.min.x + inset, rect.min.y + inset),
            pos2(rect.max.x - inset, rect.min.y + inset),
            pos2(rect.max.x - inset, rect.max.y - inset),
            pos2(rect.min.x + inset, rect.max.y - inset),
        ];
        for turns in 0..4 {
            for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)] {
                let v = View { turns, flip_h, flip_v, ..View::default() };
                let uvs = v.uvs();
                for (corner, uv) in corners.iter().zip(uvs) {
                    let (px, py) = v.pixel_at(*corner, rect, 2, 2).unwrap();
                    assert_eq!((px, py), (uv.x as u32, uv.y as u32), "turns {turns} h {flip_h} v {flip_v}");
                }
            }
        }
    }

    #[test]
    fn rotate_right_is_clockwise() {
        let mut v = View::default();
        v.rotate(true);
        // After a clockwise quarter turn the image's bottom-left is drawn top-left.
        assert_eq!(v.uvs()[0], pos2(0.0, 1.0));
        v.rotate(false);
        assert_eq!(v, View::default());
    }
}
