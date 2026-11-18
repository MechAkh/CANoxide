//! Object dictionary tab: EDS table with Read / Write per entry, Read all,
//! and a manual SDO box that works without an EDS.

use super::App;
use crate::canopen::sdo::{SdoOutcome, SdoRequest};
use crate::canopen::types::{check_limits, format_value, parse_int, parse_text, type_name, DataType, DisplayMode};
use crate::eds::{eval_int, OdEntry, ObjectDictionary};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) enum Pending {
    Row { index: u16, sub: u8, write: Option<Vec<u8>> },
    Manual { write: bool },
}

enum RowValue {
    Value(Vec<u8>),
    Error(String),
    Busy,
}

pub(crate) struct OdView {
    filter: String,
    expanded: HashSet<u16>,
    selected: Option<u16>,
    values: HashMap<(u16, u8), RowValue>,
    edits: HashMap<(u16, u8), String>,
    pub(crate) pending: HashMap<u64, Pending>,
    m_index: String,
    m_sub: String,
    m_type: Option<DataType>,
    m_value: String,
    m_result: String,
    m_error: bool,
    m_data: Option<Vec<u8>>,
}

impl Default for OdView {
    fn default() -> Self {
        OdView {
            filter: String::new(),
            expanded: HashSet::new(),
            selected: None,
            values: HashMap::new(),
            edits: HashMap::new(),
            pending: HashMap::new(),
            m_index: "1000".to_string(),
            m_sub: "0".to_string(),
            m_type: Some(DataType::Unsigned32),
            m_value: String::new(),
            m_result: String::new(),
            m_error: false,
            m_data: None,
        }
    }
}

impl OdView {
    /// Forget the read values (another device or node).
    pub(crate) fn reset_values(&mut self) {
        self.values.clear();
        self.edits.clear();
    }
}

#[derive(Clone, Copy)]
enum RowRef {
    /// Header row of an ARRAY / RECORD.
    Object(u16),
    Entry(u16, u8),
}

fn matches(od: &ObjectDictionary, index: u16, f: &str) -> bool {
    if f.is_empty() {
        return true;
    }
    let o = &od.objects[&index];
    format!("{index:04x}").contains(f)
        || format!("0x{index:04x}").contains(f)
        || o.name.to_lowercase().contains(f)
        || o.subs.values().any(|e| e.name.to_lowercase().contains(f))
}

/// Parse "0x1018", "1018" (hex) for an index.
fn parse_index(s: &str) -> Option<u16> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u16::from_str_radix(t, 16).ok()
}

fn parse_sub(s: &str) -> Option<u8> {
    parse_int(s.trim(), false).and_then(|v| u8::try_from(v).ok())
}

/// A limit text with `$NODEID` evaluated; other texts stay as they are.
fn limit_text(t: Option<&str>, node: u8) -> Option<String> {
    let t = t?;
    if t.to_ascii_uppercase().contains("$NODEID") {
        eval_int(t, node).map(|v| v.to_string())
    } else {
        Some(t.to_string())
    }
}

/// Text from the user to bytes, checked against type and limits.
fn entry_bytes(e: &OdEntry, text: &str, node: u8) -> Result<Vec<u8>, String> {
    let dt = e.dtype().unwrap_or(DataType::OctetString);
    let b = parse_text(dt, text, false).map_err(|x| x.to_string())?;
    let lo = limit_text(e.low.as_deref(), node);
    let hi = limit_text(e.high.as_deref(), node);
    check_limits(dt, &b, lo.as_deref(), hi.as_deref()).map_err(|x| x.to_string())?;
    Ok(b)
}

impl App {
    pub(crate) fn on_sdo_done(&mut self, tag: u64, o: SdoOutcome) {
        let Some(p) = self.od.pending.remove(&tag) else { return };
        let what = format!("node {} 0x{:04X}:{:02X}", o.node, o.index, o.sub);
        match p {
            Pending::Row { index, sub, write } => match o.result {
                Ok(d) => {
                    let v = write.clone().unwrap_or(d);
                    self.od.values.insert((index, sub), RowValue::Value(v));
                    let verb = if write.is_some() { "Written" } else { "Read" };
                    self.set_status(format!("{verb} {what}"), false);
                }
                Err(e) => {
                    let verb = if write.is_some() { "Write" } else { "Read" };
                    self.od.values.insert((index, sub), RowValue::Error(e.to_string()));
                    self.set_status(format!("{verb} {what}: {e}"), true);
                }
            },
            Pending::Manual { write } => match o.result {
                Ok(d) => {
                    self.od.m_error = false;
                    if write {
                        self.od.m_result = format!("Written {what}");
                        self.od.m_data = None;
                    } else {
                        self.od.m_result = format!("{what}: {} byte(s)", d.len());
                        self.od.m_data = Some(d);
                    }
                    let r = self.od.m_result.clone();
                    self.set_status(r, false);
                }
                Err(e) => {
                    self.od.m_error = true;
                    self.od.m_data = None;
                    self.od.m_result = format!("{what}: {e}");
                    let r = self.od.m_result.clone();
                    self.set_status(r, true);
                }
            },
        }
    }

    fn read_entry(&mut self, node: u8, index: u16, sub: u8) {
        self.od.values.insert((index, sub), RowValue::Busy);
        self.send_sdo(node, SdoRequest::Upload { index, sub }, Pending::Row { index, sub, write: None });
    }

    pub(crate) fn od_tab(&mut self, ui: &mut egui::Ui) {
        let node = self.active_node();
        let can_send = self.can_send();
        let od = match self.active_od() {
            Some(Ok(od)) => Some(od),
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
                None
            }
            None => None,
        };
        let dev = self.active_device().map(|d| d.name.clone());

        // Toolbar.
        ui.horizontal_wrapped(|ui| {
            match &dev {
                Some(n) => ui.strong(format!("{n}, node {node}")),
                None => ui.strong(format!("No active device, node {node}")),
            };
            ui.separator();
            ui.label("Filter");
            ui.add(egui::TextEdit::singleline(&mut self.od.filter).desired_width(140.0).hint_text("index or name"));
            ui.separator();
            ui.label("Values");
            let before = self.settings.value_display;
            for m in DisplayMode::ALL {
                ui.selectable_value(&mut self.settings.value_display, m, m.label());
            }
            if before != self.settings.value_display {
                self.mark_settings();
            }
            ui.separator();
            let sel = self.od.selected;
            let label = match sel {
                Some(i) => format!("Read all 0x{i:04X}"),
                None => "Read all (select an object)".to_string(),
            };
            if ui.add_enabled(can_send && sel.is_some() && od.is_some(), egui::Button::new(label)).clicked() {
                if let (Some(i), Some(od)) = (sel, &od) {
                    self.read_all(od, node, i);
                }
            }
            if ui.button("Expand all").clicked() {
                if let Some(od) = &od {
                    self.od.expanded = od.objects.keys().copied().collect();
                }
            }
            if ui.button("Collapse all").clicked() {
                self.od.expanded.clear();
            }
            if !can_send {
                ui.colored_label(ui.visuals().warn_fg_color, if self.listen_only_active { "listen only: SDO disabled" } else { "not connected" });
            }
        });

        self.manual_box(ui, node, can_send, od.as_ref());
        ui.separator();

        let Some(od) = od else {
            ui.label("Select a device with an EDS file in the left panel to see its object dictionary. The manual SDO box above works without an EDS.");
            return;
        };
        self.od_table(ui, &od, node, can_send);
    }

    fn read_all(&mut self, od: &Arc<ObjectDictionary>, node: u8, index: u16) {
        let Some(o) = od.objects.get(&index) else { return };
        let subs: Vec<u8> = o.subs.values().filter(|e| e.access.readable()).map(|e| e.sub).collect();
        for s in &subs {
            self.read_entry(node, index, *s);
        }
        self.set_status(format!("Reading {} sub-object(s) of 0x{index:04X}", subs.len()), false);
    }

    fn manual_box(&mut self, ui: &mut egui::Ui, node: u8, can_send: bool, od: Option<&Arc<ObjectDictionary>>) {
        egui::CollapsingHeader::new("Manual SDO").default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Node {node}"));
                ui.label("Index 0x");
                ui.add(egui::TextEdit::singleline(&mut self.od.m_index).desired_width(50.0));
                ui.label("Sub");
                ui.add(egui::TextEdit::singleline(&mut self.od.m_sub).desired_width(36.0));
                ui.label("Type");
                let cur = self.od.m_type.map(|t| t.name().to_string()).unwrap_or_else(|| "raw bytes".to_string());
                egui::ComboBox::from_id_salt("m_type").selected_text(cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.od.m_type, None, "raw bytes");
                    for t in DataType::ALL {
                        ui.selectable_value(&mut self.od.m_type, Some(t), t.name());
                    }
                });
                if let (Some(od), Some(i), Some(s)) = (od, parse_index(&self.od.m_index), parse_sub(&self.od.m_sub)) {
                    if let Some(e) = od.entry(i, s) {
                        if ui.small_button("Type from EDS").on_hover_text(e.name.clone()).clicked() {
                            self.od.m_type = e.dtype();
                        }
                    }
                }
                ui.label("Value");
                ui.add(egui::TextEdit::singleline(&mut self.od.m_value).desired_width(140.0).hint_text("dec, 0x hex, text"));
                let ok = parse_index(&self.od.m_index).zip(parse_sub(&self.od.m_sub));
                if ui.add_enabled(can_send && ok.is_some(), egui::Button::new("Read")).clicked() {
                    if let Some((index, sub)) = ok {
                        self.od.m_result = "reading...".to_string();
                        self.od.m_error = false;
                        self.send_sdo(node, SdoRequest::Upload { index, sub }, Pending::Manual { write: false });
                    }
                }
                if ui.add_enabled(can_send && ok.is_some(), egui::Button::new("Write")).clicked() {
                    if let Some((index, sub)) = ok {
                        let dt = self.od.m_type.unwrap_or(DataType::OctetString);
                        match parse_text(dt, &self.od.m_value, false) {
                            Ok(data) => {
                                self.od.m_result = "writing...".to_string();
                                self.od.m_error = false;
                                self.send_sdo(node, SdoRequest::Download { index, sub, data }, Pending::Manual { write: true });
                            }
                            Err(e) => {
                                self.od.m_result = format!("Not sent: {e}");
                                self.od.m_error = true;
                            }
                        }
                    }
                }
                if ok.is_none() {
                    ui.colored_label(ui.visuals().warn_fg_color, "index: hex 0..FFFF, sub: 0..255");
                }
            });
            if !self.od.m_result.is_empty() {
                let c = if self.od.m_error { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(c, &self.od.m_result);
                    if let Some(d) = &self.od.m_data {
                        let v = format_value(self.od.m_type, d, self.settings.value_display);
                        ui.label(egui::RichText::new(v).monospace());
                    }
                });
            }
        });
    }

    fn od_table(&mut self, ui: &mut egui::Ui, od: &Arc<ObjectDictionary>, node: u8, can_send: bool) {
        let f = self.od.filter.trim().to_lowercase();
        let mut rows: Vec<RowRef> = Vec::new();
        for (&index, o) in &od.objects {
            if !matches(od, index, &f) {
                continue;
            }
            if o.is_var() {
                if let Some(e) = o.subs.values().next() {
                    rows.push(RowRef::Entry(index, e.sub));
                }
            } else {
                rows.push(RowRef::Object(index));
                if self.od.expanded.contains(&index) || !f.is_empty() {
                    for &s in o.subs.keys() {
                        rows.push(RowRef::Entry(index, s));
                    }
                }
            }
        }
        let mode = self.settings.value_display;
        let row_h = ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body)) + 2.0;
        let mut reads: Vec<(u16, u8)> = Vec::new();
        let mut writes: Vec<(u16, u8)> = Vec::new();
        let mut read_all: Option<u16> = None;
        let mut toggle: Option<u16> = None;
        let mut select: Option<u16> = None;

        TableBuilder::new(ui)
            .id_salt("od_table")
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(70.0).at_least(50.0))
            .column(Column::initial(36.0).at_least(28.0))
            .column(Column::initial(220.0).at_least(80.0).clip(true))
            .column(Column::initial(100.0).at_least(50.0).clip(true))
            .column(Column::initial(44.0).at_least(36.0))
            .column(Column::initial(150.0).at_least(60.0).clip(true))
            .column(Column::initial(100.0).at_least(40.0).clip(true))
            .column(Column::initial(120.0).at_least(60.0))
            .column(Column::remainder().at_least(110.0))
            .min_scrolled_height(0.0)
            .header(row_h, |mut h| {
                for t in ["Index", "Sub", "Name", "Type", "Access", "Value", "Default", "New value", ""] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, rows.len(), |mut row| {
                    let r = rows[row.index()];
                    match r {
                        RowRef::Object(index) => {
                            let o = &od.objects[&index];
                            let open = self.od.expanded.contains(&index) || !f.is_empty();
                            let sel = self.od.selected == Some(index);
                            row.set_selected(sel);
                            row.col(|ui| {
                                let arrow = if open { "v" } else { ">" };
                                if ui.small_button(arrow).clicked() {
                                    toggle = Some(index);
                                }
                                if ui.selectable_label(sel, format!("{index:04X}")).clicked() {
                                    select = Some(index);
                                }
                            });
                            row.col(|ui| {
                                ui.label(format!("{}", o.subs.len()));
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(egui::RichText::new(&o.name).strong()).truncate());
                            });
                            row.col(|ui| {
                                ui.label(o.object_type.label());
                            });
                            row.col(|_| {});
                            row.col(|_| {});
                            row.col(|_| {});
                            row.col(|_| {});
                            row.col(|ui| {
                                if ui.add_enabled(can_send, egui::Button::new("Read all")).clicked() {
                                    read_all = Some(index);
                                }
                            });
                        }
                        RowRef::Entry(index, sub) => {
                            let o = &od.objects[&index];
                            let e = &o.subs[&sub];
                            let is_var = o.is_var();
                            row.set_selected(self.od.selected == Some(index));
                            row.col(|ui| {
                                if is_var {
                                    if ui.selectable_label(self.od.selected == Some(index), format!("{index:04X}")).clicked() {
                                        select = Some(index);
                                    }
                                } else {
                                    ui.weak(format!("{index:04X}"));
                                }
                            });
                            row.col(|ui| {
                                ui.label(format!("{sub:02X}"));
                            });
                            row.col(|ui| {
                                let name = if is_var { o.name.as_str() } else { e.name.as_str() };
                                ui.add(egui::Label::new(name).truncate()).on_hover_text(name);
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(type_name(e.data_type)).truncate());
                            });
                            row.col(|ui| {
                                ui.label(e.access.label());
                            });
                            row.col(|ui| match self.od.values.get(&(index, sub)) {
                                Some(RowValue::Value(b)) => {
                                    let t = format_value(e.dtype(), b, mode);
                                    ui.add(egui::Label::new(egui::RichText::new(&t).monospace()).truncate()).on_hover_text(t);
                                }
                                Some(RowValue::Error(t)) => {
                                    ui.add(egui::Label::new(egui::RichText::new(t).color(ui.visuals().error_fg_color)).truncate()).on_hover_text(t);
                                }
                                Some(RowValue::Busy) => {
                                    ui.weak("...");
                                }
                                None => {}
                            });
                            row.col(|ui| {
                                let d = e.effective_text().unwrap_or("");
                                ui.add(egui::Label::new(d).truncate()).on_hover_text(d);
                            });
                            row.col(|ui| {
                                if e.access.writable() {
                                    let buf = self.od.edits.entry((index, sub)).or_default();
                                    ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY).hint_text("dec / 0x hex"));
                                }
                            });
                            row.col(|ui| {
                                if ui.add_enabled(can_send && e.access.readable(), egui::Button::new("Read")).clicked() {
                                    reads.push((index, sub));
                                }
                                if ui.add_enabled(can_send && e.access.writable(), egui::Button::new("Write")).clicked() {
                                    writes.push((index, sub));
                                }
                            });
                        }
                    }
                });
            });

        if let Some(i) = toggle {
            if !self.od.expanded.remove(&i) {
                self.od.expanded.insert(i);
            }
        }
        if let Some(i) = select {
            self.od.selected = if self.od.selected == Some(i) { None } else { Some(i) };
        }
        if let Some(i) = read_all {
            self.od.selected = Some(i);
            self.read_all(od, node, i);
        }
        for (i, s) in reads {
            self.read_entry(node, i, s);
        }
        for (i, s) in writes {
            let Some(e) = od.entry(i, s) else { continue };
            let text = self.od.edits.get(&(i, s)).cloned().unwrap_or_default();
            match entry_bytes(e, &text, node) {
                Ok(data) => {
                    self.od.values.insert((i, s), RowValue::Busy);
                    self.send_sdo(node, SdoRequest::Download { index: i, sub: s, data: data.clone() }, Pending::Row { index: i, sub: s, write: Some(data) });
                }
                Err(err) => {
                    self.od.values.insert((i, s), RowValue::Error(format!("not sent: {err}")));
                    self.set_status(format!("0x{i:04X}:{s:02X} not written: {err}"), true);
                }
            }
        }
    }
}
