//! Graph view: signals from PDOs, SDO polls and raw frame bits, drawn with
//! egui_plot over a sliding time window, with a legend table and CSV
//! export. SDO polling sends only after the user presses Start polling.

use super::{theme, App};
use crate::bus::{BusCommand, BusHandle};
use crate::can::CanId;
use crate::canopen::pdo::configs_from_od;
use crate::library::Library;
use crate::signals::{RawKind, SignalDef, SignalSource, MIN_POLL_MS};
use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotBounds, PlotPoints};
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SourceKind {
    Pdo,
    SdoPoll,
    Raw,
    /// A user signal (0.3).
    User,
}

impl SourceKind {
    fn label(self) -> &'static str {
        match self {
            SourceKind::Pdo => "PDO object",
            SourceKind::SdoPoll => "SDO poll",
            SourceKind::Raw => "Raw frame bits",
            SourceKind::User => "User signal",
        }
    }
}

/// The fields of the add-signal dialog.
pub(crate) struct AddDraft {
    pub kind: SourceKind,
    pub name: String,
    pub unit: String,
    pub scale: f64,
    pub offset: f64,
    pub node: u8,
    pub index_text: String,
    pub sub: u8,
    pub period_ms: u64,
    pub cob_text: String,
    pub extended: bool,
    pub start_bit: u32,
    pub bits: u32,
    pub raw_kind: RawKind,
    pub big_endian: bool,
    pub error: Option<String>,
    /// The name was filled from the "Mapped object" choice (a new choice
    /// may replace it; a name typed by the user stays).
    pub auto_name: bool,
    /// The chosen user signal (index).
    pub user: usize,
    /// The signal is for a new dashboard widget, not for the Graph.
    pub for_dashboard: bool,
}

impl Default for AddDraft {
    fn default() -> Self {
        AddDraft {
            kind: SourceKind::Pdo,
            name: String::new(),
            unit: String::new(),
            scale: 1.0,
            offset: 0.0,
            node: 1,
            index_text: "6000".to_string(),
            sub: 1,
            period_ms: 200,
            cob_text: "181".to_string(),
            extended: false,
            start_bit: 0,
            bits: 8,
            raw_kind: RawKind::Unsigned,
            big_endian: false,
            error: None,
            auto_name: false,
            user: 0,
            for_dashboard: false,
        }
    }
}

impl AddDraft {
    /// The signal definition of the draft.
    pub(crate) fn build(&self) -> Result<SignalDef, String> {
        let hex = |s: &str, what: &str| {
            let t = s.trim();
            let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
            u32::from_str_radix(t, 16).map_err(|_| format!("{what}: hex value"))
        };
        let source = match self.kind {
            SourceKind::Pdo | SourceKind::SdoPoll => {
                let index = hex(&self.index_text, "index")?;
                if index > 0xFFFF {
                    return Err("index: 0000..FFFF".to_string());
                }
                if !(1..=127).contains(&self.node) {
                    return Err("node: 1..127".to_string());
                }
                if self.kind == SourceKind::Pdo {
                    SignalSource::Pdo { node: self.node, index: index as u16, sub: self.sub }
                } else {
                    SignalSource::SdoPoll { node: self.node, index: index as u16, sub: self.sub, period_ms: self.period_ms.max(MIN_POLL_MS), data_type: None }
                }
            }
            SourceKind::User => return Err("choose a user signal".to_string()),
            SourceKind::Raw => {
                // The text and the Ext box agree: a text that makes the
                // ID extended wins over an empty box.
                let c = CanId::parse_with_flag(&self.cob_text, self.extended).map_err(|e| format!("COB-ID: {e}"))?;
                let (cob_id, extended) = (c.id, c.extended);
                if self.bits == 0 || self.bits > 64 || self.start_bit + self.bits > 64 {
                    return Err("bits: the field must fit in 64 bits".to_string());
                }
                if self.raw_kind == RawKind::Float && self.bits != 32 && self.bits != 64 {
                    return Err("a float has 32 or 64 bits".to_string());
                }
                SignalSource::Raw { cob_id, extended, start_bit: self.start_bit, bits: self.bits, raw_kind: self.raw_kind, big_endian: self.big_endian }
            }
        };
        let name = if self.name.trim().is_empty() { default_name(&source) } else { self.name.trim().to_string() };
        let mut d = SignalDef::new(name, source);
        d.unit = self.unit.trim().to_string();
        d.scale = self.scale;
        d.offset = self.offset;
        Ok(d)
    }
}

pub(crate) fn default_name(s: &SignalSource) -> String {
    match s {
        SignalSource::Pdo { node, index, sub } => format!("N{node} {index:04X}:{sub:02X}"),
        SignalSource::SdoPoll { node, index, sub, .. } => format!("N{node} {index:04X}:{sub:02X} (SDO)"),
        SignalSource::Raw { cob_id, extended, start_bit, bits, .. } => format!("{} bits {start_bit}+{bits}", CanId::new(*cob_id, *extended)),
        SignalSource::J1939 { cob_id, start_bit, bits, .. } => format!("PGN {} bits {start_bit}+{bits}", crate::j1939::J1939Id::split(*cob_id).pgn()),
    }
}

fn kind_text(k: &RawKind) -> &'static str {
    match k {
        RawKind::Unsigned => "unsigned",
        RawKind::Signed => "signed",
        RawKind::Float => "float",
    }
}

pub(crate) fn source_text(s: &SignalSource) -> String {
    match s {
        SignalSource::Pdo { node, index, sub } => format!("PDO, node {node}, {index:04X}:{sub:02X}"),
        SignalSource::SdoPoll { node, index, sub, period_ms, .. } => format!("SDO, node {node}, {index:04X}:{sub:02X}, {period_ms} ms"),
        SignalSource::Raw { cob_id, extended, start_bit, bits, raw_kind, big_endian } => {
            let k = kind_text(raw_kind);
            format!("raw {}, bit {start_bit}, {bits} bit {k}{}", CanId::new(*cob_id, *extended), if *big_endian { ", big endian" } else { "" })
        }
        SignalSource::J1939 { cob_id, source, start_bit, bits, raw_kind, big_endian } => {
            let k = kind_text(raw_kind);
            let from = match source {
                Some(sa) => format!("source address {sa}"),
                None => "any source address (with two or more addresses the signal takes each frame in time order)".to_string(),
            };
            format!(
                "J1939 {}, {from}, bit {start_bit}, {bits} bit {k}{}",
                crate::j1939::pgn_text(crate::j1939::J1939Id::split(*cob_id).pgn()),
                if *big_endian { ", big endian" } else { "" }
            )
        }
    }
}

fn fmt_val(v: Option<f64>) -> String {
    match v {
        Some(v) if v.fract() == 0.0 && v.abs() < 1e15 => format!("{v:.0}"),
        Some(v) => format!("{v:.4}"),
        None => "-".to_string(),
    }
}

#[derive(Default)]
pub(crate) struct GraphView {
    /// The user started SDO polling (mirrors `Shared::signal_polling`).
    pub polling: bool,
    pub paused_at: Option<f64>,
    pub follow: bool,
    pub hidden: HashSet<u64>,
    pub add: Option<AddDraft>,
    started: bool,
    /// The time range of the plot as last drawn (s).
    pub visible: (f64, f64),
    /// The Export range dialog: (from, to) in seconds.
    pub range: Option<(f64, f64)>,
    /// Tests: per drawn series, the number of plot points and the time of
    /// the first one, as last drawn.
    #[cfg(test)]
    pub drawn: Vec<(usize, Option<f64>)>,
}

impl GraphView {
    /// Load the saved signal definitions (no polling starts).
    pub(crate) fn load(&mut self, lib: Option<&Library>, bus: &BusHandle) -> Option<String> {
        self.follow = true;
        let lib = lib?;
        let r = bus.signals.lock().ok()?.load_defs(&lib.signals_path());
        r.err().map(|e| format!("Graph signals not read: {e}"))
    }

    /// After Connect the worker has polling off.
    pub(crate) fn on_connected(&mut self, _bus: &BusHandle) {
        self.polling = false;
    }
}

impl App {
    fn save_signals(&mut self) {
        if self.data_error.is_some() {
            return;
        }
        let Some(lib) = &self.lib else { return };
        let path = lib.signals_path();
        let r = match self.bus.signals.lock() {
            Ok(s) => s.save_defs(&path),
            Err(_) => return,
        };
        if let Err(e) = r {
            self.set_status(format!("Graph signals not saved: {e}"), true);
        }
    }

    /// Add a signal (and save the definitions). Returns its id.
    pub(crate) fn add_signal(&mut self, def: SignalDef) -> Option<u64> {
        let id = self.bus.signals.lock().ok()?.add(def);
        self.save_signals();
        Some(id)
    }

    /// Write every point as CSV.
    pub(crate) fn write_graph_csv(&self, path: &Path) -> std::io::Result<usize> {
        let s = self.bus.signals.lock().map_err(|_| std::io::Error::other("signal store lock"))?;
        let file = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::new(file);
        s.write_csv(&mut w)?;
        std::io::Write::flush(&mut w)?;
        Ok(s.graph_signals().map(|x| x.points().len()).sum())
    }

    pub(crate) fn graph_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let view_h = ui.available_height();
        // Zoom factor of a toolbar button for this frame (> 1 zooms in).
        let mut zoom: Option<f32> = None;
        if !self.graph.started {
            self.graph.started = true;
            self.graph.follow = true;
        }
        let (now, polling_on) = self.bus.shared.lock().map(|s| (s.now_us as f64 / 1e6, s.signal_polling)).unwrap_or((0.0, false));
        self.graph.polling = polling_on;
        let has_polls = self.bus.signals.lock().map(|s| s.has_polls()).unwrap_or(false);
        // Toolbar: one row (icon buttons with tooltips; Add signal keeps its label).
        use egui_phosphor::regular as icon;
        theme::toolbar(ui, |ui| {
            if ui.button(format!("{} Add signal...", icon::PLUS)).clicked() {
                self.graph.add = Some(AddDraft { node: self.active_node(), ..AddDraft::default() });
            }
            ui.separator();
            if polling_on {
                if theme::icon_button(ui, true, icon::STOP, "Stop polling", "Stop polling the SDO signals").clicked() {
                    self.bus.send(BusCommand::SetSignalPolling(false));
                }
            } else {
                let can = self.can_send();
                let tip = if can { "Start polling: read the SDO signals periodically" } else { self.send_hint() };
                if theme::icon_button(ui, can && has_polls, icon::PLAY, "Start polling", tip).clicked() {
                    self.bus.send(BusCommand::SetSignalPolling(true));
                }
            }
            let mut paused = self.graph.paused_at.is_some();
            if theme::icon_toggle(ui, &mut paused, icon::PAUSE, "Pause", "Freeze the picture; the signals keep recording").changed() {
                self.graph.paused_at = if paused { Some(now) } else { None };
            }
            theme::icon_toggle(ui, &mut self.graph.follow, icon::ARROW_LINE_RIGHT, "Follow", "Follow the newest points (off: drag and zoom; a click, drag or scroll in the plot switches it off)");
            if theme::icon_button(ui, true, icon::MAGNIFYING_GLASS_PLUS, "Zoom in", "Zoom in (also Ctrl + scroll in the plot); switches Follow off").clicked() {
                zoom = Some(1.25);
            }
            if theme::icon_button(ui, true, icon::MAGNIFYING_GLASS_MINUS, "Zoom out", "Zoom out (also Ctrl + scroll in the plot); switches Follow off").clicked() {
                zoom = Some(0.8);
            }
            if zoom.is_some() {
                self.graph.follow = false;
            }
            ui.separator();
            ui.label(icon::CLOCK).on_hover_text("Time window");
            if ui.add(egui::DragValue::new(&mut self.settings.graph_window_s).range(1.0..=3600.0).speed(0.5).suffix(" s")).on_hover_text("Time window").changed() {
                self.mark_settings();
            }
            ui.separator();
            if theme::icon_button(ui, true, icon::TRASH, "Clear", "Delete the recorded points").clicked() {
                if let Ok(mut s) = self.bus.signals.lock() {
                    s.clear();
                }
            }
            if theme::icon_button(ui, true, icon::FLOPPY_DISK, "Export CSV...", "Export the points as CSV").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_file_name("signals.csv").add_filter("CSV", &["csv"]).save_file() {
                    match self.write_graph_csv(&path) {
                        Ok(n) => self.set_status(format!("Saved {n} points to {}", path.display()), false),
                        Err(e) => self.set_status(format!("CSV not saved: {e}"), true),
                    }
                }
            }
            if theme::icon_button(ui, true, icon::EXPORT, "Export range...", "Export a time range as a table: one time column, one column per shown signal").clicked() {
                self.graph.range = Some(self.graph.visible);
            }
            if polling_on {
                theme::pill(ui, "polling", p.info, p.info_tint, self.settings.reduce_colour);
            }
        });
        if has_polls && !polling_on && !self.can_send() {
            ui.label(egui::RichText::new(format!("SDO signals record only while polling runs. {}.", self.send_hint())).color(p.text_secondary));
        }

        // Data for the plot.
        let window = self.settings.graph_window_s.max(1.0) as f64;
        let t_end = self.graph.paused_at.unwrap_or(now);
        let t0 = t_end - window;
        let max_points = (ui.available_width().max(100.0) as usize).min(4000);
        // The time range to draw: with Follow the newest window; without it
        // the range that the plot shows (the user dragged or zoomed), so
        // that older points in the buffer stay visible.
        let (q0, q1) = match self.graph.visible {
            (a, b) if !self.graph.follow && b > a => (a - (b - a) * 0.05, b + (b - a) * 0.05),
            _ => (t0, t_end),
        };
        struct Series {
            id: u64,
            name: String,
            unit: String,
            source: String,
            last: Option<f64>,
            min: Option<f64>,
            max: Option<f64>,
            count: u64,
            pts: Vec<[f64; 2]>,
        }
        let series: Vec<Series> = match self.bus.signals.lock() {
            Ok(s) => s
                .graph_signals()
                .map(|sig| {
                    let mut pts = Vec::new();
                    if !self.graph.hidden.contains(&sig.id) {
                        for m in s.slice(sig.id, q0, q1, max_points) {
                            pts.push([m.t, m.min]);
                            if m.max != m.min {
                                pts.push([m.t, m.max]);
                            }
                        }
                    }
                    Series { id: sig.id, name: sig.def.name.clone(), unit: sig.def.unit.clone(), source: source_text(&sig.def.source), last: sig.last, min: sig.min, max: sig.max, count: sig.count, pts }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        #[cfg(test)]
        {
            self.graph.drawn = series.iter().map(|s| (s.pts.len(), s.pts.first().map(|p| p[0]))).collect();
        }

        if series.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::CHART_LINE).size(28.0).color(p.text_secondary));
                ui.label("No signals.");
                ui.label(egui::RichText::new("Press Add signal to plot a PDO object, an SDO poll or raw frame bits.").color(p.text_secondary));
            });
            return;
        }

        // The plot gets at least 60 % of the height; the legend scrolls in
        // the rest.
        let avail = ui.available_height();
        let legend_h = (series.len() as f32 + 1.0) * 24.0 + 12.0;
        let plot_h = (avail - legend_h.min(220.0)).max(view_h * 0.6).min(avail - 40.0).max(120.0);
        let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in &series {
            for p in &s.pts {
                ymin = ymin.min(p[1]);
                ymax = ymax.max(p[1]);
            }
        }
        if !ymin.is_finite() {
            ymin = -1.0;
            ymax = 1.0;
        }
        let pad = ((ymax - ymin).abs() * 0.08).max(0.5);
        let follow = self.graph.follow;
        let width = self.settings.graph_line_width;
        let readout = self.settings.graph_cursor_readout;
        let mut plot = Plot::new("graph_plot").height(plot_h).legend(Legend::default()).x_axis_label("time (s)").allow_drag(!follow).allow_zoom(!follow).allow_scroll(!follow);
        if !readout {
            plot = plot.show_x(false).show_y(false);
        }
        let shown = plot.show(ui, |pui| {
            if follow {
                pui.set_plot_bounds(PlotBounds::from_min_max([t0, ymin - pad], [t_end, ymax + pad]));
            }
            if let Some(z) = zoom {
                let c = pui.plot_bounds().center();
                pui.zoom_bounds(egui::Vec2::splat(z), c);
            }
            for (i, s) in series.iter().enumerate() {
                if self.graph.hidden.contains(&s.id) {
                    continue;
                }
                pui.line(Line::new(PlotPoints::from(s.pts.clone())).name(&s.name).color(theme::series_colour(p, i)).width(width));
            }
        });
        // A press, drag or scroll in the plot means the user wants to move
        // the view: Follow goes off, and the plot takes the drag from the
        // next frame on.
        let r = &shown.response;
        if follow && (r.is_pointer_button_down_on() || r.dragged() || (r.hovered() && ui.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO || i.zoom_delta() != 1.0))) {
            self.graph.follow = false;
        }
        let b = shown.transform.bounds();
        self.graph.visible = (b.min()[0], b.max()[0]);

        // Legend table.
        let mut remove = None;
        egui::ScrollArea::both().id_salt("graph_legend").max_height(ui.available_height()).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("graph_legend_grid").num_columns(9).striped(self.settings.zebra).show(ui, |ui| {
                for h in ["", "Show", "Signal", "Source", "Last", "Min", "Max", "Points", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, s) in series.iter().enumerate() {
                    let (r, painter) = ui.allocate_painter(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    painter.rect_filled(r.rect.shrink(2.0), 2.0, theme::series_colour(p, i));
                    let mut vis = !self.graph.hidden.contains(&s.id);
                    if ui.checkbox(&mut vis, "").changed() {
                        if vis {
                            self.graph.hidden.remove(&s.id);
                        } else {
                            self.graph.hidden.insert(s.id);
                        }
                    }
                    ui.label(if s.unit.is_empty() { s.name.clone() } else { format!("{} [{}]", s.name, s.unit) });
                    ui.label(egui::RichText::new(&s.source).color(p.text_secondary));
                    ui.monospace(fmt_val(s.last));
                    ui.monospace(fmt_val(s.min));
                    ui.monospace(fmt_val(s.max));
                    ui.monospace(s.count.to_string());
                    if ui.small_button("Remove").clicked() {
                        remove = Some(s.id);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(id) = remove {
            let left_polls = match self.bus.signals.lock() {
                Ok(mut s) => {
                    s.remove(id);
                    s.has_polls()
                }
                Err(_) => true,
            };
            self.graph.hidden.remove(&id);
            if !left_polls && polling_on {
                self.bus.send(BusCommand::SetSignalPolling(false));
            }
            self.save_signals();
        }
        if self.connected && self.graph.paused_at.is_none() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Write the shown signals in [t0, t1] as one table
    /// (`SignalStore::write_range_csv`).
    pub(crate) fn write_range_csv(&self, path: &Path, t0: f64, t1: f64) -> std::io::Result<usize> {
        let s = self.bus.signals.lock().map_err(|_| std::io::Error::other("signal store lock"))?;
        let ids: Vec<u64> = s.graph_signals().map(|x| x.id).filter(|id| !self.graph.hidden.contains(id)).collect();
        let file = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::new(file);
        let n = s.write_range_csv(&mut w, &ids, t0, t1)?;
        std::io::Write::flush(&mut w)?;
        Ok(n)
    }

    fn graph_range_dialog(&mut self, ctx: &egui::Context) {
        let Some((mut t0, mut t1)) = self.graph.range.take() else { return };
        let mut open = true;
        let mut keep = true;
        let mut save = false;
        let visible = self.graph.visible;
        egui::Window::new("Export range")
            .id(egui::Id::new("graph_range"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                egui::Grid::new("range_grid").num_columns(2).show(ui, |ui| {
                    ui.label("From");
                    ui.add(egui::DragValue::new(&mut t0).speed(0.1).suffix(" s").max_decimals(3));
                    ui.end_row();
                    ui.label("To");
                    ui.add(egui::DragValue::new(&mut t1).speed(0.1).suffix(" s").max_decimals(3));
                    ui.end_row();
                });
                if ui.small_button("Visible range").on_hover_text("The time range of the plot now").clicked() {
                    (t0, t1) = visible;
                }
                ui.label(egui::RichText::new("CSV: time_s, then one column per shown signal; one row per time stamp, an empty cell where a signal has no sample.").small());
                ui.horizontal(|ui| {
                    if ui.button("Save...").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        if save {
            let (a, b) = (t0.min(t1), t0.max(t1));
            if let Some(path) = rfd::FileDialog::new().set_file_name("signals_range.csv").add_filter("CSV", &["csv"]).save_file() {
                match self.write_range_csv(&path, a, b) {
                    Ok(n) => self.set_status(format!("Saved {n} rows ({a:.3} s to {b:.3} s) to {}", path.display()), false),
                    Err(e) => self.set_status(format!("CSV not saved: {e}"), true),
                }
                keep = false;
            }
        }
        if open && keep {
            self.graph.range = Some((t0, t1));
        }
    }

    pub(crate) fn graph_dialogs(&mut self, ctx: &egui::Context) {
        self.graph_range_dialog(ctx);
        let Some(mut d) = self.graph.add.take() else { return };
        let mut keep = true;
        let mut add = false;
        // Mapped objects of the node's PDOs (read or library configuration,
        // else the EDS defaults): the TPDOs that the node sends, then the
        // RPDOs that it receives (sent by another node or the transmit list).
        // The text names the PDO as the trace does ("RPDO1 6200:01 ...").
        let mapped: Vec<(u16, u8, bool, String)> = if d.kind == SourceKind::Pdo {
            use crate::canopen::pdo::PdoKind;
            let od = self.od_of_node(d.node);
            let cfgs = match self.decode_ctx.node_pdos(d.node) {
                Some(c) => c.to_vec(),
                None => od.as_ref().map(|o| configs_from_od(o, d.node)).unwrap_or_default(),
            };
            let mut v = Vec::new();
            for kind in [PdoKind::Tpdo, PdoKind::Rpdo] {
                for c in cfgs.iter().filter(|c| c.kind == kind) {
                    for m in c.mapping.iter().filter(|m| !m.is_dummy()) {
                        let name = od.as_ref().and_then(|o| o.name_of(m.index, m.sub)).unwrap_or_default();
                        let t = format!("{} {:04X}:{:02X} {name}", c.label(), m.index, m.sub);
                        v.push((m.index, m.sub, kind == PdoKind::Rpdo, t.trim_end().to_string()));
                    }
                }
            }
            v
        } else {
            Vec::new()
        };
        let users: Vec<String> = self.an.usigs.iter().map(|s| format!("{} ({}, bit {}, {} bit)", s.name, CanId::new(s.cob_id, s.extended), s.start_bit, s.bits)).collect();
        let err_c = if self.is_dark() { theme::DARK.error } else { theme::LIGHT.error };
        // A movable window (spec: the Add signal dialog moves by its title
        // bar and stays inside the app window) over a scrim that blocks the
        // views behind it, as the modal did.
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("add_signal_scrim")).order(egui::Order::Middle).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, egui::Color32::from_black_alpha(90));
        });
        let mut open = true;
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.memory(|m| m.any_popup_open());
        egui::Window::new("Add signal")
            .id(egui::Id::new("add_signal"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(true)
            .constrain(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for k in [SourceKind::Pdo, SourceKind::SdoPoll, SourceKind::Raw, SourceKind::User] {
                    ui.selectable_value(&mut d.kind, k, k.label());
                }
            });
            ui.separator();
            egui::Grid::new("add_sig_grid").num_columns(2).show(ui, |ui| {
                match d.kind {
                    SourceKind::Pdo | SourceKind::SdoPoll => {
                        ui.label("Node");
                        ui.add(egui::DragValue::new(&mut d.node).range(1..=127));
                        ui.end_row();
                        if d.kind == SourceKind::Pdo && !mapped.is_empty() {
                            ui.label("Mapped object");
                            let cur = u16::from_str_radix(d.index_text.trim(), 16).ok().and_then(|i| mapped.iter().find(|m| m.0 == i && m.1 == d.sub));
                            let cur_rpdo = cur.is_some_and(|m| m.2);
                            let cur = cur.map(|m| m.3.clone()).unwrap_or_else(|| "(choose)".to_string());
                            egui::ComboBox::from_id_salt("sig_mapped").selected_text(cur).width(260.0).show_ui(ui, |ui| {
                                for (i, s, _, t) in &mapped {
                                    if ui.selectable_label(false, t).clicked() {
                                        d.index_text = format!("{i:04X}");
                                        d.sub = *s;
                                        if d.name.trim().is_empty() || d.auto_name {
                                            d.name = format!("N{} {t}", d.node);
                                            d.auto_name = true;
                                        }
                                    }
                                }
                            });
                            ui.end_row();
                            if cur_rpdo {
                                ui.label("");
                                ui.label(egui::RichText::new("RPDO: decoded from the frames that another node or the transmit list sends to this node.").small());
                                ui.end_row();
                            }
                        }
                        ui.label("Index 0x");
                        ui.add(egui::TextEdit::singleline(&mut d.index_text).desired_width(60.0));
                        ui.end_row();
                        ui.label("Sub");
                        ui.add(egui::DragValue::new(&mut d.sub).hexadecimal(2, false, true));
                        ui.end_row();
                        if d.kind == SourceKind::SdoPoll {
                            ui.label("Period");
                            ui.add(egui::DragValue::new(&mut d.period_ms).range(MIN_POLL_MS..=60_000).suffix(" ms"));
                            ui.end_row();
                        }
                    }
                    SourceKind::Raw => {
                        ui.label("COB-ID 0x");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.cob_text).desired_width(80.0));
                            ui.checkbox(&mut d.extended, "Ext");
                        });
                        ui.end_row();
                        ui.label("Start bit");
                        ui.add(egui::DragValue::new(&mut d.start_bit).range(0..=63));
                        ui.end_row();
                        ui.label("Bits");
                        ui.add(egui::DragValue::new(&mut d.bits).range(1..=64));
                        ui.end_row();
                        ui.label("Type");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.raw_kind, RawKind::Unsigned, "Unsigned");
                            ui.selectable_value(&mut d.raw_kind, RawKind::Signed, "Signed");
                            ui.selectable_value(&mut d.raw_kind, RawKind::Float, "Float");
                        });
                        ui.end_row();
                        ui.label("Byte order");
                        ui.checkbox(&mut d.big_endian, "Big endian");
                        ui.end_row();
                    }
                    SourceKind::User => {
                        ui.label("User signal");
                        if users.is_empty() {
                            ui.label(egui::RichText::new("None yet: define one in the Messages view (select bits, Define signal).").small());
                        } else {
                            d.user = d.user.min(users.len() - 1);
                            egui::ComboBox::from_id_salt("sig_user").selected_text(users[d.user].clone()).width(260.0).show_ui(ui, |ui| {
                                for (i, t) in users.iter().enumerate() {
                                    ui.selectable_value(&mut d.user, i, t);
                                }
                            });
                        }
                        ui.end_row();
                        return;
                    }
                }
                ui.label("Name");
                if ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("automatic")).changed() {
                    d.auto_name = false;
                }
                ui.end_row();
                ui.label("Unit");
                ui.add(egui::TextEdit::singleline(&mut d.unit).desired_width(60.0));
                ui.end_row();
                ui.label("Scale");
                ui.add(egui::DragValue::new(&mut d.scale).speed(0.01));
                ui.end_row();
                ui.label("Offset");
                ui.add(egui::DragValue::new(&mut d.offset).speed(0.1));
                ui.end_row();
            });
            if d.kind == SourceKind::SdoPoll {
                ui.label("An SDO signal sends a read request each period, only while polling runs.");
            }
            if let Some(e) = &d.error {
                ui.colored_label(err_c, e);
            }
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    add = true;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
        if !open || escape {
            keep = false;
        }
        if add {
            let built = if d.kind == SourceKind::User { self.an.usigs.get(d.user).map(|s| s.to_def()).ok_or_else(|| "no user signal yet".to_string()) } else { d.build() };
            match built {
                Ok(def) => {
                    if d.for_dashboard {
                        self.dash_add_widget(def);
                    } else {
                        self.add_signal(def);
                    }
                    keep = false;
                }
                Err(e) => d.error = Some(e),
            }
        }
        if keep {
            self.graph.add = Some(d);
        }
    }
}
