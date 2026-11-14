//! The window's colours and its pale backdrop with fine ruled lines, drawn by hand: no pictures anywhere.

use eframe::egui::epaint::Mesh;
use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

pub const INK: Color32 = Color32::from_rgb(70, 74, 82);
pub const SOFT: Color32 = Color32::from_rgb(128, 134, 144);
pub const BLUE: Color32 = Color32::from_rgb(52, 190, 240);
pub const BLUE_INK: Color32 = Color32::from_rgb(20, 126, 186);
pub const BLUE_EDGE: Color32 = Color32::from_rgb(150, 212, 240);
pub const RED: Color32 = Color32::from_rgb(208, 76, 76);
pub const GREEN: Color32 = Color32::from_rgb(40, 158, 100);
/// A state that waits for the person.
pub const AMBER: Color32 = Color32::from_rgb(196, 128, 24);
const BACKDROP_TOP: Color32 = Color32::from_rgb(252, 253, 254);
const BACKDROP_BOTTOM: Color32 = Color32::from_rgb(232, 236, 240);

fn vertical_gradient(painter: &Painter, area: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(area.left_top(), top);
    mesh.colored_vertex(area.right_top(), top);
    mesh.colored_vertex(area.left_bottom(), bottom);
    mesh.colored_vertex(area.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// The pale backdrop with its fine ruled lines.
pub fn backdrop(painter: &Painter, area: Rect) {
    vertical_gradient(painter, area, BACKDROP_TOP, BACKDROP_BOTTOM);
    let mut y = area.top() + 3.0;
    while y < area.bottom() {
        painter.line_segment([Pos2::new(area.left(), y), Pos2::new(area.right(), y)], Stroke::new(1.0, Color32::from_black_alpha(5)));
        y += 4.0;
    }
}
