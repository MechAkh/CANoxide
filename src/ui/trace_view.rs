//! Trace tabs (up to four): each one is a view of the shared ring buffer
//! with its own filter, display mode, chronological / fixed mode and
//! pause. Rows are virtualised; each frame is decoded once on receive.

use super::{theme, App};
use crate::canopen::decode::MsgClass;
use crate::canopen::types::{format_bytes, parse_int, DisplayMode};
use crate::settings::{DataView, Settings, TimestampFormat, TraceColour};
use crate::trace::{id_text, TraceEntry, TraceFilter, ViewId};
use crate::tracefmt as canoxide_fmt;
use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};

pub(crate) struct TraceView {
    /// The view of the trace store.
    pub(crate) view: ViewId,
    autoscroll: bool,
    fixed: bool,
    pub(crate) delta: bool,
    mode: DisplayMode,
    filter: TraceFilter,
    node_text: String,
    id_min_text: String,
    id_max_text: String,
    /// Frames received when the view was paused.
    paused_at: u64,
    /// Top row while auto-scroll is off: its sequence number and the pixels
    /// scrolled into it. The view keeps this row in place while new frames
    /// arrive and while the ring drops old ones.
    anchor: Option<(u64, f32)>,
    /// Height and rect of the table body in the last frame.
    body_h: f32,
    body_rect: Option<egui::Rect>,
}

/// A key that scrolls the trace table (the pointer is over the table and no
/// widget has the keyboard focus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// The auto-scroll state after one frame of the table. `requested` is the
/// offset the view asked for, `offset` the one egui ended the frame with,
/// `max` the largest offset. When the offset moved by itself the user
/// scrolled (wheel, scroll bar, drag): auto-scroll is on when that left the
/// view at the end, off otherwise. Otherwise the state stays as it was.
pub(crate) fn stick_after(autoscroll: bool, requested: f32, offset: f32, max: f32) -> bool {
    let max = max.max(0.0);
    if (offset - requested.clamp(0.0, max)).abs() <= 0.5 {
        return autoscroll;
    }
    offset >= max - 0.5
}

/// The offset and auto-scroll state after a scroll key. Up keys switch
/// auto-scroll off; down keys switch it on when they reach the end.
pub(crate) fn key_scroll(key: ScrollKey, offset: f32, row: f32, page: f32, max: f32) -> (f32, bool) {
    let max = max.max(0.0);
    let o = match key {
        ScrollKey::Up => offset - row,
        ScrollKey::Down => offset + row,
        ScrollKey::PageUp => offset - page,
        ScrollKey::PageDown => offset + page,
        ScrollKey::Home => 0.0,
        ScrollKey::End => max,
    }
    .clamp(0.0, max);
    let down = matches!(key, ScrollKey::Down | ScrollKey::PageDown | ScrollKey::End);
    (o, down && o >= max - 0.5)
}

/// The scroll key pressed this frame, if the pointer is over `rect` and no
/// widget has the keyboard focus.
fn pressed_scroll_key(ui: &egui::Ui, rect: Option<egui::Rect>) -> Option<ScrollKey> {
    let rect = rect?;
    if !ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p)) || ui.memory(|m| m.focused().is_some()) {
        return None;
    }
    use egui::Key;
    ui.input(|i| {
        [(Key::ArrowUp, ScrollKey::Up), (Key::ArrowDown, ScrollKey::Down), (Key::PageUp, ScrollKey::PageUp), (Key::PageDown, ScrollKey::PageDown), (Key::Home, ScrollKey::Home), (Key::End, ScrollKey::End)]
            .into_iter()
            .find(|(k, _)| i.key_pressed(*k))
            .map(|(_, s)| s)
    })
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
            anchor: None,
            body_h: 0.0,
            body_rect: None,
        }
    }
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

/// A small combo box without a side label: `name` is its accessible name
/// and the tooltip says what it sets.
pub(crate) fn combo<T: PartialEq + Copy>(ui: &mut egui::Ui, salt: impl std::hash::Hash, name: &str, tip: &str, value: &mut T, options: &[(T, &str)], width: f32) {
    let current = options.iter().find(|(v, _)| v == value).map(|(_, l)| *l).unwrap_or("");
    let ir = egui::ComboBox::from_id_salt(salt).width(width).selected_text(current).show_ui(ui, |ui| {
        for (v, l) in options {
            if ui.selectable_value(value, *v, *l).clicked() {
                // Also closes the list after a keyboard or screen reader choice.
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
    let r = ir.response.on_hover_text(tip);
    let name = name.to_string();
    let enabled = ui.is_enabled();
    r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, &name));
}

/// Number of filters that differ from "show everything".
fn active_filters(tv: &TraceView) -> usize {
    let all_types = MsgClass::ALL.iter().all(|c| tv.filter.class_enabled(*c));
    [!tv.node_text.trim().is_empty(), !tv.id_min_text.trim().is_empty() || !tv.id_max_text.trim().is_empty(), !all_types, !tv.filter.text.trim().is_empty()]
        .iter()
        .filter(|b| **b)
        .count()
}

/// The `Filters` button with its count badge and popup (spec 8.4): node,
/// COB-ID range, message types, text, reset.
fn filters_button(ui: &mut egui::Ui, tv: &mut TraceView, n: u8) {
    let count = active_filters(tv);
    let r = ui.button(format!("{}  Filters", egui_phosphor::regular::FUNNEL)).on_hover_text("Filter the rows of this trace tab");
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Filters"));
    theme::count_badge(ui, r.rect, count);
    let id = ui.make_persistent_id(("trace_filters", n));
    if r.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup_below_widget(ui, id, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(300.0);
        egui::Grid::new(("trace_filter_grid", n)).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Node ID");
            ui.add(egui::TextEdit::singleline(&mut tv.node_text).desired_width(48.0).hint_text("all"));
            ui.end_row();
            ui.label("COB-ID 0x");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut tv.id_min_text).desired_width(70.0).hint_text("min"));
                ui.label("..");
                ui.add(egui::TextEdit::singleline(&mut tv.id_max_text).desired_width(70.0).hint_text("max"));
            });
            ui.end_row();
            ui.label("Text");
            ui.add(egui::TextEdit::singleline(&mut tv.filter.text).desired_width(180.0));
            ui.end_row();
        });
        ui.separator();
        let n_on = MsgClass::ALL.iter().filter(|c| tv.filter.class_enabled(**c)).count();
        ui.horizontal(|ui| {
            ui.label(format!("Types ({n_on}/{})", MsgClass::ALL.len()));
            if ui.small_button("All").clicked() {
                tv.filter.classes = 0xFFFF;
            }
            if ui.small_button("None").clicked() {
                tv.filter.classes = 0;
            }
        });
        egui::Grid::new(("trace_type_grid", n)).num_columns(3).show(ui, |ui| {
            for (i, c) in MsgClass::ALL.into_iter().enumerate() {
                let mut on = tv.filter.class_enabled(c);
                if ui.checkbox(&mut on, c.label()).changed() {
                    tv.filter.set_class(c, on);
                }
                if i % 3 == 2 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        if ui.button("Reset filters").clicked() {
            tv.filter = TraceFilter::default();
            tv.node_text.clear();
            tv.id_min_text.clear();
            tv.id_max_text.clear();
        }
    });
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
        ui.add(egui::Label::new(t).truncate());
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

    fn trace_body(&mut self, ui: &mut egui::Ui, tv: &mut TraceView, n: u8) {
        use egui_phosphor::regular as icon;
        let p = theme::pal(ui);
        let mut paused = self.trace.view_paused(tv.view);
        let counts = format!(
            "{} shown, {} in buffer (max {}), {} received",
            if tv.fixed { self.trace.fixed_rows_of(tv.view).len() } else { self.trace.view_len_of(tv.view) },
            self.trace.len(),
            self.trace.capacity(),
            self.trace.total()
        );
        let skipped = self.trace.total().saturating_sub(tv.paused_at);
        let skipped_text = (paused && skipped > 0).then(|| format!("paused, {skipped} frame(s) not recorded"));
        // Toolbar: one row (spec 8.4); the counters go to its right end, or
        // to a second row when there is no room.
        let mut counts_placed = false;
        ui.horizontal_wrapped(|ui| {
            let (g, name) = if paused { (icon::PLAY, "Pause") } else { (icon::PAUSE, "Pause") };
            if theme::icon_toggle(ui, &mut paused, g, name, "Pause: stop adding rows to this trace tab").changed() {
                self.trace.set_view_paused(tv.view, paused);
                tv.paused_at = self.trace.total();
            }
            theme::icon_toggle(ui, &mut tv.autoscroll, icon::ARROW_LINE_DOWN, "Auto-scroll", "Auto-scroll to the newest row");
            ui.separator();
            combo(ui, ("trace_mode", n), "Display mode", "Scroll: one row per frame; Fixed: one row per COB-ID", &mut tv.fixed, &[(false, "Scroll"), (true, "Fixed")], 64.0);
            combo(ui, ("trace_time", n), "Time format", "Time column: absolute or delta to the previous row", &mut tv.delta, &[(false, "Absolute"), (true, "Delta")], 76.0);
            let modes: Vec<(DisplayMode, &str)> = DisplayMode::ALL.iter().map(|m| (*m, m.label())).collect();
            combo(ui, ("trace_data", n), "Data format", "Data column format", &mut tv.mode, &modes, 70.0);
            ui.separator();
            if theme::icon_button(ui, true, icon::TRASH, "Clear", "Clear the buffer of every trace tab").clicked() {
                self.trace.clear();
                tv.paused_at = self.trace.total();
            }
            if theme::icon_button(ui, true, icon::FLOPPY_DISK, "Save trace...", "Save the rows of this tab as CSV, PEAK .trc (version 1.1) or Vector .asc").clicked() {
                self.save_view_csv(tv.view, tv.mode);
            }
            // This tab is taken out of `traces` while it is drawn.
            let open = self.traces.len() + 1;
            if theme::icon_button(ui, open < super::dock::MAX_TRACES as usize, icon::PLUS, "New trace tab", "New trace tab (at most 4)").clicked() {
                self.new_trace_tab();
            }
            ui.separator();
            filters_button(ui, tv, n);
            // Counters at the right end when they fit.
            let font = egui::TextStyle::Small.resolve(ui.style());
            let w = ui.fonts(|f| f.layout_no_wrap(counts.clone(), font, p.text_secondary).size().x) + 8.0;
            let extra = skipped_text.as_ref().map(|_| 200.0).unwrap_or(0.0);
            if ui.available_width() >= w + extra {
                counts_placed = true;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&counts).small().color(p.text_secondary));
                    if let Some(s) = &skipped_text {
                        ui.label(egui::RichText::new(s).small().color(p.warning));
                    }
                });
            }
        });
        if !counts_placed {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&counts).small().color(p.text_secondary));
                if let Some(s) = &skipped_text {
                    ui.label(egui::RichText::new(s).small().color(p.warning));
                }
            });
        }

        // Apply the filter (recomputed only when it changed).
        let mut f = tv.filter.clone();
        f.node = parse_int(tv.node_text.trim(), false).and_then(|v| u8::try_from(v).ok());
        f.id_min = parse_hex(&tv.id_min_text).unwrap_or(0);
        f.id_max = parse_hex(&tv.id_max_text).unwrap_or(0x1FFF_FFFF);
        self.trace.set_view_filter(tv.view, f);
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

    /// Auto-scroll state of trace tab `n` (tests).
    #[cfg(test)]
    pub(crate) fn trace_autoscroll(&self, n: u8) -> bool {
        self.traces.get(&n).is_some_and(|t| t.autoscroll)
    }

    /// Save the rows of trace tab `n` (file dialog: CSV, .trc or .asc).
    pub(crate) fn save_trace_csv(&mut self, n: u8) {
        let (view, mode) = self.traces.get(&n).map(|t| (t.view, t.mode)).unwrap_or((0, self.settings.trace_display));
        self.save_view_csv(view, mode);
    }

    fn save_view_csv(&mut self, view: ViewId, mode: DisplayMode) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("trace.csv")
            .add_filter("CSV", &["csv"])
            .add_filter("PEAK trace (version 1.1)", &["trc"])
            .add_filter("Vector ASC", &["asc"])
            .save_file()
        else {
            return;
        };
        if let Err(e) = self.write_view_file(view, mode, &path) {
            self.set_status(format!("Trace not saved: {e}"), true);
        } else {
            self.set_status(format!("Saved {} rows to {}", self.trace.view_len_of(view), path.display()), false);
        }
    }

    /// Write the rows of a view; the extension of `path` picks the format
    /// (`.trc`, `.asc`, else CSV).
    pub(crate) fn write_view_file(&self, view: ViewId, mode: DisplayMode, path: &std::path::Path) -> std::io::Result<()> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let file = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::new(file);
        match ext.as_str() {
            "trc" | "asc" => {
                let frames = self.trace.frames_of(view);
                // The trace keeps no wall time: the start is now minus the
                // span of the rows.
                let span = match (frames.first(), frames.last()) {
                    (Some(a), Some(b)) => b.timestamp_us.saturating_sub(a.timestamp_us),
                    _ => 0,
                };
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0);
                let start = now.saturating_sub(span);
                if ext == "trc" {
                    canoxide_fmt::write_trc(&mut w, &frames, start)?;
                } else {
                    canoxide_fmt::write_asc(&mut w, &frames, start)?;
                }
            }
            _ => self.trace.write_csv_of(view, &mut w, mode)?,
        }
        std::io::Write::flush(&mut w)
    }

    fn chrono_table(&mut self, ui: &mut egui::Ui, tv: &mut TraceView, look: Look) {
        let delta = tv.delta;
        let view = tv.view;
        let n = self.trace.view_len_of(view);
        let row_h = self.settings.row_density.row_height();
        let us = self.settings.timestamp_us;
        let trace = &self.trace;
        let p = look.p;
        // The offset this frame (spec: each tab scrolls on its own). With
        // auto-scroll: the end. Without: the anchor row stays on top.
        let rhs = row_h + ui.spacing().item_spacing.y;
        if tv.body_h <= 0.0 {
            tv.body_h = (ui.available_height() - row_h).max(rhs);
        }
        let bottom = (n as f32 * rhs - tv.body_h).max(0.0);
        let mut requested = match (tv.autoscroll, tv.anchor) {
            (false, Some((seq, frac))) => trace.view_index_of_seq(view, seq) as f32 * rhs + frac,
            _ => bottom,
        };
        if let Some(k) = pressed_scroll_key(ui, tv.body_rect) {
            let (o, stick) = key_scroll(k, requested.min(bottom), rhs, (tv.body_h - rhs).max(rhs), bottom);
            requested = o;
            tv.autoscroll = stick;
        }
        let out = TableBuilder::new(ui)
            .id_salt(("trace_chrono", view))
            .striped(self.settings.zebra)
            .resizable(true)
            .stick_to_bottom(tv.autoscroll)
            .vertical_scroll_offset(requested)
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
        let max = out.content_size.y - out.inner_rect.height();
        let offset = out.state.offset.y;
        tv.body_h = out.inner_rect.height();
        tv.body_rect = Some(out.inner_rect);
        tv.autoscroll = stick_after(tv.autoscroll, requested, offset, max);
        tv.anchor = if tv.autoscroll {
            None
        } else {
            let i = (offset / rhs).floor().max(0.0) as usize;
            trace.view_row_of(view, i).map(|e| (e.seq, offset - i as f32 * rhs))
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_after_keeps_the_state_without_user_scroll() {
        // Requested the end, egui stayed there (also when it clamped).
        assert!(stick_after(true, 500.0, 500.0, 500.0));
        assert!(stick_after(true, 520.0, 500.0, 500.0));
        // Auto-scroll off, the anchor offset was kept: still off.
        assert!(!stick_after(false, 200.0, 200.0, 500.0));
        // Off and the anchor is at the end by chance: stays off.
        assert!(!stick_after(false, 500.0, 500.0, 500.0));
        // Content shorter than the view.
        assert!(stick_after(true, 0.0, 0.0, -30.0));
        assert!(!stick_after(false, 0.0, 0.0, -30.0));
    }

    #[test]
    fn stick_after_follows_the_user() {
        // Wheel or scroll bar up: off.
        assert!(!stick_after(true, 500.0, 380.0, 500.0));
        // Stick forced the exact end a little below the estimate: still on.
        assert!(stick_after(true, 496.0, 500.0, 500.0));
        // Scrolled back down to the end: on again.
        assert!(stick_after(false, 300.0, 500.0, 500.0));
        // Scrolled, not to the end: off.
        assert!(!stick_after(false, 300.0, 340.0, 500.0));
    }

    #[test]
    fn key_scroll_moves_and_sets_the_state() {
        let (row, page, max) = (26.0, 260.0, 1000.0);
        assert_eq!(key_scroll(ScrollKey::Up, 1000.0, row, page, max), (974.0, false));
        assert_eq!(key_scroll(ScrollKey::PageUp, 100.0, row, page, max), (0.0, false));
        assert_eq!(key_scroll(ScrollKey::Home, 600.0, row, page, max), (0.0, false));
        assert_eq!(key_scroll(ScrollKey::Down, 500.0, row, page, max), (526.0, false));
        assert_eq!(key_scroll(ScrollKey::PageDown, 900.0, row, page, max), (1000.0, true));
        assert_eq!(key_scroll(ScrollKey::End, 0.0, row, page, max), (1000.0, true));
        // Nothing to scroll: End still means auto-scroll.
        assert_eq!(key_scroll(ScrollKey::End, 0.0, row, page, -5.0), (0.0, true));
        assert_eq!(key_scroll(ScrollKey::Up, 0.0, row, page, -5.0), (0.0, false));
    }
}