//! Dashboard view: widgets (value, bar, dial, lamp) over the signals of the
//! signal store, in a wrapping grid. A widget gets its signal through the
//! Add signal picker of the Graph; the samples are shared with the Graph
//! (`SignalStore::acquire_dash`), so a signal that both show is polled
//! once. The widget settings and the rules are in `crate::dashboard`.

use super::graph::{source_text, AddDraft};
use super::theme;
use super::App;
use crate::bus::BusCommand;
use crate::dashboard::{age_text, is_stale, Dashboard, Level, Widget, WidgetKind, WidgetSize, DASHBOARD_FILE};
use crate::signals::SignalDef;
use eframe::egui;
use egui_phosphor::regular as icon;

/// The widget dialog: `idx` None for a new widget.
pub(crate) struct DashEdit {
    pub idx: Option<usize>,
    pub w: Widget,
}

#[derive(Default)]
pub(crate) struct DashView {
    pub board: Dashboard,
    pub edit: Option<DashEdit>,
}

/// What a widget shows in this frame.
struct Reading {
    value: Option<f64>,
    age_s: Option<f64>,
    stale: bool,
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

    /// The Add signal picker gave a signal for a new widget: open the widget
    /// dialog.
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
        self.dash_save();
    }

    fn dash_commit(&mut self, e: DashEdit) {
        let mut w = e.w;
        match e.idx {
            Some(i) if i < self.dash.board.widgets.len() => {
                w.signal_id = self.dash.board.widgets[i].signal_id;
                self.dash.board.widgets[i] = w;
            }
            _ => {
                w.signal_id = self.bus.signals.lock().ok().map(|mut s| s.acquire_dash(&w.signal));
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
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{} Add signal...", icon::PLUS)).on_hover_text("Add a widget for a signal (the picker of the Graph)").clicked() {
                self.graph.add = Some(AddDraft { node: self.active_node(), for_dashboard: true, ..AddDraft::default() });
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
        if self.dash.board.widgets.is_empty() {
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
        let mut edit = None;
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("dash_scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                for (i, (w, r)) in self.dash.board.widgets.iter().zip(readings.iter()).enumerate() {
                    let (cw, ch) = w.size.card();
                    ui.push_id(("dash_widget", i), |ui| {
                        ui.allocate_ui(egui::vec2(cw, ch), |ui| {
                            egui::Frame::new().fill(p.bg_panel).stroke(egui::Stroke::new(1.0_f32, p.border_subtle)).corner_radius(6).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                                ui.set_min_size(egui::vec2(cw - 16.0, ch - 16.0));
                                ui.set_max_size(egui::vec2(cw - 16.0, ch - 16.0));
                                let (e, rm) = widget_card(ui, p, w, r);
                                if e {
                                    edit = Some(i);
                                }
                                if rm {
                                    remove = Some(i);
                                }
                            });
                        });
                    });
                }
            });
        });
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
                    ui.label("Size");
                    egui::ComboBox::from_id_salt("dash_size").selected_text(w.size.name()).show_ui(ui, |ui| {
                        for s in WidgetSize::ALL {
                            ui.selectable_value(&mut w.size, s, s.name());
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

/// One widget card; returns (edit clicked, remove clicked).
fn widget_card(ui: &mut egui::Ui, p: &theme::Palette, w: &Widget, r: &Reading) -> (bool, bool) {
    let mut edit = false;
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(&w.title).strong()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, true, icon::TRASH, "Remove", "Remove the widget").clicked() {
                remove = true;
            }
            if theme::icon_button(ui, true, icon::PENCIL_SIMPLE, "Edit", "Edit the widget").clicked() {
                edit = true;
            }
        });
    });
    let Some(v) = r.value else {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("No value yet").color(p.text_secondary));
        return (edit, remove);
    };
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
    let value_text = if w.signal.unit.is_empty() { w.format(v) } else { format!("{} {}", w.format(v), w.signal.unit) };
    let big = match w.size {
        WidgetSize::Small => 22.0,
        WidgetSize::Medium => 30.0,
        WidgetSize::Large => 42.0,
    };
    let foot = ui.spacing().interact_size.y + 4.0;
    match w.kind {
        WidgetKind::Value => {
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(&value_text).size(big).monospace().color(text_col));
            });
        }
        WidgetKind::Bar => {
            let h = (ui.available_height() - foot * 1.5).clamp(12.0, 28.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 3.0, p.bg_button);
            let mut f = rect;
            f.set_width(rect.width() * w.fraction(v) as f32);
            painter.rect_filled(f, 3.0, col);
            for t in [w.warn, w.alarm].into_iter().flatten() {
                let x = rect.left() + rect.width() * w.fraction(t) as f32;
                painter.line_segment([egui::pos2(x, rect.top() - 2.0), egui::pos2(x, rect.bottom() + 2.0)], egui::Stroke::new(1.5_f32, p.text_secondary));
            }
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(&value_text).size(big * 0.75).monospace().color(text_col));
            });
        }
        WidgetKind::Dial => {
            let avail = egui::vec2(ui.available_width(), (ui.available_height() - foot).max(30.0));
            let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
            let radius = (rect.width() / 2.0).min(rect.height() * 0.62) - 4.0;
            let c = egui::pos2(rect.center().x, rect.top() + radius + 4.0);
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
            painter.text(egui::pos2(c.x, c.y + radius * 0.45), egui::Align2::CENTER_CENTER, &value_text, egui::FontId::monospace(big * 0.6), text_col);
            // The value as text for the accessibility tree.
            ui.label(egui::RichText::new(&value_text).small().color(p.text_secondary));
        }
        WidgetKind::Lamp => {
            let on = w.lamp_on(v);
            let d = (ui.available_height() - foot * 1.5).clamp(16.0, 80.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), d), egui::Sense::hover());
            let lamp = if on {
                let base = if level == Level::Normal { p.success } else { col };
                if r.stale { base.gamma_multiply(0.35) } else { base }
            } else {
                p.bg_button
            };
            ui.painter().circle(rect.center(), d / 2.0 - 2.0, lamp, egui::Stroke::new(1.0_f32, p.border_strong));
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(format!("{} ({value_text})", if on { "ON" } else { "OFF" })).monospace().color(text_col));
            });
        }
    }
    if r.stale {
        if let Some(a) = r.age_s {
            ui.label(egui::RichText::new(format!("{}  age {}", icon::CLOCK, age_text(a))).small().color(p.warning));
        }
    }
    (edit, remove)
}
