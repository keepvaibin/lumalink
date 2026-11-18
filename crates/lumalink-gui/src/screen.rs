//! What the window shows. Home: the mods as tiles, four across and three down a page, the slot after the last one
//! adding more, a tab on the window's edge turning to the next page; along the bottom, the bar with Settings (left), the
//! clock and the date (middle) and Add mods (right). A tile opens that mod's own screen: what it is and its jobs, what
//! came of the last job, and Menu and its main job as large buttons on the bar. Settings is a screen of its own: a list
//! of rows that scrolls, Back on the bar. Every word is a row of LumaLinkText.tsv.
//!
//! Every distance comes from [`Measure`]: one gap between tiles, rows and cards, twice it between a block and what
//! is around it, and the margin at the window's sides.

use super::*;
use eframe::egui::{pos2, vec2, Align, Align2, Id, Key, Label, Order, Pos2, Rect, Sense, UiBuilder, Vec2};
use lumalink_gui::chrome::{self, End, Icon, Mark};

/// The screens besides home and a mod's own.
#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Home,
    Settings,
    Languages,
}

/// The shapes a page of tiles can take, across and down, most tiles first: the window uses the first whose tiles are at
/// least TILE_LEAST wide (the one with the largest tiles of those with as many), or the one with the largest tiles when
/// none are.
const SHAPES: [(usize, usize); 13] = [(6, 2), (4, 3), (3, 4), (2, 6), (5, 2), (2, 5), (3, 3), (4, 2), (2, 4), (3, 2), (2, 3), (2, 2), (1, 2)];
/// The narrowest a tile is made while a shape with fewer, larger tiles would fit.
const TILE_LEAST: f32 = 150.0;
/// A tile's width to its height.
const TILE_ASPECT: f32 = 1.42;

/// The window's distances at its size.
struct Measure {
    /// The whole window: the bars run across it.
    full: Rect,
    /// Its middle, no wider than 16:9: everything else stands in it.
    area: Rect,
    /// Between tiles, rows and cards.
    gap: f32,
    /// At the window's sides.
    margin: f32,
    /// The bottom bar's height.
    bar: f32,
    /// The page tabs' width.
    tab: f32,
}

impl Measure {
    fn of(full: Rect) -> Measure {
        let width = full.width().min(full.height() * 16.0 / 9.0).round();
        let area = Rect::from_center_size(full.center(), vec2(width, full.height()));
        let gap = (area.width() * 0.016).clamp(12.0, 20.0).round();
        let margin = (area.width() * 0.045).clamp(28.0, 64.0).round();
        let bar = (area.height() * 0.2).clamp(108.0, 148.0).round();
        Measure { full, area, gap, margin, bar, tab: 34.0 }
    }

    fn bar_top(&self) -> f32 {
        self.area.bottom() - self.bar
    }

    /// The room at each side of a full screen's content: the page tabs' room in a wide window, the margin alone in a
    /// narrow one.
    fn side(&self) -> f32 {
        if self.area.width() < 760.0 { self.margin } else { self.margin + self.tab + 2.0 * self.gap }
    }

    /// The large buttons on the bar of a full screen (Menu, Back, a mod's main job).
    fn big_button(&self) -> Vec2 {
        vec2(((self.area.width() - 2.0 * self.margin - 2.0 * self.gap) / 2.0).min(270.0), (self.bar * 0.46).clamp(52.0, 64.0).round())
    }

    /// The middle of the bar of a full screen (it has no dip).
    fn bar_middle(&self) -> f32 {
        self.bar_top() + self.bar / 2.0
    }
}

impl Kind {
    /// A tile's colours, top and bottom, and its picture.
    fn art(self) -> (Color32, Color32, Icon) {
        match self {
            Kind::Folder => (Color32::from_rgb(92, 196, 244), Color32::from_rgb(32, 136, 214), Icon::Folder),
            Kind::Luma => (Color32::from_rgb(128, 150, 248), Color32::from_rgb(78, 92, 214), Icon::Package),
            Kind::Model => (Color32::from_rgb(212, 140, 232), Color32::from_rgb(150, 82, 196), Icon::Figure),
            Kind::Other => (Color32::from_rgb(252, 188, 92), Color32::from_rgb(234, 132, 42), Icon::Archive),
        }
    }
}

/// What a finished job's lines say at a glance: its last line's colour.
pub fn mark_of(lines: &[Line]) -> Option<Mark> {
    match lines.last().map(|l| l.color) {
        Some(c) if c == GREEN => Some(Mark::Good),
        Some(c) if c == AMBER => Some(Mark::Warn),
        Some(c) if c == RED => Some(Mark::Bad),
        _ => None,
    }
}

/// The art of a kind in `rect`: its colours, a soft gloss fading down from the top, its picture at `icon_at` (`size`
/// wide).
fn art(painter: &egui::Painter, rect: Rect, r: f32, kind: Kind, icon_at: Pos2, size: f32) {
    let (top, bottom, icon) = kind.art();
    let shape = chrome::rounded(rect, r);
    chrome::fill(painter, &shape, top, bottom);
    let gloss = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.top() + rect.height() * 0.6));
    let gloss_shape: Vec<Pos2> = chrome::rounded(gloss, r).into_iter().map(|p| pos2(p.x, p.y.min(gloss.bottom()))).collect();
    chrome::fill(painter, &gloss_shape, Color32::from_white_alpha(46), Color32::TRANSPARENT);
    chrome::outline(painter, &chrome::rounded(rect.shrink(1.0), (r - 1.0).max(1.0)), Stroke::new(1.0, Color32::from_white_alpha(60)));
    if size >= 12.0 {
        chrome::paint_icon(painter, icon, icon_at, size, Color32::WHITE, bottom);
    }
}

/// An item's name: its file's or folder's.
fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The clock and date in the window's language, and the seconds to the next minute.
fn clock() -> (String, String, u64) {
    let now = jiff::Zoned::now();
    let (h, m) = (now.hour(), now.minute());
    let twelve = text::language() == "UsEnglish";
    let hour = if twelve { ((h + 11) % 12 + 1).to_string() } else { format!("{h:02}") };
    let ampm = match (twelve, h < 12) {
        (false, _) => "",
        (true, true) => "AM",
        (true, false) => "PM",
    };
    let time = say!("LumaLink.Window.Time", hour = hour, minute = format!("{m:02}"), ampm = ampm);
    let weekday = match now.weekday().to_monday_one_offset() {
        1 => say!("LumaLink.Window.Weekday.1"),
        2 => say!("LumaLink.Window.Weekday.2"),
        3 => say!("LumaLink.Window.Weekday.3"),
        4 => say!("LumaLink.Window.Weekday.4"),
        5 => say!("LumaLink.Window.Weekday.5"),
        6 => say!("LumaLink.Window.Weekday.6"),
        _ => say!("LumaLink.Window.Weekday.7"),
    };
    let date = say!("LumaLink.Window.Date", weekday = weekday, month = now.month(), day = now.day());
    (time, date, 60u64.saturating_sub(now.second().max(0) as u64).max(1))
}

/// A switch with its words beside it, in a layout.
fn switch(ui: &mut egui::Ui, on: &mut bool, words: &str) -> egui::Response {
    let galley = ui.painter().layout(words.to_string(), chrome::body(15.0), INK, (ui.available_width() - 54.0).max(80.0));
    let track = vec2(42.0, 24.0);
    let size = vec2(track.x + 12.0 + galley.size().x, track.y.max(galley.size().y));
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, words));
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.15);
    let lit = chrome::lit(ui, &response);
    let track_rect = Rect::from_min_size(pos2(rect.left(), rect.center().y - track.y / 2.0), track);
    chrome::switch(ui.painter(), track_rect, t, lit);
    ui.painter().galley(pos2(track_rect.right() + 12.0, rect.center().y - galley.size().y / 2.0), galley, INK);
    response
}

/// What was done to a tile.
enum TileAction {
    Open(u64),
    Remove(u64),
}

impl App {
    /// One frame of the window (eframe's update, and the tests' frames).
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.take_done();
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        self.added(dropped);
        if self.items.iter().any(|i| i.busy) {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        sky::backdrop(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect());
        self.update.poll(ctx);
        if !ctx.wants_keyboard_input() && self.new_form.is_none() && !self.add_menu && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.back();
        }
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ctx, |ui| {
            let m = Measure::of(ui.max_rect());
            if self.chosen.is_some_and(|id| self.items.iter().any(|i| i.id == id)) {
                self.chosen_screen(ui, &m);
            } else {
                self.chosen = None;
                match self.view {
                    View::Home => self.home(ui, &m),
                    View::Settings | View::Languages => self.settings(ui, &m),
                }
            }
        });
        self.banner(ctx);
        self.add_window(ctx);
        self.new_window(ctx);
    }

    /// Paths dropped or chosen: one opens its own screen, several show the page with the last of them.
    fn added(&mut self, paths: Vec<PathBuf>) {
        let one = paths.len() == 1;
        let Some(last) = paths.into_iter().map(|p| self.add(p)).last() else { return };
        self.view = View::Home;
        if one {
            self.chosen = Some(last);
        } else {
            self.chosen = None;
            self.show_tile(last);
        }
    }

    /// One screen back: a mod's screen and Settings to home, the languages to Settings.
    fn back(&mut self) {
        if self.chosen.is_some() {
            self.chosen = None;
        } else {
            self.view = match self.view {
                View::Languages => View::Settings,
                _ => View::Home,
            };
        }
    }

    /// Shows the page holding item `id`'s tile.
    fn show_tile(&mut self, id: u64) {
        if let Some(index) = self.items.iter().position(|i| i.id == id) {
            self.sheet = index / self.per_page.max(1);
        }
    }

    /// Mods chosen in the system's dialog: .luma files, published mods (archives and patches) and models.
    fn open_files(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title(say!("LumaLink.Window.OpenFiles"))
            .add_filter(say!("LumaLink.Window.Filter.Mods"), &["luma", "zip", "7z", "rar", "glb", "gltf", "xdelta", "vcdiff", "bps", "ips", "ppf"])
            .add_filter(say!("LumaLink.Window.Filter.All"), &["*"])
            .pick_files()
            .unwrap_or_default();
        self.added(files);
    }

    /// Folders chosen in the system's dialog: mod folders, or published mods laid out as folders.
    fn open_folders(&mut self) {
        let folders = rfd::FileDialog::new().set_title(say!("LumaLink.Window.OpenFolder")).pick_folders().unwrap_or_default();
        self.added(folders);
    }

    fn choose_game(&mut self) {
        if let Some(pak) = rfd::FileDialog::new().add_filter("game.pak", &["pak"]).pick_file() {
            self.game = Some(pak);
        }
    }

    // ---- home ---------------------------------------------------------------------------------------------------

    fn home(&mut self, ui: &mut egui::Ui, m: &Measure) {
        let area = m.area;
        let ctx = ui.ctx().clone();
        let bar_top = m.bar_top();
        let dip = (m.bar * 0.3).round();
        let r = (m.bar * 0.34).clamp(38.0, 50.0).round();
        // The round buttons: their centres a margin and a radius in from the sides, a third of the way down the bar.
        let button_y = bar_top + (m.bar * 0.34).round();
        let left = pos2(area.left() + m.margin + r, button_y);
        let right = pos2(area.right() - m.margin - r, button_y);
        let socket = r + 9.0;

        // The grid between the top and the bar: the bar's highest point (around the buttons) counts.
        let bar_highest = bar_top.min(button_y - socket);
        let grid = Rect::from_min_max(
            pos2(area.left() + m.tab + 2.0 * m.gap, area.top() + 2.0 * m.gap),
            pos2(area.right() - m.tab - 2.0 * m.gap, bar_highest - 2.0 * m.gap),
        );
        self.grid(ui, m, grid);

        let painter = ui.painter().clone();
        chrome::bar(&painter, m.full, bar_top, dip, &[(left, socket), (right, socket)]);

        // Settings and Add mods, set into the bar, their words under them.
        let settings_words = say!("LumaLink.Window.Settings");
        let add_words = say!("LumaLink.Window.Add");
        if chrome::round_button(ui, Id::new("settings"), left, r, &settings_words, Icon::Gear).clicked() {
            self.view = View::Settings;
        }
        if chrome::round_button(ui, Id::new("add"), right, r, &add_words, Icon::Plus).clicked() {
            self.add_menu = true;
        }
        let under_top = button_y + r;
        let under = area.bottom() - under_top;
        let words_size = (under * 0.4).clamp(12.0, 15.0).round();
        for (at, words) in [(left, &settings_words), (right, &add_words)] {
            let galley = chrome::fit_text(&painter, words, chrome::bold(words_size), chrome::CLOCK, 2.0 * (m.margin + r) - m.gap, egui::Align::Center);
            painter.galley(pos2(at.x, under_top + (under - galley.size().y) / 2.0), galley, chrome::CLOCK);
        }

        // The clock in the dip.
        let (time, date, next) = clock();
        ctx.request_repaint_after(std::time::Duration::from_secs(next));
        let middle = area.center().x;
        let clock_galley = painter.layout_no_wrap(time, chrome::body((dip * 1.05).clamp(30.0, 46.0).round()), chrome::CLOCK);
        // Its figures standing on the dip's edge, as on a shelf.
        let clock_top = bar_top + dip - clock_galley.size().y * 0.92;
        painter.galley(pos2(middle - clock_galley.size().x / 2.0, clock_top), clock_galley, chrome::CLOCK);

        // Below the dip, between the round buttons: the date, and above it what to do while there is nothing yet. The
        // two are one block, centred both ways in that space.
        let below = Rect::from_min_max(
            pos2(left.x + socket + m.gap, bar_top + dip + 4.0),
            pos2(right.x - socket - m.gap, area.bottom() - 4.0),
        );
        let date_galley = painter.layout_no_wrap(date, chrome::bold((m.bar * 0.15).clamp(17.0, 22.0).round()), INK);
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let spacing = 6.0;
        let hint = (self.items.is_empty() || hovering_files).then(|| {
            // The largest size from 15 down to 12 at which it fits in two rows above the date.
            let room = below.height() - date_galley.size().y - spacing;
            let words = say!("LumaLink.Window.Drop");
            let mut size = 15.0;
            loop {
                let galley = chrome::fitted(&painter, &words, chrome::body(size), INK, below.width(), 2, Align::Center);
                if galley.size().y <= room || size <= 12.0 {
                    break galley;
                }
                size -= 1.0;
            }
        });
        let block = hint.as_ref().map_or(0.0, |h| h.size().y + spacing) + date_galley.size().y;
        let mut y = (below.center().y - block / 2.0).round();
        if let Some(hint) = hint {
            let height = hint.size().y;
            ui.put(Rect::from_min_size(pos2(below.left(), y), vec2(below.width(), height)), Label::new(hint).halign(Align::Center));
            y += height + spacing;
        }
        painter.galley(pos2(middle - date_galley.size().x / 2.0, y), date_galley, INK);
    }

    /// The pages of tiles in `grid`, the page tabs on the window's edges.
    fn grid(&mut self, ui: &mut egui::Ui, m: &Measure, grid: Rect) {
        let ctx = ui.ctx().clone();
        let gap = m.gap;
        // The largest tiles that fit both ways in a shape; the most tiles a page holds at TILE_LEAST or wider.
        let fit = |(c, r): (usize, usize)| {
            let (c, r) = (c as f32, r as f32);
            ((grid.width() - gap * (c - 1.0)) / c).min(((grid.height() - gap * (r - 1.0)) / r) * TILE_ASPECT).floor()
        };
        let larger = |best: (usize, usize), s: (usize, usize)| if fit(s) > fit(best) + 0.5 { s } else { best };
        let (columns, rows_down) = match SHAPES.into_iter().filter(|&s| fit(s) >= TILE_LEAST).map(|(c, r)| c * r).max() {
            Some(most) => SHAPES.into_iter().filter(|&(c, r)| c * r == most && fit((c, r)) >= TILE_LEAST).reduce(larger),
            None => SHAPES.into_iter().reduce(larger),
        }
        .unwrap_or(SHAPES[1]);
        let (cols, rows) = (columns as f32, rows_down as f32);
        let tile_w = fit((columns, rows_down));
        let tile = vec2(tile_w, (tile_w / TILE_ASPECT).floor());
        let block = vec2(cols * tile.x + gap * (cols - 1.0), rows * tile.y + gap * (rows - 1.0));
        let origin = (grid.center() - block / 2.0).round();
        let per_page = columns * rows_down;
        // A new shape keeps the first tile of the page shown on the page shown.
        if per_page != self.per_page {
            self.sheet = self.sheet * self.per_page.max(1) / per_page;
            self.per_page = per_page;
        }
        let pages = (self.items.len() + 1).div_ceil(per_page).max(1);
        self.sheet = self.sheet.min(pages - 1);
        let width = m.area.width();

        // Turning the page: the wheel over the grid, the arrow keys, the tabs.
        if ui.rect_contains_pointer(grid) {
            let delta = ctx.input(|i| i.raw_scroll_delta);
            self.wheel += if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y };
            if delta == Vec2::ZERO {
                self.wheel *= 0.85;
            }
        }
        let typing = ctx.wants_keyboard_input();
        let (forward, back) = ctx.input(|i| {
            (!typing && (i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::PageDown)), !typing && (i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::PageUp)))
        });
        if (forward || self.wheel <= -60.0) && self.sheet + 1 < pages {
            self.sheet += 1;
            self.wheel = 0.0;
        } else if (back || self.wheel >= 60.0) && self.sheet > 0 {
            self.sheet -= 1;
            self.wheel = 0.0;
        }
        self.wheel = self.wheel.clamp(-60.0, 60.0);

        let shown = ctx.animate_value_with_time(Id::new("sheet"), self.sheet as f32, 0.3);
        let clip = Rect::from_min_max(pos2(m.area.left(), grid.top() - gap), pos2(m.area.right(), grid.bottom() + gap));
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let mut open_item = None;
        let mut remove = None;
        let mut add = false;
        for page in 0..pages {
            let offset = (page as f32 - shown) * width;
            if offset.abs() >= width {
                continue;
            }
            let here = page == self.sheet && (shown - page as f32).abs() < 0.02;
            let painter = ui.painter().with_clip_rect(clip);
            for slot in 0..per_page {
                let index = page * per_page + slot;
                let (row, col) = ((slot / columns) as f32, (slot % columns) as f32);
                let rect = Rect::from_min_size(origin + vec2(col * (tile.x + gap) + offset, row * (tile.y + gap)), tile);
                if index < self.items.len() {
                    match self.tile(ui, &painter, rect, index, here) {
                        Some(TileAction::Open(id)) => open_item = Some(id),
                        Some(TileAction::Remove(id)) => remove = Some(id),
                        None => {}
                    }
                } else if index == self.items.len() {
                    add |= self.add_tile(ui, &painter, rect, here);
                } else {
                    chrome::empty_slot(&painter, rect, (tile.y * 0.13).round());
                }
            }
        }
        if let Some(id) = open_item {
            self.chosen = Some(id);
        }
        if let Some(id) = remove {
            self.items.retain(|i| i.id != id);
        }
        if add {
            self.add_menu = true;
        }

        // The tabs on the window's edges, at the middle of the tiles.
        let tab_y = origin.y + block.y / 2.0;
        if self.sheet > 0 && self.tab(ui, m, pos2(m.area.left(), tab_y), false) {
            self.sheet -= 1;
        }
        if self.sheet + 1 < pages && self.tab(ui, m, pos2(m.area.right(), tab_y), true) {
            self.sheet += 1;
        }

        // Files carried over the window: the tiles light up.
        if hovering_files {
            let painter = ui.painter();
            let lit = Rect::from_min_size(origin, block).expand(gap / 2.0);
            painter.rect_filled(lit, gap, chrome::faded(sky::BLUE, 0.06));
            chrome::glow(painter, &chrome::rounded(lit, gap), lit.center(), 1.0);
        }
    }

    /// One mod's tile; what was done to it.
    fn tile(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, index: usize, here: bool) -> Option<TileAction> {
        let item = &self.items[index];
        let id = Id::new(("tile", item.id));
        let name = name_of(&item.path);
        let response = here.then(|| chrome::hit(ui, rect, id, &name, true));
        let pointer_in = here && ui.rect_contains_pointer(rect);
        let focus = response.as_ref().is_some_and(egui::Response::has_focus);
        let t = ui.ctx().animate_bool_with_time(id.with("lit"), pointer_in || focus, 0.14);
        let rect = Rect::from_center_size(rect.center(), rect.size() * (1.0 + 0.04 * t));
        let r = rect.height() * 0.13;
        chrome::shadow(painter, rect, r, 1.0 + t);
        let face = chrome::rounded(rect, r);
        chrome::fill(painter, &face, chrome::FACE_TOP, chrome::FACE_BOTTOM);
        chrome::outline(painter, &face, Stroke::new(1.5, chrome::EDGE));
        // The art inset by the same share on every side; the picture and the name share its height.
        let inset = rect.height() * 0.045;
        let inner = rect.shrink(inset);
        let size = (inner.height() * 0.12).clamp(12.0, 17.0).round();
        let text_width = inner.width() - 2.0 * size;
        let galley = chrome::centred_text(painter, &name, chrome::bold(size), Color32::WHITE, text_width, 2);
        let shade = chrome::centred_text(painter, &name, chrome::bold(size), Color32::from_black_alpha(60), text_width, 2);
        let text_top = inner.bottom() - inner.height() * 0.08 - galley.size().y;
        // At most the room above the name, so a small tile's picture stays inside it.
        let icon = (inner.height() * 0.38).clamp(26.0, 72.0).min((text_top - inner.top() - 8.0).max(0.0));
        // The picture centred in the space above the name.
        let icon_at = pos2(inner.center().x, inner.top() + (text_top - inner.top()) / 2.0);
        art(painter, inner, (r - inset).max(4.0), item.kind, icon_at, icon);
        painter.galley(pos2(inner.center().x + 0.5, text_top + 1.2), shade, Color32::BLACK);
        painter.galley(pos2(inner.center().x, text_top), galley, Color32::WHITE);
        chrome::glow(painter, &face, rect.center(), t);
        // How its last job went, or that one runs, in the corner opposite the remove button.
        let corner = (inner.height() * 0.13).clamp(12.0, 16.0);
        let badge = inner.left_top() + Vec2::splat(corner + 2.0);
        if item.busy {
            painter.circle_filled(badge, corner - 1.0, Color32::from_white_alpha(235));
            chrome::spinner(painter, badge, corner * 0.55, sky::BLUE_INK, ui.input(|i| i.time));
        } else if let Some(mark) = item.mark {
            chrome::mark(painter, badge, corner * 0.75, mark);
        }
        let mut action = None;
        // Remove: a small round × in the corner while the pointer is on the tile.
        if here && t > 0.3 && !item.busy {
            let at = inner.right_top() + vec2(-(corner + 2.0), corner + 2.0);
            let words = say!("LumaLink.Window.Remove");
            let x = chrome::hit(ui, Rect::from_center_size(at, Vec2::splat(2.0 * corner)), id.with("remove"), &words, true).on_hover_text(words);
            let near = chrome::lit(ui, &x);
            let painter = ui.painter();
            painter.circle_filled(at + vec2(0.0, 1.0), corner, chrome::faded(Color32::from_black_alpha(40), t));
            painter.circle_filled(at, corner - 1.0, chrome::faded(chrome::mix(Color32::WHITE, Color32::from_rgb(255, 236, 236), near), t));
            let s = corner * 0.33;
            let stroke = Stroke::new(2.2, chrome::faded(chrome::mix(INK, RED, near), t));
            painter.line_segment([at + vec2(-s, -s), at + vec2(s, s)], stroke);
            painter.line_segment([at + vec2(s, -s), at + vec2(-s, s)], stroke);
            if x.clicked() {
                action = Some(TileAction::Remove(item.id));
            }
        }
        if action.is_none() && response.is_some_and(|r| r.on_hover_text(item.path.display().to_string()).clicked()) {
            action = Some(TileAction::Open(item.id));
        }
        action
    }

    /// The slot after the last tile: adds mods. Whether it was pressed.
    fn add_tile(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, here: bool) -> bool {
        let words = say!("LumaLink.Window.Add");
        let response = here.then(|| chrome::hit(ui, rect, Id::new("add-tile"), &words, true));
        let t = response.as_ref().map_or(0.0, |r| chrome::lit(ui, r));
        let r = (rect.height() * 0.13).round();
        chrome::empty_slot(painter, rect, r);
        let color = chrome::mix(chrome::CLOCK, sky::BLUE_INK, t);
        let size = (rect.height() * 0.28).clamp(22.0, 52.0);
        let galley = chrome::fit_text(painter, &words, chrome::bold((rect.height() * 0.1).clamp(12.0, 16.0).round()), color, rect.width() - 16.0, egui::Align::Center);
        // The plus and the words, as one block, centred in the slot.
        let spacing = size * 0.35;
        let top = rect.center().y - (size + spacing + galley.size().y) / 2.0;
        chrome::paint_icon(painter, Icon::Plus, pos2(rect.center().x, top + size / 2.0), size, color, Color32::WHITE);
        painter.galley(pos2(rect.center().x, top + size + spacing), galley, color);
        chrome::glow(painter, &chrome::rounded(rect, r), rect.center(), t);
        response.is_some_and(|r| r.clicked())
    }

    /// A page tab on the window's edge at `at` (`next`: the right one, pointing on). Whether it was pressed.
    fn tab(&self, ui: &mut egui::Ui, m: &Measure, at: Pos2, next: bool) -> bool {
        let size = vec2(m.tab, (m.tab * 2.3).round());
        let rect = if next { Rect::from_min_size(pos2(at.x - size.x, at.y - size.y / 2.0), size) } else { Rect::from_min_size(pos2(at.x, at.y - size.y / 2.0), size) };
        let label = if next { "▶" } else { "◀" };
        let response = chrome::hit(ui, rect, Id::new(("tab", next)), label, true);
        let t = chrome::lit(ui, &response);
        let painter = ui.painter();
        // A tab coming out of the edge: rounded on its inner side only.
        let r = (size.x * 0.47).round();
        let body = if next { Rect::from_min_max(rect.min, rect.max + vec2(r + 4.0, 0.0)) } else { Rect::from_min_max(rect.min - vec2(r + 4.0, 0.0), rect.max) };
        let shape = chrome::rounded(body, r);
        let clipped = painter.with_clip_rect(rect.expand2(vec2(0.0, 10.0)));
        chrome::shadow(&clipped, body, r, 1.0 + t);
        // Lit: the face takes a light blue and its edge the bar's blue; no ring, which the window's edge would cut.
        let tint = Color32::from_rgb(226, 244, 253);
        chrome::fill(&clipped, &shape, chrome::mix(chrome::FACE_TOP, tint, t), chrome::mix(chrome::FACE_BOTTOM, tint, t));
        chrome::outline(&clipped, &shape, Stroke::new(1.5, chrome::mix(chrome::EDGE, chrome::BAR_LINE, t)));
        // The triangle's centre of mass at the tab's middle.
        let (w, h) = (size.x * 0.4, size.x * 0.36);
        let c = rect.center();
        let points = if next {
            vec![c + vec2(-w / 3.0, -h), c + vec2(2.0 * w / 3.0, 0.0), c + vec2(-w / 3.0, h)]
        } else {
            vec![c + vec2(w / 3.0, -h), c + vec2(-2.0 * w / 3.0, 0.0), c + vec2(w / 3.0, h)]
        };
        chrome::fill(painter, &points, chrome::mix(sky::BLUE, sky::BLUE_INK, t), sky::BLUE_INK);
        response.clicked()
    }

    // ---- settings -------------------------------------------------------------------------------------------------

    /// Settings, or the list of languages: a title along the top, rows that scroll, Back on the bar.
    fn settings(&mut self, ui: &mut egui::Ui, m: &Measure) {
        let area = m.area;
        let languages = self.view == View::Languages;
        let title = if languages { say!("LumaLink.Window.Language") } else { say!("LumaLink.Window.Settings") };
        let header = Rect::from_min_size(m.full.min, vec2(m.full.width(), (m.bar * 0.56).round()));
        let painter = ui.painter().clone();
        chrome::header(&painter, header);
        let title_galley = painter.layout_no_wrap(title, chrome::bold((header.height() * 0.36).round()), INK);
        painter.galley(pos2(area.left() + m.margin, header.center().y - title_galley.size().y / 2.0), title_galley, INK);

        let bar_top = m.bar_top();
        let list = Rect::from_min_max(pos2(m.full.left(), header.bottom() + 2.0), pos2(m.full.right(), bar_top - 2.0));
        let row_size = vec2((area.width() - 2.0 * m.side()).min(680.0).round(), (m.bar * 0.46).clamp(52.0, 64.0).round());
        let mut go = None;
        let mut list_ui = ui.new_child(UiBuilder::new().max_rect(list));
        // The rows' height at the last frame: rows that fit stand in the middle, with at least two gaps above them.
        let measured = Id::new(("rows-height", languages));
        let rows_height: f32 = ui.ctx().data(|d| d.get_temp(measured)).unwrap_or(0.0);
        let above = ((list.height() - rows_height) / 2.0).max(2.0 * m.gap).round();
        let scrolled = egui::ScrollArea::vertical().id_salt(("rows", languages)).auto_shrink([false, false]).show(&mut list_ui, |ui| {
            ui.set_clip_rect(ui.clip_rect().expand2(vec2(m.gap, 0.0)));
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(above);
                let start = ui.cursor().top();
                if languages {
                    let now = text::language();
                    for (i, l) in luma_validate::LANGUAGES.into_iter().enumerate() {
                        if i > 0 {
                            ui.add_space(m.gap);
                        }
                        let end = if l == now { End::Chosen } else { End::Nothing };
                        if chrome::row(ui, row_size, Id::new(("language", l)), words::name(l), None, end, true).clicked() {
                            text::set_language(l);
                            go = Some(View::Settings);
                        }
                    }
                } else {
                    go = self.setting_rows(ui, row_size, m.gap);
                }
                let height = ui.min_rect().bottom() - start;
                if (height - rows_height).abs() > 0.5 {
                    ui.ctx().data_mut(|d| d.insert_temp(measured, height));
                    ui.ctx().request_repaint();
                }
                ui.add_space(2.0 * m.gap);
            });
        });
        if let Some(view) = go {
            self.view = view;
        }
        // More above or below: the rows fade into the backdrop at that edge.
        let view = scrolled.inner_rect;
        let offset = scrolled.state.offset.y;
        let more_below = scrolled.content_size.y - view.height() - offset;
        let fade = 2.0 * m.gap;
        for (more, edge, toward) in [(offset, view.top(), 1.0), (more_below, view.bottom(), -1.0)] {
            if more > 0.5 {
                let t = (more / fade).min(1.0);
                let solid = chrome::faded(sky::backdrop_at(m.full, edge), t);
                let inner_y = edge + toward * fade;
                let mut mesh = egui::epaint::Mesh::default();
                mesh.colored_vertex(pos2(m.full.left(), edge), solid);
                mesh.colored_vertex(pos2(m.full.right(), edge), solid);
                mesh.colored_vertex(pos2(m.full.left(), inner_y), Color32::TRANSPARENT);
                mesh.colored_vertex(pos2(m.full.right(), inner_y), Color32::TRANSPARENT);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(1, 2, 3);
                painter.add(egui::Shape::mesh(mesh));
            }
        }

        // The bar: Back at its left.
        chrome::bar(&painter, m.full, bar_top, 0.0, &[]);
        let size = m.big_button();
        let back = Rect::from_min_size(pos2(area.left() + m.margin, m.bar_middle() - size.y / 2.0), size);
        if chrome::big_button(ui, back, Id::new("back"), &say!("LumaLink.Window.Back"), true).clicked() {
            self.back();
        }
    }

    /// Settings' rows; the screen a row goes to.
    fn setting_rows(&mut self, ui: &mut egui::Ui, size: Vec2, gap: f32) -> Option<View> {
        let mut go = None;
        let game = self.game.clone().or_else(|| self.installed.as_ref().map(|f| f.pak.clone()));
        let (value, color) = match &game {
            Some(pak) => (pak.display().to_string(), SOFT),
            None => (String::new(), AMBER),
        };
        if chrome::row(ui, size, Id::new("game"), &say!("LumaLink.Window.ChooseGame"), None, End::Value(value, color), true).clicked() {
            self.choose_game();
        }
        if game.is_none() {
            ui.add_space(gap / 2.0);
            ui.add_sized(vec2(size.x - 56.0, 0.0), Label::new(RichText::new(say!("LumaLink.Window.NoGame")).size(14.0).color(AMBER)).wrap());
        }
        let language = words::name(text::language()).to_string();
        ui.add_space(gap);
        if chrome::row(ui, size, Id::new("language"), &say!("LumaLink.Window.Language"), None, End::Value(language, SOFT), true).clicked() {
            go = Some(View::Languages);
        }
        let at_start = self.check_for_updates;
        ui.add_space(gap);
        if chrome::row(ui, size, Id::new("at-start"), &say!("LumaLink.Window.CheckAtStart"), None, End::Switch(at_start), true).clicked() {
            self.check_for_updates = !at_start;
            let mut settings = lumalink_core::update::Settings::load();
            settings.check_for_updates = self.check_for_updates;
            if let Err(e) = settings.save() {
                self.update = Update::Failed { release: None, why: format!("{e:#}") };
            }
        }
        ui.add_space(gap);
        if chrome::row(ui, size, Id::new("check-now"), &say!("LumaLink.Window.CheckNow"), None, End::Nothing, true).clicked() {
            self.update = Update::check(true);
        }
        ui.add_space(gap);
        if chrome::row(ui, size, Id::new("new-mod"), &say!("LumaLink.Window.New"), None, End::Nothing, true).clicked() {
            self.new_form = Some(NewForm::default());
        }
        // LumaScript in Visual Studio Code: errors, completion and help as a mod's code is typed.
        self.vscode.poll();
        let working = matches!(self.vscode, VsCode::Working(_));
        ui.add_space(gap);
        if chrome::row(ui, size, Id::new("vscode"), &say!("LumaLink.Window.VsCode"), None, End::Nothing, !working).clicked() && !working {
            self.vscode = VsCode::start();
        }
        let said = match &self.vscode {
            VsCode::Idle => None,
            VsCode::Working(_) => {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                Some((say!("LumaLink.Window.VsCode.Working"), SOFT))
            }
            VsCode::Said(text, color) => Some((text.clone(), *color)),
        };
        if let Some((text, color)) = said {
            ui.add_space(gap / 2.0);
            ui.add_sized(vec2(size.x - 56.0, 0.0), Label::new(RichText::new(text).size(14.0).color(color)).wrap());
        }
        ui.add_space(gap);
        ui.label(RichText::new(say!("LumaLink.Window.Version", version = lumalink_core::update::VERSION)).size(13.0).color(SOFT));
        go
    }

    // ---- a mod's own screen ---------------------------------------------------------------------------------------

    /// The chosen item: what it is and its jobs, what came of the last, Menu and its main job.
    fn chosen_screen(&mut self, ui: &mut egui::Ui, m: &Measure) {
        let area = m.area;
        let Some(index) = self.chosen.and_then(|id| self.items.iter().position(|i| i.id == id)) else { return };
        let (id, path, kind, busy, has_lines) = {
            let item = &self.items[index];
            (item.id, item.path.clone(), item.kind, item.busy, !item.lines.is_empty())
        };
        let bar_top = m.bar_top();
        let side = m.side();
        let content = Rect::from_min_max(pos2(area.left() + side, area.top() + 2.0 * m.gap), pos2(area.right() - side, bar_top - 2.0 * m.gap));
        let mut job: Option<(u64, Job)> = None;
        let mut add = None;
        let mut remove = false;
        let mut open_item: Option<PathBuf> = None;

        let mut outer = ui.new_child(UiBuilder::new().max_rect(content));
        egui::ScrollArea::vertical().id_salt(("screen", id)).auto_shrink([false, false]).show(&mut outer, |inner| {
            // The cards' shadows show past the sides (only up and down scroll).
            inner.set_clip_rect(inner.clip_rect().expand2(vec2(2.0 * m.gap, 0.0)));
            inner.spacing_mut().item_spacing.y = 0.0;
            // With nothing below it, the card stands in the middle.
            let free = (content.height() - self.card_height).max(0.0);
            let lift = inner.ctx().animate_value_with_time(Id::new(("lift", id)), if has_lines { 0.0 } else { free / 2.0 }, 0.25);
            inner.add_space(lift);
            let card = chrome::card().show(inner, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                // Narrow: a smaller picture, and Remove under the jobs instead of in the corner.
                let narrow = content.width() < 600.0;
                ui.horizontal_top(|ui| {
                    let art_side = if narrow { 64.0 } else if content.width() < 640.0 { 92.0 } else { 116.0 };
                    let (art_rect, _) = ui.allocate_exact_size(Vec2::splat(art_side), Sense::hover());
                    let painter = ui.painter();
                    chrome::shadow(painter, art_rect, art_side * 0.19, 1.0);
                    art(painter, art_rect, art_side * 0.19, kind, art_rect.center(), art_side * 0.5);
                    ui.add_space(if narrow { 14.0 } else { 22.0 });
                    // Remove, quietly, in the card's corner; the name keeps clear of it.
                    let remove_words = say!("LumaLink.Window.Remove");
                    let remove_width = ui.painter().layout_no_wrap(remove_words.clone(), chrome::body(14.0), SOFT).size().x + 34.0;
                    let corner = ui.max_rect().right_top();
                    ui.vertical(|ui| {
                        let name_width = (ui.available_width() - if narrow { 0.0 } else { remove_width + 12.0 }).max(80.0);
                        let name = chrome::fitted(ui.painter(), &name_of(&path), chrome::bold(26.0), INK, name_width, 2, Align::Min);
                        ui.add(Label::new(name));
                        ui.add_space(2.0);
                        ui.label(RichText::new(kind.label()).size(15.0).color(SOFT));
                        let shown = chrome::fitted_tail(ui.painter(), &path.display().to_string(), chrome::body(13.0), SOFT, ui.available_width());
                        ui.add(Label::new(shown)).on_hover_text(path.display().to_string());
                        ui.add_space(14.0);
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                                let button = |ui: &mut egui::Ui, row: &str| ui.button(text::say(row, &[])).clicked();
                                match kind {
                                    Kind::Folder => {
                                        if button(ui, "LumaLink.Window.Check") {
                                            job = Some((id, Job::Check));
                                        }
                                        if jobs::has_source(&path) && button(ui, "LumaLink.Window.Build") {
                                            job = Some((id, Job::Build));
                                        }
                                        if button(ui, "LumaLink.Window.Pack") {
                                            job = Some((id, Job::Pack { to: None }));
                                        }
                                        if button(ui, "LumaLink.Window.Install") {
                                            job = Some((id, Job::Install { play: false }));
                                        }
                                        if button(ui, "LumaLink.Window.Verify") {
                                            if let Some(luma) = rfd::FileDialog::new().add_filter("luma", &["luma"]).pick_file() {
                                                add = Some((luma, Job::Verify { folder: path.clone() }));
                                            }
                                        }
                                    }
                                    Kind::Luma => {
                                        if button(ui, "LumaLink.Window.Check") {
                                            job = Some((id, Job::Check));
                                        }
                                        if button(ui, "LumaLink.Window.Install") {
                                            job = Some((id, Job::Install { play: false }));
                                        }
                                        if button(ui, "LumaLink.Window.List") {
                                            job = Some((id, Job::List));
                                        }
                                        if button(ui, "LumaLink.Window.Extract") {
                                            if let Some(place) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChoosePlace")).pick_folder() {
                                                job = Some((id, Job::Extract { into: free_folder(&place, &stem_of(&path)) }));
                                            }
                                        }
                                        if button(ui, "LumaLink.Window.Verify") {
                                            if let Some(folder) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChooseModFolder")).pick_folder() {
                                                job = Some((id, Job::Verify { folder }));
                                            }
                                        }
                                    }
                                    Kind::Model => {
                                        ui.label(RichText::new(say!("LumaLink.Window.Replaces")).color(SOFT));
                                        let shown = |who: &str| if who == "luigi" { say!("LumaLink.Window.Luigi") } else { say!("LumaLink.Window.Mario") };
                                        egui::ComboBox::from_id_salt("replaces").selected_text(shown(&self.replaces)).show_ui(ui, |ui| {
                                            for who in ["mario", "luigi"] {
                                                ui.selectable_value(&mut self.replaces, who.to_string(), shown(who));
                                            }
                                        });
                                        if button(ui, "LumaLink.Window.AddCharacter") {
                                            if let Some(folder) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChooseMod")).pick_folder() {
                                                job = Some((id, Job::AddCharacter { folder, replaces: self.replaces.clone() }));
                                            }
                                        }
                                    }
                                    Kind::Other => {
                                        switch(ui, &mut self.accept_losses, &say!("LumaLink.Window.AcceptLosses"));
                                        ui.end_row();
                                        if button(ui, "LumaLink.Window.Convert") {
                                            if let Some(place) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChoosePlace")).pick_folder() {
                                                job = Some((id, Job::Convert { folder: free_folder(&place, &stem_of(&path)), accept_losses: self.accept_losses }));
                                            }
                                        }
                                    }
                                }
                            });
                        });
                        if narrow {
                            ui.add_space(4.0);
                            let button = egui::Button::new(RichText::new(remove_words.clone()).size(14.0).color(SOFT)).frame(false);
                            if ui.add_enabled(!busy, button).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                remove = true;
                            }
                        }
                    });
                    if !narrow {
                        let at = Rect::from_min_size(corner - vec2(remove_width, 0.0), vec2(remove_width, 30.0));
                        let button = egui::Button::new(RichText::new(remove_words).size(14.0).color(SOFT)).frame(false);
                        if ui.add_enabled_ui(!busy, |ui| ui.put(at, button)).inner.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            remove = true;
                        }
                    }
                });
            });
            let height = card.response.rect.height();
            if (height - self.card_height).abs() > 0.5 {
                self.card_height = height;
                inner.ctx().request_repaint();
            }

            // What came of the last job, down to the bar: a short window scrolls, with the result at least this high.
            if has_lines {
                inner.add_space(m.gap);
                let frame = chrome::card().inner_margin(20.0);
                let rest = (content.height() - lift - self.card_height - m.gap - 42.0).max(150.0);
                let (explain, open, open_made) = frame
                    .show(inner, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_height(rest);
                        self.lines(ui, id)
                    })
                    .inner;
                if let Some(made) = open_made {
                    open_item = Some(made);
                }
                if let Some(code) = explain {
                    // What the code means, in place of the report; Check brings the report back.
                    self.start(id, Job::Explain(code));
                }
                if let Some(file) = open {
                    show_file(&file);
                }
            }
        });

        // The bar: Menu and the main job, a gap either side of its middle.
        let painter = ui.painter().clone();
        chrome::bar(&painter, m.full, bar_top, 0.0, &[]);
        let size = m.big_button();
        let y = m.bar_middle() - size.y / 2.0;
        let menu = Rect::from_min_size(pos2(area.center().x - m.gap - size.x, y), size);
        let main = Rect::from_min_size(pos2(area.center().x + m.gap, y), size);
        if chrome::big_button(ui, menu, Id::new("menu"), &say!("LumaLink.Window.Menu"), true).clicked() {
            self.chosen = None;
            self.show_tile(id);
        }
        let main_words = match kind {
            Kind::Folder | Kind::Luma => say!("LumaLink.Window.Play"),
            Kind::Model => say!("LumaLink.Window.CharacterMod"),
            Kind::Other => say!("LumaLink.Window.MakeLuma"),
        };
        if chrome::primary_button(ui, main, Id::new("main"), &main_words, !busy, false).clicked() {
            match kind {
                Kind::Folder | Kind::Luma => job = Some((id, Job::Install { play: true })),
                Kind::Model => job = Some((id, Job::CharacterMod { replaces: self.replaces.clone() })),
                Kind::Other => {
                    if let Some(luma) = save_luma(&path) {
                        job = Some((id, Job::MakeLuma { luma, accept_losses: self.accept_losses }));
                    }
                }
            }
        }

        if let Some((luma, verify)) = add {
            let luma_id = self.add(luma);
            self.chosen = Some(luma_id);
            job = Some((luma_id, verify));
        }
        if let Some((on, job)) = job {
            self.start(on, job);
        }
        if remove {
            self.items.retain(|i| i.id != id);
            self.chosen = None;
        }
        if let Some(made) = open_item {
            let made_id = self.add(made);
            self.chosen = Some(made_id);
        }
    }

    /// The item's lines, each finding's code a link to what it means and its place a link to the file; the code and
    /// the file asked for.
    fn lines(&self, ui: &mut egui::Ui, id: u64) -> (Option<u16>, Option<PathBuf>, Option<PathBuf>) {
        let Some(item) = self.items.iter().find(|i| i.id == id) else { return (None, None, None) };
        let mut explain = None;
        let mut open = None;
        let mut open_made = None;
        egui::ScrollArea::vertical().id_salt("lines").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for line in &item.lines {
                ui.horizontal_wrapped(|ui| {
                    if item.busy {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        chrome::spinner(ui.painter(), rect.center(), 7.0, sky::BLUE_INK, ui.input(|i| i.time));
                    }
                    if let Some(code) = line.code {
                        if ui.link(RichText::new(format!("L{code}")).color(line.color).strong()).on_hover_text(say!("LumaLink.Window.Explain")).clicked() {
                            explain = Some(code);
                        }
                    }
                    ui.label(RichText::new(&line.text).color(line.color).size(15.0));
                    if let Some((place, on_disk)) = &line.place {
                        match on_disk {
                            Some(file) => {
                                if ui.link(RichText::new(place).size(14.0)).on_hover_text(say!("LumaLink.Window.Show")).clicked() {
                                    open = Some(file.clone());
                                }
                            }
                            None => {
                                ui.label(RichText::new(place).color(SOFT).size(14.0));
                            }
                        }
                    }
                });
            }
            if let Some(made) = &item.made {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    // A mod it made opens on its own screen, where it can be played or packed.
                    if made != &item.path && matches!(Kind::of(made), Kind::Folder | Kind::Luma) && ui.button(RichText::new(say!("LumaLink.Window.OpenMade")).strong()).clicked() {
                        open_made = Some(made.clone());
                    }
                    if ui.button(say!("LumaLink.Window.Show")).clicked() {
                        open = Some(made.clone());
                    }
                });
            }
        });
        (explain, open, open_made)
    }

    /// The Add mods choices: mod files, a folder, or a new mod.
    fn add_window(&mut self, ctx: &egui::Context) {
        if !self.add_menu {
            return;
        }
        let mut choice = None;
        let modal = egui::Modal::new(Id::new("add-menu")).frame(chrome::card().inner_margin(26.0)).backdrop_color(Color32::from_black_alpha(70)).show(ctx, |ui| {
            let width = (ctx.screen_rect().width() - 120.0).clamp(300.0, 500.0);
            ui.set_width(width);
            ui.label(RichText::new(say!("LumaLink.Window.Add")).font(chrome::bold(22.0)).color(INK));
            ui.add_space(14.0);
            let rows = [
                ("LumaLink.Window.OpenFiles", "LumaLink.Window.OpenFiles.Detail"),
                ("LumaLink.Window.OpenFolder", "LumaLink.Window.OpenFolder.Detail"),
                ("LumaLink.Window.New", "LumaLink.Window.New.Detail"),
            ];
            for (n, (row, detail)) in rows.into_iter().enumerate() {
                if n > 0 {
                    ui.add_space(10.0);
                }
                if chrome::row(ui, vec2(width, 78.0), Id::new(("add-choice", n)), &text::say(row, &[]), Some(&text::say(detail, &[])), End::Nothing, true).clicked() {
                    choice = Some(n);
                }
            }
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                if ui.add_sized(vec2(150.0, 44.0), egui::Button::new(say!("LumaLink.Window.Cancel"))).clicked() {
                    choice = Some(usize::MAX);
                }
            });
        });
        if choice.is_some() || modal.should_close() {
            self.add_menu = false;
        }
        match choice {
            Some(0) => self.open_files(),
            Some(1) => self.open_folders(),
            Some(2) => self.new_form = Some(NewForm::default()),
            _ => {}
        }
    }

    // ---- over the window --------------------------------------------------------------------------------------------

    /// A newer LumaLink: a card along the top.
    fn banner(&mut self, ctx: &egui::Context) {
        if !self.update.showing() {
            return;
        }
        let busy = self.items.iter().any(|i| i.busy);
        let width = (ctx.screen_rect().width() - 48.0).min(760.0);
        let mut close = false;
        egui::Area::new(Id::new("update")).order(Order::Foreground).anchor(Align2::CENTER_TOP, vec2(0.0, 16.0)).show(ctx, |ui| {
            chrome::card().inner_margin(egui::Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.set_max_width(width);
                close = self.update.banner(ui, busy);
            });
        });
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn new_window(&mut self, ctx: &egui::Context) {
        let Some(form) = self.new_form.as_mut() else { return };
        let mut close = false;
        let mut made = None;
        let modal = egui::Modal::new(Id::new("new")).frame(chrome::card().inner_margin(26.0)).backdrop_color(Color32::from_black_alpha(70)).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new(say!("LumaLink.Window.New")).font(chrome::bold(22.0)).color(INK));
            ui.add_space(14.0);
            for (row, value) in [("LumaLink.Window.Name", &mut form.name), ("LumaLink.Window.Summary", &mut form.summary), ("LumaLink.Window.Authors", &mut form.authors), ("LumaLink.Window.Id", &mut form.id)] {
                ui.add_space(6.0);
                ui.label(RichText::new(text::say(row, &[])).size(14.0).color(SOFT));
                ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(12, 8)));
            }
            ui.add_space(12.0);
            // Where it goes: a folder chosen, the mod's own folder made in it under the mod's name.
            let name = if form.name.trim().is_empty() { say!("LumaLink.Window.NewName") } else { form.name.trim().to_string() };
            ui.horizontal(|ui| {
                if ui.button(say!("LumaLink.Window.ChoosePlace")).clicked() {
                    if let Some(place) = rfd::FileDialog::new().set_title(say!("LumaLink.Window.ChoosePlace")).pick_folder() {
                        form.place = Some(place);
                    }
                }
                if let Some(place) = &form.place {
                    let shown = chrome::fitted_tail(ui.painter(), &free_folder(place, &name).display().to_string(), chrome::body(13.0), SOFT, ui.available_width());
                    ui.add(Label::new(shown));
                }
            });
            for line in &form.refused {
                ui.label(RichText::new(&line.text).color(line.color));
            }
            ui.add_space(18.0);
            let size = vec2(150.0, 44.0);
            let spacing = 16.0;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 2.0 * size.x - spacing) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = spacing;
                if ui.add_sized(size, egui::Button::new(say!("LumaLink.Window.Update.Close"))).clicked() {
                    close = true;
                }
                if ui.add_enabled(form.place.is_some(), egui::Button::new(RichText::new(say!("LumaLink.Window.Make")).strong()).min_size(size)).clicked() {
                    if let Some(place) = form.place.clone() {
                        let folder = free_folder(&place, &name);
                        let authors: Vec<String> = form.authors.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
                        let id = if form.id.trim().is_empty() { id_from("", &name) } else { form.id.trim().to_string() };
                        form.refused = match jobs::new_mod(&folder, &id, &name, form.summary.trim(), &authors) {
                            Ok(NewMod::Made(folder)) => {
                                made = Some(folder);
                                vec![]
                            }
                            Ok(NewMod::Refused(r)) => report(&r, None),
                            Err(error) => vec![Line::words(format!("{error:#}"), RED)],
                        };
                    }
                }
            });
        });
        if let Some(folder) = made {
            self.new_form = None;
            self.added(vec![folder]);
        } else if close || modal.should_close() {
            self.new_form = None;
        }
    }
}

/// A file's or folder's name without its ending: the name a .luma or folder made from it takes.
fn stem_of(path: &Path) -> String {
    let stem = if path.is_dir() { path.file_name() } else { path.file_stem() };
    stem.map_or_else(|| "mod".into(), |s| s.to_string_lossy().into_owned())
}

/// `place`/`name`, or `name 2`, `name 3`... when that is taken.
fn free_folder(place: &Path, name: &str) -> PathBuf {
    let name = folder_name(name);
    (1..1000).map(|n| place.join(if n == 1 { name.clone() } else { format!("{name} {n}") })).find(|f| !f.exists()).unwrap_or_else(|| place.join(&name))
}

/// The name of a folder made for a mod named `name`, one folder in `place` on every system (N-4061): each character no
/// system's file names take (`<>:"/\|?*` and control characters) made a space, runs of spaces made one, the spaces and
/// dots at its end left off (Windows drops them), and a name Windows keeps for a device (CON, NUL, COM1...) given
/// ` mod`; `mod` when nothing is left.
pub(crate) fn folder_name(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect();
    let name = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim_end_matches(['.', ' ']).to_string();
    let device = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4 && (device.starts_with("COM") || device.starts_with("LPT")) && device.as_bytes()[3].is_ascii_digit());
    match name.as_str() {
        "" => "mod".to_string(),
        _ if reserved => format!("{name} mod"),
        _ => name,
    }
}

/// Where to save a .luma made from `from`: asked in the system's dialog, beside it and named after it to start with;
/// `.luma` added when the name chosen has another ending.
fn save_luma(from: &Path) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title(say!("LumaLink.Window.SaveLuma"))
        .add_filter(say!("LumaLink.Window.Filter.Luma"), &["luma"])
        .set_file_name(format!("{}.luma", stem_of(from)));
    if let Some(dir) = from.parent() {
        dialog = dialog.set_directory(dir);
    }
    let chosen = dialog.save_file()?;
    if chosen.extension().is_some_and(|e| e.eq_ignore_ascii_case("luma")) {
        return Some(chosen);
    }
    let mut named = chosen.into_os_string();
    named.push(".luma");
    Some(PathBuf::from(named))
}
