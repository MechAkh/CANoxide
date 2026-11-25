//! Dashboard view: widgets (value, bar, dial, lamp) over the signals of the
//! signal store, on a canvas where each card has its own place and size
//! (drag the title row to move, the corner grip to resize; both snap to a
//! 10-point grid). A widget gets its signal through the Add signal picker
//! of the Graph; the samples are shared with the Graph
//! (`SignalStore::acquire_dash`), so a signal that both show is polled
//! once. The widget settings, the layout rules and the file are in
//! `crate::dashboard`.
//!
//! Draw order: the widget list. The card that was last moved, resized or
//! added is at the end of the list, so it is drawn (and hit) on top.

use super::graph::{source_text, AddDraft};
use super::theme;
use super::App;
use crate::bus::BusCommand;
use crate::dashboard::{age_text, is_stale, snap, Dashboard, Level, Widget, WidgetKind, WidgetRect, WidgetSize, DASHBOARD_FILE, DEFAULT_ARRANGE_WIDTH, MAX_H, MAX_W, MIN_H, MIN_W};
use crate::signals::SignalDef;
use eframe::egui;
use egui_phosphor::regular as icon;

/// The widget dialog: `idx` None for a new widget.
pub(crate) struct DashEdit {
    pub idx: Option<usize>,
    pub w: Widget,
}

/// A move or a resize by the pointer.
#[derive(Clone, Copy, Debug)]
struct CardDrag {
    uid: u64,
    resize: bool,
    /// The rectangle when the drag started.
    start: WidgetRect,
    /// Where the button went down.
    press: egui::Pos2,
}

#[derive(Default)]
pub(crate) struct DashView {
    pub board: Dashboard,
    pub edit: Option<DashEdit>,
    drag: Option<CardDrag>,
    /// The visible width of the canvas in the last frame (Arrange, the
    /// place of a new widget).
    pub view_width: f32,
    next_uid: u64,
    /// The card rectangles (screen) of the last frame, in list order (tests).
    #[cfg(test)]
    pub cards: Vec<egui::Rect>,
}

impl DashView {
    fn new_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }

    /// A move or a resize is running (tests).
    #[cfg(test)]
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn width(&self) -> f32 {
        if self.view_width > 0.0 {
            self.view_width
        } else {
            DEFAULT_ARRANGE_WIDTH
        }
    }
}

/// What a widget shows in this frame.
struct Reading {
    value: Option<f64>,
    age_s: Option<f64>,
    stale: bool,
}

/// What the pointer did on a card.
struct CardOut {
    edit: bool,
    remove: bool,
    mv: egui::Response,
    grip: egui::Response,
}

impl App {
    /// Load the dashboard of the data folder and take its signals (no
    /// polling starts).
    pub(crate) fn dash_load(&mut self) -> Option<String> {
        let lib = self.lib.as_ref()?;
        let path = lib.root().join(DASHBOARD_FILE);
        let r = Dashboard::load(&path);
        match r {
            Ok(b) => {
                self.dash.board = b;
                for i in 0..self.dash.board.widgets.len() {
                    let u = self.dash.new_uid();
                    self.dash.board.widgets[i].uid = u;
                }
                if let Ok(mut s) = self.bus.signals.lock() {
                    for w in &mut self.dash.board.widgets {
                        w.signal_id = Some(s.acquire_dash(&w.signal));
                    }
                }
                None
            }
            Err(e) => Some(format!("Dashboard not read: {e}")),
        }
    }

    fn dash_save(&mut self) {
        if self.data_error.is_some() {
            return;
        }
        let Some(lib) = &self.lib else { return };
        let path = lib.root().join(DASHBOARD_FILE);
        if let Err(e) = self.dash.board.save(&path) {
            self.set_status(format!("Dashboard not saved: {e}"), true);
        }
    }

    /// The Add signal picker (or a button of the Messages view) gave a
    /// signal for a new widget: open the widget dialog.
    pub(crate) fn dash_add_widget(&mut self, def: SignalDef) {
        self.dash.edit = Some(DashEdit { idx: None, w: Widget::new(def) });
    }

    fn dash_remove(&mut self, i: usize) {
        if i >= self.dash.board.widgets.len() {
            return;
        }
        let w = self.dash.board.widgets.remove(i);
        if let (Some(id), Ok(mut s)) = (w.signal_id, self.bus.signals.lock()) {
            s.release_dash(id);
        }
        if let Some(e) = self.dash.edit.as_mut() {
            match e.idx {
                Some(j) if j == i => self.dash.edit = None,
                Some(j) if j > i => e.idx = Some(j - 1),
                _ => {}
            }
        }
        self.dash_save();
    }

    /// Draw widget `i` on top (the end of the list); the widget dialog keeps
    /// its widget.
    fn dash_front(&mut self, i: usize) {
        let n = self.dash.board.widgets.len();
        if i + 1 >= n {
            return;
        }
        self.dash.board.bring_to_front(i);
        if let Some(e) = self.dash.edit.as_mut() {
            e.idx = e.idx.map(|j| if j == i { n - 1 } else if j > i { j - 1 } else { j });
        }
    }

    /// Place all widgets in rows that fit the visible width.
    pub(crate) fn dash_arrange(&mut self) {
        let w = self.dash.width();
        self.dash.board.arrange(w);
        self.dash_save();
    }

    fn dash_commit(&mut self, e: DashEdit) {
        let mut w = e.w;
        match e.idx {
            Some(i) if i < self.dash.board.widgets.len() => {
                let old = &self.dash.board.widgets[i];
                w.signal_id = old.signal_id;
                w.uid = old.uid;
                // The place stays (it may have moved while the dialog was
                // open); the size is the one of the dialog.
                let (o, n) = (old.rect(), w.rect());
                w.rect = Some(WidgetRect { x: o.x, y: o.y, w: n.w, h: n.h }.snapped());
                self.dash.board.widgets[i] = w;
            }
            _ => {
                w.signal_id = self.bus.signals.lock().ok().map(|mut s| s.acquire_dash(&w.signal));
                w.uid = self.dash.new_uid();
                let r = w.rect().snapped();
                let (x, y) = self.dash.board.free_position(r.w, r.h, self.dash.width());
                w.rect = Some(WidgetRect { x, y, ..r });
                let t = w.title.clone();
                self.dash.board.widgets.push(w);
                self.set_status(format!("Dashboard widget {t} added"), false);
            }
        }
        self.dash_save();
    }

    pub(crate) fn dashboard_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let (now, polling_on) = self.bus.shared.lock().map(|s| (s.now_us as f64 / 1e6, s.signal_polling)).unwrap_or((0.0, false));
        let has_polls = self.bus.signals.lock().map(|s| s.has_polls()).unwrap_or(false);
        let mut arrange = false;
        theme::toolbar(ui, |ui| {
            if ui.button(format!("{} Add signal...", icon::PLUS)).on_hover_text("Add a widget for a signal (the picker of the Graph)").clicked() {
                self.graph.add = Some(AddDraft { node: self.active_node(), for_dashboard: true, ..AddDraft::default() });
            }
            if theme::icon_button(ui, !self.dash.board.widgets.is_empty(), icon::SQUARES_FOUR, "Arrange", "Place the widgets in rows, in their order").clicked() {
                arrange = true;
            }
            ui.separator();
            if polling_on {
                if theme::icon_button(ui, true, icon::STOP, "Stop polling", "Stop polling the SDO signals").clicked() {
                    self.bus.send(BusCommand::SetSignalPolling(false));
                }
                theme::pill(ui, "polling", p.info, p.info_tint, self.settings.reduce_colour);
            } else {
                let can = self.can_send();
                let tip = if can { "Start polling: read the SDO signals periodically" } else { self.send_hint() };
                if theme::icon_button(ui, can && has_polls, icon::PLAY, "Start polling", tip).clicked() {
                    self.bus.send(BusCommand::SetSignalPolling(true));
                }
            }
        });
        if arrange {
            self.dash_arrange();
        }
        if self.dash.board.widgets.is_empty() {
            #[cfg(test)]
            self.dash.cards.clear();
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(icon::GAUGE).size(28.0).color(p.text_secondary));
                ui.label("No widgets.");
                ui.label(egui::RichText::new("Press Add signal to show a value as a number, a bar, a dial or a lamp.").color(p.text_secondary));
            });
            return;
        }
        let readings: Vec<Reading> = match self.bus.signals.lock() {
            Ok(s) => self
                .dash
                .board
                .widgets
                .iter()
                .map(|w| {
                    let sig = w.signal_id.and_then(|id| s.get(id));
                    match sig.and_then(|g| g.last_point().map(|pt| (pt, g.period_s()))) {
                        Some(((t, v), per)) => Reading { value: Some(v), age_s: Some(now - t), stale: is_stale(t, now, per) },
                        None => Reading { value: None, age_s: None, stale: false },
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let mut outs: Vec<CardOut> = Vec::new();
        let dragging = self.dash.drag.is_some();
        let mut view_width = self.dash.view_width;
        #[cfg(test)]
        let mut cards = Vec::new();
        egui::ScrollArea::both().id_salt("dash_scroll").auto_shrink([false, false]).show(ui, |ui| {
            let view = ui.available_size();
            view_width = view.x;
            let board = &self.dash.board;
            let right = board.widgets.iter().map(|w| w.rect().x + w.rect().w).fold(0.0, f32::max);
            let size = egui::vec2(view.x.max(right + 10.0), view.y.max(board.bottom() + 10.0));
            let (canvas, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let origin = canvas.min;
            if dragging {
                grid(ui, p, canvas, origin);
            }
            for (w, r) in board.widgets.iter().zip(readings.iter()) {
                let wr = w.rect();
                let card = egui::Rect::from_min_size(origin + egui::vec2(wr.x, wr.y), egui::vec2(wr.w, wr.h));
                #[cfg(test)]
                cards.push(card);
                let mut cui = ui.new_child(egui::UiBuilder::new().id_salt(("dash_card", w.uid)).max_rect(card).layout(egui::Layout::top_down(egui::Align::Min)));
                cui.set_clip_rect(card.intersect(ui.clip_rect()));
                cui.style_mut().interaction.selectable_labels = false;
                outs.push(widget_card(&mut cui, p, w, r, card));
            }
        });
        self.dash.view_width = view_width;
        #[cfg(test)]
        {
            self.dash.cards = cards;
        }
        // Edit / Remove, then the drag (a move or a resize of one card).
        let (ptr, press, down) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.press_origin(), i.pointer.primary_down()));
        let mut edit = None;
        let mut remove = None;
        let mut front = None;
        let mut save = false;
        for (i, o) in outs.iter().enumerate() {
            if o.edit {
                edit = Some(i);
            }
            if o.remove {
                remove = Some(i);
            }
            let (uid, wrect) = {
                let w = &self.dash.board.widgets[i];
                (w.uid, w.rect())
            };
            for (resp, resize) in [(&o.mv, false), (&o.grip, true)] {
                if resp.drag_started() {
                    self.dash.drag = Some(CardDrag { uid, resize, start: wrect, press: press.or(ptr).unwrap_or(resp.rect.center()) });
                    front = Some(i);
                }
                let mine = self.dash.drag.is_some_and(|d| d.uid == uid && d.resize == resize);
                if mine && (resp.dragged() || resp.drag_stopped()) {
                    if let (Some(d), Some(q)) = (self.dash.drag, ptr) {
                        let dv = q - d.press;
                        let s = d.start;
                        let r = if resize {
                            WidgetRect { w: snap(s.w + dv.x).clamp(MIN_W, MAX_W), h: snap(s.h + dv.y).clamp(MIN_H, MAX_H), ..s }
                        } else {
                            WidgetRect { x: snap(s.x + dv.x).max(0.0), y: snap(s.y + dv.y).max(0.0), ..s }
                        };
                        self.dash.board.widgets[i].rect = Some(r);
                    }
                }
                if mine && resp.drag_stopped() {
                    self.dash.drag = None;
                    save = true;
                }
            }
        }
        if let Some(d) = self.dash.drag {
            if !down && !outs.iter().any(|o| o.mv.dragged() || o.grip.dragged()) {
                // The card went away during the drag (or the button came up
                // outside of egui).
                self.dash.drag = None;
                save = true;
            } else {
                ui.ctx().set_cursor_icon(if d.resize { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::Grabbing });
                ui.ctx().request_repaint();
            }
        }
        if let Some(i) = front {
            self.dash_front(i);
            let n = self.dash.board.widgets.len();
            edit = edit.map(|j| if j == i { n - 1 } else if j > i { j - 1 } else { j });
            remove = remove.map(|j| if j == i { n - 1 } else if j > i { j - 1 } else { j });
        }
        if save {
            self.dash_save();
        }
        if let Some(i) = edit {
            self.dash.edit = Some(DashEdit { idx: Some(i), w: self.dash.board.widgets[i].clone() });
        }
        if let Some(i) = remove {
            self.dash_remove(i);
        }
        if self.connected {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub(crate) fn dash_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.dash.edit.take() else { return };
        let mut keep = true;
        let mut save = false;
        let mut open = true;
        egui::Window::new("Dashboard widget")
            .id(egui::Id::new("dash_edit"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                let w = &mut e.w;
                egui::Grid::new("dash_edit_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Signal");
                    ui.label(source_text(&w.signal.source));
                    ui.end_row();
                    ui.label("Title");
                    ui.add(egui::TextEdit::singleline(&mut w.title).desired_width(200.0));
                    ui.end_row();
                    ui.label("Type");
                    egui::ComboBox::from_id_salt("dash_kind").selected_text(w.kind.name()).show_ui(ui, |ui| {
                        for k in WidgetKind::ALL {
                            ui.selectable_value(&mut w.kind, k, k.name());
                        }
                    });
                    ui.end_row();
                    // A preset of the card size; a card resized by the
                    // pointer shows its size.
                    ui.label("Size");
                    let r = w.rect();
                    let cur = WidgetSize::of_card(r.w, r.h);
                    let text = match cur {
                        Some(s) => s.name().to_string(),
                        None => format!("Custom ({:.0} × {:.0})", r.w, r.h),
                    };
                    egui::ComboBox::from_id_salt("dash_size").selected_text(text).show_ui(ui, |ui| {
                        for s in WidgetSize::ALL {
                            let (cw, ch) = s.card();
                            if ui.selectable_label(cur == Some(s), s.name()).on_hover_text(format!("{cw:.0} × {ch:.0}")).clicked() {
                                w.size = s;
                                w.rect = Some(WidgetRect { w: cw, h: ch, ..r }.snapped());
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Min");
                    ui.add(egui::DragValue::new(&mut w.min).speed(0.1));
                    ui.end_row();
                    ui.label("Max");
                    ui.add(egui::DragValue::new(&mut w.max).speed(0.1));
                    ui.end_row();
                    ui.label("Decimals");
                    ui.add(egui::DragValue::new(&mut w.decimals).range(0..=6));
                    ui.end_row();
                    for (label, v) in [("Warning at", &mut w.warn), ("Alarm at", &mut w.alarm)] {
                        ui.label(label);
                        ui.horizontal(|ui| {
                            let mut on = v.is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                *v = if on { Some(0.0) } else { None };
                            }
                            if let Some(x) = v.as_mut() {
                                ui.add(egui::DragValue::new(x).speed(0.1));
                            }
                        });
                        ui.end_row();
                    }
                    if w.kind == WidgetKind::Lamp {
                        ui.label("Lamp on above");
                        ui.horizontal(|ui| {
                            let mut on = w.lamp_threshold.is_some();
                            if ui.checkbox(&mut on, "").on_hover_text("Off: the lamp is on when the value is not 0").changed() {
                                w.lamp_threshold = if on { Some(0.0) } else { None };
                            }
                            if let Some(x) = w.lamp_threshold.as_mut() {
                                ui.add(egui::DragValue::new(x).speed(0.1));
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        if save {
            self.dash_commit(e);
        } else if keep && open {
            self.dash.edit = Some(e);
        }
    }
}

/// The snap grid, faint, while a card is dragged (only the visible part).
fn grid(ui: &egui::Ui, p: &theme::Palette, canvas: egui::Rect, origin: egui::Pos2) {
    let vis = canvas.intersect(ui.clip_rect());
    let c = p.border_subtle.gamma_multiply(0.45);
    let painter = ui.painter();
    let step = crate::dashboard::GRID;
    let mut x = origin.x + ((vis.left() - origin.x) / step).ceil() * step;
    while x <= vis.right() {
        painter.line_segment([egui::pos2(x, vis.top()), egui::pos2(x, vis.bottom())], egui::Stroke::new(1.0_f32, c));
        x += step;
    }
    let mut y = origin.y + ((vis.top() - origin.y) / step).ceil() * step;
    while y <= vis.bottom() {
        painter.line_segment([egui::pos2(vis.left(), y), egui::pos2(vis.right(), y)], egui::Stroke::new(1.0_f32, c));
        y += step;
    }
}

/// The largest monospace size (12 to 96 pt) with which `text` fits in
/// `avail` on one line.
fn fit_size(ui: &egui::Ui, text: &str, avail: egui::Vec2) -> f32 {
    const REF: f32 = 32.0;
    let g = ui.fonts(|f| f.layout_no_wrap(text.to_string(), egui::FontId::monospace(REF), egui::Color32::WHITE)).size();
    let by_w = if g.x > 0.0 { REF * avail.x / g.x } else { 96.0 };
    let by_h = if g.y > 0.0 { REF * avail.y / g.y } else { 96.0 };
    // A little under the exact fit: the glyph sizes do not scale exactly.
    (by_w.min(by_h) * 0.95).floor().clamp(12.0, 96.0)
}

/// One line of text, centred in `area`, never wrapped.
fn centred_text(ui: &mut egui::Ui, area: egui::Rect, text: egui::RichText, line_h: f32) {
    let r = egui::Rect::from_center_size(area.center(), egui::vec2(area.width(), line_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
        ui.add(egui::Label::new(text).extend());
    });
}

/// The value text in `area`, as large as it fits.
fn value_text(ui: &mut egui::Ui, area: egui::Rect, text: &str, col: egui::Color32) {
    let size = fit_size(ui, text, area.size());
    let line_h = ui.fonts(|f| f.row_height(&egui::FontId::monospace(size)));
    centred_text(ui, area, egui::RichText::new(text).size(size).monospace().color(col), line_h);
}

/// One widget card in its own top-down `Ui` on the rect `card`: the frame,
/// the title row (the move handle) with Edit and Remove, the content scaled
/// to the card, the resize grip.
fn widget_card(ui: &mut egui::Ui, p: &theme::Palette, w: &Widget, r: &Reading, card: egui::Rect) -> CardOut {
    // The card takes the pointer, so that a card below it gets nothing.
    let _ = ui.interact(card, ui.id().with("bg"), egui::Sense::click_and_drag());
    ui.painter().rect(card, 6.0, p.bg_panel, egui::Stroke::new(1.0_f32, p.border_subtle), egui::StrokeKind::Inside);
    let inner = card.shrink(8.0);
    let pad = ui.spacing().button_padding;
    let glyph_h = ui.fonts(|f| f.row_height(&egui::FontId::proportional(16.0)));
    let row_h = ui.spacing().interact_size.y.max(glyph_h + 2.0 * pad.y);
    let title_row = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), row_h));
    let gap = ui.spacing().item_spacing.x;
    let bw = theme::icon_button_width(ui, icon::TRASH) + theme::icon_button_width(ui, icon::PENCIL_SIMPLE) + gap;
    let btn_rect = egui::Rect::from_min_max(egui::pos2((title_row.right() - bw).max(title_row.left()), title_row.top()), title_row.max);
    let title_rect = egui::Rect::from_min_max(title_row.min, egui::pos2((btn_rect.left() - 4.0).max(title_row.left()), title_row.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(title_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.add(egui::Label::new(egui::RichText::new(&w.title).strong()).truncate());
    });
    let mv = ui.interact(title_rect, ui.id().with("move"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
    let mut edit = false;
    let mut remove = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(btn_rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        if theme::icon_button(ui, true, icon::TRASH, "Remove", "Remove the widget").clicked() {
            remove = true;
        }
        if theme::icon_button(ui, true, icon::PENCIL_SIMPLE, "Edit", "Edit the widget").clicked() {
            edit = true;
        }
    });
    let small_h = ui.fonts(|f| f.row_height(&egui::TextStyle::Small.resolve(ui.style())));
    let foot = if r.stale && r.age_s.is_some() { small_h + 2.0 } else { 0.0 };
    let body = egui::Rect::from_min_max(egui::pos2(inner.left(), title_row.bottom() + 4.0), egui::pos2(inner.right(), (inner.bottom() - foot).max(title_row.bottom() + 4.0)));
    match r.value {
        None => {
            let h = ui.fonts(|f| f.row_height(&egui::TextStyle::Body.resolve(ui.style())));
            centred_text(ui, body, egui::RichText::new("No value yet").color(p.text_secondary), h);
        }
        Some(v) => card_content(ui, p, w, r, v, body),
    }
    if foot > 0.0 {
        if let Some(a) = r.age_s {
            let fr = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - foot), inner.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(fr).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(format!("{}  age {}", icon::CLOCK, age_text(a))).small().color(p.warning)).truncate());
            });
        }
    }
    // The resize grip in the bottom-right corner: three short diagonal
    // strokes.
    let grip_rect = egui::Rect::from_min_max(card.max - egui::vec2(14.0, 14.0), card.max);
    let grip = ui.interact(grip_rect, ui.id().with("resize"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeNwSe);
    let gc = if grip.hovered() || grip.dragged() { p.text_primary } else { p.text_secondary };
    for k in [4.0, 8.0, 12.0] {
        let a = egui::pos2(card.right() - 3.0, card.bottom() - 3.0 - k + 2.0);
        let b = egui::pos2(card.right() - 3.0 - k + 2.0, card.bottom() - 3.0);
        ui.painter().line_segment([a, b], egui::Stroke::new(1.0_f32, gc));
    }
    CardOut { edit, remove, mv, grip }
}

/// The value as a number, a bar, a dial or a lamp, scaled to `body`.
fn card_content(ui: &mut egui::Ui, p: &theme::Palette, w: &Widget, r: &Reading, v: f64, body: egui::Rect) {
    let level = w.level(v);
    let mut col = match level {
        Level::Normal => p.text_accent,
        Level::Warning => p.warning,
        Level::Alarm => p.error,
    };
    let mut text_col = if level == Level::Normal { p.text_primary } else { col };
    if r.stale {
        col = col.gamma_multiply(0.35);
        text_col = p.text_disabled;
    }
    let value = if w.signal.unit.is_empty() { w.format(v) } else { format!("{} {}", w.format(v), w.signal.unit) };
    // The text part under a bar, a dial or a lamp.
    let text_h = (body.height() * 0.3).clamp(16.0, 110.0).min(body.height());
    let split = |top_gap: f32| {
        let top = egui::Rect::from_min_max(body.min, egui::pos2(body.right(), (body.bottom() - text_h).max(body.top())));
        let text = egui::Rect::from_min_max(egui::pos2(body.left(), top.bottom() + top_gap), body.max);
        (top, text)
    };
    match w.kind {
        WidgetKind::Value => value_text(ui, body, &value, text_col),
        WidgetKind::Bar => {
            let bar_h = (body.height() * 0.22).clamp(8.0, 40.0);
            let track = egui::Rect::from_min_size(body.min + egui::vec2(0.0, 4.0), egui::vec2(body.width(), bar_h));
            let painter = ui.painter();
            painter.rect_filled(track, 3.0, p.bg_button);
            let mut f = track;
            f.set_width(track.width() * w.fraction(v) as f32);
            painter.rect_filled(f, 3.0, col);
            for t in [w.warn, w.alarm].into_iter().flatten() {
                let x = track.left() + track.width() * w.fraction(t) as f32;
                painter.line_segment([egui::pos2(x, track.top() - 2.0), egui::pos2(x, track.bottom() + 2.0)], egui::Stroke::new(1.5_f32, p.text_secondary));
            }
            let text = egui::Rect::from_min_max(egui::pos2(body.left(), track.bottom() + 6.0), egui::pos2(body.right(), body.bottom().max(track.bottom() + 6.0)));
            value_text(ui, text, &value, text_col);
        }
        WidgetKind::Dial => {
            let (dial, text) = split(2.0);
            // The 270° arc reaches 0.71 r below the centre.
            let radius = ((dial.width() / 2.0 - 4.0).min((dial.height() - 8.0) / 1.71)).max(4.0);
            let free = (dial.height() - 8.0 - 1.71 * radius).max(0.0);
            let c = egui::pos2(dial.center().x, dial.top() + 4.0 + radius + free / 2.0);
            let at = |f: f64, rr: f32| {
                let a = (225.0 - 270.0 * f).to_radians() as f32;
                egui::pos2(c.x + rr * a.cos(), c.y - rr * a.sin())
            };
            let arc = |f0: f64, f1: f64| -> Vec<egui::Pos2> {
                let n = 48;
                (0..=n).map(|k| at(f0 + (f1 - f0) * k as f64 / n as f64, radius)).collect()
            };
            let painter = ui.painter();
            let width = (radius * 0.14).max(3.0);
            painter.add(egui::Shape::line(arc(0.0, 1.0), egui::Stroke::new(width, p.bg_button)));
            let fv = w.fraction(v);
            if fv > 0.0 {
                painter.add(egui::Shape::line(arc(0.0, fv), egui::Stroke::new(width, col)));
            }
            for t in [w.warn, w.alarm].into_iter().flatten() {
                let f = w.fraction(t);
                painter.line_segment([at(f, radius - width), at(f, radius + width * 0.6)], egui::Stroke::new(1.5_f32, p.text_secondary));
            }
            painter.line_segment([c, at(fv, radius - width * 1.2)], egui::Stroke::new(2.0_f32, text_col));
            painter.circle_filled(c, 3.0, text_col);
            value_text(ui, text, &value, text_col);
        }
        WidgetKind::Lamp => {
            let on = w.lamp_on(v);
            let (top, text) = split(4.0);
            let d = (top.width().min(top.height()) - 4.0).max(4.0);
            let lamp = if on {
                let base = if level == Level::Normal { p.success } else { col };
                if r.stale {
                    base.gamma_multiply(0.35)
                } else {
                    base
                }
            } else {
                p.bg_button
            };
            ui.painter().circle(top.center(), d / 2.0, lamp, egui::Stroke::new(1.0_f32, p.border_strong));
            value_text(ui, text, &format!("{} ({value})", if on { "ON" } else { "OFF" }), text_col);
        }
    }
}
