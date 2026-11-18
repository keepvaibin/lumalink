//! The pieces LumaLink's windows are made of, drawn in the manner of a console's home screen: soft tiles, a curved
//! bar along the bottom with round buttons set into it, large rounded buttons, cards, a spinner and small icons.
//! Every shape is drawn here; there are no pictures.

use crate::sky::{self, BLUE, INK};
use eframe::egui::epaint::Mesh;
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, FontFamily, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType,
};
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

/// The tiles' and buttons' face: white at the top, a pale grey at the bottom.
pub const FACE_TOP: Color32 = Color32::from_rgb(255, 255, 255);
pub const FACE_BOTTOM: Color32 = Color32::from_rgb(230, 233, 237);
/// The grey edge of a tile or a button at rest.
pub const EDGE: Color32 = Color32::from_rgb(196, 202, 209);
/// An empty slot.
pub const SLOT: Color32 = Color32::from_rgb(234, 237, 240);
pub const SLOT_EDGE: Color32 = Color32::from_rgb(212, 217, 222);
/// The bar along the bottom.
const BAR_TOP: Color32 = Color32::from_rgb(244, 245, 247);
const BAR_BOTTOM: Color32 = Color32::from_rgb(214, 219, 224);
/// The bar's edge line.
pub const BAR_LINE: Color32 = Color32::from_rgb(86, 196, 238);
/// The light ring of a round button at rest.
const RING: Color32 = Color32::from_rgb(160, 218, 242);
/// The clock's and the quiet words' grey.
pub const CLOCK: Color32 = Color32::from_rgb(132, 138, 146);

/// The rounded font at `size`.
pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// Its bold weight.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// `a` to `b` by `t` (0 to 1), on premultiplied channels.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// `c` at `alpha` (0 to 1) of its own opacity.
pub fn faded(c: Color32, alpha: f32) -> Color32 {
    c.gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// The outline of `rect` with corners of radius `r`, clockwise from the top edge's right end.
pub fn rounded(rect: Rect, r: f32) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let steps = ((r * 0.5).ceil() as usize).clamp(3, 20);
    let corners = [
        (rect.right_top() + vec2(-r, r), -PI / 2.0),
        (rect.right_bottom() + vec2(-r, -r), 0.0),
        (rect.left_bottom() + vec2(r, -r), PI / 2.0),
        (rect.left_top() + vec2(r, r), PI),
    ];
    let mut points = Vec::with_capacity(4 * (steps + 1));
    for (centre, start) in corners {
        for i in 0..=steps {
            let a = start + PI / 2.0 * i as f32 / steps as f32;
            points.push(centre + vec2(a.cos(), a.sin()) * r);
        }
    }
    points
}

/// A circle's outline.
pub fn circle(centre: Pos2, r: f32) -> Vec<Pos2> {
    let n = ((r * 0.8).ceil() as usize).clamp(16, 96);
    (0..n).map(|i| centre + Vec2::angled(TAU * i as f32 / n as f32) * r).collect()
}

/// A shape seen from its centre (each edge visible from it: convex shapes, stars), filled from `top` at its highest
/// point to `bottom` at its lowest, its edge softened over a pixel.
pub fn fill(painter: &Painter, points: &[Pos2], top: Color32, bottom: Color32) {
    if points.len() < 3 {
        return;
    }
    let (y0, y1) = points.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let shade = |y: f32| mix(top, bottom, (y - y0) / (y1 - y0).max(1.0));
    let n = points.len();
    let centre = points.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / n as f32;
    let centre = centre.to_pos2();
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, shade(centre.y));
    for i in 0..n {
        let (before, here, after) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let along = (after - before).normalized();
        let mut out = vec2(along.y, -along.x);
        if out.dot(here - centre) < 0.0 {
            out = -out;
        }
        let c = shade(here.y);
        mesh.colored_vertex(here - out * 0.5, c);
        mesh.colored_vertex(here + out * 0.5, Color32::TRANSPARENT);
    }
    for i in 0..n as u32 {
        let (a, b) = (1 + 2 * i, 1 + 2 * ((i + 1) % n as u32));
        mesh.add_triangle(0, a, b);
        mesh.add_triangle(a, a + 1, b);
        mesh.add_triangle(b, a + 1, b + 1);
    }
    painter.add(Shape::mesh(mesh));
}

/// A closed outline.
pub fn outline(painter: &Painter, points: &[Pos2], stroke: Stroke) {
    let mut closed = points.to_vec();
    if let Some(first) = points.first() {
        closed.push(*first);
    }
    painter.add(Shape::line(closed, stroke));
}

/// A soft shadow under a rounded rectangle.
pub fn shadow(painter: &Painter, rect: Rect, r: f32, strength: f32) {
    let shadow = egui::Shadow { offset: [0, 3], blur: 14, spread: 0, color: Color32::from_black_alpha((26.0 * strength) as u8) };
    painter.add(shadow.as_shape(rect, r));
}

/// The blue glow of what the pointer or the keyboard is on, `t` from 0 (none) to 1.
pub fn glow(painter: &Painter, points: &[Pos2], centre: Pos2, t: f32) {
    if t <= 0.0 {
        return;
    }
    for (k, alpha) in [(4.5, 0.10), (3.0, 0.20), (1.5, 0.38)] {
        let grown: Vec<Pos2> = points.iter().map(|p| *p + (*p - centre).normalized() * k).collect();
        outline(painter, &grown, Stroke::new(2.0, faded(BLUE, alpha * t)));
    }
    outline(painter, points, Stroke::new(3.0, faded(BLUE, t)));
}

/// `text` laid out in `width` aligned by `halign`, in at most `rows` rows. Text that does not fit is cut at a character
/// and ends in three full stops, which every font draws on the line (the ellipsis character is raised in the rounded
/// font).
pub fn fitted(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32, rows: usize, halign: Align) -> Arc<Galley> {
    let lay = |t: String| {
        let mut job = LayoutJob::simple(t, font.clone(), color, width.max(1.0));
        job.wrap.max_rows = rows;
        job.wrap.overflow_character = None;
        job.halign = halign;
        painter.layout_job(job)
    };
    let whole = lay(text.to_string());
    if !whole.elided {
        return whole;
    }
    // The longest start of the text that fits with the stops.
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).collect();
    let (mut low, mut high) = (0usize, ends.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if lay(format!("{}...", text[..ends[mid - 1]].trim_end())).elided {
            high = mid - 1;
        } else {
            low = mid;
        }
    }
    let kept = if low == 0 { "" } else { text[..ends[low - 1]].trim_end() };
    lay(format!("{kept}..."))
}

/// `text` on one row in `width`, its start cut away when it does not fit (a path keeps its file's name).
pub fn fitted_tail(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let lay = |t: String| painter.layout_no_wrap(t, font.clone(), color);
    let whole = lay(text.to_string());
    if whole.size().x <= width {
        return whole;
    }
    let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let (mut low, mut high) = (1usize, starts.len());
    while low < high {
        let mid = (low + high) / 2;
        if lay(format!("...{}", &text[starts[mid]..])).size().x <= width {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    lay(format!("...{}", &text[starts.get(low).copied().unwrap_or(text.len())..]))
}

/// `text` laid out centred in `width`, at most `rows` rows.
pub fn centred_text(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32, rows: usize) -> Arc<Galley> {
    fitted(painter, text, font, color, width, rows, Align::Center)
}

/// A button's words in `width`: on one row at `size`, or as much smaller as they need down to three quarters of it;
/// past that on two rows at that size. Laid out centred: drawn at the button's middle.
pub fn label_fit(painter: &Painter, text: &str, size: f32, color: Color32, width: f32) -> Arc<Galley> {
    fit_text(painter, text, bold(size), color, width, Align::Center)
}

/// `text` in `width` in `font`: on one row at its size, or as much smaller as it needs down to three quarters of it;
/// past that on two rows at that size, cut when even those are not enough.
pub fn fit_text(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32, halign: Align) -> Arc<Galley> {
    let least = (font.size * 0.75).max(11.0).floor();
    let mut s = font.size.round();
    loop {
        let mut job = LayoutJob::simple(text.to_string(), FontId::new(s, font.family.clone()), color, width.max(1.0));
        job.wrap.max_rows = 1;
        job.halign = halign;
        let galley = painter.layout_job(job);
        if !galley.elided {
            return galley;
        }
        if s <= least {
            break;
        }
        s -= 1.0;
    }
    fitted(painter, text, FontId::new(least, font.family), color, width, 2, halign)
}

/// A clickable area named for screen readers; the pointer is a hand over it.
pub fn hit(ui: &mut Ui, rect: Rect, id: Id, label: &str, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// How far the pointer or the keyboard is on `response`, eased over a short time (0 to 1).
pub fn lit(ui: &Ui, response: &Response) -> f32 {
    let on = response.hovered() || response.has_focus();
    ui.ctx().animate_bool_with_time(response.id.with("lit"), on, 0.14)
}

// ---- the bar ----------------------------------------------------------------------------------------------------

/// The bar along the bottom of `area`: its edge at `top` at the sides, `dip` lower across the middle, curving up
/// around each round button set into it (`sockets`: centre and radius).
pub fn bar(painter: &Painter, area: Rect, top: f32, dip: f32, sockets: &[(Pos2, f32)]) {
    let width = area.width().min(area.height() * 16.0 / 9.0);
    let middle = area.center().x;
    let (flat, ramp) = (width * 0.16, width * 0.11);
    let edge = |x: f32| {
        let d = (x - middle).abs();
        let lowered = if d <= flat {
            1.0
        } else if d >= flat + ramp {
            0.0
        } else {
            0.5 * (1.0 + (PI * (d - flat) / ramp).cos())
        };
        let mut y = top + dip * lowered;
        for (centre, r) in sockets {
            let dx = x - centre.x;
            if dx.abs() < *r {
                let around = centre.y - (r * r - dx * dx).sqrt();
                // A smooth minimum: the edge flows into the curve around the button.
                let k = 14.0;
                let h = (0.5 + 0.5 * (y - around) / k).clamp(0.0, 1.0);
                y = y + (around - y) * h - k * h * (1.0 - h);
            }
        }
        y
    };
    let mut line = Vec::new();
    let mut x = area.left();
    while x < area.right() + 2.0 {
        let at = x.min(area.right());
        line.push(pos2(at, edge(at)));
        x += 2.0;
    }
    let highest = line.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let bottom = area.bottom();
    let shade = |y: f32| mix(BAR_TOP, BAR_BOTTOM, (y - highest) / (bottom - highest).max(1.0));
    // Its shadow on what is above it.
    for (k, alpha) in [(6.0, 4u8), (4.0, 7), (2.0, 11)] {
        let raised: Vec<Pos2> = line.iter().map(|p| *p - vec2(0.0, k)).collect();
        painter.add(Shape::line(raised, Stroke::new(3.0, Color32::from_black_alpha(alpha))));
    }
    let mut mesh = Mesh::default();
    for p in &line {
        mesh.colored_vertex(*p, shade(p.y));
        mesh.colored_vertex(pos2(p.x, bottom), BAR_BOTTOM);
    }
    for i in 0..(line.len() as u32).saturating_sub(1) {
        let (a, b) = (2 * i, 2 * i + 2);
        mesh.add_triangle(a, a + 1, b);
        mesh.add_triangle(a + 1, b, b + 1);
    }
    painter.add(Shape::mesh(mesh));
    // Fine ruled lines, as on the backdrop, below the edge.
    let mut y = highest + 3.0;
    while y < bottom {
        let mut runs: Vec<(f32, f32)> = Vec::new();
        let mut start = None;
        for p in &line {
            match (p.y + 2.0 < y, start) {
                (true, None) => start = Some(p.x),
                (false, Some(s)) => {
                    runs.push((s, p.x));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push((s, area.right()));
        }
        for (a, b) in runs {
            painter.line_segment([pos2(a, y), pos2(b, y)], Stroke::new(1.0, Color32::from_black_alpha(6)));
        }
        y += 4.0;
    }
    let below: Vec<Pos2> = line.iter().map(|p| *p + vec2(0.0, 2.0)).collect();
    painter.add(Shape::line(below, Stroke::new(1.5, Color32::from_white_alpha(220))));
    painter.add(Shape::line(line, Stroke::new(2.5, BAR_LINE)));
}

/// A round button set into the bar, named `label`, with `icon` drawn in it. Its tip is `label`.
pub fn round_button(ui: &mut Ui, id: Id, centre: Pos2, r: f32, label: &str, icon: Icon) -> Response {
    let rect = Rect::from_center_size(centre, Vec2::splat(2.0 * r));
    let response = hit(ui, rect, id, label, true);
    let t = lit(ui, &response);
    let pressed = response.is_pointer_button_down_on();
    let painter = ui.painter();
    painter.circle_filled(centre + vec2(0.0, 2.5), r + 1.0, Color32::from_black_alpha(22));
    let face = circle(centre, r);
    let (top, bottom) = if pressed { (FACE_BOTTOM, FACE_TOP) } else { (FACE_TOP, FACE_BOTTOM) };
    fill(painter, &face, top, bottom);
    outline(painter, &face, Stroke::new(1.5, EDGE));
    painter.circle_stroke(centre, r - 5.5, Stroke::new(3.0, mix(RING, BLUE, t)));
    glow(painter, &circle(centre, r + 1.5), centre, t);
    paint_icon(painter, icon, centre, r * 0.95, mix(sky::BLUE_INK, BLUE, t), Color32::WHITE);
    response.on_hover_text(label)
}

/// A large rounded button in `rect` (Menu, Start and the like); `enabled` false greys it.
pub fn big_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, enabled: bool) -> Response {
    let response = hit(ui, rect, id, label, enabled);
    let t = if enabled { lit(ui, &response) } else { 0.0 };
    let pressed = enabled && response.is_pointer_button_down_on();
    let painter = ui.painter();
    let r = rect.height() * 0.34;
    shadow(painter, rect, r, 1.0);
    let face = rounded(rect, r);
    let (top, bottom) = if pressed { (FACE_BOTTOM, FACE_TOP) } else { (FACE_TOP, FACE_BOTTOM) };
    fill(painter, &face, top, bottom);
    outline(painter, &face, Stroke::new(1.5, EDGE));
    glow(painter, &face, rect.center(), t);
    let color = if enabled { mix(INK, sky::BLUE_INK, t) } else { faded(INK, 0.35) };
    let galley = label_fit(painter, label, rect.height() * 0.32, color, rect.width() - 28.0);
    let top = rect.center().y - galley.size().y / 2.0;
    painter.galley(pos2(rect.center().x, top), galley, color);
    response
}

/// A turning arc: something is working.
pub fn spinner(painter: &Painter, centre: Pos2, r: f32, color: Color32, time: f64) {
    let start = (time * 4.5) as f32 % TAU;
    let points: Vec<Pos2> = (0..=28).map(|i| centre + Vec2::angled(start + 4.4 * i as f32 / 28.0) * r).collect();
    painter.add(Shape::line(points, Stroke::new((r * 0.3).max(2.0), color)));
}

/// A small round mark on a tile: how its last job went.
#[derive(Clone, Copy, PartialEq)]
pub enum Mark {
    Good,
    Warn,
    Bad,
}

pub fn mark(painter: &Painter, centre: Pos2, r: f32, mark: Mark) {
    let color = match mark {
        Mark::Good => sky::GREEN,
        Mark::Warn => sky::AMBER,
        Mark::Bad => sky::RED,
    };
    painter.circle_filled(centre + vec2(0.0, 1.5), r + 1.0, Color32::from_black_alpha(30));
    painter.circle_filled(centre, r + 1.5, Color32::WHITE);
    painter.circle_filled(centre, r, color);
    let w = Stroke::new(r * 0.28, Color32::WHITE);
    let s = r * 0.5;
    match mark {
        Mark::Good => {
            painter.add(Shape::line(vec![centre + vec2(-s, 0.05 * r), centre + vec2(-0.15 * r, s * 0.9), centre + vec2(s, -s * 0.8)], w));
        }
        Mark::Warn => {
            painter.line_segment([centre + vec2(0.0, -s), centre + vec2(0.0, s * 0.25)], w);
            painter.circle_filled(centre + vec2(0.0, s * 0.85), r * 0.15, Color32::WHITE);
        }
        Mark::Bad => {
            painter.line_segment([centre + vec2(-s, -s) * 0.8, centre + vec2(s, s) * 0.8], w);
            painter.line_segment([centre + vec2(s, -s) * 0.8, centre + vec2(-s, s) * 0.8], w);
        }
    }
}

// ---- icons --------------------------------------------------------------------------------------------------------

/// The small pictures drawn on tiles and buttons.
#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    /// Settings: a gear.
    Gear,
    /// Add.
    Plus,
    /// A mod folder.
    Folder,
    /// A packed mod: a box with the sparkle on it.
    Package,
    /// A rigged model: a figure.
    Figure,
    /// A published mod to convert: a zipped archive.
    Archive,
}

/// LumaLink's sparkle: an astroid, `r` from its centre to a point.
pub fn sparkle(painter: &Painter, centre: Pos2, r: f32, color: Color32) {
    let points: Vec<Pos2> = (0..72)
        .map(|i| {
            let t = TAU * i as f32 / 72.0;
            let (c, s) = (t.cos(), t.sin());
            centre + vec2(c * c * c, s * s * s) * r
        })
        .collect();
    fill(painter, &points, color, color);
}

/// The gear's outline for a gear of radius 1 (a tooth's top), `GEAR_POINTS` points at equal steps along it, the first
/// on the tooth pointing right, going round by increasing angle. The shape: a disk of radius 0.74 and 8 teeth, each a
/// box 0.33 wide from 0.6 to 1 along its axis with corners rounded to 0.07, joined to the disk by fillets of 0.1 (a
/// polynomial smooth minimum of the distances). The outline is where that distance is 0, found along each ray from the
/// centre (the shape is star-shaped: each ray leaves it once), then walked at equal steps so every curve has its points.
const GEAR_POINTS: usize = 1024;
fn gear_outline() -> &'static [Vec2] {
    static OUTLINE: std::sync::OnceLock<Vec<Vec2>> = std::sync::OnceLock::new();
    OUTLINE.get_or_init(|| {
        let distance = |x: f64, y: f64| {
            let mut d = (x * x + y * y).sqrt() - 0.74;
            for k in 0..8 {
                let a = std::f64::consts::TAU * k as f64 / 8.0;
                let (u, v) = (x * a.cos() + y * a.sin(), -x * a.sin() + y * a.cos());
                let (corner, (qx, qy)) = (0.07, ((u - 0.8).abs() - 0.2 + 0.07, v.abs() - 0.165 + 0.07));
                let tooth = (qx.max(0.0).hypot(qy.max(0.0))) + qx.max(qy).min(0.0) - corner;
                let k = 0.1;
                let h = (k - (d - tooth).abs()).max(0.0) / k;
                d = d.min(tooth) - h * h * k * 0.25;
            }
            d
        };
        // The edge along a fine fan of rays, then its length walked in equal steps.
        let rays = 16 * GEAR_POINTS;
        let fine: Vec<(f64, f64)> = (0..=rays)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / rays as f64;
                let (c, s) = (a.cos(), a.sin());
                let (mut inside, mut outside) = (0.3, 1.2);
                for _ in 0..32 {
                    let m = 0.5 * (inside + outside);
                    if distance(m * c, m * s) < 0.0 { inside = m } else { outside = m }
                }
                let r = 0.5 * (inside + outside);
                (r * c, r * s)
            })
            .collect();
        let mut along = vec![0.0f64; fine.len()];
        for i in 1..fine.len() {
            along[i] = along[i - 1] + (fine[i].0 - fine[i - 1].0).hypot(fine[i].1 - fine[i - 1].1);
        }
        let total = along[fine.len() - 1];
        let mut out = Vec::with_capacity(GEAR_POINTS);
        let mut j = 0;
        for i in 0..GEAR_POINTS {
            let want = total * i as f64 / GEAR_POINTS as f64;
            while along[j + 1] < want {
                j += 1;
            }
            let t = (want - along[j]) / (along[j + 1] - along[j]).max(1e-12);
            let (x, y) = (fine[j].0 + (fine[j + 1].0 - fine[j].0) * t, fine[j].1 + (fine[j + 1].1 - fine[j].1) * t);
            out.push(vec2(x as f32, y as f32));
        }
        out
    })
}

/// A gear of 8 teeth around `centre`, `r` from its centre to a tooth's top (`gear_outline`): a ring whose middle is a
/// round hole of 0.32 r (transparent: the face shows through). Its teeth point right, down, left and up, so it is the
/// same mirrored about both axes through `centre`. Both edges are softened over a pixel.
pub fn gear(centre: Pos2, r: f32, color: Color32) -> Mesh {
    let hole = 0.32 * r;
    let unit = gear_outline();
    let n = unit.len();
    let outer: Vec<Pos2> = unit.iter().map(|p| centre + *p * r).collect();
    let mut mesh = Mesh::default();
    for i in 0..n {
        let dir = unit[i].normalized();
        let (before, here, after) = (outer[(i + n - 1) % n], outer[i], outer[(i + 1) % n]);
        let along = (after - before).normalized();
        let mut out = vec2(along.y, -along.x);
        if out.dot(dir) < 0.0 {
            out = -out;
        }
        // Four vertices a sample: the outer edge inside and outside, the hole's edge inside the ring and in the hole.
        mesh.colored_vertex(here - out * 0.5, color);
        mesh.colored_vertex(here + out * 0.5, Color32::TRANSPARENT);
        mesh.colored_vertex(centre + dir * (hole + 0.5), color);
        mesh.colored_vertex(centre + dir * (hole - 0.5), Color32::TRANSPARENT);
    }
    for i in 0..n as u32 {
        let (a, b) = (4 * i, 4 * ((i + 1) % n as u32));
        // The ring.
        mesh.add_triangle(a, b, a + 2);
        mesh.add_triangle(a + 2, b, b + 2);
        // The outer edge's soft pixel.
        mesh.add_triangle(a, a + 1, b);
        mesh.add_triangle(b, a + 1, b + 1);
        // The hole's soft pixel.
        mesh.add_triangle(a + 2, a + 3, b + 2);
        mesh.add_triangle(b + 2, a + 3, b + 3);
    }
    mesh
}

/// `icon` in a square `size` wide around `centre`, in `color`; `cut` is the colour of what is cut out of it.
pub fn paint_icon(painter: &Painter, icon: Icon, centre: Pos2, size: f32, color: Color32, cut: Color32) {
    let s = size / 2.0;
    match icon {
        Icon::Gear => {
            painter.add(Shape::mesh(gear(centre, 0.8 * s, color)));
        }
        Icon::Plus => {
            let (long, thick) = (0.84 * s, 0.15 * size);
            for bar in [vec2(2.0 * long, thick), vec2(thick, 2.0 * long)] {
                painter.rect_filled(Rect::from_center_size(centre, bar), thick / 2.0, color);
            }
        }
        Icon::Folder => {
            let tab = Rect::from_min_max(centre + vec2(-0.7, -0.52) * s, centre + vec2(-0.08, -0.22) * s);
            painter.rect_filled(tab, 0.1 * s, color);
            let body = Rect::from_min_max(centre + vec2(-0.7, -0.36) * s, centre + vec2(0.7, 0.5) * s);
            fill(painter, &rounded(body, 0.14 * s), color, color);
            painter.line_segment([centre + vec2(-0.7, -0.2) * s, centre + vec2(0.7, -0.2) * s], Stroke::new(0.06 * s, faded(cut, 0.55)));
        }
        Icon::Package => {
            let body = Rect::from_center_size(centre, Vec2::splat(1.2 * s));
            fill(painter, &rounded(body, 0.22 * s), color, color);
            sparkle(painter, centre, 0.42 * s, cut);
        }
        Icon::Figure => {
            painter.circle_filled(centre + vec2(0.0, -0.38) * s, 0.24 * s, color);
            let body = Rect::from_min_max(centre + vec2(-0.42, -0.06) * s, centre + vec2(0.42, 0.62) * s);
            let mut shape = rounded(body, 0.36 * s);
            // A flat foot: the lower corners squarer.
            for p in &mut shape {
                p.y = p.y.min(centre.y + 0.62 * s);
            }
            fill(painter, &shape, color, color);
        }
        Icon::Archive => {
            let body = Rect::from_min_max(centre + vec2(-0.5, -0.62) * s, centre + vec2(0.5, 0.62) * s);
            fill(painter, &rounded(body, 0.14 * s), color, color);
            for i in 0..5 {
                let y = -0.5 + 0.17 * i as f32;
                let x = if i % 2 == 0 { -0.06 } else { 0.06 };
                let tooth = Rect::from_center_size(centre + vec2(x, y) * s, vec2(0.16, 0.1) * s);
                painter.rect_filled(tooth, 0.03 * s, cut);
            }
            let pull = Rect::from_center_size(centre + vec2(0.0, 0.22) * s, vec2(0.26, 0.3) * s);
            fill(painter, &rounded(pull, 0.08 * s), cut, cut);
            painter.rect_filled(Rect::from_center_size(pull.center(), vec2(0.1, 0.12) * s), 0.02 * s, color);
        }
    }
}

/// A card: white, rounded, with a soft edge and shadow, for `egui::Frame`.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 245))
        .stroke(Stroke::new(1.0, Color32::from_rgb(214, 219, 224)))
        .corner_radius(20.0)
        .inner_margin(22.0)
        .shadow(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(22) })
}

/// The quiet pale tile behind an empty slot.
pub fn empty_slot(painter: &Painter, rect: Rect, r: f32) {
    let shape = rounded(rect, r);
    fill(painter, &shape, faded(SLOT, 0.85), faded(Color32::from_rgb(226, 229, 233), 0.85));
    outline(painter, &shape, Stroke::new(1.5, SLOT_EDGE));
    let inner = rounded(rect.shrink(2.0), (r - 2.0).max(1.0));
    outline(painter, &inner, Stroke::new(1.0, Color32::from_white_alpha(170)));
}

/// A flat bar along the top of a full screen: the bottom bar's colours and edge, turned over.
pub fn header(painter: &Painter, rect: Rect) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), BAR_BOTTOM);
    mesh.colored_vertex(rect.right_top(), BAR_BOTTOM);
    mesh.colored_vertex(rect.left_bottom(), BAR_TOP);
    mesh.colored_vertex(rect.right_bottom(), BAR_TOP);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));
    for (k, alpha) in [(6.0, 4u8), (4.0, 7), (2.0, 11)] {
        painter.line_segment([rect.left_bottom() + vec2(0.0, k), rect.right_bottom() + vec2(0.0, k)], Stroke::new(3.0, Color32::from_black_alpha(alpha)));
    }
    painter.line_segment([rect.left_bottom() - vec2(0.0, 2.0), rect.right_bottom() - vec2(0.0, 2.0)], Stroke::new(1.5, Color32::from_white_alpha(220)));
    painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.5, BAR_LINE));
}

/// A check mark `size` wide around `centre`.
pub fn check(painter: &Painter, centre: Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    let points = vec![centre + vec2(-0.8, 0.0) * s, centre + vec2(-0.25, 0.55) * s, centre + vec2(0.8, -0.6) * s];
    painter.add(Shape::line(points, Stroke::new(0.26 * size, color)));
}

/// What ends a settings row.
pub enum End {
    /// Nothing: the row is an action.
    Nothing,
    /// A value, in its colour.
    Value(String, Color32),
    /// A switch, on or off.
    Switch(bool),
    /// A check mark: the row is the one chosen.
    Chosen,
}

/// A settings row of `size`: a white rounded face, `words` bold at the left with `detail` under them in small soft
/// words, and `end` at the right. Clicked: the row's action (a switch's owner flips it). `enabled` false: the row only
/// shows its value.
pub fn row(ui: &mut Ui, size: Vec2, id: Id, words: &str, detail: Option<&str>, end: End, enabled: bool) -> Response {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = match &end {
        End::Switch(on) if enabled => {
            let response = ui.interact(rect, id, Sense::click());
            response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, *on, words));
            response.on_hover_cursor(CursorIcon::PointingHand)
        }
        End::Chosen if enabled => {
            let response = ui.interact(rect, id, Sense::click());
            response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, true, words));
            response.on_hover_cursor(CursorIcon::PointingHand)
        }
        _ => hit(ui, rect, id, words, enabled),
    };
    let t = if enabled { lit(ui, &response) } else { 0.0 };
    let pressed = enabled && response.is_pointer_button_down_on();
    let painter = ui.painter();
    // The corners, padding and words follow a one-line row's height (a row with detail is 20 points taller), so rows
    // with and without detail match.
    let line = if detail.is_some() { size.y - 20.0 } else { size.y }.min(64.0);
    let r = (line * 0.32).round();
    shadow(painter, rect, r, 0.8);
    let face = rounded(rect, r);
    let (top, bottom) = if pressed { (FACE_BOTTOM, FACE_TOP) } else { (FACE_TOP, Color32::from_rgb(242, 244, 247)) };
    fill(painter, &face, top, bottom);
    let chosen = matches!(end, End::Chosen);
    outline(painter, &face, Stroke::new(2.0, if chosen { BLUE } else { sky::BLUE_EDGE }));
    glow(painter, &face, rect.center(), t);
    // Words and end, a padding in from each side and a gap between them.
    let pad = (line * 0.45).round();
    let inner = rect.width() - 2.0 * pad;
    let font = bold((line * 0.3).clamp(16.0, 19.0).round());
    let end_width = match &end {
        End::Nothing => 0.0,
        // A value has the room the words leave, and at least 30% of the row.
        End::Value(..) => (inner - pad - painter.layout_no_wrap(words.to_string(), font.clone(), INK).size().x).max(inner * 0.3),
        End::Switch(_) => 44.0,
        End::Chosen => line * 0.36,
    };
    let words_width = inner - if end_width > 0.0 { end_width + pad } else { 0.0 };
    let ink = if enabled || matches!(end, End::Value(..)) { INK } else { faded(INK, 0.45) };
    let words_galley = fit_text(painter, words, font, ink, words_width, Align::Min);
    let detail_galley = detail.map(|d| fitted(painter, d, body((line * 0.24).clamp(13.0, 14.0).round()), sky::SOFT, words_width, 2, Align::Min));
    // The words, and the detail 4 points under them, stand together in the row's middle.
    let block = words_galley.size().y + detail_galley.as_ref().map_or(0.0, |g| 4.0 + g.size().y);
    let mut y = (rect.center().y - block / 2.0).round();
    let words_height = words_galley.size().y;
    painter.galley(pos2(rect.left() + pad, y), words_galley, ink);
    y += words_height + 4.0;
    if let Some(galley) = detail_galley {
        painter.galley(pos2(rect.left() + pad, y), galley, sky::SOFT);
    }
    let right = rect.right() - pad;
    match end {
        End::Nothing => {}
        End::Value(value, color) => {
            let galley = fitted_tail(painter, &value, body((line * 0.26).clamp(14.0, 16.0).round()), color, end_width);
            painter.galley(pos2(right - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, color);
        }
        End::Switch(on) => {
            let on = ui.ctx().animate_bool_with_time(id.with("on"), on, 0.15);
            let track = Rect::from_min_size(pos2(right - end_width, rect.center().y - 12.0), vec2(end_width, 24.0));
            switch(ui.painter(), track, on, 0.0);
        }
        End::Chosen => check(painter, pos2(right - end_width / 2.0, rect.center().y), end_width, sky::BLUE_INK),
    }
    response
}

/// The width a big button needs for `label` at `height`: its words and a padding each side, at least `least`.
pub fn big_button_width(painter: &Painter, label: &str, height: f32, least: f32) -> f32 {
    let galley = painter.layout_no_wrap(label.to_string(), bold(height * 0.32), INK);
    (galley.size().x + height * 1.1).max(least).round()
}

/// The button that does what the screen is for (Install, Open): `big_button` with a blue face and white words; red
/// when `danger` (Uninstall).
pub fn primary_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, enabled: bool, danger: bool) -> Response {
    let response = hit(ui, rect, id, label, enabled);
    let t = if enabled { lit(ui, &response) } else { 0.0 };
    let pressed = enabled && response.is_pointer_button_down_on();
    let painter = ui.painter();
    let r = rect.height() * 0.34;
    shadow(painter, rect, r, 1.0);
    let face = rounded(rect, r);
    let (light, deep) = if danger { (Color32::from_rgb(232, 112, 112), sky::RED) } else { (Color32::from_rgb(92, 206, 246), Color32::from_rgb(28, 160, 222)) };
    let (light, deep) = if enabled { (light, deep) } else { (faded(light, 0.4), faded(deep, 0.4)) };
    let (top, bottom) = if pressed { (deep, light) } else { (mix(light, Color32::WHITE, 0.12 * t), mix(deep, light, 0.2 * t)) };
    fill(painter, &face, top, bottom);
    outline(painter, &face, Stroke::new(1.5, mix(deep, Color32::BLACK, 0.12)));
    // A gloss on the upper half, as on the tiles.
    let inner = rounded(rect.shrink(2.0), (r - 2.0).max(1.0));
    outline(painter, &inner, Stroke::new(1.0, Color32::from_white_alpha(if enabled { 90 } else { 40 })));
    let galley = label_fit(painter, label, rect.height() * 0.32, Color32::WHITE, rect.width() - 28.0);
    let top = rect.center().y - galley.size().y / 2.0;
    painter.galley(pos2(rect.center().x, top), galley, Color32::WHITE);
    response
}

/// A progress bar in `rect`: a rounded slot filled in blue up to `fraction` (0 to 1).
pub fn progress(painter: &Painter, rect: Rect, fraction: f32) {
    let r = rect.height() / 2.0;
    let slot = rounded(rect, r);
    fill(painter, &slot, Color32::from_rgb(222, 226, 231), SLOT);
    outline(painter, &slot, Stroke::new(1.0, SLOT_EDGE));
    let width = rect.width() * fraction.clamp(0.0, 1.0);
    if width >= 1.0 {
        // The fill keeps its round ends however short it is: at least as wide as it is tall, clipped to the slot.
        let filled = Rect::from_min_size(rect.min, vec2(width.max(rect.height()), rect.height()));
        let clipped = painter.with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.left() + width, rect.bottom())));
        let shape = rounded(filled, r);
        fill(&clipped, &shape, Color32::from_rgb(92, 206, 246), Color32::from_rgb(28, 160, 222));
        outline(&clipped, &rounded(filled.shrink(1.5), (r - 1.5).max(1.0)), Stroke::new(1.0, Color32::from_white_alpha(80)));
    }
}

/// LumaLink's badge: the program's icon (icon.rs) drawn at `size` around `centre`: the rounded square, sky blue to deep
/// blue, and its two sparkles.
pub fn badge(painter: &Painter, centre: Pos2, size: f32) {
    let unit = |x: f32, y: f32| centre + vec2(x - 0.5, y - 0.5) * size;
    let rect = Rect::from_min_max(unit(0.04, 0.04), unit(0.96, 0.96));
    let r = 0.22 * size;
    shadow(painter, rect, r, 0.9);
    fill(painter, &rounded(rect, r), BLUE, sky::BLUE_INK);
    // The icon's sparkle: |dx/r|^0.5 + |dy/r|^0.5 = 1, so x = r cos^4 t and y = r sin^4 t.
    for (x, y, k) in [(0.46, 0.55, 0.33), (0.74, 0.26, 0.13)] {
        let points: Vec<Pos2> = (0..96)
            .map(|i| {
                let t = TAU * i as f32 / 96.0;
                let (c, s) = (t.cos(), t.sin());
                unit(x, y) + vec2(c.signum() * c.powi(4), s.signum() * s.powi(4)) * k * size
            })
            .collect();
        fill(painter, &points, Color32::WHITE, Color32::WHITE);
    }
}

/// A switch's track and knob in `rect`: `on` from 0 (off) to 1, `lit` the pointer's glow.
pub fn switch(painter: &Painter, rect: Rect, on: f32, lit: f32) {
    let r = rect.height() / 2.0;
    painter.rect_filled(rect, r, mix(Color32::from_rgb(214, 219, 224), BLUE, on));
    let knob = pos2(rect.left() + r + on * (rect.width() - 2.0 * r), rect.center().y);
    painter.circle_filled(knob + vec2(0.0, 1.0), r - 2.0, Color32::from_black_alpha(30));
    painter.circle_filled(knob, r - 2.5, Color32::WHITE);
    if lit > 0.0 {
        painter.rect_stroke(rect.expand(2.0), r + 2.0, Stroke::new(2.0, faded(BLUE, 0.5 * lit)), StrokeKind::Outside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mesh's coverage (0 to 1) on a `size` x `size` grid of pixels, 4 x 4 samples a pixel, from its vertices' alpha.
    fn coverage(mesh: &Mesh, size: usize) -> Vec<f32> {
        // 4x4 samples a pixel; each sample takes the first triangle (in the mesh's order) that holds it. Triangle by
        // triangle over its own box of samples, so the cost is the triangles' area, not pixels times triangles.
        let side = size * 4;
        let mut sample = vec![None::<f32>; side * side];
        for t in mesh.indices.chunks(3) {
            let t = [t[0] as usize, t[1] as usize, t[2] as usize];
            let [a, b, c] = t.map(|i| mesh.vertices[i].pos);
            let d = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
            if d.abs() < 1e-9 {
                continue;
            }
            let alpha = t.map(|i| mesh.vertices[i].color.a() as f32 / 255.0);
            let range = |lo: f32, hi: f32| ((lo * 4.0 - 0.5).floor().max(0.0) as usize, ((hi * 4.0 - 0.5).ceil().max(0.0) as usize).min(side - 1));
            let (x0, x1) = range(a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
            let (y0, y1) = range(a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
            for sy in y0..=y1 {
                for sx in x0..=x1 {
                    let slot = &mut sample[sy * side + sx];
                    if slot.is_some() {
                        continue;
                    }
                    let p = pos2((sx as f32 + 0.5) / 4.0, (sy as f32 + 0.5) / 4.0);
                    let u = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / d;
                    let v = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / d;
                    let w = 1.0 - u - v;
                    if u >= 0.0 && v >= 0.0 && w >= 0.0 {
                        *slot = Some(u * alpha[0] + v * alpha[1] + w * alpha[2]);
                    }
                }
            }
        }
        let mut out = vec![0.0f32; size * size];
        for (i, s) in sample.iter().enumerate() {
            let (sx, sy) = (i % side, i / side);
            out[(sy / 4) * size + sx / 4] += s.unwrap_or(0.0) / 16.0;
        }
        out
    }

    /// The gear at the Settings button's size and at a large size: centred on both axes, the same mirrored left to
    /// right and top to bottom, a clear hole, a solid ring, and an outline with no corner. LUMALINK_GEAR_RAW=<folder>
    /// also writes each coverage as raw 8-bit grey (gear-<size>.raw) to look at.
    #[test]
    fn gear_is_centred_symmetric_and_round() {
        for size in [32usize, 48, 256] {
            let c = size as f32 / 2.0;
            let mesh = gear(pos2(c, c), 0.42 * size as f32, Color32::WHITE);
            let cov = coverage(&mesh, size);
            if let Some(dir) = std::env::var_os("LUMALINK_GEAR_RAW") {
                let bytes: Vec<u8> = cov.iter().map(|a| (a * 255.0).round() as u8).collect();
                std::fs::write(std::path::Path::new(&dir).join(format!("gear-{size}.raw")), bytes).unwrap();
            }
            let at = |x: usize, y: usize| cov[y * size + x];
            let (mut total, mut mx, mut my) = (0.0f64, 0.0f64, 0.0f64);
            for y in 0..size {
                for x in 0..size {
                    let a = at(x, y) as f64;
                    total += a;
                    mx += a * (x as f64 + 0.5);
                    my += a * (y as f64 + 0.5);
                    // Mirrored left to right and top to bottom.
                    assert!((at(x, y) - at(size - 1 - x, y)).abs() < 0.03, "size {size}: not mirrored left-right at {x},{y}");
                    assert!((at(x, y) - at(x, size - 1 - y)).abs() < 0.03, "size {size}: not mirrored top-bottom at {x},{y}");
                }
            }
            let (cx, cy) = (mx / total, my / total);
            assert!((cx - c as f64).abs() < 0.01 && (cy - c as f64).abs() < 0.01, "size {size}: centred at {cx},{cy}, not {c}");
            let mid = size / 2;
            assert!(at(mid, mid) < 0.01, "size {size}: the hole is not clear");
            // The ring between the hole (0.33 r) and the root (0.76 r): solid.
            let ring = (c + 0.55 * 0.42 * size as f32) as usize;
            assert!(at(ring, mid) > 0.99 && at(mid, ring) > 0.99, "size {size}: the ring is not solid");
        }
        // No corner: along the outer edge (every fourth vertex, its inside), each turn is small.
        let mesh = gear(pos2(0.0, 0.0), 100.0, Color32::WHITE);
        let edge: Vec<Pos2> = mesh.vertices.iter().step_by(4).map(|v| v.pos).collect();
        let n = edge.len();
        for i in 0..n {
            let (a, b, c) = (edge[(i + n - 1) % n], edge[i], edge[(i + 1) % n]);
            let turn = (b - a).angle().sub_angle((c - b).angle());
            assert!(turn.abs() < 0.2, "a corner at vertex {i}: it turns {turn} radians");
        }
    }

    trait SubAngle {
        fn sub_angle(self, other: f32) -> f32;
    }
    impl SubAngle for f32 {
        /// self - other, wrapped to -pi..pi.
        fn sub_angle(self, other: f32) -> f32 {
            let d = (self - other) % TAU;
            if d > PI {
                d - TAU
            } else if d < -PI {
                d + TAU
            } else {
                d
            }
        }
    }
}
