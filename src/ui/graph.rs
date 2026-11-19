//! Graph view: signals from PDOs, SDO polls and raw frame bits, drawn with
//! egui_plot over a sliding time window, with a legend table and CSV
//! export. SDO polling sends only after the user presses Start polling.

use super::{theme, App};
use crate::bus::{BusCommand, BusHandle};
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
}

impl SourceKind {
    fn label(self) -> &'static str {
        match self {
            SourceKind::Pdo => "PDO object",
            SourceKind::SdoPoll => "SDO poll",
            SourceKind::Raw => "Raw frame bits",
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
            SourceKind::Raw => {
                let cob_id = hex(&self.cob_text, "COB-ID")?;
                let max = if self.extended { 0x1FFF_FFFF } else { 0x7FF };
                if cob_id > max {
                    return Err(format!("COB-ID above 0x{max:X}"));
                }
                if self.bits == 0 || self.bits > 64 || self.start_bit + self.bits > 64 {
                    return Err("bits: the field must fit in 64 bits".to_string());
                }
                if self.raw_kind == RawKind::Float && self.bits != 32 && self.bits != 64 {
                    return Err("a float has 32 or 64 bits".to_string());
                }
                SignalSource::Raw { cob_id, extended: self.extended, start_bit: self.start_bit, bits: self.bits, raw_kind: self.raw_kind, big_endian: self.big_endian }
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

fn default_name(s: &SignalSource) -> String {
    match s {
        SignalSource::Pdo { node, index, sub } => format!("N{node} {index:04X}:{sub:02X}"),
        SignalSource::SdoPoll { node, index, sub, .. } => format!("N{node} {index:04X}:{sub:02X} (SDO)"),
        SignalSource::Raw { cob_id, start_bit, bits, .. } => format!("{cob_id:X} bits {start_bit}+{bits}"),
    }
}

fn source_text(s: &SignalSource) -> String {
    match s {
        SignalSource::Pdo { node, index, sub } => format!("PDO, node {node}, {index:04X}:{sub:02X}"),
        SignalSource::SdoPoll { node, index, sub, period_ms, .. } => format!("SDO, node {node}, {index:04X}:{sub:02X}, {period_ms} ms"),
        SignalSource::Raw { cob_id, extended, start_bit, bits, raw_kind, big_endian } => {
            let k = match raw_kind {
                RawKind::Unsigned => "unsigned",
                RawKind::Signed => "signed",
                RawKind::Float => "float",
            };
            format!("raw {}{cob_id:X}, bit {start_bit}, {bits} bit {k}{}", if *extended { "x" } else { "" }, if *big_endian { ", big endian" } else { "" })
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
        Ok(s.list().iter().map(|x| x.points().len()).sum())
    }

    pub(crate) fn graph_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        if !self.graph.started {
            self.graph.started = true;
            self.graph.follow = true;
        }
        let (now, polling_on) = self.bus.shared.lock().map(|s| (s.now_us as f64 / 1e6, s.signal_polling)).unwrap_or((0.0, false));
        self.graph.polling = polling_on;
        let has_polls = self.bus.signals.lock().map(|s| s.has_polls()).unwrap_or(false);
        // Toolbar.
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{} Add signal...", egui_phosphor::regular::PLUS)).clicked() {
                self.graph.add = Some(AddDraft { node: self.active_node(), ..AddDraft::default() });
            }
            if polling_on {
                if ui.button(format!("{} Stop polling", egui_phosphor::regular::STOP)).clicked() {
                    self.bus.send(BusCommand::SetSignalPolling(false));
                }
            } else if self.send_button(ui, has_polls, format!("{} Start polling", egui_phosphor::regular::PLAY)).on_hover_text("Read the SDO signals periodically").clicked() {
                self.bus.send(BusCommand::SetSignalPolling(true));
            }
            let paused = self.graph.paused_at.is_some();
            if ui.selectable_label(paused, format!("{} Pause", egui_phosphor::regular::PAUSE)).on_hover_text("Freeze the picture; the signals keep recording").clicked() {
                self.graph.paused_at = if paused { None } else { Some(now) };
            }
            ui.checkbox(&mut self.graph.follow, "Follow");
            ui.label("Window");
            if ui.add(egui::DragValue::new(&mut self.settings.graph_window_s).range(1.0..=3600.0).speed(0.5).suffix(" s")).changed() {
                self.mark_settings();
            }
            if ui.button("Clear").on_hover_text("Delete the recorded points").clicked() {
                if let Ok(mut s) = self.bus.signals.lock() {
                    s.clear();
                }
            }
            if ui.button("Export CSV...").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_file_name("signals.csv").add_filter("CSV", &["csv"]).save_file() {
                    match self.write_graph_csv(&path) {
                        Ok(n) => self.set_status(format!("Saved {n} points to {}", path.display()), false),
                        Err(e) => self.set_status(format!("CSV not saved: {e}"), true),
                    }
                }
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
                .list()
                .iter()
                .map(|sig| {
                    let mut pts = Vec::new();
                    if !self.graph.hidden.contains(&sig.id) {
                        for m in s.slice(sig.id, t0, t_end, max_points) {
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

        if series.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::CHART_LINE).size(28.0).color(p.text_secondary));
                ui.label("No signals.");
                ui.label(egui::RichText::new("Press Add signal to plot a PDO object, an SDO poll or raw frame bits.").color(p.text_secondary));
            });
            return;
        }

        let legend_h = (series.len() as f32 + 1.0) * 24.0 + 12.0;
        let plot_h = (ui.available_height() - legend_h.min(220.0)).max(120.0);
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
        plot.show(ui, |pui| {
            if follow {
                pui.set_plot_bounds(PlotBounds::from_min_max([t0, ymin - pad], [t_end, ymax + pad]));
            }
            for (i, s) in series.iter().enumerate() {
                if self.graph.hidden.contains(&s.id) {
                    continue;
                }
                pui.line(Line::new(PlotPoints::from(s.pts.clone())).name(&s.name).color(theme::series_colour(p, i)).width(width));
            }
        });

        // Legend table.
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("graph_legend").show(ui, |ui| {
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

    pub(crate) fn graph_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.graph.add.take() else { return };
        let mut keep = true;
        let mut add = false;
        // Mapped objects of the node's PDOs (read configuration, else EDS).
        let mapped: Vec<(u16, u8, String)> = if d.kind == SourceKind::Pdo {
            let od = self.od_of_node(d.node);
            let cfgs = match self.decode_ctx.node_pdos(d.node) {
                Some(c) => c.to_vec(),
                None => od.as_ref().map(|o| configs_from_od(o, d.node)).unwrap_or_default(),
            };
            let mut v = Vec::new();
            for c in cfgs.iter().filter(|c| c.kind == crate::canopen::pdo::PdoKind::Tpdo) {
                for m in c.mapping.iter().filter(|m| !m.is_dummy()) {
                    let name = od.as_ref().and_then(|o| o.name_of(m.index, m.sub)).unwrap_or_default();
                    v.push((m.index, m.sub, format!("{} {:04X}:{:02X} {name}", c.label(), m.index, m.sub)));
                }
            }
            v
        } else {
            Vec::new()
        };
        let err_c = if self.is_dark() { theme::DARK.error } else { theme::LIGHT.error };
        egui::Modal::new(egui::Id::new("add_signal")).show(ctx, |ui| {
            ui.heading("Add signal");
            ui.horizontal(|ui| {
                for k in [SourceKind::Pdo, SourceKind::SdoPoll, SourceKind::Raw] {
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
                            let cur = u16::from_str_radix(d.index_text.trim(), 16).ok().and_then(|i| mapped.iter().find(|m| m.0 == i && m.1 == d.sub)).map(|m| m.2.clone()).unwrap_or_else(|| "(choose)".to_string());
                            egui::ComboBox::from_id_salt("sig_mapped").selected_text(cur).width(260.0).show_ui(ui, |ui| {
                                for (i, s, t) in &mapped {
                                    if ui.selectable_label(false, t).clicked() {
                                        d.index_text = format!("{i:04X}");
                                        d.sub = *s;
                                    }
                                }
                            });
                            ui.end_row();
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
                }
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("automatic"));
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
        if add {
            match d.build() {
                Ok(def) => {
                    self.add_signal(def);
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
