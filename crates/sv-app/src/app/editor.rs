//! Editing in the viewer: the tool panel, the crop frame, live previews, undo, and the
//! "Save as" window.

use super::*;
use sv_core::decode::raw::Linear;
use sv_core::edit::{self, Adjust, RawParams};
use sv_core::save::SaveFormat;

/// Edits are previewed on a copy no larger than this, so sliders respond instantly even
/// on a 50-megapixel photo. Applying works on the full picture.
const PROXY_SIDE: u32 = 1400;
const MAX_UNDO_STEPS: usize = 12;
const MAX_UNDO_BYTES: usize = 1536 * 1024 * 1024;

/// The picture being edited, with its history.
pub struct Session {
    /// The file it came from; `None` for a pasted picture.
    path: Option<PathBuf>,
    pub current: Arc<Ready>,
    undo: Vec<Arc<Ready>>,
    redo: Vec<Arc<Ready>>,
    pub dirty: bool,
}

impl Session {
    fn bitmap(&self) -> &sv_core::decode::Bitmap {
        &self.current.loaded.frames[0].bitmap
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tool {
    #[default]
    None,
    Crop,
    Straighten,
    Adjust,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Aspect {
    Free,
    Original,
    Fixed(f32, &'static str),
}

const ASPECTS: [Aspect; 9] = [
    Aspect::Free,
    Aspect::Original,
    Aspect::Fixed(1.0, "1 : 1"),
    Aspect::Fixed(4.0 / 3.0, "4 : 3"),
    Aspect::Fixed(3.0 / 2.0, "3 : 2"),
    Aspect::Fixed(16.0 / 9.0, "16 : 9"),
    Aspect::Fixed(3.0 / 4.0, "3 : 4"),
    Aspect::Fixed(2.0 / 3.0, "2 : 3"),
    Aspect::Fixed(9.0 / 16.0, "9 : 16"),
];

impl Aspect {
    fn label(self) -> &'static str {
        match self {
            Aspect::Free => "Free",
            Aspect::Original => "Same as picture",
            Aspect::Fixed(_, label) => label,
        }
    }

    fn ratio(self, width: u32, height: u32) -> Option<f32> {
        match self {
            Aspect::Free => None,
            Aspect::Original => Some(width as f32 / height.max(1) as f32),
            Aspect::Fixed(r, _) => Some(r),
        }
    }
}

/// Which part of the crop frame the mouse grabbed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grab {
    /// Moving the whole frame; holds the offset from the pointer to its top-left corner.
    Move(Vec2),
    /// Dragging edges: which of left / top / right / bottom follow the pointer.
    Edges { left: bool, top: bool, right: bool, bottom: bool },
}

struct Proxy {
    /// The picture this proxy was made from.
    of: Arc<Ready>,
    small: sv_core::decode::Bitmap,
    shown_for: Option<Adjust>,
    texture: Option<egui::TextureHandle>,
}

/// A RAW file developed from its sensor data, ready for the develop sliders.
struct RawState {
    of: PathBuf,
    linear: Arc<Linear>,
    /// A small copy the sliders preview on.
    small: Linear,
    params: RawParams,
    shown_for: Option<RawParams>,
    texture: Option<egui::TextureHandle>,
}

struct SaveAs {
    format: SaveFormat,
    quality: u8,
    /// Bytes of the original file, to carry its metadata into a JPEG.
    original: Option<Vec<u8>>,
    encoded: Option<Vec<u8>>,
    encoded_for: Option<(SaveFormat, u8)>,
    preview: bool,
    preview_texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

pub struct State {
    pub panel: bool,
    pub session: Option<Session>,
    tool: Tool,
    /// Crop frame in picture pixels: left, top, right, bottom.
    crop: [f32; 4],
    crop_aspect: Aspect,
    grab: Option<Grab>,
    angle: f32,
    adjust: Adjust,
    proxy: Option<Proxy>,
    resize: (u32, u32),
    resize_for: (u32, u32),
    resize_locked: bool,
    sharpen: f32,
    blur: f32,
    denoise: f32,
    job: Option<(Receiver<Result<Ready, String>>, &'static str)>,
    raw: Option<RawState>,
    raw_job: Option<(Receiver<Result<Linear, String>>, PathBuf)>,
    /// Esc pressed once with unsaved edits; a second press before this time discards them.
    discard_armed_until: f64,
    save_as: Option<SaveAs>,
}

impl Default for State {
    fn default() -> Self {
        State {
            panel: false,
            session: None,
            tool: Tool::None,
            crop: [0.0; 4],
            crop_aspect: Aspect::Free,
            grab: None,
            angle: 0.0,
            adjust: Adjust::default(),
            proxy: None,
            resize: (0, 0),
            resize_for: (0, 0),
            resize_locked: true,
            sharpen: 40.0,
            blur: 2.0,
            denoise: 40.0,
            job: None,
            raw: None,
            raw_job: None,
            discard_armed_until: 0.0,
            save_as: None,
        }
    }
}

impl State {
    pub fn is_dirty(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.dirty)
    }

    pub fn is_busy(&self) -> bool {
        self.job.is_some() || self.raw_job.is_some()
    }

    /// What is being worked on in the background, if anything.
    pub fn working_on(&self) -> Option<&'static str> {
        self.job.as_ref().map(|(_, label)| *label).or(self.raw_job.as_ref().map(|_| "Developing the RAW file"))
    }

    pub fn crop_active(&self) -> bool {
        self.tool == Tool::Crop
    }

    /// A rectangle is actually on screen (not just selection mode waiting for a drag).
    pub fn has_selection(&self) -> bool {
        self.tool == Tool::Crop && self.crop[2] >= 0.0
    }

    fn reset_tools(&mut self) {
        self.tool = Tool::None;
        self.grab = None;
        self.angle = 0.0;
        self.adjust = Adjust::default();
        if let Some(proxy) = &mut self.proxy {
            proxy.shown_for = None;
            proxy.texture = None;
        }
        if let Some(raw) = &mut self.raw {
            raw.params = RawParams::default();
            raw.shown_for = None;
            raw.texture = None;
        }
    }

    /// Forget the edit in progress entirely.
    pub fn end_session(&mut self) {
        self.session = None;
        self.proxy = None;
        self.job = None;
        self.raw = None;
        self.raw_job = None;
        self.save_as = None;
        self.reset_tools();
    }

    /// A stand-in texture for the picture: the adjustment preview, or the quality
    /// preview of the "Save as" window.
    pub fn preview_texture(&self) -> Option<egui::TextureId> {
        if let Some(texture) = self.save_as.as_ref().filter(|s| s.preview).and_then(|s| s.preview_texture.as_ref()) {
            return Some(texture.id());
        }
        if self.tool == Tool::Adjust {
            return self.proxy.as_ref().and_then(|p| p.texture.as_ref()).map(|t| t.id());
        }
        if self.tool == Tool::Raw {
            return self.raw.as_ref().and_then(|r| r.texture.as_ref()).map(|t| t.id());
        }
        None
    }

    /// Draw the picture for the straighten tool: turned by the chosen angle, with a
    /// frame showing what will be kept. Returns false when the normal drawing applies.
    pub fn paint_straighten(&self, painter: &egui::Painter, rect: Rect, texture: egui::TextureId, size: (u32, u32)) -> bool {
        if self.tool != Tool::Straighten || self.angle == 0.0 {
            return false;
        }
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let centre = rect.center();
        let turn = |p: egui::Pos2| {
            let d = p - centre;
            centre + vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
        };
        let mut mesh = egui::Mesh::with_texture(texture);
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        for (pos, uv) in corners.into_iter().zip(View::default().uvs()) {
            mesh.vertices.push(egui::epaint::Vertex { pos: turn(pos), uv, color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));

        let (keep_w, keep_h) = edit::straighten_keeps(size.0, size.1, self.angle);
        let kept = Rect::from_center_size(centre, vec2(rect.width() * keep_w, rect.height() * keep_h));
        shade_outside(painter, painter.clip_rect(), kept);
        guides(painter, kept, 6);
        true
    }

    /// A drag on the picture starts a fresh selection rectangle.
    pub fn begin_selection(&mut self) {
        if self.tool == Tool::None {
            self.tool = Tool::Crop;
            // Nowhere yet: the drag that follows places it under the pointer.
            self.crop = [-100.0; 4];
            self.grab = None;
        }
    }

    pub fn select_all(&mut self, width: u32, height: u32) {
        self.reset_tools();
        self.tool = Tool::Crop;
        self.crop = [0.0, 0.0, width as f32, height as f32];
    }

    pub fn clear_selection(&mut self) {
        if self.tool == Tool::Crop {
            self.reset_tools();
        }
    }

    fn start_crop(&mut self, width: u32, height: u32) {
        self.reset_tools();
        self.tool = Tool::Crop;
        let (w, h) = (width as f32, height as f32);
        self.crop = [w * 0.1, h * 0.1, w * 0.9, h * 0.9];
        self.fit_crop_to_aspect(width, height);
    }

    /// Reshape the frame around its centre to the chosen proportions.
    fn fit_crop_to_aspect(&mut self, width: u32, height: u32) {
        let Some(ratio) = self.crop_aspect.ratio(width, height) else { return };
        let [l, t, r, b] = self.crop;
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        let mut w = r - l;
        let mut h = w / ratio;
        let max_h = (height as f32).min(b - t).max(8.0);
        if h > max_h {
            h = max_h;
            w = h * ratio;
        }
        self.crop = [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0];
        self.clamp_crop(width, height);
    }

    fn clamp_crop(&mut self, width: u32, height: u32) {
        let (w, h) = (width as f32, height as f32);
        let [mut l, mut t, mut r, mut b] = self.crop;
        if l > r {
            std::mem::swap(&mut l, &mut r);
        }
        if t > b {
            std::mem::swap(&mut t, &mut b);
        }
        // Slide back inside rather than squash, so a moved frame keeps its size.
        let (fw, fh) = ((r - l).min(w), (b - t).min(h));
        l = l.clamp(0.0, w - fw);
        t = t.clamp(0.0, h - fh);
        self.crop = [l, t, l + fw.max(1.0), t + fh.max(1.0)];
    }

    /// Crop frame as whole pixels: x, y, width, height.
    fn crop_pixels(&self) -> (u32, u32, u32, u32) {
        let [l, t, r, b] = self.crop;
        let (x, y) = (l.round().max(0.0) as u32, t.round().max(0.0) as u32);
        (x, y, ((r - l).round() as u32).max(1), ((b - t).round() as u32).max(1))
    }

    /// Draw the crop frame over the picture and let the mouse reshape it.
    pub fn crop_ui(&mut self, response: &egui::Response, painter: &egui::Painter, rect: Rect, width: u32, height: u32, panning: bool) {
        if self.tool != Tool::Crop || rect.width() <= 0.0 {
            return;
        }
        // While Shift moves the picture, the frame only follows along.
        let response = &if panning { response.clone().interact(Sense::hover()) } else { response.clone() };
        let scale = rect.width() / width as f32;
        let to_screen = |x: f32, y: f32| rect.min + vec2(x, y) * scale;
        let to_picture = |p: egui::Pos2| (p - rect.min) / scale;
        let frame = |crop: [f32; 4]| Rect::from_min_max(to_screen(crop[0], crop[1]), to_screen(crop[2], crop[3]));

        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let on_screen = frame(self.crop);
                let reach = 10.0;
                let near = |a: f32, b: f32| (a - b).abs() <= reach;
                let inside_x = pos.x >= on_screen.left() - reach && pos.x <= on_screen.right() + reach;
                let inside_y = pos.y >= on_screen.top() - reach && pos.y <= on_screen.bottom() + reach;
                let left = near(pos.x, on_screen.left()) && inside_y;
                let right = near(pos.x, on_screen.right()) && inside_y && !left;
                let top = near(pos.y, on_screen.top()) && inside_x;
                let bottom = near(pos.y, on_screen.bottom()) && inside_x && !top;
                self.grab = Some(if left || right || top || bottom {
                    Grab::Edges { left, top, right, bottom }
                } else if on_screen.contains(pos) {
                    Grab::Move(to_picture(pos) - vec2(self.crop[0], self.crop[1]))
                } else {
                    // Outside the frame: start a new one from this point.
                    let p = to_picture(pos);
                    self.crop = [p.x, p.y, p.x, p.y];
                    Grab::Edges { left: false, top: false, right: true, bottom: true }
                });
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some(grab), Some(pos)) = (self.grab, response.interact_pointer_pos()) {
                let p = to_picture(pos);
                let (px, py) = (p.x.clamp(0.0, width as f32), p.y.clamp(0.0, height as f32));
                match grab {
                    Grab::Move(offset) => {
                        let (w, h) = (self.crop[2] - self.crop[0], self.crop[3] - self.crop[1]);
                        let (l, t) = (p.x - offset.x, p.y - offset.y);
                        self.crop = [l, t, l + w, t + h];
                    }
                    Grab::Edges { left, top, right, bottom } => {
                        if left {
                            self.crop[0] = px.min(self.crop[2] - 8.0);
                        }
                        if right {
                            self.crop[2] = px.max(self.crop[0] + 8.0);
                        }
                        if top {
                            self.crop[1] = py.min(self.crop[3] - 8.0);
                        }
                        if bottom {
                            self.crop[3] = py.max(self.crop[1] + 8.0);
                        }
                        if let Some(ratio) = self.crop_aspect.ratio(width, height) {
                            // Width leads when a side edge moves, height when only top or bottom does.
                            if left || right {
                                let h = (self.crop[2] - self.crop[0]) / ratio;
                                if top {
                                    self.crop[1] = self.crop[3] - h;
                                } else {
                                    self.crop[3] = self.crop[1] + h;
                                }
                            } else {
                                let w = (self.crop[3] - self.crop[1]) * ratio;
                                self.crop[2] = self.crop[0] + w;
                            }
                        }
                    }
                }
                self.clamp_crop(width, height);
            }
        }
        if response.drag_stopped() {
            self.grab = None;
            // A slip of the mouse is not a selection.
            if self.crop[2] - self.crop[0] < 4.0 || self.crop[3] - self.crop[1] < 4.0 {
                self.tool = Tool::None;
                return;
            }
        }
        if self.crop[2] < 0.0 {
            // Selection mode began but no drag has placed the frame yet.
            return;
        }

        let on_screen = frame(self.crop);
        shade_outside(painter, rect, on_screen);
        guides(painter, on_screen, 3);
        for corner in [on_screen.left_top(), on_screen.right_top(), on_screen.right_bottom(), on_screen.left_bottom()] {
            painter.rect_filled(Rect::from_center_size(corner, vec2(9.0, 9.0)), 1.0, Color32::WHITE);
        }
        let (_, _, w, h) = self.crop_pixels();
        badge(painter, on_screen.center_bottom() + vec2(0.0, -8.0), Align2::CENTER_BOTTOM, &format!("{w} × {h}"));
    }
}

/// Darken everything in `area` that lies outside `keep`.
fn shade_outside(painter: &egui::Painter, area: Rect, keep: Rect) {
    let shade = Color32::from_black_alpha(150);
    let keep = keep.intersect(area);
    painter.rect_filled(Rect::from_min_max(area.min, egui::pos2(area.max.x, keep.min.y)), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(egui::pos2(area.min.x, keep.max.y), area.max), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(egui::pos2(area.min.x, keep.min.y), egui::pos2(keep.min.x, keep.max.y)), 0.0, shade);
    painter.rect_filled(Rect::from_min_max(egui::pos2(keep.max.x, keep.min.y), egui::pos2(area.max.x, keep.max.y)), 0.0, shade);
}

/// A white frame with evenly spaced guide lines.
fn guides(painter: &egui::Painter, rect: Rect, divisions: usize) {
    let thin = egui::Stroke::new(1.0, Color32::from_white_alpha(90));
    for i in 1..divisions {
        let f = i as f32 / divisions as f32;
        let x = rect.left() + rect.width() * f;
        let y = rect.top() + rect.height() * f;
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], thin);
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], thin);
    }
    painter.rect_stroke(rect, 0.0, (1.5, Color32::WHITE), egui::StrokeKind::Inside);
}

impl App {
    /// Start editing the picture on screen, if that has not happened yet. A rotation
    /// that was only in the view becomes part of the picture.
    fn ensure_session(&mut self, ctx: &egui::Context) -> bool {
        if self.editor.session.is_some() {
            return true;
        }
        let Some(shown) = &self.shown else { return false };
        let loaded = &shown.ready.loaded;
        let frame = self.frame.min(loaded.frames.len() - 1);
        let turned = !self.view.turns.is_multiple_of(4) || self.view.flip_h || self.view.flip_v;
        let current = if !turned && loaded.frames.len() == 1 {
            shown.ready.clone()
        } else {
            let bitmap = loaded.frames[frame].bitmap.turned(self.view.turns, self.view.flip_h, self.view.flip_v);
            Arc::new(Ready::from_bitmap(ctx, bitmap, loaded.format))
        };
        self.view.reset_orientation();
        let path = self.current_path().filter(|_| self.pasted.is_none()).cloned();
        self.editor.session = Some(Session { path, current, undo: Vec::new(), redo: Vec::new(), dirty: turned });
        true
    }

    /// Run an edit on a background thread; the result arrives in `poll_edit`.
    fn start_edit(&mut self, ctx: &egui::Context, label: &'static str, work: impl FnOnce(&sv_core::decode::Bitmap) -> sv_core::decode::Bitmap + Send + 'static) {
        if self.editor.is_busy() || !self.ensure_session(ctx) {
            return;
        }
        let Some(session) = &self.editor.session else { return };
        let (source, ctx) = (session.current.clone(), ctx.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let format = source.loaded.format;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&source.loaded.frames[0].bitmap)))
                .map(|bitmap| Ready::from_bitmap(&ctx, bitmap, format))
                .map_err(|_| "the edit failed".to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.editor.job = Some((rx, label));
    }

    /// Read the RAW file's sensor data on a background thread.
    fn start_raw_develop(&mut self, ctx: &egui::Context) {
        if self.editor.is_busy() || !self.ensure_session(ctx) {
            return;
        }
        let Some(path) = self.editor.session.as_ref().and_then(|s| s.path.clone()) else { return };
        let (tx, rx) = mpsc::channel();
        let (source, ctx) = (path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = std::fs::read(&source).map_err(|e| e.to_string()).and_then(|data| sv_core::decode::raw::develop_linear(&data).map_err(|e| e.to_string()));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.editor.raw_job = Some((rx, path));
    }

    fn poll_raw(&mut self, now: f64) {
        let Some((rx, _)) = &self.editor.raw_job else { return };
        let Ok(result) = rx.try_recv() else { return };
        let Some((_, path)) = self.editor.raw_job.take() else { return };
        match result {
            Ok(linear) => {
                self.editor.reset_tools();
                let small = linear.downscaled(PROXY_SIDE);
                self.editor.raw = Some(RawState { of: path, linear: Arc::new(linear), small, params: RawParams::default(), shown_for: None, texture: None });
                self.editor.tool = Tool::Raw;
            }
            Err(e) => self.say(format!("Could not develop this RAW file: {e}"), now),
        }
    }

    /// Keep the preview of the RAW develop sliders up to date.
    fn refresh_raw_preview(&mut self, ctx: &egui::Context) {
        if self.editor.tool != Tool::Raw {
            return;
        }
        if let Some(raw) = &mut self.editor.raw {
            if raw.shown_for != Some(raw.params) {
                let b = edit::develop(&raw.small, &raw.params);
                let image = egui::ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba);
                raw.texture = Some(ctx.load_texture("raw-preview", image, egui::TextureOptions::LINEAR));
                raw.shown_for = Some(raw.params);
            }
        }
    }

    pub(super) fn poll_edit(&mut self, now: f64) {
        self.poll_raw(now);
        let Some((rx, label)) = &self.editor.job else { return };
        let label = *label;
        let Ok(result) = rx.try_recv() else { return };
        self.editor.job = None;
        match result {
            Ok(ready) => {
                let Some(session) = &mut self.editor.session else { return };
                let resized = (ready.loaded.width(), ready.loaded.height()) != (session.current.loaded.width(), session.current.loaded.height());
                let previous = std::mem::replace(&mut session.current, Arc::new(ready));
                session.undo.push(previous);
                session.redo.clear();
                session.dirty = true;
                // Old steps go when the history gets long or heavy.
                while session.undo.len() > MAX_UNDO_STEPS || session.undo.iter().map(|r| r.loaded.bytes()).sum::<usize>() > MAX_UNDO_BYTES {
                    session.undo.remove(0);
                }
                self.editor.reset_tools();
                if resized {
                    self.view.reset_placement();
                }
                self.say(format!("{label} applied"), now);
            }
            Err(e) => self.say(format!("{label}: {e}"), now),
        }
    }

    pub(super) fn undo(&mut self, now: f64) {
        if self.editor.is_busy() {
            return;
        }
        let Some(session) = &mut self.editor.session else { return };
        let Some(previous) = session.undo.pop() else {
            self.say("Nothing to undo".to_string(), now);
            return;
        };
        session.redo.push(std::mem::replace(&mut session.current, previous));
        session.dirty = true;
        self.editor.reset_tools();
        self.view.reset_placement();
    }

    pub(super) fn redo(&mut self, now: f64) {
        if self.editor.is_busy() {
            return;
        }
        let Some(session) = &mut self.editor.session else { return };
        let Some(next) = session.redo.pop() else {
            self.say("Nothing to redo".to_string(), now);
            return;
        };
        session.undo.push(std::mem::replace(&mut session.current, next));
        session.dirty = true;
        self.editor.reset_tools();
        self.view.reset_placement();
    }

    /// Rotation and flips become real edits once a picture is being edited.
    pub(super) fn edit_turn(&mut self, ctx: &egui::Context, turns: u8, flip_h: bool, flip_v: bool) {
        self.start_edit(ctx, "Rotate", move |b| b.turned(turns, flip_h, flip_v));
    }

    pub(super) fn toggle_crop(&mut self, ctx: &egui::Context) {
        if self.editor.tool == Tool::Crop {
            self.editor.reset_tools();
        } else if self.ensure_session(ctx) {
            if let Some(session) = &self.editor.session {
                let (w, h) = (session.bitmap().width, session.bitmap().height);
                self.editor.start_crop(w, h);
                self.editor.panel = true;
            }
        }
    }

    pub(super) fn apply_crop(&mut self, ctx: &egui::Context) {
        let (x, y, w, h) = self.editor.crop_pixels();
        self.start_edit(ctx, "Crop", move |b| edit::crop(b, x, y, w, h));
    }

    /// The picture as it looks on screen: with edits, and with a rotation that so far
    /// only exists in the view.
    fn displayed_bitmap(&self) -> Option<std::borrow::Cow<'_, sv_core::decode::Bitmap>> {
        if let Some(session) = &self.editor.session {
            return Some(std::borrow::Cow::Borrowed(session.bitmap()));
        }
        let shown = self.shown.as_ref()?;
        let frames = &shown.ready.loaded.frames;
        let bitmap = &frames[self.frame.min(frames.len() - 1)].bitmap;
        Some(if !self.view.turns.is_multiple_of(4) || self.view.flip_h || self.view.flip_v {
            std::borrow::Cow::Owned(bitmap.turned(self.view.turns, self.view.flip_h, self.view.flip_v))
        } else {
            std::borrow::Cow::Borrowed(bitmap)
        })
    }

    /// The selected rectangle at the picture's full resolution.
    fn selection_bitmap(&self) -> Option<sv_core::decode::Bitmap> {
        if !self.editor.has_selection() {
            return None;
        }
        let (x, y, w, h) = self.editor.crop_pixels();
        Some(edit::crop(self.displayed_bitmap()?.as_ref(), x, y, w, h))
    }

    /// Ctrl+C with a selection. Returns false when there is none.
    pub(super) fn copy_selection(&mut self, ctx: &egui::Context, now: f64) -> bool {
        let Some(b) = self.selection_bitmap() else { return false };
        ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba));
        self.say(format!("Selection copied ({} × {})", b.width, b.height), now);
        true
    }

    fn save_selection_as_jpg(&mut self, now: f64) {
        let Some(bitmap) = self.selection_bitmap() else { return };
        let source = self.current_path().filter(|_| self.pasted.is_none()).cloned();
        let original = source.as_ref().and_then(|p| std::fs::read(p).ok());
        let bytes = match save::encode(&bitmap, SaveFormat::Jpeg, 92, original.as_deref()) {
            Ok(bytes) => bytes,
            Err(e) => return self.say(format!("Could not save the selection: {e}"), now),
        };
        let stem = source.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".to_string());
        let mut picker = rfd::FileDialog::new().set_title("Save selection as JPG").set_file_name(format!("{stem}_selection.jpg")).add_filter("JPEG", &["jpg"]);
        if let Some(dir) = source.as_ref().and_then(|p| p.parent()).or(self.dir.as_deref()) {
            picker = picker.set_directory(dir);
        }
        let Some(target) = picker.save_file() else { return };
        match std::fs::write(&target, bytes) {
            Ok(()) => {
                self.say(format!("Saved {}", file_name(&target)), now);
                self.editor.clear_selection();
                self.relist_if_in_current_folder(&target);
            }
            Err(e) => self.say(format!("Could not save {}: {e}", file_name(&target)), now),
        }
    }

    /// Cut the file down to the selection and overwrite it, without asking.
    fn overwrite_with_selection(&mut self, ctx: &egui::Context, now: f64) {
        let Some(bitmap) = self.selection_bitmap() else { return };
        let Some(path) = self.current_path().filter(|_| self.pasted.is_none()).cloned() else {
            return self.say("A pasted picture has no file to overwrite. Use Save selection as JPG".to_string(), now);
        };
        match save::save_edited(&path, &bitmap) {
            Ok(()) => {
                self.thumbs.invalidate(&path);
                if let Ok(meta) = std::fs::metadata(&path) {
                    for list in [&mut self.entries, &mut self.all_entries] {
                        if let Some(e) = list.iter_mut().find(|e| e.path == path) {
                            e.size = meta.len();
                            e.modified = meta.modified().unwrap_or(e.modified);
                        }
                    }
                }
                self.loader.forget(&path);
                self.editor.end_session();
                self.view.reset_orientation();
                self.view.reset_placement();
                // The old, larger picture must not linger while the file is read back.
                self.shown = Some(Shown { key: self.current_key(), ready: Arc::new(Ready::from_bitmap(ctx, bitmap, Format::from_extension(&path).unwrap_or(Format::Png))) });
                self.say("Saved: the file now holds only the selection".to_string(), now);
            }
            Err(save::SaveError::ReadOnly(name)) => self.say(format!("{name} files cannot be overwritten. Use Save selection as JPG"), now),
            Err(e) => self.say(format!("Could not save {}: {e}", file_name(&path)), now),
        }
    }

    /// Google Lens has no way to take a local picture directly, so the selection goes to
    /// the clipboard and Lens opens ready for a paste. Nothing is sent until the user pastes.
    fn search_in_lens(&mut self, ctx: &egui::Context, now: f64) {
        if self.copy_selection(ctx, now) {
            ctx.open_url(egui::OpenUrl::new_tab("https://lens.google.com/"));
            self.say("Selection copied. Press Ctrl+V in the Google Lens page".to_string(), now + 4.0);
        }
    }

    fn zoom_to_selection(&mut self) {
        let Some(shown) = &self.shown else { return };
        let size = self.view.shown_size(shown.ready.loaded.width(), shown.ready.loaded.height());
        let [l, t, r, b] = self.editor.crop;
        let (canvas, ppp) = self.last_canvas;
        if r - l < 1.0 || b - t < 1.0 || canvas.x <= 0.0 {
            return;
        }
        let zoom = ((canvas.x * ppp / (r - l)).min(canvas.y * ppp / (b - t)) * 0.96).clamp(crate::viewer::MIN_ZOOM, crate::viewer::MAX_ZOOM);
        let centre = vec2((l + r) / 2.0 - size.x / 2.0, (t + b) / 2.0 - size.y / 2.0);
        self.view.zoom = Some(zoom);
        self.view.offset = -centre * zoom / ppp;
        self.editor.clear_selection();
    }

    fn relist_if_in_current_folder(&mut self, target: &Path) {
        if target.parent() == self.dir.as_deref() {
            self.relist = true;
        }
    }

    /// The right-click menu while a rectangle is selected.
    pub(super) fn selection_menu(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>, now: f64) {
        enlarge_menu(ui);
        let ctx = ui.ctx().clone();
        let (_, _, w, h) = self.editor.crop_pixels();
        ui.weak(format!("Selection  {w} × {h}"));
        let item = |ui: &mut egui::Ui, label: &str, keys: &str| {
            let clicked = ui.add(egui::Button::new(label).shortcut_text(keys)).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        if item(ui, "Copy selection", "Ctrl+C") {
            self.copy_selection(&ctx, now);
        }
        if item(ui, "Save selection as JPG…", "") {
            self.save_selection_as_jpg(now);
        }
        if item(ui, "Save (overwrite the file with the selection)", "") {
            self.overwrite_with_selection(&ctx, now);
        }
        if item(ui, "Search in Google Lens", "") {
            self.search_in_lens(&ctx, now);
        }
        ui.separator();
        if item(ui, "Crop to selection", "Enter") {
            self.apply_crop(&ctx);
        }
        if item(ui, "Zoom to selection", "") {
            self.zoom_to_selection();
        }
        if item(ui, "Blur selection", "") {
            let (x, y, w, h) = self.editor.crop_pixels();
            self.start_edit(&ctx, "Blur selection", move |b| edit::blur_region(b, x, y, w, h));
        }
        ui.separator();
        if item(ui, "Select all", "") {
            if let Some(shown) = &self.shown {
                let size = self.view.shown_size(shown.ready.loaded.width(), shown.ready.loaded.height());
                self.editor.select_all(size.x as u32, size.y as u32);
            }
        }
        if item(ui, "Clear selection", "Esc") {
            self.editor.clear_selection();
        }
        ui.separator();
        ui.menu_button("Picture menu", |ui| self.menu(ui, commands, now));
    }

    /// Esc while editing. Returns true when the key was used up here.
    pub(super) fn edit_back(&mut self, now: f64) -> bool {
        if self.editor.save_as.is_some() {
            self.editor.save_as = None;
        } else if self.editor.tool != Tool::None {
            self.editor.reset_tools();
        } else if self.editor.is_dirty() {
            if now < self.editor.discard_armed_until {
                self.editor.end_session();
                self.say("Edits discarded".to_string(), now);
            } else {
                self.editor.discard_armed_until = now + 3.0;
                self.say("Unsaved edits. Press Esc again to discard them".to_string(), now);
            }
        } else if self.editor.panel {
            self.editor.panel = false;
            self.editor.end_session();
        } else {
            return false;
        }
        true
    }

    /// Ctrl+S with edits: write them over the file, no questions asked.
    pub(super) fn save_edits(&mut self, ctx: &egui::Context, now: f64) {
        let Some(session) = &self.editor.session else { return };
        let Some(path) = session.path.clone() else {
            self.open_save_as(ctx, now);
            return;
        };
        match save::save_edited(&path, session.bitmap()) {
            Ok(()) => {
                self.thumbs.invalidate(&path);
                if let Ok(meta) = std::fs::metadata(&path) {
                    for list in [&mut self.entries, &mut self.all_entries] {
                        if let Some(e) = list.iter_mut().find(|e| e.path == path) {
                            e.size = meta.len();
                            e.modified = meta.modified().unwrap_or(e.modified);
                        }
                    }
                }
                self.loader.forget(&path);
                // The edited picture stays on screen until the saved file is read back.
                self.editor.end_session();
                self.say("Saved".to_string(), now);
            }
            Err(save::SaveError::ReadOnly(_)) | Err(save::SaveError::MultiFrame) => {
                self.say("This file type cannot be overwritten. Choose a new file".to_string(), now);
                self.open_save_as(ctx, now);
            }
            Err(e) => self.say(format!("Could not save {}: {e}", file_name(&path)), now),
        }
    }

    pub(super) fn open_save_as(&mut self, ctx: &egui::Context, _now: f64) {
        if !self.ensure_session(ctx) {
            return;
        }
        let Some(session) = &self.editor.session else { return };
        let original = session.path.as_ref().and_then(|p| std::fs::read(p).ok());
        self.editor.save_as = Some(SaveAs {
            format: SaveFormat::for_source(session.current.loaded.format),
            quality: 85,
            original,
            encoded: None,
            encoded_for: None,
            preview: false,
            preview_texture: None,
            error: None,
        });
    }

    pub(super) fn edit_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        let ctx = ui.ctx().clone();
        let busy = self.editor.is_busy();
        let (can_undo, can_redo, dirty) = match &self.editor.session {
            Some(s) => (!s.undo.is_empty(), !s.redo.is_empty(), s.dirty),
            None => (false, false, false),
        };
        let size = self.editor.session.as_ref().map(|s| (s.bitmap().width, s.bitmap().height)).or_else(|| {
            self.shown.as_ref().map(|s| {
                let v = self.view.shown_size(s.ready.loaded.width(), s.ready.loaded.height());
                (v.x as u32, v.y as u32)
            })
        });
        let Some((width, height)) = size else {
            ui.weak("Open a picture to edit it.");
            return;
        };

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    if ui.add_enabled(can_undo, egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
                        self.undo(now);
                    }
                    if ui.add_enabled(can_redo, egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
                        self.redo(now);
                    }
                    if ui.add_enabled(dirty, egui::Button::new("Discard all")).clicked() {
                        self.editor.end_session();
                    }
                });
                ui.label(format!("{width} × {height}{}", if dirty { "   ·   edited, not saved" } else { "" }));
                ui.separator();

                if ui.button("Auto-fix").on_hover_text("Stretch flat, hazy tones to full contrast").clicked() {
                    self.start_edit(&ctx, "Auto-fix", edit::auto_fix);
                }
                ui.separator();

                ui.strong("Crop");
                let cropping = self.editor.tool == Tool::Crop;
                ui.horizontal(|ui| {
                    if ui.selectable_label(cropping, "Crop frame").on_hover_text("C").clicked() {
                        self.toggle_crop(&ctx);
                    }
                    let mut aspect = self.editor.crop_aspect;
                    egui::ComboBox::from_id_salt("crop_aspect").selected_text(aspect.label()).show_ui(ui, |ui| {
                        for a in ASPECTS {
                            ui.selectable_value(&mut aspect, a, a.label());
                        }
                    });
                    if aspect != self.editor.crop_aspect {
                        self.editor.crop_aspect = aspect;
                        if cropping {
                            self.editor.fit_crop_to_aspect(width, height);
                        }
                    }
                });
                if cropping {
                    ui.weak("Drag the frame or its edges. Drag outside it for a new frame.");
                    ui.horizontal(|ui| {
                        if ui.button("Apply crop").on_hover_text("Enter").clicked() {
                            self.apply_crop(&ctx);
                        }
                        if ui.button("Cancel").on_hover_text("Esc").clicked() {
                            self.editor.reset_tools();
                        }
                    });
                }
                ui.separator();

                ui.strong("Resize");
                if self.editor.resize_for != (width, height) {
                    self.editor.resize_for = (width, height);
                    self.editor.resize = (width, height);
                }
                let (mut w, mut h) = self.editor.resize;
                ui.horizontal(|ui| {
                    let w_changed = ui.add(egui::DragValue::new(&mut w).range(1..=60_000).suffix(" px")).changed();
                    ui.label("×");
                    let h_changed = ui.add(egui::DragValue::new(&mut h).range(1..=60_000).suffix(" px")).changed();
                    if self.editor.resize_locked {
                        if w_changed {
                            h = ((w as f64 * height as f64 / width as f64).round() as u32).max(1);
                        } else if h_changed {
                            w = ((h as f64 * width as f64 / height as f64).round() as u32).max(1);
                        }
                    }
                });
                ui.checkbox(&mut self.editor.resize_locked, "Keep proportions");
                ui.horizontal(|ui| {
                    for (label, factor) in [("25%", 0.25), ("50%", 0.5), ("75%", 0.75), ("200%", 2.0)] {
                        if ui.small_button(label).clicked() {
                            w = ((width as f64 * factor).round() as u32).max(1);
                            h = ((height as f64 * factor).round() as u32).max(1);
                        }
                    }
                });
                self.editor.resize = (w, h);
                if ui.add_enabled((w, h) != (width, height), egui::Button::new("Apply resize")).clicked() {
                    self.start_edit(&ctx, "Resize", move |b| edit::resize(b, w, h));
                }
                ui.separator();

                ui.strong("Straighten");
                let mut angle = self.editor.angle;
                if ui.add(egui::Slider::new(&mut angle, -20.0..=20.0).step_by(0.1).suffix("°")).changed() {
                    if self.editor.tool != Tool::Straighten {
                        self.editor.reset_tools();
                        self.editor.tool = Tool::Straighten;
                        self.ensure_session(&ctx);
                    }
                    self.editor.angle = angle;
                }
                if self.editor.tool == Tool::Straighten {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(angle != 0.0, egui::Button::new("Apply straighten")).clicked() {
                            self.start_edit(&ctx, "Straighten", move |b| edit::straighten(b, angle));
                        }
                        if ui.button("Cancel").clicked() {
                            self.editor.reset_tools();
                        }
                    });
                }
                ui.separator();

                let raw_file = self.editor.session.as_ref().and_then(|s| s.path.clone()).or_else(|| self.current_path().filter(|_| self.pasted.is_none()).cloned());
                let is_raw = raw_file.as_ref().is_some_and(|p| Format::from_extension(p) == Some(Format::Raw));
                if is_raw {
                    ui.strong("RAW develop");
                    let loaded = self.editor.raw.as_ref().is_some_and(|r| Some(&r.of) == raw_file.as_ref());
                    if !loaded {
                        ui.weak("The picture on screen is the camera's own JPEG. Developing reads the sensor data instead, which takes a few seconds and allows cleaner exposure and colour changes.");
                        if ui.button("Develop from sensor data").clicked() {
                            self.start_raw_develop(&ctx);
                        }
                    } else if let Some(raw) = &mut self.editor.raw {
                        let mut p = raw.params;
                        ui.add(egui::Slider::new(&mut p.exposure, -3.0..=3.0).step_by(0.05).text("Exposure (stops)"));
                        ui.add(egui::Slider::new(&mut p.temperature, -100.0..=100.0).step_by(1.0).text("Warmth"));
                        ui.add(egui::Slider::new(&mut p.tint, -100.0..=100.0).step_by(1.0).text("Tint (green / magenta)"));
                        ui.add(egui::Slider::new(&mut p.highlights, 0.0..=100.0).step_by(1.0).text("Recover highlights"));
                        let changed = p != raw.params;
                        raw.params = p;
                        let linear = raw.linear.clone();
                        if changed && self.editor.tool != Tool::Raw {
                            self.editor.tool = Tool::Raw;
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Apply develop").on_hover_text("Use the developed picture, with these settings").clicked() {
                                self.editor.tool = Tool::Raw;
                                self.start_edit(&ctx, "RAW develop", move |_| edit::develop(&linear, &p));
                            }
                            if self.editor.tool == Tool::Raw && ui.button("Cancel").clicked() {
                                self.editor.reset_tools();
                            }
                        });
                        ui.weak("RAW files are never overwritten: save the result with Save as.");
                    }
                    ui.separator();
                }

                ui.strong("Light and colour");
                let mut adjust = self.editor.adjust;
                let slider = |ui: &mut egui::Ui, value: &mut f32, label: &str| {
                    ui.add(egui::Slider::new(value, -100.0..=100.0).step_by(1.0).text(label));
                };
                slider(ui, &mut adjust.brightness, "Brightness");
                slider(ui, &mut adjust.contrast, "Contrast");
                ui.add(egui::Slider::new(&mut adjust.gamma, 0.3..=3.0).step_by(0.01).text("Gamma"));
                slider(ui, &mut adjust.saturation, "Saturation");
                slider(ui, &mut adjust.red, "Red");
                slider(ui, &mut adjust.green, "Green");
                slider(ui, &mut adjust.blue, "Blue");
                if adjust != self.editor.adjust {
                    if self.editor.tool != Tool::Adjust {
                        self.editor.reset_tools();
                        self.editor.tool = Tool::Adjust;
                        self.ensure_session(&ctx);
                    }
                    self.editor.adjust = adjust;
                }
                if self.editor.tool == Tool::Adjust {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!adjust.is_neutral(), egui::Button::new("Apply")).clicked() {
                            self.start_edit(&ctx, "Light and colour", move |b| adjust.apply(b));
                        }
                        if ui.button("Reset").clicked() {
                            self.editor.reset_tools();
                        }
                    });
                }
                ui.separator();

                ui.strong("Sharpen, blur, noise");
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.editor.sharpen, 1.0..=100.0).step_by(1.0).text("Sharpen"));
                    if ui.button("Apply").clicked() {
                        let amount = self.editor.sharpen;
                        self.start_edit(&ctx, "Sharpen", move |b| edit::sharpen(b, amount));
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.editor.blur, 0.3..=20.0).step_by(0.1).text("Blur"));
                    if ui.button("Apply").clicked() {
                        let sigma = self.editor.blur;
                        self.start_edit(&ctx, "Blur", move |b| edit::blur(b, sigma));
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.editor.denoise, 1.0..=100.0).step_by(1.0).text("Reduce noise"));
                    if ui.button("Apply").clicked() {
                        let strength = self.editor.denoise;
                        self.start_edit(&ctx, "Reduce noise", move |b| edit::denoise(b, strength));
                    }
                });
                ui.weak("These three show their result after Apply; Undo takes it back.");
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.add_enabled(dirty, egui::Button::new("Save")).on_hover_text("Overwrite the file (Ctrl+S)").clicked() {
                        self.save_edits(&ctx, now);
                    }
                    if ui.button("Save as…").on_hover_text("Ctrl+Shift+S").clicked() {
                        self.open_save_as(&ctx, now);
                    }
                });
            });
            if let Some((_, label)) = &self.editor.job {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{label}…"));
                });
            }
        });
        self.refresh_adjust_preview(&ctx);
        self.refresh_raw_preview(&ctx);
    }

    /// Keep the small preview picture of the "Light and colour" sliders up to date.
    fn refresh_adjust_preview(&mut self, ctx: &egui::Context) {
        if self.editor.tool != Tool::Adjust {
            return;
        }
        let Some(session) = &self.editor.session else { return };
        let stale = self.editor.proxy.as_ref().is_none_or(|p| !Arc::ptr_eq(&p.of, &session.current));
        if stale {
            let full = session.bitmap();
            let longest = full.width.max(full.height);
            let small = if longest > PROXY_SIDE {
                let scale = PROXY_SIDE as f64 / longest as f64;
                edit::resize(full, (full.width as f64 * scale).round() as u32, (full.height as f64 * scale).round() as u32)
            } else {
                sv_core::decode::Bitmap { width: full.width, height: full.height, rgba: full.rgba.clone() }
            };
            self.editor.proxy = Some(Proxy { of: session.current.clone(), small, shown_for: None, texture: None });
        }
        let adjust = self.editor.adjust;
        if let Some(proxy) = &mut self.editor.proxy {
            if proxy.shown_for != Some(adjust) {
                let b = adjust.apply(&proxy.small);
                let image = egui::ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba);
                proxy.texture = Some(ctx.load_texture("adjust-preview", image, egui::TextureOptions::LINEAR));
                proxy.shown_for = Some(adjust);
            }
        }
    }

    pub(super) fn save_as_window(&mut self, ctx: &egui::Context, now: f64) {
        if self.editor.save_as.is_none() {
            return;
        }
        let Some(session) = &self.editor.session else {
            self.editor.save_as = None;
            return;
        };
        let current = session.current.clone();
        let source_path = session.path.clone();
        let bitmap = &current.loaded.frames[0].bitmap;
        let mut open = true;
        let mut save_clicked = false;
        let dragging = ctx.input(|i| i.pointer.any_down());

        egui::Window::new("Save as").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            let Some(dialog) = &mut self.editor.save_as else { return };
            ui.horizontal(|ui| {
                ui.label("Format");
                egui::ComboBox::from_id_salt("save_format").selected_text(dialog.format.name()).show_ui(ui, |ui| {
                    for f in SaveFormat::ALL {
                        ui.selectable_value(&mut dialog.format, f, f.name());
                    }
                });
            });
            if dialog.format.has_quality() {
                ui.add(egui::Slider::new(&mut dialog.quality, 1..=100).text("Quality"));
            } else {
                ui.weak("This format keeps every pixel exactly.");
            }

            // Encode when the settings have settled, so the size shown is the real one.
            let wanted = (dialog.format, if dialog.format.has_quality() { dialog.quality } else { 0 });
            if dialog.encoded_for != Some(wanted) && !dragging {
                dialog.encoded_for = Some(wanted);
                dialog.preview_texture = None;
                match save::encode(bitmap, dialog.format, dialog.quality, dialog.original.as_deref()) {
                    Ok(bytes) => {
                        dialog.encoded = Some(bytes);
                        dialog.error = None;
                    }
                    Err(e) => {
                        dialog.encoded = None;
                        dialog.error = Some(e.to_string());
                    }
                }
            }
            match (&dialog.encoded, &dialog.error) {
                (_, Some(error)) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                (Some(bytes), None) if dialog.encoded_for == Some(wanted) => {
                    ui.label(format!("{} × {}   ·   file size {}", bitmap.width, bitmap.height, human_size(bytes.len() as u64)));
                }
                _ => {
                    ui.label("Release the slider to see the file size");
                }
            }
            if dialog.format.has_quality() {
                ui.checkbox(&mut dialog.preview, "Show the picture at this quality").on_hover_text("Zoom in to judge the loss before saving");
                if dialog.preview && dialog.preview_texture.is_none() {
                    if let Some(bytes) = &dialog.encoded {
                        let format = if dialog.format == SaveFormat::Jpeg { Format::Jpeg } else { Format::WebP };
                        if let Ok(loaded) = sv_core::decode::load_bytes(bytes, format, 0) {
                            let b = &loaded.frames[0].bitmap;
                            let image = egui::ColorImage::from_rgba_unmultiplied([b.width as usize, b.height as usize], &b.rgba);
                            dialog.preview_texture = Some(ctx.load_texture("quality-preview", image, egui::TextureOptions::LINEAR));
                        }
                    }
                }
            } else {
                dialog.preview = false;
            }
            ui.add_space(6.0);
            save_clicked = ui.add_enabled(dialog.encoded.is_some(), egui::Button::new("Save…")).clicked();
        });

        if save_clicked {
            let Some(dialog) = &self.editor.save_as else { return };
            let (Some(bytes), format) = (dialog.encoded.clone(), dialog.format) else { return };
            let stem = source_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "picture".to_string());
            let mut picker = rfd::FileDialog::new()
                .set_title("Save as")
                .set_file_name(format!("{stem}.{}", format.extension()))
                .add_filter(format.name(), &[format.extension()]);
            if let Some(dir) = source_path.as_ref().and_then(|p| p.parent()).or(self.dir.as_deref()) {
                picker = picker.set_directory(dir);
            }
            if let Some(target) = picker.save_file() {
                match std::fs::write(&target, &bytes) {
                    Ok(()) => {
                        self.say(format!("Saved {}", file_name(&target)), now);
                        self.editor.save_as = None;
                        if let Some(session) = &mut self.editor.session {
                            session.dirty = false;
                        }
                        // A new file in the folder being shown should appear in the list.
                        self.relist_if_in_current_folder(&target);
                    }
                    Err(e) => self.say(format!("Could not save {}: {e}", file_name(&target)), now),
                }
            }
        } else if !open {
            self.editor.save_as = None;
        }
    }
}
