//! Trace tab: chronological (ring buffer) or fixed (one row per COB-ID)
//! view with filters, Pause, Clear and CSV export. Rows are virtualised;
//! each frame is decoded once on receive.

use super::App;
use crate::canopen::decode::MsgClass;
use crate::canopen::types::{format_bytes, parse_int, DisplayMode};
use crate::trace::{id_text, TraceEntry, TraceFilter};
use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};

pub(crate) struct TraceView {
    /// Pause stops the recording; frames that arrive meanwhile are counted.
    pub(crate) paused: bool,
    pub(crate) skipped: u64,
    autoscroll: bool,
    fixed: bool,
    delta: bool,
    filter: TraceFilter,
    node_text: String,
    id_min_text: String,
    id_max_text: String,
    capacity_text: String,
}

impl Default for TraceView {
    fn default() -> Self {
        TraceView {
            paused: false,
            skipped: 0,
            autoscroll: true,
            fixed: false,
            delta: false,
            filter: TraceFilter::default(),
            node_text: String::new(),
            id_min_text: String::new(),
            id_max_text: String::new(),
            capacity_text: String::new(),
        }
    }
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

fn common_cols(row: &mut TableRow<'_, '_>, e: &TraceEntry, mode: DisplayMode) {
    let f = &e.frame;
    row.col(|ui| {
        ui.label(f.dlc.to_string());
    });
    row.col(|ui| {
        let d = if f.rtr { "RTR".to_string() } else { format_bytes(f.payload(), mode) };
        ui.add(egui::Label::new(egui::RichText::new(d).monospace()).truncate());
    });
    row.col(|ui| {
        ui.add(egui::Label::new(&e.decoded.label).truncate());
    });
    row.col(|ui| {
        ui.label(e.decoded.node.map(|n| n.to_string()).unwrap_or_default());
    });
    row.col(|ui| {
        ui.add(egui::Label::new(&e.decoded.text).truncate()).on_hover_text(&e.decoded.text);
    });
}

impl App {
    pub(crate) fn trace_tab(&mut self, ui: &mut egui::Ui) {
        if self.tv.capacity_text.is_empty() {
            self.tv.capacity_text = self.settings.trace_capacity.to_string();
        }
        // Toolbar.
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.tv.paused, "Pause").on_hover_text("Stop recording into the trace");
            ui.checkbox(&mut self.tv.autoscroll, "Auto-scroll");
            ui.separator();
            ui.selectable_value(&mut self.tv.fixed, false, "Chronological");
            ui.selectable_value(&mut self.tv.fixed, true, "Fixed");
            ui.separator();
            ui.selectable_value(&mut self.tv.delta, false, "Abs time");
            ui.selectable_value(&mut self.tv.delta, true, "Delta time");
            ui.separator();
            let before = self.settings.trace_display;
            for m in DisplayMode::ALL {
                ui.selectable_value(&mut self.settings.trace_display, m, m.label());
            }
            if before != self.settings.trace_display {
                self.mark_settings();
            }
            ui.separator();
            if ui.button("Clear").clicked() {
                self.trace.clear();
                self.tv.skipped = 0;
            }
            if ui.button("Save CSV...").clicked() {
                self.save_csv();
            }
        });
        // Filters.
        ui.horizontal_wrapped(|ui| {
            ui.label("Node");
            ui.add(egui::TextEdit::singleline(&mut self.tv.node_text).desired_width(36.0).hint_text("all"));
            ui.label("COB-ID 0x");
            ui.add(egui::TextEdit::singleline(&mut self.tv.id_min_text).desired_width(70.0).hint_text("min"));
            ui.label("..");
            ui.add(egui::TextEdit::singleline(&mut self.tv.id_max_text).desired_width(70.0).hint_text("max"));
            let n_on = MsgClass::ALL.iter().filter(|c| self.tv.filter.class_enabled(**c)).count();
            ui.menu_button(format!("Types ({n_on}/{})", MsgClass::ALL.len()), |ui| {
                for c in MsgClass::ALL {
                    let mut on = self.tv.filter.class_enabled(c);
                    if ui.checkbox(&mut on, c.label()).changed() {
                        self.tv.filter.set_class(c, on);
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("All").clicked() {
                        self.tv.filter.classes = 0xFFFF;
                    }
                    if ui.button("None").clicked() {
                        self.tv.filter.classes = 0;
                    }
                });
            });
            ui.label("Text");
            ui.add(egui::TextEdit::singleline(&mut self.tv.filter.text).desired_width(140.0));
            if ui.button("Reset filters").clicked() {
                self.tv.filter = TraceFilter::default();
                self.tv.node_text.clear();
                self.tv.id_min_text.clear();
                self.tv.id_max_text.clear();
            }
            ui.separator();
            ui.label("Buffer");
            let r = ui.add(egui::TextEdit::singleline(&mut self.tv.capacity_text).desired_width(70.0));
            if r.lost_focus() {
                match self.tv.capacity_text.trim().parse::<usize>() {
                    Ok(c) if (1000..=2_000_000).contains(&c) => {
                        self.settings.trace_capacity = c;
                        self.trace.set_capacity(c);
                        self.mark_settings();
                    }
                    _ => {
                        self.set_status("Trace buffer: 1000..2000000 frames", true);
                        self.tv.capacity_text = self.settings.trace_capacity.to_string();
                    }
                }
            }
            ui.label("frames");
        });

        // Apply the filter (recomputed only when it changed).
        let mut f = self.tv.filter.clone();
        f.node = parse_int(self.tv.node_text.trim(), false).and_then(|v| u8::try_from(v).ok());
        f.id_min = parse_hex(&self.tv.id_min_text).unwrap_or(0);
        f.id_max = parse_hex(&self.tv.id_max_text).unwrap_or(0x1FFF_FFFF);
        self.trace.set_filter(f);

        ui.horizontal(|ui| {
            ui.label(format!(
                "{} shown, {} in buffer (max {}), {} received",
                if self.tv.fixed { self.trace.fixed_rows().len() } else { self.trace.view_len() },
                self.trace.len(),
                self.trace.capacity(),
                self.trace.total()
            ));
            if self.tv.paused && self.tv.skipped > 0 {
                ui.colored_label(ui.visuals().warn_fg_color, format!("paused, {} frame(s) not recorded", self.tv.skipped));
            }
        });
        ui.separator();

        if self.tv.fixed {
            self.fixed_table(ui);
        } else {
            self.chrono_table(ui);
        }
    }

    fn save_csv(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_file_name("trace.csv").add_filter("CSV", &["csv"]).save_file() else { return };
        let r = std::fs::File::create(&path).and_then(|file| {
            let mut w = std::io::BufWriter::new(file);
            self.trace.write_csv(&mut w, self.settings.trace_display)?;
            std::io::Write::flush(&mut w)
        });
        match r {
            Ok(()) => self.set_status(format!("Saved {} rows to {}", self.trace.view_len(), path.display()), false),
            Err(e) => self.set_status(format!("CSV not saved: {e}"), true),
        }
    }

    fn chrono_table(&mut self, ui: &mut egui::Ui) {
        let mode = self.settings.trace_display;
        let delta = self.tv.delta;
        let n = self.trace.view_len();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 2.0;
        let trace = &self.trace;
        TableBuilder::new(ui)
            .id_salt("trace_chrono")
            .striped(true)
            .resizable(true)
            .stick_to_bottom(self.tv.autoscroll)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).at_least(60.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(190.0).at_least(60.0).clip(true))
            .column(Column::initial(80.0).at_least(40.0).clip(true))
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
                    let Some(e) = trace.view_row(i) else { return };
                    let f = &e.frame;
                    row.col(|ui| {
                        let t = if delta {
                            match i.checked_sub(1).and_then(|p| trace.view_row(p)) {
                                Some(p) => format!("{:.3}", f.timestamp_us.saturating_sub(p.frame.timestamp_us) as f64 / 1000.0),
                                None => "0.000".to_string(),
                            }
                        } else {
                            format!("{:.6}", f.timestamp_us as f64 / 1e6)
                        };
                        ui.monospace(t);
                    });
                    row.col(|ui| {
                        ui.label(f.dir.label());
                    });
                    row.col(|ui| {
                        ui.monospace(id_text(f));
                    });
                    common_cols(&mut row, e, mode);
                });
            });
    }

    fn fixed_table(&mut self, ui: &mut egui::Ui) {
        let mode = self.settings.trace_display;
        let rows = self.trace.fixed_rows();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 2.0;
        TableBuilder::new(ui)
            .id_salt("trace_fixed")
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(70.0).at_least(40.0))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(30.0).at_least(24.0))
            .column(Column::initial(190.0).at_least(60.0).clip(true))
            .column(Column::initial(80.0).at_least(40.0).clip(true))
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
                        ui.label(f.dir.label());
                    });
                    row.col(|ui| {
                        ui.label(r.count.to_string());
                    });
                    row.col(|ui| {
                        ui.monospace(r.cycle_us.map(|c| format!("{:.1}", c as f64 / 1000.0)).unwrap_or_default());
                    });
                    common_cols(&mut row, &r.last, mode);
                });
            });
    }
}
