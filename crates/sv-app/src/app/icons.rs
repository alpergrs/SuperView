//! Small line-drawn icons for the thumbnail view's top bar. They are drawn rather than
//! taken from a font, so they look the same everywhere and follow the light or dark theme.

use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Sense, Shape, Stroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Up,
    CopyTo,
    MoveTo,
    Delete,
    MarkAll,
    Unmark,
    Batch,
    Rename,
    Info,
    Search,
    Duplicates,
    Folders,
    Preview,
}

/// A square button showing `icon`. `selected` draws it pressed in, for on/off switches.
pub fn button(ui: &mut egui::Ui, icon: Icon, selected: bool) -> egui::Response {
    let side = ui.spacing().interact_size.y * 1.45;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.visuals();
    let painter = ui.painter();
    let radius = side * 0.18;
    if selected {
        painter.rect_filled(rect, radius, visuals.selection.bg_fill);
    } else if response.hovered() {
        painter.rect_filled(rect, radius, visuals.widgets.hovered.weak_bg_fill);
    }
    if response.has_focus() {
        painter.rect_stroke(rect, radius, visuals.selection.stroke, egui::StrokeKind::Inside);
    }
    let ink = if selected { visuals.selection.stroke.color } else { visuals.text_color() };
    // Behind overlapping outlines, so the one in front hides the one behind.
    let paper = if selected { visuals.selection.bg_fill } else if response.hovered() { visuals.widgets.hovered.weak_bg_fill } else { visuals.panel_fill };
    draw(painter, rect.shrink(side * 0.2), icon, ink, paper);
    response
}

fn draw(painter: &egui::Painter, area: Rect, icon: Icon, ink: Color32, paper: Color32) {
    let at = |x: f32, y: f32| -> Pos2 { pos2(area.left() + x * area.width(), area.top() + y * area.height()) };
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    let line_width = (area.width() / 11.0).max(1.2);
    let stroke = Stroke::new(line_width, ink);
    let line = |points: &[(f32, f32)]| {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
        painter.add(Shape::line(points, stroke));
    };
    let outline = |points: &[(f32, f32)]| {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
        painter.add(Shape::closed_line(points, stroke));
    };
    // A rectangle that covers what lies behind it.
    let sheet = |r: Rect| {
        painter.rect_filled(r, line_width, paper);
        painter.rect_stroke(r, line_width, stroke, egui::StrokeKind::Middle);
    };
    let folder = || outline(&[(0.05, 0.22), (0.4, 0.22), (0.48, 0.32), (0.95, 0.32), (0.95, 0.85), (0.05, 0.85)]);

    match icon {
        Icon::Up => {
            line(&[(0.5, 0.88), (0.5, 0.14)]);
            line(&[(0.2, 0.44), (0.5, 0.14), (0.8, 0.44)]);
        }
        Icon::CopyTo => {
            sheet(rect(0.08, 0.32, 0.62, 0.92));
            sheet(rect(0.38, 0.08, 0.92, 0.68));
        }
        Icon::MoveTo => {
            folder();
            line(&[(0.25, 0.6), (0.72, 0.6)]);
            line(&[(0.55, 0.45), (0.72, 0.6), (0.55, 0.75)]);
        }
        Icon::Delete => {
            line(&[(0.12, 0.22), (0.88, 0.22)]);
            line(&[(0.38, 0.22), (0.38, 0.08), (0.62, 0.08), (0.62, 0.22)]);
            outline(&[(0.22, 0.3), (0.78, 0.3), (0.72, 0.94), (0.28, 0.94)]);
            line(&[(0.42, 0.42), (0.42, 0.82)]);
            line(&[(0.58, 0.42), (0.58, 0.82)]);
        }
        Icon::MarkAll => {
            painter.rect_stroke(rect(0.08, 0.08, 0.92, 0.92), line_width * 1.5, stroke, egui::StrokeKind::Middle);
            line(&[(0.26, 0.52), (0.43, 0.7), (0.76, 0.32)]);
        }
        Icon::Unmark => {
            painter.rect_stroke(rect(0.08, 0.08, 0.92, 0.92), line_width * 1.5, stroke, egui::StrokeKind::Middle);
            line(&[(0.3, 0.3), (0.7, 0.7)]);
            line(&[(0.7, 0.3), (0.3, 0.7)]);
        }
        Icon::Batch => {
            sheet(rect(0.04, 0.46, 0.6, 0.96));
            sheet(rect(0.22, 0.25, 0.78, 0.75));
            sheet(rect(0.4, 0.04, 0.96, 0.54));
        }
        Icon::Rename => {
            let font = egui::FontId::proportional(area.height() * 0.78);
            painter.text(at(0.0, 0.55), egui::Align2::LEFT_CENTER, "ab", font, ink);
            line(&[(0.86, 0.12), (0.86, 0.92)]);
            line(&[(0.74, 0.12), (0.98, 0.12)]);
            line(&[(0.74, 0.92), (0.98, 0.92)]);
        }
        Icon::Info => {
            painter.circle_stroke(at(0.5, 0.5), area.width() * 0.45, stroke);
            painter.circle_filled(at(0.5, 0.3), line_width * 0.9, ink);
            line(&[(0.5, 0.45), (0.5, 0.74)]);
        }
        Icon::Search => {
            painter.circle_stroke(at(0.42, 0.42), area.width() * 0.3, stroke);
            painter.line_segment([at(0.64, 0.64), at(0.92, 0.92)], Stroke::new(line_width * 1.6, ink));
        }
        Icon::Duplicates => {
            for x in [0.04, 0.54] {
                painter.rect_stroke(rect(x, 0.2, x + 0.42, 0.8), line_width, stroke, egui::StrokeKind::Middle);
                line(&[(x + 0.06, 0.72), (x + 0.18, 0.5), (x + 0.28, 0.64), (x + 0.36, 0.56)]);
            }
        }
        Icon::Folders => {
            folder();
        }
        Icon::Preview => {
            painter.rect_stroke(rect(0.04, 0.16, 0.96, 0.84), line_width, stroke, egui::StrokeKind::Middle);
            painter.rect_filled(rect(0.58, 0.16, 0.96, 0.84), line_width, ink);
        }
    }
}
