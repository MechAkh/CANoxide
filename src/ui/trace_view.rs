//! Trace tabs (up to four): each one is a view of the shared ring buffer
//! with its own filter, display mode, chronological / fixed mode and
//! pause. Rows are virtualised; each frame is decoded once on receive.

use super::{theme, App};
use crate::canopen::decode::MsgClass;
use crate::canopen::types::{format_bytes, parse_int, DisplayMode};
use crate::settings::{DataView, Settings, TimestampFormat, TraceColour};
use crate::trace::{id_text, TraceEntry, TraceFilter, ViewId};
use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};

pub(crate) struct TraceView {
    /// The view of the trace store.
    pub(crate) view: ViewId,
    autoscroll: bool,
    fixed: bool,
    delta: bool,
    mode: DisplayMode,
    filter: TraceFilter,
    node_text: String,
    id_min_text: String,
    id_max_text: String,
    /// Frames received when the view was paused.
    paused_at: u64,
}

impl TraceView {
    pub(crate) fn new(view: ViewId, s: &Settings) -> TraceView {
        TraceView {
            view,
            autoscroll: s.trace_autoscroll,
            fixed: s.default_data_view == DataView::Fixed,
            delta: s.timestamp_format == TimestampFormat::Delta,
            mode: s.trace_display,
            filter: TraceFilter::default(),
            node_text: String::new(),
            id_min_text: String::new(),
            id_max_text: String::new(),
            paused_at: 0,
        }
    }
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

#[derive(Clone, Copy)]
struct Look {
    mode: DisplayMode,
    colour: TraceColour,
    outlined: bool,
    p: &'static theme::Palette,
}

fn common_cols(row: &mut TableRow<'_, '_>, e: &TraceEntry, look: Look) {
    let f = &e.frame;
    row.col(|ui| {
        ui.label(f.dlc.to_string());
    });
    row.col(|ui| {
        let d = if f.rtr { "RTR".to_string() } else { format_bytes(f.payload(), look.mode) };
        ui.add(egui::Label::new(egui::RichText::new(d).monospace()).truncate());
    });
    let (fg, bg) = theme::class_colours(look.p, e.decoded.class);
    row.col(|ui| {
        if look.colour == TraceColour::Off {
            ui.add(egui::Label::new(&e.decoded.label).truncate());
        } else {
            theme::pill(ui, e.decoded.label.clone(), fg, bg, look.outlined);
        }
    });
    row.col(|ui| {
        ui.label(e.decoded.node.map(|n| n.to_string()).unwrap_or_default());
    });
    row.col(|ui| {
        let t = if look.colour == TraceColour::ChipsAndTint { egui::RichText::new(&e.decoded.text).color(fg) } else { egui::RichText::new(&e.decoded.text) };
        ui.add(egui::Label::new(t).truncate()).on_hover_text(&e.decoded.text);
    });
}

impl App {
    pub(crate) fn trace_tab(&mut self, ui: &mut egui::Ui, n: u8) {
        let Some(mut tv) = self.traces.remove(&n) else {
            ui.label("This trace tab has no view.");
            return;
        };
        self.trace_body(ui, &mut tv, n);
        self.traces.insert(n, tv);
    }

    fn trace_body(&mut self, ui: &mut egui::Ui, tv: &mut TraceView, _n: u8) {
        let p = theme::pal(ui);
        let mut paused = self.trace.view_paused(tv.view);
        // Toolbar.
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut paused, "Pause").on_hover_text("Stop adding rows to this trace tab").changed() {
                self.trace.set_view_paused(tv.view, paused);
                tv.paused_at = self.trace.total();
            }
            ui.checkbox(&mut tv.autoscroll, "Auto-scroll");
            ui.separator();
            ui.selectable_value(&mut tv.fixed, false, "Chronological");
            ui.selectable_value(&mut tv.fixed, true, "Fixed");
            ui.separator();
            ui.selectable_value(&mut tv.delta, false, "Abs time");
            ui.selectable_value(&mut tv.delta, true, "Delta time");
            ui.separator();
            for m in DisplayMode::ALL {
                ui.selectable_value(&mut tv.mode, m, m.label());
            }
            ui.separator();
            if ui.button("Clear").on_hover_text("Clear the buffer of every trace tab").clicked() {
                self.trace.clear();
                tv.paused_at = self.trace.total();
            }
            if ui.button("Save CSV...").clicked() {
                self.save_view_csv(tv.view, tv.mode);
            }
            // This tab is taken out of `traces` while it is drawn.
            let open = self.traces.len() + 1;
            if ui.add_enabled(open < super::dock::MAX_TRACES as usize, egui::Button::new(egui_phosphor::regular::PLUS)).on_hover_text("New trace tab").clicked() {
                self.new_trace_tab();
            }
        });
        // Filters.
        ui.horizontal_wrapped(|ui| {
            ui.label("Node");
            ui.add(egui::TextEdit::singleline(&mut tv.node_text).desired_width(36.0).hint_text("all"));
            ui.label("COB-ID 0x");
            ui.add(egui::TextEdit::singleline(&mut tv.id_min_text).desired_width(70.0).hint_text("min"));
            ui.label("..");
            ui.add(egui::TextEdit::singleline(&mut tv.id_max_text).desired_width(70.0).hint_text("max"));
            let n_on = MsgClass::ALL.iter().filter(|c| tv.filter.class_enabled(**c)).count();
            ui.menu_button(format!("Types ({n_on}/{})", MsgClass::ALL.len()), |ui| {
                for c in MsgClass::ALL {
                    let mut on = tv.filter.class_enabled(c);
                    if ui.checkbox(&mut on, c.label()).changed() {
                        tv.filter.set_class(c, on);
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("All").clicked() {
                        tv.filter.classes = 0xFFFF;
                    }
                    if ui.button("None").clicked() {
                        tv.filter.classes = 0;
                    }
                });
            });
            ui.label("Text");
            ui.add(egui::TextEdit::singleline(&mut tv.filter.text).desired_width(140.0));
            if ui.button("Reset filters").clicked() {
                tv.filter = TraceFilter::default();
                tv.node_text.clear();
                tv.id_min_text.clear();
                tv.id_max_text.clear();
            }
        });

        // Apply the filter (recomputed only when it changed).
        let mut f = tv.filter.clone();
        f.node = parse_int(tv.node_text.trim(), false).and_then(|v| u8::try_from(v).ok());
        f.id_min = parse_hex(&tv.id_min_text).unwrap_or(0);
        f.id_max = parse_hex(&tv.id_max_text).unwrap_or(0x1FFF_FFFF);
        self.trace.set_view_filter(tv.view, f);

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!(
                "{} shown, {} in buffer (max {}), {} received",
                if tv.fixed { self.trace.fixed_rows_of(tv.view).len() } else { self.trace.view_len_of(tv.view) },
                self.trace.len(),
                self.trace.capacity(),
                self.trace.total()
            ))
            .color(p.text_secondary));
            let skipped = self.trace.total().saturating_sub(tv.paused_at);
            if paused && skipped > 0 {
                ui.colored_label(p.warning, format!("paused, {skipped} frame(s) not recorded"));
            }
        });
        ui.separator();

        let look = Look { mode: tv.mode, colour: self.settings.trace_colour, outlined: self.settings.reduce_colour, p };
        if self.trace.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::LIST_DASHES).size(32.0).color(p.text_secondary));
                ui.label(if self.connected { "No frames yet." } else { "No frames. Connect to see the bus traffic." });
            });
            return;
        }
        if tv.fixed {
            self.fixed_table(ui, tv, look);
        } else {
            self.chrono_table(ui, tv, look);
        }
    }

    /// Save the rows of trace tab `n` as CSV (file dialog).
    pub(crate) fn save_trace_csv(&mut self, n: u8) {
        let (view, mode) = self.traces.get(&n).map(|t| (t.view, t.mode)).unwrap_or((0, self.settings.trace_display));
        self.save_view_csv(view, mode);
    }

    fn save_view_csv(&mut self, view: ViewId, mode: DisplayMode) {
        let Some(path) = rfd::FileDialog::new().set_file_name("trace.csv").add_filter("CSV", &["csv"]).save_file() else { return };
        let r = std::fs::File::create(&path).and_then(|file| {
            let mut w = std::io::BufWriter::new(file);
            self.trace.write_csv_of(view, &mut w, mode)?;
            std::io::Write::flush(&mut w)
        });
        match r {
            Ok(()) => self.set_status(format!("Saved {} rows to {}", self.trace.view_len_of(view), path.display()), false),
            Err(e) => self.set_status(format!("CSV not saved: {e}"), true),
        }
    }

    fn chrono_table(&mut self, ui: &mut egui::Ui, tv: &TraceView, look: Look) {
        let delta = tv.delta;
        let view = tv.view;
        let n = self.trace.view_len_of(view);
        let row_h = self.settings.row_density.row_height();
        let us = self.settings.timestamp_us;
        let trace = &self.trace;
        let p = look.p;
        TableBuilder::new(ui)
            .id_salt(("trace_chrono", view))
            .striped(self.settings.zebra)
            .resizable(true)
            .stick_to_bottom(tv.autoscroll)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).at_least(60.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(190.0).at_least(60.0).clip(true))
            .column(Column::initial(90.0).at_least(40.0).clip(true))
            .column(Column::initial(40.0).at_least(30.0))
            .column(Column::remainder().at_least(100.0).clip(true))
            .min_scrolled_height(0.0)
            .header(row_h, |mut h| {
                for t in [if delta { "Delta ms" } else { "Time s" }, "Dir", "COB-ID", "DLC", "Data", "Type", "Node", "Decoded"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, n, |mut row| {
                    let i = row.index();
                    let Some(e) = trace.view_row_of(view, i) else { return };
                    let f = &e.frame;
                    row.col(|ui| {
                        let t = if delta {
                            match i.checked_sub(1).and_then(|p| trace.view_row_of(view, p)) {
                                Some(p) => format!("{:.3}", f.timestamp_us.saturating_sub(p.frame.timestamp_us) as f64 / 1000.0),
                                None => "0.000".to_string(),
                            }
                        } else if us {
                            format!("{:.6}", f.timestamp_us as f64 / 1e6)
                        } else {
                            format!("{:.3}", f.timestamp_us as f64 / 1e6)
                        };
                        ui.monospace(t);
                    });
                    row.col(|ui| {
                        let (glyph, c) = if f.dir == crate::can::Dir::Tx { (egui_phosphor::regular::ARROW_UP_RIGHT, p.text_accent) } else { (egui_phosphor::regular::ARROW_DOWN_LEFT, p.text_secondary) };
                        ui.label(egui::RichText::new(glyph).color(c)).on_hover_text(f.dir.label());
                    });
                    row.col(|ui| {
                        ui.monospace(id_text(f));
                    });
                    common_cols(&mut row, e, look);
                });
            });
    }

    fn fixed_table(&mut self, ui: &mut egui::Ui, tv: &TraceView, look: Look) {
        let rows = self.trace.fixed_rows_of(tv.view);
        let row_h = self.settings.row_density.row_height();
        let p = look.p;
        TableBuilder::new(ui)
            .id_salt(("trace_fixed", tv.view))
            .striped(self.settings.zebra)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(70.0).at_least(40.0))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(190.0).at_least(60.0).clip(true))
            .column(Column::initial(90.0).at_least(40.0).clip(true))
            .column(Column::initial(40.0).at_least(30.0))
            .column(Column::remainder().at_least(100.0).clip(true))
            .min_scrolled_height(0.0)
            .header(row_h, |mut h| {
                for t in ["COB-ID", "Dir", "Count", "Cycle ms", "DLC", "Data", "Type", "Node", "Decoded"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, rows.len(), |mut row| {
                    let r = rows[row.index()];
                    let f = &r.last.frame;
                    row.col(|ui| {
                        ui.monospace(id_text(f));
                    });
                    row.col(|ui| {
                        let (glyph, c) = if f.dir == crate::can::Dir::Tx { (egui_phosphor::regular::ARROW_UP_RIGHT, p.text_accent) } else { (egui_phosphor::regular::ARROW_DOWN_LEFT, p.text_secondary) };
                        ui.label(egui::RichText::new(glyph).color(c)).on_hover_text(f.dir.label());
                    });
                    row.col(|ui| {
                        ui.monospace(r.count.to_string());
                    });
                    row.col(|ui| {
                        ui.monospace(r.cycle_us.map(|c| format!("{:.1}", c as f64 / 1000.0)).unwrap_or_default());
                    });
                    common_cols(&mut row, &r.last, look);
                });
            });
    }
}
